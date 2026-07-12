use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::Draft;
use serde_json::{Map, Value};

use crate::context::CompileContext;

use super::super::{
    compile_schema,
    draft::DraftExt,
    helpers::{
        dynamic_ref_anchor_name, get_or_create_is_valid_fn_with, get_or_create_item_eval_fn,
        get_or_create_key_eval_fn,
    },
    refs::{resolve_ref, ResolvedRef},
    CompiledExpr,
};

/// Break a self-referential eval-tracking cycle under an unevaluated keyword: a re-entry on
/// the same (helper fn pointer, instance pointer) evaluates nothing, mirroring the engine's marking.
fn cycle_guarded_dispatch(call: &TokenStream) -> TokenStream {
    quote! {
        {
            let __mark = (target as usize, std::ptr::from_ref(instance) as usize);
            if __JSONSCHEMA_EVAL_MARK.with(|__marks| __marks.borrow().contains(&__mark)) {
                false
            } else {
                __JSONSCHEMA_EVAL_MARK.with(|__marks| __marks.borrow_mut().push(__mark));
                let __evaluated = #call;
                __JSONSCHEMA_EVAL_MARK.with(|__marks| __marks.borrow_mut().pop());
                __evaluated
            }
        }
    }
}

/// Branch validity guard for unevaluated* evaluation tracking. Non-trivial subschemas become
/// shared helper fns keyed by (base URI, draft, schema content) so guards compose from further
/// guard calls instead of re-inlining subtrees (which grows code quadratically with nesting depth).
fn compile_branch_guard(ctx: &mut CompileContext<'_>, subschema: &Value) -> CompiledExpr {
    let Value::Object(obj) = subschema else {
        return compile_schema(ctx, subschema);
    };
    if obj.len() <= 1 && !has_composable_applicators(ctx, obj) {
        return compile_schema(ctx, subschema);
    }
    let location = format!(
        "json-schema-guard:{}|{:?}|{}",
        ctx.current_base_uri,
        ctx.draft,
        serde_json::to_string(subschema).expect("schema is valid JSON"),
    );
    let func_name = get_or_create_is_valid_fn_with(
        ctx,
        &location,
        subschema,
        ctx.current_base_uri.clone(),
        compile_guard_validity,
    );
    let func_ident = format_ident!("{}", func_name);
    CompiledExpr::from_bool_expr(quote! { #func_ident(instance) }, "")
}

fn has_composable_applicators(ctx: &CompileContext<'_>, obj: &Map<String, Value>) -> bool {
    obj.contains_key("allOf")
        || obj.contains_key("anyOf")
        || obj.contains_key("oneOf")
        || obj.contains_key("not")
        || (ctx.draft.supports_if_then_else_keyword() && obj.contains_key("if"))
}

/// Validity-only compilation that spells applicator branches as guard-helper
/// calls instead of inlined subtrees.
fn compile_guard_validity(ctx: &mut CompileContext<'_>, schema: &Value) -> CompiledExpr {
    let obj = schema
        .as_object()
        .expect("branch guards only compile object subschemas");
    // unevaluated* semantics depend on sibling applicator annotations, so the
    // keywords cannot be compiled separately from each other.
    if obj.contains_key("unevaluatedProperties") || obj.contains_key("unevaluatedItems") {
        return compile_schema(ctx, schema);
    }
    if !has_composable_applicators(ctx, obj) {
        return compile_schema(ctx, schema);
    }

    let mut rest = obj.clone();
    for key in ["allOf", "anyOf", "oneOf", "not", "if", "then", "else"] {
        rest.remove(key);
    }

    let mut result = if rest.is_empty() {
        CompiledExpr::always_true()
    } else {
        compile_schema(ctx, &Value::Object(rest))
    };

    if let Some(all_of) = obj.get("allOf").and_then(Value::as_array) {
        for branch in all_of {
            result = result.and(compile_branch_guard(ctx, branch));
        }
    }
    if let Some(any_of) = obj.get("anyOf").and_then(Value::as_array) {
        let calls: Vec<_> = any_of
            .iter()
            .map(|branch| compile_branch_guard(ctx, branch).is_valid_token_stream())
            .collect();
        result = result.and(CompiledExpr::from_bool_expr(
            quote! { (#(( #calls ))||*) },
            "",
        ));
    }
    if let Some(one_of) = obj.get("oneOf").and_then(Value::as_array) {
        let calls: Vec<_> = one_of
            .iter()
            .map(|branch| compile_branch_guard(ctx, branch).is_valid_token_stream())
            .collect();
        result = result.and(CompiledExpr::from_bool_expr(
            quote! {
                {
                    let mut __guard_matches = 0usize;
                    #(if #calls { __guard_matches += 1; })*
                    __guard_matches == 1
                }
            },
            "",
        ));
    }
    if let Some(not_schema) = obj.get("not") {
        let call = compile_branch_guard(ctx, not_schema).is_valid_token_stream();
        result = result.and(CompiledExpr::from_bool_expr(quote! { !(#call) }, ""));
    }
    if let Some(if_schema) = obj.get("if") {
        let if_call = compile_branch_guard(ctx, if_schema).is_valid_token_stream();
        let then_call = obj.get("then").map_or(quote! { true }, |s| {
            compile_branch_guard(ctx, s).is_valid_token_stream()
        });
        let else_call = obj.get("else").map_or(quote! { true }, |s| {
            compile_branch_guard(ctx, s).is_valid_token_stream()
        });
        result = result.and(CompiledExpr::from_bool_expr(
            quote! { if #if_call { #then_call } else { #else_call } },
            "",
        ));
    }
    result
}

pub(crate) struct GuardHoist {
    bindings: Vec<TokenStream>,
    enabled: bool,
    guard_count: usize,
    match_count: usize,
}

impl GuardHoist {
    pub(crate) fn hoisting() -> Self {
        Self {
            bindings: Vec::new(),
            enabled: true,
            guard_count: 0,
            match_count: 0,
        }
    }

    pub(crate) fn inline() -> Self {
        Self {
            bindings: Vec::new(),
            enabled: false,
            guard_count: 0,
            match_count: 0,
        }
    }

    pub(crate) fn bindings(&self) -> &[TokenStream] {
        &self.bindings
    }

    fn guard(&mut self, value_ty: &TokenStream, valid: &CompiledExpr) -> TokenStream {
        if valid.is_trivially_true() {
            return quote! { true };
        }
        let call = quote! { (|instance: &#value_ty| #valid)(instance) };
        if !self.enabled {
            return call;
        }
        let ident = format_ident!("__guard_{}", self.guard_count);
        self.guard_count += 1;
        self.bindings.push(quote! { let #ident = #call; });
        quote! { #ident }
    }

    fn hoist_match_count(&mut self, expr: &TokenStream) -> proc_macro2::Ident {
        let ident = format_ident!("__one_of_matches_{}", self.match_count);
        self.match_count += 1;
        self.bindings.push(quote! { let #ident = #expr; });
        ident
    }
}

fn compile_guarded_eval(
    value_ty: &TokenStream,
    valid_expr: &CompiledExpr,
    eval_expr: &TokenStream,
    hoist: &mut GuardHoist,
) -> TokenStream {
    let guard = hoist.guard(value_ty, valid_expr);
    quote! {
        #guard && (#eval_expr)
    }
}

fn compile_one_of_evaluated(
    value_ty: &TokenStream,
    cases: &[(CompiledExpr, TokenStream)],
    hoist: &mut GuardHoist,
) -> Option<TokenStream> {
    if cases.is_empty() {
        return None;
    }

    if hoist.enabled {
        let guards: Vec<_> = cases
            .iter()
            .map(|(valid, _)| hoist.guard(value_ty, valid))
            .collect();
        let count_terms = guards.iter().map(|guard| quote! { (#guard as usize) });
        let matches_ident = hoist.hoist_match_count(&quote! { #(#count_terms)+* });
        let selectors = guards
            .iter()
            .zip(cases)
            .map(|(guard, (_, eval))| quote! { (#guard && (#eval)) });
        return Some(quote! {
            #matches_ident == 1 && (#(#selectors)||*)
        });
    }

    let valid_exprs: Vec<_> = cases.iter().map(|(valid, _)| valid).collect();
    let eval_exprs: Vec<_> = cases.iter().map(|(_, eval)| eval).collect();

    Some(quote! {
        {
            let mut __one_of_matches = 0usize;
            let mut __one_of_evaluates = false;
            #(
                if (|instance: &#value_ty| #valid_exprs)(instance) {
                    __one_of_matches += 1;
                    __one_of_evaluates = __one_of_evaluates || (#eval_exprs);
                }
            )*
            __one_of_matches == 1 && __one_of_evaluates
        }
    })
}

fn compile_if_then_else_evaluated(
    value_ty: &TokenStream,
    if_valid_expr: &CompiledExpr,
    if_eval_expr: &TokenStream,
    then_eval_expr: &TokenStream,
    else_eval_expr: &TokenStream,
    hoist: &mut GuardHoist,
) -> TokenStream {
    let guard = hoist.guard(value_ty, if_valid_expr);
    quote! {
        if #guard {
            (#if_eval_expr) || (#then_eval_expr)
        } else {
            #else_eval_expr
        }
    }
}

fn compile_pattern_coverage_for_key(
    ctx: &mut CompileContext<'_>,
    patterns: Option<&Value>,
) -> Option<TokenStream> {
    let coverage = match super::pattern_coverage::build_pattern_coverage(ctx, patterns) {
        Ok(coverage) => coverage,
        Err(error_expr) => return Some(error_expr.into_token_stream()),
    };
    let combined = coverage.combined_check()?;
    let statics = coverage.statics;
    Some(if statics.is_empty() {
        combined
    } else {
        quote! { { #(#statics)* (#combined) } }
    })
}

/// Which unevaluated dimension an eval expression is being compiled for. The
/// property (`unevaluatedProperties`) and item (`unevaluatedItems`) passes share
/// applicator and reference dispatch, differing only in the emitted call shape.
#[derive(Clone, Copy)]
enum EvalKind {
    Key,
    Item,
}

impl EvalKind {
    /// Arguments passed to an eval helper fn call.
    fn call_args(self) -> TokenStream {
        match self {
            EvalKind::Key => quote! { instance, obj, key_str },
            EvalKind::Item => quote! { instance, arr, idx, item },
        }
    }

    fn recursive_stack(self) -> proc_macro2::Ident {
        match self {
            EvalKind::Key => format_ident!("__JSONSCHEMA_RECURSIVE_KEY_EVAL_STACK"),
            EvalKind::Item => format_ident!("__JSONSCHEMA_RECURSIVE_ITEM_EVAL_STACK"),
        }
    }

    fn dynamic_stack(self) -> proc_macro2::Ident {
        match self {
            EvalKind::Key => format_ident!("__JSONSCHEMA_DYNAMIC_KEY_EVAL_STACK"),
            EvalKind::Item => format_ident!("__JSONSCHEMA_DYNAMIC_ITEM_EVAL_STACK"),
        }
    }

    fn is_compiling(self, ctx: &CompileContext<'_>, location: &str) -> bool {
        match self {
            EvalKind::Key => ctx.key_eval_fns.is_compiling(location),
            EvalKind::Item => ctx.item_eval_fns.is_compiling(location),
        }
    }

    fn create_eval_fn(
        self,
        ctx: &mut CompileContext<'_>,
        resolved: &ResolvedRef,
    ) -> proc_macro2::Ident {
        let name = match self {
            EvalKind::Key => get_or_create_key_eval_fn(
                ctx,
                &resolved.location,
                &resolved.schema,
                resolved.base_uri.clone(),
            ),
            EvalKind::Item => get_or_create_item_eval_fn(
                ctx,
                &resolved.location,
                &resolved.schema,
                resolved.base_uri.clone(),
            ),
        };
        format_ident!("{}", name)
    }
}

/// Recurse into a subschema for the same evaluation dimension.
fn recurse_eval(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
    kind: EvalKind,
    hoist: &mut GuardHoist,
) -> TokenStream {
    match kind {
        EvalKind::Key => compile_key_evaluated_expr(ctx, schema, true),
        EvalKind::Item => compile_index_evaluated_expr(ctx, schema, hoist),
    }
}

/// Push the eval expressions contributed by the `allOf`/`anyOf`/`oneOf`/
/// `if`-`then`-`else` applicators. Callers are already inside an
/// applicator-vocabulary guard.
fn push_applicator_eval(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
    parts: &mut Vec<TokenStream>,
    value_ty: &TokenStream,
    kind: EvalKind,
    hoist: &mut GuardHoist,
) {
    if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
        for subschema in all_of {
            if let Value::Object(subschema_obj) = subschema {
                parts.push(recurse_eval(ctx, subschema_obj, kind, hoist));
            }
        }
    }
    if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array) {
        for subschema in any_of {
            if let Value::Object(subschema_obj) = subschema {
                let sub_eval = recurse_eval(ctx, subschema_obj, kind, hoist);
                let sub_valid = compile_branch_guard(ctx, subschema);
                parts.push(compile_guarded_eval(value_ty, &sub_valid, &sub_eval, hoist));
            }
        }
    }
    if let Some(one_of) = schema.get("oneOf").and_then(Value::as_array) {
        let cases: Vec<_> = one_of
            .iter()
            .filter_map(|subschema| {
                let Value::Object(subschema_obj) = subschema else {
                    return None;
                };
                let sub_eval = recurse_eval(ctx, subschema_obj, kind, hoist);
                let sub_valid = compile_branch_guard(ctx, subschema);
                Some((sub_valid, sub_eval))
            })
            .collect();
        if let Some(one_of_eval) = compile_one_of_evaluated(value_ty, &cases, hoist) {
            parts.push(one_of_eval);
        }
    }
    if let Some(if_schema) = schema.get("if") {
        let if_valid = compile_branch_guard(ctx, if_schema);
        let if_eval = if let Value::Object(if_obj) = if_schema {
            recurse_eval(ctx, if_obj, kind, hoist)
        } else {
            quote! { false }
        };
        let then_eval = schema.get("then").and_then(Value::as_object).map_or_else(
            || quote! { false },
            |then_obj| recurse_eval(ctx, then_obj, kind, hoist),
        );
        let else_eval = schema.get("else").and_then(Value::as_object).map_or_else(
            || quote! { false },
            |else_obj| recurse_eval(ctx, else_obj, kind, hoist),
        );
        parts.push(compile_if_then_else_evaluated(
            value_ty, &if_valid, &if_eval, &then_eval, &else_eval, hoist,
        ));
    }
}

/// Push the eval expressions contributed by `$ref`, `$recursiveRef`, and
/// `$dynamicRef` for the given dimension.
fn push_ref_dispatch(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
    parts: &mut Vec<TokenStream>,
    kind: EvalKind,
) {
    let call_args = kind.call_args();
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        if let Ok(resolved) = resolve_ref(ctx, reference) {
            if !kind.is_compiling(ctx, &resolved.location) {
                let func_ident = kind.create_eval_fn(ctx, &resolved);
                parts.push(quote! { #func_ident(#call_args) });
            }
        }
    }
    if ctx.draft.supports_recursive_ref_keyword() {
        if let Some(reference) = schema.get("$recursiveRef").and_then(Value::as_str) {
            if let Ok(resolved) = resolve_ref(ctx, reference) {
                let target_has_recursive_anchor = resolved
                    .schema
                    .as_object()
                    .and_then(|obj| obj.get("$recursiveAnchor"))
                    .and_then(Value::as_bool)
                    == Some(true);
                let fallback = if kind.is_compiling(ctx, &resolved.location) {
                    quote! { false }
                } else {
                    let func_ident = kind.create_eval_fn(ctx, &resolved);
                    quote! { #func_ident(#call_args) }
                };
                if target_has_recursive_anchor {
                    // Ensure the recursive stack infrastructure is emitted even if
                    // `$recursiveRef` in the same schema hasn't been compiled yet.
                    ctx.uses_recursive_ref = true;
                    let stack = kind.recursive_stack();
                    let guarded = cycle_guarded_dispatch(&quote! { target(#call_args) });
                    parts.push(quote! {
                        {
                            let __recursive_target = #stack.with(|stack| {
                                let stack = stack.borrow();
                                let mut selected = None;
                                for (validate, is_anchor) in stack.iter() {
                                    if *is_anchor {
                                        selected = Some(*validate);
                                        break;
                                    }
                                }
                                selected
                            });
                            if let Some(target) = __recursive_target {
                                #guarded
                            } else {
                                #fallback
                            }
                        }
                    });
                } else {
                    parts.push(fallback);
                }
            }
        }
    }
    if ctx.draft.supports_dynamic_ref_keyword() {
        if let Some(reference) = schema.get("$dynamicRef").and_then(Value::as_str) {
            if let Ok(resolved) = resolve_ref(ctx, reference) {
                let fallback = if kind.is_compiling(ctx, &resolved.location) {
                    quote! { false }
                } else {
                    let func_ident = kind.create_eval_fn(ctx, &resolved);
                    quote! { #func_ident(#call_args) }
                };
                if let Some(anchor_name) = dynamic_ref_anchor_name(reference, &resolved.schema) {
                    ctx.uses_dynamic_ref = true;
                    let stack = kind.dynamic_stack();
                    let guarded = cycle_guarded_dispatch(&quote! { target(#call_args) });
                    parts.push(quote! {
                        {
                            let __dynamic_target = #stack.with(|stack| {
                                let stack = stack.borrow();
                                let mut selected = None;
                                for (dynamic_anchor, validate) in stack.iter().rev() {
                                    if *dynamic_anchor == #anchor_name {
                                        selected = Some(*validate);
                                    }
                                }
                                selected
                            });
                            if let Some(target) = __dynamic_target {
                                #guarded
                            } else {
                                #fallback
                            }
                        }
                    });
                } else {
                    parts.push(fallback);
                }
            }
        }
    }
}

pub(crate) fn compile_key_evaluated_expr(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
    include_own_unevaluated: bool,
) -> TokenStream {
    let mut parts = Vec::new();
    let applicator_vocab_enabled = ctx.supports_applicator_vocabulary();
    let value_ty = crate::codegen::emit_serde::value_ty();

    if applicator_vocab_enabled
        && schema
            .get("additionalProperties")
            .is_some_and(|additional| additional.as_bool() != Some(false))
    {
        return quote! { true };
    }

    let properties_obj = if applicator_vocab_enabled {
        schema.get("properties").and_then(Value::as_object)
    } else {
        None
    };
    if applicator_vocab_enabled {
        if let Some(properties) = properties_obj {
            if !properties.is_empty() {
                let names: Vec<&str> = properties.keys().map(String::as_str).collect();
                parts.push(quote! { matches!(key_str, #(#names)|*) });
            }
        }
    }

    if applicator_vocab_enabled {
        if let Some(pattern_expr) =
            compile_pattern_coverage_for_key(ctx, schema.get("patternProperties"))
        {
            parts.push(pattern_expr);
        }
    }

    if applicator_vocab_enabled && ctx.draft.supports_dependent_schemas_keyword() {
        if let Some(dependent_schemas) = schema.get("dependentSchemas").and_then(Value::as_object) {
            for (property, subschema) in dependent_schemas {
                if let Value::Object(subschema_obj) = subschema {
                    let sub_eval = compile_key_evaluated_expr(ctx, subschema_obj, true);
                    parts.push(quote! { obj.contains_key(#property) && (#sub_eval) });
                }
            }
        }
    }

    if applicator_vocab_enabled {
        let mut hoist = GuardHoist::inline();
        push_applicator_eval(
            ctx,
            schema,
            &mut parts,
            &value_ty,
            EvalKind::Key,
            &mut hoist,
        );
    }

    push_ref_dispatch(ctx, schema, &mut parts, EvalKind::Key);

    let evaluated_without_unevaluated = super::combine_or(parts);

    if include_own_unevaluated && ctx.supports_unevaluated_properties() {
        if let Some(unevaluated) = schema.get("unevaluatedProperties") {
            if unevaluated.as_bool() == Some(true) {
                return quote! { true };
            }
            if unevaluated.as_bool() != Some(false) {
                let schema_check = ctx.with_instance_scope(|ctx| compile_schema(ctx, unevaluated));
                return quote! {
                    (#evaluated_without_unevaluated) || {
                        obj.get(key_str).is_some_and(|instance| {
                            #schema_check
                        })
                    }
                };
            }
        }
    }

    evaluated_without_unevaluated
}

pub(crate) fn compile_index_evaluated_expr(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
    hoist: &mut GuardHoist,
) -> TokenStream {
    let mut parts = Vec::new();
    let applicator_vocab_enabled = ctx.supports_applicator_vocabulary();
    let value_ty = crate::codegen::emit_serde::value_ty();

    if applicator_vocab_enabled {
        if let Some(items_schema) = schema.get("items") {
            match (ctx.draft, items_schema) {
                (Draft::Draft202012 | Draft::Unknown, _) => {
                    parts.push(quote! { true });
                }
                (_, Value::Array(tuple)) => {
                    if schema.contains_key("additionalItems") {
                        parts.push(quote! { true });
                    } else {
                        let tuple_len = tuple.len();
                        parts.push(quote! { idx < #tuple_len });
                    }
                }
                _ => {
                    parts.push(quote! { true });
                }
            }
        }

        if ctx.draft.supports_prefix_items_keyword() {
            if let Some(prefix_items) = schema.get("prefixItems").and_then(Value::as_array) {
                let prefix_len = prefix_items.len();
                parts.push(quote! { idx < #prefix_len });
            }
        }

        if let Some(contains_schema) = schema.get("contains") {
            let contains_check = compile_branch_guard(ctx, contains_schema);
            parts.push(quote! {
                (|instance: &#value_ty| #contains_check)(item)
            });
        }

        push_applicator_eval(ctx, schema, &mut parts, &value_ty, EvalKind::Item, hoist);
    }

    push_ref_dispatch(ctx, schema, &mut parts, EvalKind::Item);

    let evaluated_without_unevaluated = super::combine_or(parts);

    if ctx.supports_unevaluated_items() {
        if let Some(unevaluated) = schema.get("unevaluatedItems") {
            if unevaluated.as_bool() == Some(true) {
                return quote! { true };
            }
            if unevaluated.as_bool() != Some(false) {
                let schema_check = ctx.with_instance_scope(|ctx| compile_schema(ctx, unevaluated));
                let value_ty = crate::codegen::emit_serde::value_ty();
                return quote! {
                    (#evaluated_without_unevaluated)
                        || (|instance: &#value_ty| #schema_check)(item)
                };
            }
        }
    }

    evaluated_without_unevaluated
}

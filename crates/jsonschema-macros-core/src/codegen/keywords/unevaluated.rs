use crate::codegen::emit::ValueEmitter;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::Draft;
use serde_json::{Map, Value};

use crate::context::CompileContext;

use super::super::{
    compile_schema,
    draft::DraftExt,
    helpers::{
        collect_dynamic_anchor_bindings, dynamic_ref_anchor_name, get_or_create_is_valid_fn_with,
        get_or_create_item_eval_fn, get_or_create_key_eval_fn,
    },
    object_schema::resource_base_uri,
    refs::{resolve_ref, ResolvedRef},
    stack_emit::{StackFamily, ITEM_EVAL_FAMILY, KEY_EVAL_FAMILY},
    CompiledExpr,
};

/// Break a self-referential eval-tracking cycle under an unevaluated keyword: a re-entry on
/// the same (helper fn pointer, instance pointer) evaluates nothing, as the runtime's marking does.
fn cycle_guarded_dispatch(node_address: &TokenStream, call: &TokenStream) -> TokenStream {
    quote! {
        {
            let __mark = (target as usize, #node_address);
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
fn compile_branch_guard<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    subschema: &Value,
) -> CompiledExpr {
    let err_instance = E::err_instance(format_ident!("instance"));
    let Value::Object(obj) = subschema else {
        return compile_schema(ctx, subschema);
    };
    if obj.len() <= 1 && !has_composable_applicators(ctx, obj) {
        return compile_schema(ctx, subschema);
    }
    let base_uri = resource_base_uri(ctx, obj);
    let location = format!(
        "json-schema-guard:{}|{:?}|{}",
        base_uri,
        ctx.draft,
        serde_json::to_string(subschema).expect("schema is valid JSON"),
    );
    let func_name = get_or_create_is_valid_fn_with(
        ctx,
        &location,
        0,
        subschema,
        base_uri,
        ctx.dialect_of(subschema),
        compile_guard_validity,
    );
    let func_ident = format_ident!("{}", func_name);
    CompiledExpr::from_bool_expr(quote! { #func_ident(instance) }, "", &err_instance)
}

fn has_composable_applicators<E: ValueEmitter>(
    ctx: &CompileContext<'_, E>,
    obj: &Map<String, Value>,
) -> bool {
    obj.contains_key("allOf")
        || obj.contains_key("anyOf")
        || obj.contains_key("oneOf")
        || obj.contains_key("not")
        || (ctx.draft.supports_if_then_else_keyword() && obj.contains_key("if"))
}

/// Validity-only compilation that emits applicator branches as guard-helper
/// calls instead of inlined subtrees.
fn compile_guard_validity<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    schema: &Value,
) -> CompiledExpr {
    let err_instance = E::err_instance(format_ident!("instance"));
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
            &err_instance,
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
            &err_instance,
        ));
    }
    if let Some(not_schema) = obj.get("not") {
        let call = compile_branch_guard(ctx, not_schema).is_valid_token_stream();
        result = result.and(CompiledExpr::from_bool_expr(
            quote! { !(#call) },
            "",
            &err_instance,
        ));
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
            &err_instance,
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

    fn guard(&mut self, node: &TokenStream, valid: &CompiledExpr) -> TokenStream {
        if valid.is_trivially_true() {
            return quote! { true };
        }
        let call = quote! { (|instance: #node| #valid)(instance) };
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

/// What the schema with an `unevaluated*` keyword reports as evaluated: a subschema it applies in
/// place that fails evaluates nothing. Deeper in-place subschemas pass whenever these pass, so only
/// the schema's own are checked, each at most once per instance and only for a member it evaluates.
#[derive(Default)]
pub(crate) struct ReportGuards {
    cells: Vec<TokenStream>,
}

impl ReportGuards {
    /// Declarations to emit ahead of the loop over the instance's members.
    pub(crate) fn cells(&self) -> &[TokenStream] {
        &self.cells
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    fn guard(&mut self, valid: &CompiledExpr, eval: TokenStream) -> TokenStream {
        if valid.is_trivially_true() {
            return eval;
        }
        let cell = format_ident!("__report_guard_{}", self.cells.len());
        let valid = valid.is_valid_token_stream();
        self.cells
            .push(quote! { let mut #cell: Option<bool> = None; });
        quote! {
            (#eval) && match #cell {
                Some(valid) => valid,
                None => {
                    let valid = #valid;
                    #cell = Some(valid);
                    valid
                }
            }
        }
    }
}

/// `eval`, counted only where `check` validates when reporting.
fn report_guarded<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    report: &mut Option<&mut ReportGuards>,
    eval: TokenStream,
    check: impl FnOnce(&mut CompileContext<'_, E>) -> CompiledExpr,
) -> TokenStream {
    match report {
        Some(report) => {
            let valid = check(ctx);
            report.guard(&valid, eval)
        }
        None => eval,
    }
}

fn compile_guarded_eval(
    node: &TokenStream,
    valid_expr: &CompiledExpr,
    eval_expr: &TokenStream,
    hoist: &mut GuardHoist,
) -> TokenStream {
    let guard = hoist.guard(node, valid_expr);
    quote! {
        #guard && (#eval_expr)
    }
}

fn compile_one_of_evaluated(
    node: &TokenStream,
    cases: &[(CompiledExpr, TokenStream)],
    hoist: &mut GuardHoist,
) -> Option<TokenStream> {
    if cases.is_empty() {
        return None;
    }

    if hoist.enabled {
        let guards: Vec<_> = cases
            .iter()
            .map(|(valid, _)| hoist.guard(node, valid))
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
                if (|instance: #node| #valid_exprs)(instance) {
                    __one_of_matches += 1;
                    __one_of_evaluates = __one_of_evaluates || (#eval_exprs);
                }
            )*
            __one_of_matches == 1 && __one_of_evaluates
        }
    })
}

fn compile_if_then_else_evaluated(
    node: &TokenStream,
    if_valid_expr: &CompiledExpr,
    if_eval_expr: &TokenStream,
    then_eval_expr: &TokenStream,
    else_eval_expr: &TokenStream,
    hoist: &mut GuardHoist,
) -> TokenStream {
    let guard = hoist.guard(node, if_valid_expr);
    quote! {
        if #guard {
            (#if_eval_expr) || (#then_eval_expr)
        } else {
            #else_eval_expr
        }
    }
}

fn compile_pattern_coverage_for_key<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    patterns: Option<&Value>,
    located: bool,
) -> Option<TokenStream> {
    let coverage = match super::pattern_coverage::build_pattern_coverage(ctx, patterns, located) {
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

    fn family(self) -> &'static StackFamily {
        match self {
            EvalKind::Key => &KEY_EVAL_FAMILY,
            EvalKind::Item => &ITEM_EVAL_FAMILY,
        }
    }

    fn is_compiling<E: ValueEmitter>(self, ctx: &CompileContext<'_, E>, location: &str) -> bool {
        match self {
            EvalKind::Key => ctx.key_eval_fns.is_compiling(location),
            EvalKind::Item => ctx.item_eval_fns.is_compiling(location),
        }
    }

    fn create_eval_fn<E: ValueEmitter>(
        self,
        ctx: &mut CompileContext<'_, E>,
        resolved: &ResolvedRef,
    ) -> proc_macro2::Ident {
        let name = match self {
            EvalKind::Key => get_or_create_key_eval_fn(
                ctx,
                &resolved.location,
                resolved.schema,
                resolved.base_uri.clone(),
                resolved.dialect.clone(),
            ),
            EvalKind::Item => get_or_create_item_eval_fn(
                ctx,
                &resolved.location,
                resolved.schema,
                resolved.base_uri.clone(),
                resolved.dialect.clone(),
            ),
        };
        format_ident!("{}", name)
    }
}

/// The code `f` gives in the resource `schema` is in. A subschema with an `$id` is a resource of
/// its own, which that code enters into the dynamic scope of the `kind` stacks.
fn in_resource_of<'cfg, E: ValueEmitter>(
    ctx: &mut CompileContext<'cfg, E>,
    schema: &Map<String, Value>,
    kind: EvalKind,
    f: impl FnOnce(&mut CompileContext<'cfg, E>) -> TokenStream,
) -> TokenStream {
    let base_uri = resource_base_uri(ctx, schema);
    let enters = base_uri.as_str() != ctx.current_base_uri.as_str();
    let evaluated = ctx.with_base_uri_scope(base_uri.clone(), f);
    if !enters || !ctx.uses_dynamic_ref {
        return evaluated;
    }
    let bindings = collect_dynamic_anchor_bindings(ctx, base_uri);
    let family = kind.family();
    let pushes = family.dynamic_pushes(&bindings);
    let pop = (family.pop_dynamic_n)(bindings.len());
    quote! {
        {
            #(#pushes)*
            let __evaluated = #evaluated;
            #pop
            __evaluated
        }
    }
}

/// Recurse into a subschema for the same evaluation dimension.
fn recurse_eval<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    schema: &Map<String, Value>,
    kind: EvalKind,
    hoist: &mut GuardHoist,
) -> TokenStream {
    in_resource_of(ctx, schema, kind, |ctx| match kind {
        EvalKind::Key => compile_key_evaluated_expr(ctx, schema, true, false, None),
        EvalKind::Item => compile_index_evaluated_expr(ctx, schema, hoist, None),
    })
}

/// Push the eval expressions contributed by the `allOf`/`anyOf`/`oneOf`/
/// `if`-`then`-`else` applicators. Callers are already inside an
/// applicator-vocabulary guard.
fn push_applicator_eval<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    schema: &Map<String, Value>,
    parts: &mut Vec<TokenStream>,
    node: &TokenStream,
    kind: EvalKind,
    hoist: &mut GuardHoist,
    report: &mut Option<&mut ReportGuards>,
) {
    if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
        for subschema in all_of {
            if let Value::Object(subschema_obj) = subschema {
                let sub_eval = recurse_eval(ctx, subschema_obj, kind, hoist);
                parts.push(report_guarded(ctx, report, sub_eval, |ctx| {
                    compile_branch_guard(ctx, subschema)
                }));
            }
        }
    }
    if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array) {
        for subschema in any_of {
            if let Value::Object(subschema_obj) = subschema {
                let sub_eval = recurse_eval(ctx, subschema_obj, kind, hoist);
                let sub_valid = compile_branch_guard(ctx, subschema);
                parts.push(compile_guarded_eval(node, &sub_valid, &sub_eval, hoist));
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
        if let Some(one_of_eval) = compile_one_of_evaluated(node, &cases, hoist) {
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
        let mut branch_eval = |ctx: &mut CompileContext<'_, E>, keyword: &str| {
            let Some(branch @ Value::Object(branch_obj)) = schema.get(keyword) else {
                return quote! { false };
            };
            let eval = recurse_eval(ctx, branch_obj, kind, hoist);
            report_guarded(ctx, report, eval, |ctx| compile_branch_guard(ctx, branch))
        };
        let then_eval = branch_eval(ctx, "then");
        let else_eval = branch_eval(ctx, "else");
        parts.push(compile_if_then_else_evaluated(
            node, &if_valid, &if_eval, &then_eval, &else_eval, hoist,
        ));
    }
}

/// Push the eval expressions contributed by `$ref`, `$recursiveRef`, and
/// `$dynamicRef` for the given dimension.
fn push_ref_dispatch<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    schema: &Map<String, Value>,
    parts: &mut Vec<TokenStream>,
    kind: EvalKind,
    report: &mut Option<&mut ReportGuards>,
) {
    let call_args = kind.call_args();
    if let Some(value @ Value::String(reference)) = schema.get("$ref") {
        if let Ok(resolved) = resolve_ref(ctx, reference) {
            if !kind.is_compiling(ctx, &resolved.location) {
                let func_ident = kind.create_eval_fn(ctx, &resolved);
                parts.push(report_guarded(
                    ctx,
                    report,
                    quote! { #func_ident(#call_args) },
                    |ctx| super::ref_::compile(ctx, value),
                ));
            }
        }
    }
    if ctx.draft.supports_recursive_ref_keyword() {
        if let Some(value @ Value::String(reference)) = schema.get("$recursiveRef") {
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
                    ctx.looked_up_stacks.insert(stack.to_string());
                    let guarded = cycle_guarded_dispatch(
                        &E::node_address(format_ident!("instance")),
                        &quote! { target(#call_args) },
                    );
                    let eval = quote! {
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
                    };
                    parts.push(report_guarded(ctx, report, eval, |ctx| {
                        super::ref_::compile_recursive(ctx, value)
                    }));
                } else {
                    parts.push(report_guarded(ctx, report, fallback, |ctx| {
                        super::ref_::compile_recursive(ctx, value)
                    }));
                }
            }
        }
    }
    if ctx.draft.supports_dynamic_ref_keyword() {
        if let Some(value @ Value::String(reference)) = schema.get("$dynamicRef") {
            if let Ok(resolved) = resolve_ref(ctx, reference) {
                let fallback = if kind.is_compiling(ctx, &resolved.location) {
                    quote! { false }
                } else {
                    let func_ident = kind.create_eval_fn(ctx, &resolved);
                    quote! { #func_ident(#call_args) }
                };
                if let Some(anchor_name) = dynamic_ref_anchor_name(reference, resolved.schema) {
                    ctx.uses_dynamic_ref = true;
                    let stack = kind.dynamic_stack();
                    ctx.looked_up_stacks.insert(stack.to_string());
                    let guarded = cycle_guarded_dispatch(
                        &E::node_address(format_ident!("instance")),
                        &quote! { target(#call_args) },
                    );
                    let eval = quote! {
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
                    };
                    parts.push(report_guarded(ctx, report, eval, |ctx| {
                        super::ref_::compile_dynamic(ctx, value)
                    }));
                } else {
                    parts.push(report_guarded(ctx, report, fallback, |ctx| {
                        super::ref_::compile_dynamic(ctx, value)
                    }));
                }
            }
        }
    }
}

/// `located` emits the expression into `validate` code, where `__path` is the object's location;
/// `report` guards what it reports as evaluated, for the schema with the keyword.
pub(crate) fn compile_key_evaluated_expr<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    schema: &Map<String, Value>,
    include_own_unevaluated: bool,
    located: bool,
    mut report: Option<&mut ReportGuards>,
) -> TokenStream {
    let mut parts = Vec::new();
    let applicator_vocab_enabled = ctx.supports_applicator_vocabulary();
    let node = E::node_param(None);

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
            compile_pattern_coverage_for_key(ctx, schema.get("patternProperties"), located)
        {
            parts.push(pattern_expr);
        }
    }

    if applicator_vocab_enabled && ctx.draft.supports_dependent_schemas_keyword() {
        if let Some(dependent_schemas) = schema.get("dependentSchemas").and_then(Value::as_object) {
            for (property, subschema) in dependent_schemas {
                if let Value::Object(subschema_obj) = subschema {
                    let sub_eval = in_resource_of(ctx, subschema_obj, EvalKind::Key, |ctx| {
                        compile_key_evaluated_expr(ctx, subschema_obj, true, located, None)
                    });
                    let sub_eval = report_guarded(ctx, &mut report, sub_eval, |ctx| {
                        compile_branch_guard(ctx, subschema)
                    });
                    let contains_key = E::object_contains_key(format_ident!("obj"), property);
                    parts.push(quote! { #contains_key && (#sub_eval) });
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
            &node,
            EvalKind::Key,
            &mut hoist,
            &mut report,
        );
    }

    push_ref_dispatch(ctx, schema, &mut parts, EvalKind::Key, &mut report);

    let evaluated_without_unevaluated = super::combine_or(parts);

    if include_own_unevaluated && ctx.supports_unevaluated_properties() {
        if let Some(unevaluated) = schema.get("unevaluatedProperties") {
            if unevaluated.as_bool() == Some(true) {
                return quote! { true };
            }
            if unevaluated.as_bool() != Some(false) {
                let schema_check = compile_schema(ctx, unevaluated);
                let get_by_key = E::object_get_dynamic(format_ident!("obj"), quote! { key_str });
                return quote! {
                    (#evaluated_without_unevaluated) || {
                        #get_by_key.is_some_and(|instance| {
                            #schema_check
                        })
                    }
                };
            }
        }
    }

    evaluated_without_unevaluated
}

/// `report` guards what it reports as evaluated, for the schema with the keyword.
pub(crate) fn compile_index_evaluated_expr<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    schema: &Map<String, Value>,
    hoist: &mut GuardHoist,
    mut report: Option<&mut ReportGuards>,
) -> TokenStream {
    let mut parts = Vec::new();
    let applicator_vocab_enabled = ctx.supports_applicator_vocabulary();
    let node = E::node_param(None);

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

        // `contains` annotates the items it matches from 2020-12 on; earlier drafts evaluate none.
        if let Some(contains_schema) = schema
            .get("contains")
            .filter(|_| matches!(ctx.draft, Draft::Draft202012 | Draft::Unknown))
        {
            let contains_check = compile_branch_guard(ctx, contains_schema);
            parts.push(quote! {
                (|instance: #node| #contains_check)(item)
            });
        }

        push_applicator_eval(
            ctx,
            schema,
            &mut parts,
            &node,
            EvalKind::Item,
            hoist,
            &mut report,
        );
    }

    push_ref_dispatch(ctx, schema, &mut parts, EvalKind::Item, &mut report);

    let evaluated_without_unevaluated = super::combine_or(parts);

    if ctx.supports_unevaluated_items() {
        if let Some(unevaluated) = schema.get("unevaluatedItems") {
            if unevaluated.as_bool() == Some(true) {
                return quote! { true };
            }
            if unevaluated.as_bool() != Some(false) {
                let schema_check = compile_schema(ctx, unevaluated);
                let node = E::node_param(None);
                return quote! {
                    (#evaluated_without_unevaluated)
                        || (|instance: #node| #schema_check)(item)
                };
            }
        }
    }

    evaluated_without_unevaluated
}

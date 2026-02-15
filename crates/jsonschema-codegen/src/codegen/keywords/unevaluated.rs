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
    refs::resolve_ref,
    CompiledExpr,
};

/// Branch validity guard for unevaluated* evaluation tracking.
///
/// Guards re-check subschemas that the main validation path already compiles,
/// so inlining them via `compile_schema` grows emitted code quadratically with
/// applicator nesting depth. Instead, non-trivial subschemas become shared
/// helper fns keyed by (base URI, draft, schema content), and applicator
/// branches inside a guard compose from further guard calls rather than
/// re-inlined subtrees.
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
    if !ctx.supports_applicator_vocabulary() {
        return false;
    }
    obj.contains_key("allOf")
        || obj.contains_key("anyOf")
        || obj.contains_key("oneOf")
        || obj.contains_key("not")
        || (ctx.draft.supports_if_then_else_keyword() && obj.contains_key("if"))
}

/// Validity-only compilation that spells applicator branches as guard-helper
/// calls instead of inlined subtrees.
fn compile_guard_validity(ctx: &mut CompileContext<'_>, schema: &Value) -> CompiledExpr {
    let Value::Object(obj) = schema else {
        return compile_schema(ctx, schema);
    };
    // unevaluated* semantics depend on sibling applicator annotations, so the
    // keywords cannot be compiled separately from each other.
    if obj.contains_key("unevaluatedProperties") || obj.contains_key("unevaluatedItems") {
        return compile_schema(ctx, schema);
    }
    if !has_composable_applicators(ctx, obj) {
        return compile_schema(ctx, schema);
    }

    let if_then_else = ctx.draft.supports_if_then_else_keyword();
    let mut rest = obj.clone();
    for key in ["allOf", "anyOf", "oneOf", "not"] {
        rest.remove(key);
    }
    if if_then_else {
        for key in ["if", "then", "else"] {
            rest.remove(key);
        }
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
            .map(|branch| compile_branch_guard(ctx, branch).is_valid_ts())
            .collect();
        result = result.and(CompiledExpr::from_bool_expr(
            quote! { (#(( #calls ))||*) },
            "",
        ));
    }
    if let Some(one_of) = obj.get("oneOf").and_then(Value::as_array) {
        let calls: Vec<_> = one_of
            .iter()
            .map(|branch| compile_branch_guard(ctx, branch).is_valid_ts())
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
        let call = compile_branch_guard(ctx, not_schema).is_valid_ts();
        result = result.and(CompiledExpr::from_bool_expr(quote! { !(#call) }, ""));
    }
    if if_then_else {
        if let Some(if_schema) = obj.get("if") {
            let if_call = compile_branch_guard(ctx, if_schema).is_valid_ts();
            let then_call = obj.get("then").map_or(quote! { true }, |s| {
                compile_branch_guard(ctx, s).is_valid_ts()
            });
            let else_call = obj.get("else").map_or(quote! { true }, |s| {
                compile_branch_guard(ctx, s).is_valid_ts()
            });
            result = result.and(CompiledExpr::from_bool_expr(
                quote! { if #if_call { #then_call } else { #else_call } },
                "",
            ));
        }
    }
    result
}

fn compile_guarded_eval(
    value_ty: &TokenStream,
    valid_expr: &CompiledExpr,
    eval_expr: &TokenStream,
) -> TokenStream {
    quote! {
        (|instance: &#value_ty| #valid_expr)(instance) && (#eval_expr)
    }
}

fn compile_one_of_evaluated(
    value_ty: &TokenStream,
    cases: &[(CompiledExpr, TokenStream)],
) -> Option<TokenStream> {
    if cases.is_empty() {
        return None;
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
) -> TokenStream {
    quote! {
        if (|instance: &#value_ty| #if_valid_expr)(instance) {
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

pub(crate) fn compile_key_evaluated_expr(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> TokenStream {
    let mut parts = Vec::new();
    let applicator_vocab_enabled = ctx.supports_applicator_vocabulary();
    let value_ty = crate::codegen::emit_serde::value_ty();

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

    let pattern_coverage = if applicator_vocab_enabled {
        compile_pattern_coverage_for_key(ctx, schema.get("patternProperties"))
    } else {
        None
    };
    if applicator_vocab_enabled {
        if let Some(pattern_expr) = pattern_coverage.clone() {
            parts.push(pattern_expr);
        }
    }

    if applicator_vocab_enabled {
        if let Some(additional) = schema.get("additionalProperties") {
            if additional.as_bool() != Some(false) {
                let mut covered_parts = Vec::new();
                if let Some(properties) = properties_obj {
                    if !properties.is_empty() {
                        let names: Vec<&str> = properties.keys().map(String::as_str).collect();
                        covered_parts.push(quote! { matches!(key_str, #(#names)|*) });
                    }
                }
                if let Some(pattern_expr) = pattern_coverage {
                    covered_parts.push(pattern_expr);
                }
                let covered = super::combine_or(covered_parts);
                parts.push(quote! { !(#covered) });
            }
        }
    }

    if applicator_vocab_enabled && ctx.draft.supports_dependent_schemas_keyword() {
        if let Some(dependent_schemas) = schema.get("dependentSchemas").and_then(Value::as_object) {
            for (property, subschema) in dependent_schemas {
                if let Value::Object(subschema_obj) = subschema {
                    let sub_eval = compile_key_evaluated_expr(ctx, subschema_obj);
                    parts.push(quote! { obj.contains_key(#property) && (#sub_eval) });
                }
            }
        }
    }

    if applicator_vocab_enabled {
        if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
            for subschema in all_of {
                if let Value::Object(subschema_obj) = subschema {
                    let sub_eval = compile_key_evaluated_expr(ctx, subschema_obj);
                    parts.push(sub_eval);
                }
            }
        }

        if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array) {
            for subschema in any_of {
                if let Value::Object(subschema_obj) = subschema {
                    let sub_eval = compile_key_evaluated_expr(ctx, subschema_obj);
                    let sub_valid = compile_branch_guard(ctx, subschema);
                    parts.push(compile_guarded_eval(&value_ty, &sub_valid, &sub_eval));
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
                    let sub_eval = compile_key_evaluated_expr(ctx, subschema_obj);
                    let sub_valid = compile_branch_guard(ctx, subschema);
                    Some((sub_valid, sub_eval))
                })
                .collect();
            if let Some(one_of_eval) = compile_one_of_evaluated(&value_ty, &cases) {
                parts.push(one_of_eval);
            }
        }

        if ctx.draft.supports_if_then_else_keyword() {
            if let Some(if_schema) = schema.get("if") {
                let if_valid = compile_branch_guard(ctx, if_schema);
                let if_eval = if let Value::Object(if_obj) = if_schema {
                    compile_key_evaluated_expr(ctx, if_obj)
                } else {
                    quote! { false }
                };
                let then_eval = schema.get("then").and_then(Value::as_object).map_or_else(
                    || quote! { false },
                    |then_obj| compile_key_evaluated_expr(ctx, then_obj),
                );
                let else_eval = schema.get("else").and_then(Value::as_object).map_or_else(
                    || quote! { false },
                    |else_obj| compile_key_evaluated_expr(ctx, else_obj),
                );
                parts.push(compile_if_then_else_evaluated(
                    &value_ty, &if_valid, &if_eval, &then_eval, &else_eval,
                ));
            }
        }
    }

    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        if let Ok(resolved) = resolve_ref(ctx, reference) {
            if !ctx.key_eval_helpers.is_compiling(&resolved.location) {
                let func_name = get_or_create_key_eval_fn(
                    ctx,
                    &resolved.location,
                    &resolved.schema,
                    resolved.base_uri,
                );
                let func_ident = format_ident!("{}", func_name);
                parts.push(quote! { #func_ident(instance, obj, key_str) });
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
                let fallback = if ctx.key_eval_helpers.is_compiling(&resolved.location) {
                    quote! { false }
                } else {
                    let func_name = get_or_create_key_eval_fn(
                        ctx,
                        &resolved.location,
                        &resolved.schema,
                        resolved.base_uri,
                    );
                    let func_ident = format_ident!("{}", func_name);
                    quote! { #func_ident(instance, obj, key_str) }
                };
                if target_has_recursive_anchor {
                    // Ensure the recursive stack infrastructure is emitted even if
                    // `$recursiveRef` in the same schema hasn't been compiled yet.
                    ctx.uses_recursive_ref = true;
                    parts.push(quote! {
                        {
                            let __recursive_target = __JSONSCHEMA_RECURSIVE_KEY_EVAL_STACK.with(|stack| {
                                let stack = stack.borrow();
                                let mut selected = None;
                                for (validate, is_anchor) in stack.iter().rev() {
                                    if *is_anchor {
                                        selected = Some(*validate);
                                    } else {
                                        break;
                                    }
                                }
                                selected
                            });
                            if let Some(target) = __recursive_target {
                                target(instance, obj, key_str)
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
                let fallback = if ctx.key_eval_helpers.is_compiling(&resolved.location) {
                    quote! { false }
                } else {
                    let func_name = get_or_create_key_eval_fn(
                        ctx,
                        &resolved.location,
                        &resolved.schema,
                        resolved.base_uri,
                    );
                    let func_ident = format_ident!("{}", func_name);
                    quote! { #func_ident(instance, obj, key_str) }
                };
                if let Some(anchor_name) = dynamic_ref_anchor_name(reference, &resolved.schema) {
                    ctx.uses_dynamic_ref = true;
                    parts.push(quote! {
                        {
                            let __dynamic_target = __JSONSCHEMA_DYNAMIC_KEY_EVAL_STACK.with(|stack| {
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
                                target(instance, obj, key_str)
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
    let evaluated_without_unevaluated = super::combine_or(parts);

    if ctx.supports_unevaluated_properties() {
        if let Some(unevaluated) = schema.get("unevaluatedProperties") {
            if unevaluated.as_bool() == Some(true) {
                return quote! { true };
            }
            if unevaluated.as_bool() != Some(false) {
                let schema_check = compile_schema(ctx, unevaluated);
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

        if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
            for subschema in all_of {
                if let Value::Object(subschema_obj) = subschema {
                    parts.push(compile_index_evaluated_expr(ctx, subschema_obj));
                }
            }
        }

        if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array) {
            for subschema in any_of {
                if let Value::Object(subschema_obj) = subschema {
                    let sub_eval = compile_index_evaluated_expr(ctx, subschema_obj);
                    let sub_valid = compile_branch_guard(ctx, subschema);
                    parts.push(compile_guarded_eval(&value_ty, &sub_valid, &sub_eval));
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
                    let sub_eval = compile_index_evaluated_expr(ctx, subschema_obj);
                    let sub_valid = compile_branch_guard(ctx, subschema);
                    Some((sub_valid, sub_eval))
                })
                .collect();
            if let Some(one_of_eval) = compile_one_of_evaluated(&value_ty, &cases) {
                parts.push(one_of_eval);
            }
        }

        if ctx.draft.supports_if_then_else_keyword() {
            if let Some(if_schema) = schema.get("if") {
                let if_valid = compile_branch_guard(ctx, if_schema);
                let if_eval = if let Value::Object(if_obj) = if_schema {
                    compile_index_evaluated_expr(ctx, if_obj)
                } else {
                    quote! { false }
                };
                let then_eval = schema.get("then").and_then(Value::as_object).map_or_else(
                    || quote! { false },
                    |then_obj| compile_index_evaluated_expr(ctx, then_obj),
                );
                let else_eval = schema.get("else").and_then(Value::as_object).map_or_else(
                    || quote! { false },
                    |else_obj| compile_index_evaluated_expr(ctx, else_obj),
                );
                parts.push(compile_if_then_else_evaluated(
                    &value_ty, &if_valid, &if_eval, &then_eval, &else_eval,
                ));
            }
        }
    }

    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        if let Ok(resolved) = resolve_ref(ctx, reference) {
            if !ctx.item_eval_helpers.is_compiling(&resolved.location) {
                let func_name = get_or_create_item_eval_fn(
                    ctx,
                    &resolved.location,
                    &resolved.schema,
                    resolved.base_uri,
                );
                let func_ident = format_ident!("{}", func_name);
                parts.push(quote! { #func_ident(instance, arr, idx, item) });
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
                let fallback = if ctx.item_eval_helpers.is_compiling(&resolved.location) {
                    quote! { false }
                } else {
                    let func_name = get_or_create_item_eval_fn(
                        ctx,
                        &resolved.location,
                        &resolved.schema,
                        resolved.base_uri,
                    );
                    let func_ident = format_ident!("{}", func_name);
                    quote! { #func_ident(instance, arr, idx, item) }
                };
                if target_has_recursive_anchor {
                    // Ensure the recursive stack infrastructure is emitted even if
                    // `$recursiveRef` in the same schema hasn't been compiled yet.
                    ctx.uses_recursive_ref = true;
                    parts.push(quote! {
                        {
                            let __recursive_target = __JSONSCHEMA_RECURSIVE_ITEM_EVAL_STACK.with(|stack| {
                                let stack = stack.borrow();
                                let mut selected = None;
                                for (validate, is_anchor) in stack.iter().rev() {
                                    if *is_anchor {
                                        selected = Some(*validate);
                                    } else {
                                        break;
                                    }
                                }
                                selected
                            });
                            if let Some(target) = __recursive_target {
                                target(instance, arr, idx, item)
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
                let fallback = if ctx.item_eval_helpers.is_compiling(&resolved.location) {
                    quote! { false }
                } else {
                    let func_name = get_or_create_item_eval_fn(
                        ctx,
                        &resolved.location,
                        &resolved.schema,
                        resolved.base_uri,
                    );
                    let func_ident = format_ident!("{}", func_name);
                    quote! { #func_ident(instance, arr, idx, item) }
                };
                if let Some(anchor_name) = dynamic_ref_anchor_name(reference, &resolved.schema) {
                    ctx.uses_dynamic_ref = true;
                    parts.push(quote! {
                        {
                            let __dynamic_target = __JSONSCHEMA_DYNAMIC_ITEM_EVAL_STACK.with(|stack| {
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
                                target(instance, arr, idx, item)
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

    let evaluated_without_unevaluated = super::combine_or(parts);

    if ctx.supports_unevaluated_items() {
        if let Some(unevaluated) = schema.get("unevaluatedItems") {
            if unevaluated.as_bool() == Some(true) {
                return quote! { true };
            }
            if unevaluated.as_bool() != Some(false) {
                let schema_check = compile_schema(ctx, unevaluated);
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

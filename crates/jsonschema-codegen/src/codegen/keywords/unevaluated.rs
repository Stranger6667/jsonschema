use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::Draft;
use serde_json::{Map, Value};

use crate::context::CompileContext;

use super::super::{
    compile_schema,
    draft::{
        supports_applicator_vocabulary, supports_dependent_schemas_keyword,
        supports_dynamic_ref_keyword, supports_if_then_else_keyword, supports_prefix_items_keyword,
        supports_recursive_ref_keyword, supports_unevaluated_items_keyword_for_context,
        supports_unevaluated_properties_keyword_for_context,
    },
    helpers::{dynamic_ref_anchor_name, get_or_create_item_eval_fn, get_or_create_key_eval_fn},
    refs::resolve_ref,
    regex::{compile_regex_match, translate_and_validate_regex},
    CompiledExpr,
};

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
    patterns: &Map<String, Value>,
) -> Option<TokenStream> {
    let mut checks = Vec::new();
    for pattern in patterns.keys() {
        let check = match jsonschema_regex::analyze_pattern(pattern) {
            Some(jsonschema_regex::PatternAnalysis::Prefix(prefix)) => {
                let prefix: &str = prefix.as_ref();
                quote! { key_str.starts_with(#prefix) }
            }
            Some(jsonschema_regex::PatternAnalysis::Exact(exact)) => {
                let exact: &str = exact.as_ref();
                quote! { key_str == #exact }
            }
            Some(jsonschema_regex::PatternAnalysis::Alternation(alts)) => {
                let alts: Vec<&str> = alts.iter().map(String::as_str).collect();
                quote! { matches!(key_str, #(#alts)|*) }
            }
            None => match translate_and_validate_regex(ctx, "patternProperties", pattern) {
                Ok(translated) => compile_regex_match(ctx, &translated, &quote! { key_str }),
                Err(error_expr) => error_expr.into_token_stream(),
            },
        };
        checks.push(check);
    }
    (!checks.is_empty()).then(|| super::combine_or(checks))
}

pub(crate) fn compile_key_evaluated_expr(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> TokenStream {
    let mut parts = Vec::new();
    let applicator_vocab_enabled = supports_applicator_vocabulary(ctx);
    let value_ty = ctx.config.backend.emit_symbols().value_ty();

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

    let pattern_obj = if applicator_vocab_enabled {
        schema.get("patternProperties").and_then(Value::as_object)
    } else {
        None
    };
    let pattern_coverage =
        pattern_obj.and_then(|patterns| compile_pattern_coverage_for_key(ctx, patterns));
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

    if applicator_vocab_enabled && supports_dependent_schemas_keyword(ctx.draft) {
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
                    let sub_valid = compile_schema(ctx, subschema);
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
                    let sub_valid = compile_schema(ctx, subschema);
                    Some((sub_valid, sub_eval))
                })
                .collect();
            if let Some(one_of_eval) = compile_one_of_evaluated(&value_ty, &cases) {
                parts.push(one_of_eval);
            }
        }

        if supports_if_then_else_keyword(ctx.draft) {
            if let Some(if_schema) = schema.get("if") {
                let if_valid = compile_schema(ctx, if_schema);
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
    if supports_recursive_ref_keyword(ctx.draft) {
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
    if supports_dynamic_ref_keyword(ctx.draft) {
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

    if supports_unevaluated_properties_keyword_for_context(ctx) {
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
    let applicator_vocab_enabled = supports_applicator_vocabulary(ctx);
    let value_ty = ctx.config.backend.emit_symbols().value_ty();

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

        if supports_prefix_items_keyword(ctx.draft) {
            if let Some(prefix_items) = schema.get("prefixItems").and_then(Value::as_array) {
                let prefix_len = prefix_items.len();
                parts.push(quote! { idx < #prefix_len });
            }
        }

        if let Some(contains_schema) = schema.get("contains") {
            let contains_check = compile_schema(ctx, contains_schema);
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
                    let sub_valid = compile_schema(ctx, subschema);
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
                    let sub_valid = compile_schema(ctx, subschema);
                    Some((sub_valid, sub_eval))
                })
                .collect();
            if let Some(one_of_eval) = compile_one_of_evaluated(&value_ty, &cases) {
                parts.push(one_of_eval);
            }
        }

        if supports_if_then_else_keyword(ctx.draft) {
            if let Some(if_schema) = schema.get("if") {
                let if_valid = compile_schema(ctx, if_schema);
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
    if supports_recursive_ref_keyword(ctx.draft) {
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
    if supports_dynamic_ref_keyword(ctx.draft) {
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

    if supports_unevaluated_items_keyword_for_context(ctx) {
        if let Some(unevaluated) = schema.get("unevaluatedItems") {
            if unevaluated.as_bool() == Some(true) {
                return quote! { true };
            }
            if unevaluated.as_bool() != Some(false) {
                let schema_check = compile_schema(ctx, unevaluated);
                let value_ty = ctx.config.backend.emit_symbols().value_ty();
                return quote! {
                    (#evaluated_without_unevaluated)
                        || (|instance: &#value_ty| #schema_check)(item)
                };
            }
        }
    }

    evaluated_without_unevaluated
}

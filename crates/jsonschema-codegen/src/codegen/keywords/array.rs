use super::{
    super::{
        draft::{
            supports_applicator_vocabulary, supports_contains_bounds_keyword,
            supports_contains_keyword, supports_prefix_items_keyword,
            supports_validation_vocabulary,
        },
        errors::invalid_schema_type_expression,
        parse_nonnegative_integer_keyword, CompileContext, CompiledExpr,
    },
    additional_items, contains, items, max_items, min_items, prefix_items, unevaluated_items,
    unique_items,
};
use quote::quote;
use serde_json::{Map, Value};

/// Compile all array-specific keywords.
pub(in super::super) fn compile(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> CompiledExpr {
    let mut checks: Vec<CompiledExpr> = Vec::new();
    let validation_vocab_enabled = supports_validation_vocabulary(ctx);
    let applicator_vocab_enabled = supports_applicator_vocabulary(ctx);

    // minItems / maxItems with shared-length optimization.
    let min = if validation_vocab_enabled {
        schema
            .get("minItems")
            .map(|v| parse_nonnegative_integer_keyword(ctx.draft, v))
    } else {
        None
    };
    let max = if validation_vocab_enabled {
        schema
            .get("maxItems")
            .map(|v| parse_nonnegative_integer_keyword(ctx.draft, v))
    } else {
        None
    };
    let array_len = ctx.config.backend.array_len(quote! { arr });
    match (&min, &max) {
        (Some(Ok(mn)), Some(Ok(mx))) if mn == mx => {
            let sp_min = ctx.schema_path_for_keyword("minItems");
            let sp_max = ctx.schema_path_for_keyword("maxItems");
            let sp = if schema.contains_key("minItems") {
                sp_min
            } else {
                sp_max
            };
            checks.push(CompiledExpr::from_bool_expr(
                quote! { #array_len == #mn as usize },
                &sp,
            ));
        }
        (Some(Ok(mn)), Some(Ok(mx))) => {
            let sp_min = ctx.schema_path_for_keyword("minItems");
            let sp_max = ctx.schema_path_for_keyword("maxItems");
            checks.push(CompiledExpr::from_bool_expr(
                quote! { #array_len >= #mn as usize },
                &sp_min,
            ));
            checks.push(CompiledExpr::from_bool_expr(
                quote! { #array_len <= #mx as usize },
                &sp_max,
            ));
        }
        (Some(Ok(mn)), None) => {
            let sp = ctx.schema_path_for_keyword("minItems");
            checks.push(CompiledExpr::from_bool_expr(
                quote! { #array_len >= #mn as usize },
                &sp,
            ));
        }
        (None, Some(Ok(mx))) => {
            let sp = ctx.schema_path_for_keyword("maxItems");
            checks.push(CompiledExpr::from_bool_expr(
                quote! { #array_len <= #mx as usize },
                &sp,
            ));
        }
        _ => {
            if let Some(v) = schema.get("minItems").filter(|_| validation_vocab_enabled) {
                checks.push(min_items::compile(ctx, v));
            }
            if let Some(v) = schema.get("maxItems").filter(|_| validation_vocab_enabled) {
                checks.push(max_items::compile(ctx, v));
            }
        }
    }

    // prefixItems (draft 2020-12+)
    let prefix_len = if applicator_vocab_enabled && supports_prefix_items_keyword(ctx.draft) {
        if let Some(prefix_items_value) = schema.get("prefixItems") {
            match prefix_items_value {
                Value::Array(_) => {
                    if let Some((compiled, len)) = prefix_items::compile(ctx, prefix_items_value) {
                        checks.push(compiled);
                        Some(len)
                    } else {
                        None
                    }
                }
                other => {
                    checks.push(invalid_schema_type_expression(other, &["array"]));
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // items
    if applicator_vocab_enabled {
        if let Some(value) = schema.get("items") {
            let compiled = items::compile(ctx, value, prefix_len);
            if !compiled.is_trivially_true() {
                checks.push(compiled);
            }
        }
    }

    // uniqueItems
    if validation_vocab_enabled {
        if let Some(value) = schema.get("uniqueItems") {
            if let Some(compiled) = unique_items::compile(ctx, value) {
                checks.push(compiled);
            }
        }
    }

    // additionalItems
    if applicator_vocab_enabled {
        if let Some(value) = schema.get("additionalItems") {
            if let Some(compiled) = additional_items::compile(ctx, value, schema.get("items")) {
                checks.push(compiled);
            }
        }
    }

    // contains / minContains / maxContains
    if applicator_vocab_enabled && supports_contains_keyword(ctx.draft) {
        if let Some(contains_val) = schema.get("contains") {
            let (min_contains, max_contains) =
                if supports_contains_bounds_keyword(ctx.draft) {
                    let min_c = schema.get("minContains").and_then(|v| {
                        match parse_nonnegative_integer_keyword(ctx.draft, v) {
                            Ok(n) => Some(n),
                            Err(e) => {
                                checks.push(e);
                                None
                            }
                        }
                    });
                    let max_c = schema.get("maxContains").and_then(|v| {
                        match parse_nonnegative_integer_keyword(ctx.draft, v) {
                            Ok(n) => Some(n),
                            Err(e) => {
                                checks.push(e);
                                None
                            }
                        }
                    });
                    (min_c, max_c)
                } else {
                    (None, None)
                };
            checks.push(contains::compile(
                ctx,
                contains_val,
                min_contains,
                max_contains,
            ));
        }
    }

    if let Some(compiled) = unevaluated_items::compile(ctx, schema) {
        checks.push(compiled);
    }

    CompiledExpr::combine_and(checks)
}

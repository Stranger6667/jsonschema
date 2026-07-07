use super::{
    super::{
        draft::DraftExt, errors::invalid_schema_type_expression, parse_nonnegative_integer_keyword,
        CompileContext, CompiledExpr,
    },
    additional_items, compile_count_range, contains, items, prefix_items, unevaluated_items,
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
    let validation_vocab_enabled = ctx.supports_validation_vocabulary();
    let applicator_vocab_enabled = ctx.supports_applicator_vocabulary();

    // Checks are pushed in the runtime's `keyword_priority` order so that
    // `validate` reports the same first error as the runtime validator.
    let array_len = crate::codegen::emit_serde::array_len(quote! { arr });
    compile_count_range(
        ctx,
        schema.get("minItems").filter(|_| validation_vocab_enabled),
        schema.get("maxItems").filter(|_| validation_vocab_enabled),
        &array_len,
        "minItems",
        "min_items",
        "maxItems",
        "max_items",
        &mut checks,
    );

    if validation_vocab_enabled {
        if let Some(value) = schema.get("uniqueItems") {
            if let Some(compiled) = unique_items::compile(ctx, value) {
                checks.push(compiled);
            }
        }
    }

    // prefixItems (draft 2020-12+); its check lands after `items`.
    let mut prefix_items_check = None;
    let prefix_len = if applicator_vocab_enabled && ctx.draft.supports_prefix_items_keyword() {
        if let Some(prefix_items_value) = schema.get("prefixItems") {
            match prefix_items_value {
                Value::Array(_) => {
                    if let Some((compiled, len)) = prefix_items::compile(ctx, prefix_items_value) {
                        prefix_items_check = Some(compiled);
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

    if applicator_vocab_enabled {
        if let Some(value) = schema.get("items") {
            let compiled = items::compile(ctx, value, prefix_len);
            if !compiled.is_trivially_true() {
                checks.push(compiled);
            }
        }
    }

    if let Some(compiled) = prefix_items_check {
        checks.push(compiled);
    }

    if applicator_vocab_enabled {
        if let Some(value) = schema.get("additionalItems") {
            if let Some(compiled) =
                additional_items::compile(ctx, value, schema.get("items"), schema.get("maxItems"))
            {
                checks.push(compiled);
            }
        }
    }

    if applicator_vocab_enabled && ctx.draft.supports_contains_keyword() {
        if let Some(contains_value) = schema.get("contains") {
            let (min_contains, max_contains) = if ctx.draft.supports_contains_bounds_keyword() {
                let minimum = schema.get("minContains").and_then(|value| {
                    match parse_nonnegative_integer_keyword(ctx.draft, value) {
                        Ok(count) => Some(count),
                        Err(error) => {
                            checks.push(error);
                            None
                        }
                    }
                });
                let maximum = schema.get("maxContains").and_then(|value| {
                    match parse_nonnegative_integer_keyword(ctx.draft, value) {
                        Ok(count) => Some(count),
                        Err(error) => {
                            checks.push(error);
                            None
                        }
                    }
                });
                (minimum, maximum)
            } else {
                (None, None)
            };
            checks.push(contains::compile(
                ctx,
                contains_value,
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

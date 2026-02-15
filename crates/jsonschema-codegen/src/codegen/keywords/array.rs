use super::super::{
    compile_additional_items, compile_contains, compile_prefix_items, compile_unevaluated_items,
    compile_unique_items, generate_items_check, generate_items_check_with_prefix,
    invalid_schema_type_expression, is_trivially_true, parse_nonnegative_integer_keyword,
    supports_applicator_vocabulary, supports_contains_bounds_keyword, supports_contains_keyword,
    supports_prefix_items_keyword, supports_validation_vocabulary, CompileContext,
};
use proc_macro2::TokenStream;
use quote::quote;
use serde_json::{Map, Value};

/// Compile all array-specific keywords.
pub(in super::super) fn compile(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> TokenStream {
    let mut items = Vec::new();
    let validation_vocab_enabled = supports_validation_vocabulary(ctx);
    let applicator_vocab_enabled = supports_applicator_vocabulary(ctx);

    let min_items = if validation_vocab_enabled {
        match schema.get("minItems") {
            Some(value) => match parse_nonnegative_integer_keyword(ctx.draft, value) {
                Ok(parsed) => Some(parsed),
                Err(error) => {
                    items.push(error);
                    None
                }
            },
            None => None,
        }
    } else {
        None
    };
    let max_items = if validation_vocab_enabled {
        match schema.get("maxItems") {
            Some(value) => match parse_nonnegative_integer_keyword(ctx.draft, value) {
                Ok(parsed) => Some(parsed),
                Err(error) => {
                    items.push(error);
                    None
                }
            },
            None => None,
        }
    } else {
        None
    };
    let array_len = ctx.config.backend.array_len(quote! { arr });
    match (min_items, max_items) {
        (Some(min), Some(max)) if min == max => {
            items.push(quote! { #array_len == #min as usize });
        }
        (Some(min), Some(max)) => {
            items.push(quote! { #array_len >= #min as usize });
            items.push(quote! { #array_len <= #max as usize });
        }
        (Some(min), None) => items.push(quote! { #array_len >= #min as usize }),
        (None, Some(max)) => items.push(quote! { #array_len <= #max as usize }),
        (None, None) => {}
    }

    let prefix_items_len = if applicator_vocab_enabled && supports_prefix_items_keyword(ctx.draft) {
        if let Some(prefix_items_value) = schema.get("prefixItems") {
            match prefix_items_value {
                Value::Array(_) => {
                    if let Some(compiled) = compile_prefix_items(ctx, prefix_items_value) {
                        items.push(compiled.0);
                        Some(compiled.1)
                    } else {
                        None
                    }
                }
                other => {
                    items.push(invalid_schema_type_expression(other, &["array"]));
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
            let compiled = if let Some(prefix_len) = prefix_items_len {
                generate_items_check_with_prefix(ctx, value, prefix_len)
            } else {
                generate_items_check(ctx, value)
            };
            if !is_trivially_true(&compiled) {
                items.push(compiled);
            }
        }
    }

    if validation_vocab_enabled {
        if let Some(compiled) = schema.get("uniqueItems").and_then(compile_unique_items) {
            items.push(compiled);
        }
    }

    if applicator_vocab_enabled {
        if let Some(compiled) =
            compile_additional_items(ctx, schema.get("additionalItems"), schema.get("items"))
        {
            items.push(compiled);
        }
    }

    if applicator_vocab_enabled && supports_contains_keyword(ctx.draft) {
        if let Some(compiled) = schema.get("contains").map(|v| {
            let (min_contains, max_contains) = if supports_contains_bounds_keyword(ctx.draft) {
                let min_contains = match schema.get("minContains") {
                    Some(value) => match parse_nonnegative_integer_keyword(ctx.draft, value) {
                        Ok(parsed) => Some(parsed),
                        Err(error) => {
                            items.push(error);
                            None
                        }
                    },
                    None => None,
                };
                let max_contains = match schema.get("maxContains") {
                    Some(value) => match parse_nonnegative_integer_keyword(ctx.draft, value) {
                        Ok(parsed) => Some(parsed),
                        Err(error) => {
                            items.push(error);
                            None
                        }
                    },
                    None => None,
                };
                (min_contains, max_contains)
            } else {
                (None, None)
            };
            compile_contains(ctx, v, min_contains, max_contains)
        }) {
            items.push(compiled);
        }
    }
    if let Some(compiled) = compile_unevaluated_items(ctx, schema) {
        items.push(compiled);
    }

    if items.is_empty() {
        quote! { true }
    } else {
        quote! { ( #(#items)&&* ) }
    }
}

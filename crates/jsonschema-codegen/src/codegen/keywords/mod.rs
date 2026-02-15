pub(super) mod additional_items;
pub(super) mod additional_properties;
pub(super) mod all_of;
pub(super) mod any_of;
pub(super) mod array;
pub(super) mod const_;
pub(super) mod contains;
pub(super) mod content;
pub(super) mod custom;
pub(super) mod dependencies;
pub(super) mod enum_;
pub(super) mod format;
pub(super) mod if_;
pub(super) mod items;
pub(super) mod minmax;
pub(super) mod multiple_of;
pub(super) mod not;
pub(super) mod number;
pub(super) mod object;
pub(super) mod one_of;
pub(super) mod pattern;
pub(super) mod pattern_coverage;
pub(super) mod pattern_properties;
pub(super) mod prefix_items;
pub(super) mod properties;
pub(super) mod property_names;
pub(super) mod ref_;
pub(super) mod required;
pub(super) mod string;
pub(super) mod type_;
pub(super) mod unevaluated;
pub(super) mod unevaluated_items;
pub(super) mod unevaluated_properties;
pub(super) mod unique_items;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::Value;

use super::{parse_nonnegative_integer_keyword, CompileContext, CompiledExpr};

/// Combine a list of boolean `TokenStream` expressions with `||`.
/// Returns `false` for an empty list.
pub(super) fn combine_or(parts: Vec<TokenStream>) -> TokenStream {
    match parts.len() {
        0 => quote! { false },
        1 => parts.into_iter().next().unwrap_or_else(|| quote! { false }),
        _ => quote! { (#(#parts)||*) },
    }
}

pub(super) enum Limit {
    Min,
    Max,
}

/// Compile a count-limit keyword (`minLength`, `maxItems`, `minProperties`, ...):
/// compares `count` against a non-negative integer limit, reporting failures
/// via the `__private::error` constructor named by `error_name`.
pub(super) fn compile_count_limit(
    ctx: &CompileContext<'_>,
    value: &Value,
    count: &TokenStream,
    keyword: &str,
    error_name: &str,
    limit_kind: &Limit,
) -> CompiledExpr {
    match parse_nonnegative_integer_keyword(ctx.draft, value) {
        Ok(limit) => {
            let schema_path = ctx.schema_path_for_keyword(keyword);
            let error = format_ident!("{error_name}");
            let (valid_cmp, invalid_cmp) = match limit_kind {
                Limit::Min => (quote! { >= }, quote! { < }),
                Limit::Max => (quote! { <= }, quote! { > }),
            };
            CompiledExpr::with_validate_blocks(
                quote! { #count #valid_cmp #limit as usize },
                quote! {
                    if #count #invalid_cmp #limit as usize {
                        return Some(jsonschema::__private::error::#error(
                            #schema_path, __path.clone(), instance, #limit,
                        ));
                    }
                },
            )
        }
        Err(e) => e,
    }
}

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
pub(super) mod object_pass;
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

/// Compile a min/max count-limit keyword pair, folding equal bounds into a
/// single equality check.
#[allow(clippy::too_many_arguments)]
pub(super) fn compile_count_range(
    ctx: &CompileContext<'_>,
    min_value: Option<&Value>,
    max_value: Option<&Value>,
    count: &TokenStream,
    min_keyword: &str,
    min_error: &str,
    max_keyword: &str,
    max_error: &str,
    checks: &mut Vec<CompiledExpr>,
) {
    if let (Some(min_value), Some(max_value)) = (min_value, max_value) {
        if let (Ok(min), Ok(max)) = (
            parse_nonnegative_integer_keyword(ctx.draft, min_value),
            parse_nonnegative_integer_keyword(ctx.draft, max_value),
        ) {
            if min == max {
                let min_path = ctx.schema_path_for_keyword(min_keyword);
                let max_path = ctx.schema_path_for_keyword(max_keyword);
                let min_error = format_ident!("{min_error}");
                let max_error = format_ident!("{max_error}");
                let limit = proc_macro2::Literal::u64_unsuffixed(min);
                checks.push(CompiledExpr::with_validate_blocks(
                    quote! { (#count as u64) == #limit },
                    quote! {
                        if (#count as u64) < #limit {
                            return Some(__err::#min_error(
                                #min_path, __path.into(), instance, #limit,
                            ));
                        }
                        if (#count as u64) > #limit {
                            return Some(__err::#max_error(
                                #max_path, __path.into(), instance, #limit,
                            ));
                        }
                    },
                ));
                return;
            }
        }
    }
    if let Some(value) = min_value {
        checks.push(compile_count_limit(
            ctx,
            value,
            count,
            min_keyword,
            min_error,
            &Limit::Min,
        ));
    }
    if let Some(value) = max_value {
        checks.push(compile_count_limit(
            ctx,
            value,
            count,
            max_keyword,
            max_error,
            &Limit::Max,
        ));
    }
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
        Ok(0) if matches!(limit_kind, Limit::Min) => CompiledExpr::always_true(),
        Ok(limit) => {
            let schema_path = ctx.schema_path_for_keyword(keyword);
            let error = format_ident!("{error_name}");
            let valid_cmp = match limit_kind {
                Limit::Min => quote! { >= },
                Limit::Max => quote! { <= },
            };
            let limit = proc_macro2::Literal::u64_unsuffixed(limit);
            CompiledExpr::from_check_and_error(
                quote! { (#count as u64) #valid_cmp #limit },
                quote! {
                    __err::#error(
                        #schema_path, __path.into(), instance, #limit,
                    )
                },
            )
        }
        Err(error) => error,
    }
}

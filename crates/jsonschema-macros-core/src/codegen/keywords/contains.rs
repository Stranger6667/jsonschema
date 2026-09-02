use super::super::{compile_schema, CompileContext, CompiledExpr};
use crate::codegen::emit::ValueEmitter;
use quote::{format_ident, quote};
use serde_json::Value;

pub(crate) fn compile<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    value: &Value,
    min_contains: Option<u64>,
    max_contains: Option<u64>,
) -> CompiledExpr {
    if min_contains == Some(0) && max_contains.is_none() {
        return CompiledExpr::always_true();
    }
    let schema_check = ctx.with_instance_scope(|ctx| compile_schema(ctx, value));
    let schema_is_valid = schema_check.is_valid_token_stream();
    let min = proc_macro2::Literal::u64_unsuffixed(min_contains.unwrap_or(1));

    let contains_path = ctx.schema_path_for_keyword("contains");
    // maxContains present without minContains: the runtime reports both the "too many" and
    // "too few" errors at the maxContains path.
    let min_path = if min_contains.is_some() {
        ctx.schema_path_for_keyword("minContains")
    } else if max_contains.is_some() {
        ctx.schema_path_for_keyword("maxContains")
    } else {
        contains_path.clone()
    };
    let max_path = ctx.schema_path_for_keyword("maxContains");

    let max_check_is_valid = if let Some(max) = max_contains {
        let max = proc_macro2::Literal::u64_unsuffixed(max);
        quote! { && (__contains_count as u64) <= #max }
    } else {
        quote! {}
    };

    let contains_iter = E::array_iter(format_ident!("arr"));
    let arr_iter_ref = E::array_iter_ref(format_ident!("arr"));
    let is_valid = match (min_contains.unwrap_or(1), max_contains) {
        (1, None) => quote! { #contains_iter.any(|instance| #schema_is_valid) },
        (_, None) => quote! {
            {
                let mut __contains_count = 0usize;
                for instance in #arr_iter_ref {
                    if #schema_is_valid {
                        __contains_count += 1;
                        if (__contains_count as u64) >= #min {
                            break;
                        }
                    }
                }
                (__contains_count as u64) >= #min
            }
        },
        (_, Some(_)) => quote! {
            {
                let mut __contains_count = 0usize;
                for instance in #arr_iter_ref {
                    if #schema_is_valid {
                        __contains_count += 1;
                    }
                }
                (__contains_count as u64) >= #min #max_check_is_valid
            }
        },
    };

    let min_path_str = min_path.as_str();
    let err_instance = E::err_instance(format_ident!("instance"));

    // Combined min+max: check maxContains before minContains (matching the runtime) so at
    // most one error is emitted.
    let validate_block = if let Some(max) = max_contains {
        let max = proc_macro2::Literal::u64_unsuffixed(max);
        quote! {
            {
                let mut __contains_count = 0usize;
                for item in #contains_iter {
                    let instance = item;
                    if #schema_is_valid {
                        __contains_count += 1;
                    }
                }
                if (__contains_count as u64) > #max {
                    return Some(__err::contains(
                        #max_path, __path.into(), #err_instance,
                    ));
                } else if (__contains_count as u64) < #min {
                    return Some(__err::contains(
                        #min_path_str, __path.into(), #err_instance,
                    ));
                }
            }
        }
    } else if min_contains.unwrap_or(1) == 1 {
        quote! {
            if !(#contains_iter.any(|instance| #schema_is_valid)) {
                return Some(__err::contains(
                    #min_path_str, __path.into(), #err_instance,
                ));
            }
        }
    } else {
        quote! {
            {
                let mut __contains_count = 0usize;
                for item in #contains_iter {
                    let instance = item;
                    if #schema_is_valid {
                        __contains_count += 1;
                        if (__contains_count as u64) >= #min {
                            break;
                        }
                    }
                }
                if (__contains_count as u64) < #min {
                    return Some(__err::contains(
                        #min_path_str, __path.into(), #err_instance,
                    ));
                }
            }
        }
    };

    CompiledExpr::with_validate_blocks(is_valid, validate_block)
}

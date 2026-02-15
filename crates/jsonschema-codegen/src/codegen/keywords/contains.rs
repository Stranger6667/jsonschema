use super::super::{compile_schema, CompileContext, CompiledExpr};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(
    ctx: &mut CompileContext<'_>,
    value: &Value,
    min_contains: Option<u64>,
    max_contains: Option<u64>,
) -> CompiledExpr {
    let schema_check = compile_schema(ctx, value);
    let schema_check_ts = schema_check.is_valid_ts();
    let min = min_contains.unwrap_or(1);

    let contains_path = ctx.schema_path_for_keyword("contains");
    // When maxContains is present but minContains is absent, the dynamic validator
    // uses MaxContainsValidator whose location is at "maxContains". So both the
    // "too many" and "too few" errors are reported at the maxContains path.
    let min_path = if min_contains.is_some() {
        ctx.schema_path_for_keyword("minContains")
    } else if max_contains.is_some() {
        ctx.schema_path_for_keyword("maxContains")
    } else {
        contains_path.clone()
    };
    let max_path = ctx.schema_path_for_keyword("maxContains");

    let max_check_is_valid = if let Some(max) = max_contains {
        quote! { && __contains_count <= #max as usize }
    } else {
        quote! {}
    };

    let is_valid_ts = quote! {
        {
            let mut __contains_count = 0usize;
            for instance in arr {
                if #schema_check_ts {
                    __contains_count += 1;
                }
            }
            __contains_count >= #min as usize #max_check_is_valid
        }
    };

    let min_path_str = min_path.as_str();

    // Build validate/iter_errors blocks. For the combined min+max case, the dynamic
    // validator checks maxContains first (early-return), then minContains. iter_errors
    // in the dynamic falls back to validate so it also generates at most 1 error.
    // We replicate this behavior using if-else so both blocks emit at most 1 error.
    let (validate_block, iter_errors_block) = if let Some(max) = max_contains {
        let v = quote! {
            {
                let mut __contains_count = 0usize;
                for item in arr.iter() {
                    let instance = item;
                    if #schema_check_ts {
                        __contains_count += 1;
                    }
                }
                if __contains_count > #max as usize {
                    return Some(jsonschema::keywords_helpers::error::contains(
                        #max_path, __path.clone(), instance,
                    ));
                } else if __contains_count < #min as usize {
                    return Some(jsonschema::keywords_helpers::error::contains(
                        #min_path_str, __path.clone(), instance,
                    ));
                }
            }
        };
        let ie = quote! {
            {
                let mut __contains_count = 0usize;
                for item in arr.iter() {
                    let instance = item;
                    if #schema_check_ts {
                        __contains_count += 1;
                    }
                }
                if __contains_count > #max as usize {
                    __errors.push(jsonschema::keywords_helpers::error::contains(
                        #max_path, __path.clone(), instance,
                    ));
                } else if __contains_count < #min as usize {
                    __errors.push(jsonschema::keywords_helpers::error::contains(
                        #min_path_str, __path.clone(), instance,
                    ));
                }
            }
        };
        (v, ie)
    } else {
        let v = quote! {
            {
                let mut __contains_count = 0usize;
                for item in arr.iter() {
                    let instance = item;
                    if #schema_check_ts {
                        __contains_count += 1;
                    }
                }
                if __contains_count < #min as usize {
                    return Some(jsonschema::keywords_helpers::error::contains(
                        #min_path_str, __path.clone(), instance,
                    ));
                }
            }
        };
        let ie = quote! {
            {
                let mut __contains_count = 0usize;
                for item in arr.iter() {
                    let instance = item;
                    if #schema_check_ts {
                        __contains_count += 1;
                    }
                }
                if __contains_count < #min as usize {
                    __errors.push(jsonschema::keywords_helpers::error::contains(
                        #min_path_str, __path.clone(), instance,
                    ));
                }
            }
        };
        (v, ie)
    };

    CompiledExpr::with_validate_blocks(is_valid_ts, validate_block, iter_errors_block)
}

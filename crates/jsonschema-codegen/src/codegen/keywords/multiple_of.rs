use super::super::{
    errors::{invalid_schema_exclusive_minimum_expression, invalid_schema_type_expression},
    CompileContext, CompiledExpr,
};
use proc_macro2::TokenStream;
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(ctx: &CompileContext<'_>, value: &Value) -> CompiledExpr {
    let check_ts = generate_multiple_of_check(value);

    // If the check itself is an error expression (invalid schema), just return it as-is.
    if !value.is_number() || !is_strictly_positive_number(value) {
        return CompiledExpr::from(check_ts);
    }

    let schema_path = ctx.schema_path_for_keyword("multipleOf");

    #[cfg(not(feature = "arbitrary-precision"))]
    let err_expr = if let Some(f) = value.as_f64() {
        quote! { jsonschema::keywords_helpers::error::multiple_of(#schema_path, __path.clone(), instance, #f) }
    } else {
        return CompiledExpr::from(check_ts);
    };

    #[cfg(feature = "arbitrary-precision")]
    let err_expr = {
        let value_json = serde_json::to_string(value).unwrap();
        if let Value::Number(number) = value {
            if requires_ap_multiple_of_path(number) {
                quote! {
                    {
                        static MOF: std::sync::LazyLock<serde_json::Value> =
                            std::sync::LazyLock::new(|| serde_json::from_str(#value_json).expect("multipleOf"));
                        jsonschema::keywords_helpers::error::multiple_of(#schema_path, __path.clone(), instance, MOF.clone())
                    }
                }
            } else if let Some(f) = value.as_f64() {
                quote! { jsonschema::keywords_helpers::error::multiple_of(#schema_path, __path.clone(), instance, serde_json::Value::from(#f)) }
            } else {
                return CompiledExpr::from(check_ts);
            }
        } else if let Some(f) = value.as_f64() {
            quote! { jsonschema::keywords_helpers::error::multiple_of(#schema_path, __path.clone(), instance, serde_json::Value::from(#f)) }
        } else {
            return CompiledExpr::from(check_ts);
        }
    };

    CompiledExpr::with_validate_blocks(
        check_ts.clone(),
        quote! { if !(#check_ts) { return Some(#err_expr); } },
        quote! { if !(#check_ts) { __errors.push(#err_expr); } },
    )
}

fn generate_multiple_of_check(value: &Value) -> TokenStream {
    if !value.is_number() {
        return invalid_schema_type_expression(value, &["number"]).into_token_stream();
    }
    if !is_strictly_positive_number(value) {
        return invalid_schema_exclusive_minimum_expression(value, "0").into_token_stream();
    }

    #[cfg(feature = "arbitrary-precision")]
    if let Value::Number(number) = value {
        if requires_ap_multiple_of_path(number) {
            let limit_literal = number.to_string();
            return quote! {
                jsonschema::keywords_helpers::numeric::check_compiled_multiple_of(
                    n,
                    #limit_literal
                )
            };
        }
    }

    if let Some(multiple) = value.as_f64() {
        if multiple.fract() == 0.0 {
            quote! { jsonschema::keywords_helpers::numeric::is_multiple_of_integer(n, #multiple) }
        } else {
            quote! { jsonschema::keywords_helpers::numeric::is_multiple_of_float(n, #multiple) }
        }
    } else {
        quote! { true }
    }
}

fn is_strictly_positive_number(value: &Value) -> bool {
    let Some(number) = value.as_number() else {
        return false;
    };
    if let Some(v) = number.as_u64() {
        return v > 0;
    }
    if let Some(v) = number.as_i64() {
        return v > 0;
    }
    if let Some(v) = number.as_f64() {
        return v > 0.0;
    }
    let raw = number.to_string();
    if raw.starts_with('-') {
        return false;
    }
    raw.split(['e', 'E'])
        .next()
        .is_some_and(|significand| significand.bytes().any(|b| b.is_ascii_digit() && b != b'0'))
}

#[cfg(feature = "arbitrary-precision")]
fn requires_ap_multiple_of_path(number: &serde_json::Number) -> bool {
    const MAX_SAFE_INTEGER: u64 = 1u64 << 53;
    if let Some(value) = number.as_u64() {
        return value > MAX_SAFE_INTEGER;
    }
    if let Some(value) = number.as_i64() {
        return value.unsigned_abs() > MAX_SAFE_INTEGER;
    }
    true
}

use super::super::{
    errors::{invalid_schema_exclusive_minimum_expression, invalid_schema_type_expression},
    CompileContext, CompiledExpr,
};
use proc_macro2::TokenStream;
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(ctx: &CompileContext<'_>, value: &Value) -> CompiledExpr {
    let check = generate_multiple_of_check(value);

    // If the check itself is an error expression (invalid schema), just return it as-is.
    if !value.is_number() || !is_strictly_positive_number(value) {
        return CompiledExpr::from_error(check);
    }

    let schema_path = ctx.schema_path_for_keyword("multipleOf");

    #[cfg(not(feature = "arbitrary-precision"))]
    let error_expr = {
        let divisor = value.as_f64().expect("multipleOf is a JSON number");
        quote! { jsonschema::__private::error::multiple_of(#schema_path, __path.into(), instance, #divisor) }
    };

    #[cfg(feature = "arbitrary-precision")]
    let error_expr = {
        let number = value.as_number().expect("multipleOf is a JSON number");
        // Preserve the divisor's JSON representation so the message matches the runtime:
        // an integer `multipleOf` renders as `2`, not `2.0`.
        let limit_value = if requires_arbitrary_precision_path(number) {
            let value_json = serde_json::to_string(value).unwrap();
            quote! {
                {
                    static MULTIPLE_OF: std::sync::LazyLock<serde_json::Value> =
                        std::sync::LazyLock::new(|| serde_json::from_str(#value_json).expect("multipleOf"));
                    MULTIPLE_OF.clone()
                }
            }
        } else {
            let unsigned = value
                .as_u64()
                .expect("multipleOf below the safe-integer bound is a positive u64");
            quote! { serde_json::Value::Number(serde_json::Number::from(#unsigned)) }
        };
        quote! { jsonschema::__private::error::multiple_of(#schema_path, __path.into(), instance, #limit_value) }
    };

    CompiledExpr::from_check_and_error(check, error_expr)
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
        if requires_arbitrary_precision_path(number) {
            let limit_literal = number.to_string();
            return quote! {
                jsonschema::__private::numeric::check_compiled_multiple_of(
                    n,
                    #limit_literal
                )
            };
        }
    }

    let divisor = value.as_f64().expect("multipleOf is a finite JSON number");
    if divisor.fract() == 0.0 {
        quote! { jsonschema::__private::numeric::is_multiple_of_integer(n, #divisor) }
    } else {
        quote! { jsonschema::__private::numeric::is_multiple_of_float(n, #divisor) }
    }
}

fn is_strictly_positive_number(value: &Value) -> bool {
    let number = value.as_number().expect("multipleOf is a JSON number");
    if let Some(unsigned) = number.as_u64() {
        return unsigned > 0;
    }
    if let Some(signed) = number.as_i64() {
        return signed > 0;
    }
    if let Some(float) = number.as_f64() {
        return float > 0.0;
    }
    let raw = number.to_string();
    if raw.starts_with('-') {
        return false;
    }
    raw.split(['e', 'E']).next().is_some_and(|significand| {
        significand
            .bytes()
            .any(|byte| byte.is_ascii_digit() && byte != b'0')
    })
}

#[cfg(feature = "arbitrary-precision")]
fn requires_arbitrary_precision_path(number: &serde_json::Number) -> bool {
    const MAX_SAFE_INTEGER: u64 = 1u64 << 53;
    if let Some(value) = number.as_u64() {
        return value > MAX_SAFE_INTEGER;
    }
    true
}

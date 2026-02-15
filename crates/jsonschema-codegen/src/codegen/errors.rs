use proc_macro2::TokenStream;
use quote::quote;
use serde_json::Value;

pub(super) fn invalid_regex_expression(keyword: &str, pattern: &str) -> TokenStream {
    let message = format!("Invalid `{keyword}` regular expression: {pattern}");
    invalid_schema_expression(&message)
}

pub(super) fn invalid_schema_expression(message: &str) -> TokenStream {
    quote! {{
        compile_error!(#message);
        false
    }}
}

pub(super) fn invalid_schema_unexpected_type_expression() -> TokenStream {
    invalid_schema_expression("Unexpected type")
}

pub(super) fn invalid_schema_non_empty_array_expression() -> TokenStream {
    invalid_schema_expression("[] has less than 1 item")
}

pub(super) fn invalid_schema_expected_string_keyword_expression(keyword: &str) -> TokenStream {
    let message = format!("Invalid `{keyword}`: expected a string");
    invalid_schema_expression(&message)
}

pub(super) fn invalid_schema_type_expression(
    value: &Value,
    expected_types: &[&str],
) -> TokenStream {
    let value_repr = value.to_string();
    let message = if expected_types.len() == 1 {
        let ty = expected_types[0];
        format!(r#"{value_repr} is not of type "{ty}""#)
    } else {
        let expected = expected_types
            .iter()
            .map(|ty| format!(r#""{ty}""#))
            .collect::<Vec<_>>()
            .join(", ");
        format!(r"{value_repr} is not of types {expected}")
    };
    invalid_schema_expression(&message)
}

pub(super) fn invalid_schema_minimum_expression(value: &Value, minimum: &str) -> TokenStream {
    let message = format!("{value} is less than the minimum of {minimum}");
    invalid_schema_expression(&message)
}

pub(super) fn invalid_schema_exclusive_minimum_expression(
    value: &Value,
    minimum: &str,
) -> TokenStream {
    let message = format!("{value} is less than or equal to the minimum of {minimum}");
    invalid_schema_expression(&message)
}

use super::super::errors::invalid_schema_type_expression;
use proc_macro2::TokenStream;
use quote::quote;
use serde_json::Value;

#[derive(Clone, Copy)]
pub(crate) enum ComparisonOp {
    Lt,
    Lte,
    Gt,
    Gte,
}

pub(crate) fn generate_numeric_check(op: ComparisonOp, limit: &Value) -> TokenStream {
    if !limit.is_number() {
        return invalid_schema_type_expression(limit, &["number"]).into_token_stream();
    }

    #[cfg(feature = "arbitrary-precision")]
    if let Value::Number(number) = limit {
        if number.as_u64().is_none() && number.as_i64().is_none() {
            let op_tag: u8 = match op {
                ComparisonOp::Lt => 0,
                ComparisonOp::Lte => 1,
                ComparisonOp::Gt => 2,
                ComparisonOp::Gte => 3,
            };
            let limit_literal = number.to_string();
            return quote! {
                jsonschema::keywords_helpers::numeric::check_compiled_bound(
                    n,
                    #op_tag,
                    #limit_literal
                )
            };
        }
    }

    let cmp_fn = match op {
        ComparisonOp::Lt => quote! { lt },
        ComparisonOp::Lte => quote! { le },
        ComparisonOp::Gt => quote! { gt },
        ComparisonOp::Gte => quote! { ge },
    };

    if let Some(u) = limit.as_u64() {
        quote! { jsonschema::keywords_helpers::numeric::#cmp_fn(n, #u as u64) }
    } else if let Some(i) = limit.as_i64() {
        quote! { jsonschema::keywords_helpers::numeric::#cmp_fn(n, #i as i64) }
    } else if let Some(f) = limit.as_f64() {
        quote! { jsonschema::keywords_helpers::numeric::#cmp_fn(n, #f as f64) }
    } else {
        quote! { true }
    }
}

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
    let Value::Number(number) = limit else {
        return invalid_schema_type_expression(limit, &["number"]).into_token_stream();
    };

    #[cfg(feature = "arbitrary-precision")]
    if number.as_u64().is_none() && number.as_i64().is_none() {
        let op_tag: u8 = match op {
            ComparisonOp::Lt => 0,
            ComparisonOp::Lte => 1,
            ComparisonOp::Gt => 2,
            ComparisonOp::Gte => 3,
        };
        let limit_literal = number.to_string();
        return quote! {
            jsonschema::__private::numeric::check_compiled_bound(
                n,
                #op_tag,
                #limit_literal
            )
        };
    }

    let cmp_fn = match op {
        ComparisonOp::Lt => quote! { lt },
        ComparisonOp::Lte => quote! { le },
        ComparisonOp::Gt => quote! { gt },
        ComparisonOp::Gte => quote! { ge },
    };

    if let Some(unsigned) = number.as_u64() {
        quote! { jsonschema::__private::numeric::#cmp_fn(n, #unsigned) }
    } else if let Some(signed) = number.as_i64() {
        quote! { jsonschema::__private::numeric::#cmp_fn(n, #signed) }
    } else {
        let float = number
            .as_f64()
            .expect("JSON number is representable as u64, i64, or f64");
        quote! { jsonschema::__private::numeric::#cmp_fn(n, #float) }
    }
}

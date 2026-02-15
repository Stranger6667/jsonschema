use crate::context::CompileContext;
use proc_macro2::TokenStream;
use quote::quote;
use serde_json::Value;

/// Compile the "const" keyword.
pub(in super::super) fn compile(ctx: &CompileContext<'_>, value: &Value) -> TokenStream {
    let backend = &ctx.config.backend;

    match value {
        // Scalar constants can use direct checks without constructing serde_json::Value.
        Value::Null => backend.instance_is_null(),
        Value::Bool(expected) => {
            let as_bool = backend.instance_as_bool();
            quote! { #as_bool == Some(#expected) }
        }
        Value::String(expected) => {
            let as_str = backend.instance_as_str();
            quote! { #as_str == Some(#expected) }
        }
        Value::Number(expected) => {
            let json = expected.to_string();
            let number_arm = backend
                .match_number_arm(quote! { jsonschema::ext::cmp::equal_numbers(n, &*EXPECTED) });
            quote! {
                {
                    static EXPECTED: std::sync::LazyLock<serde_json::Number> =
                        std::sync::LazyLock::new(|| {
                            serde_json::from_str(#json)
                                .expect("Failed to parse const number")
                        });
                    match instance {
                        #number_arm,
                        _ => false,
                    }
                }
            }
        }
        Value::Array(_) | Value::Object(_) => {
            let json = serde_json::to_string(value).expect("Failed to serialize const value");
            quote! {
                {
                    static EXPECTED: std::sync::LazyLock<serde_json::Value> =
                        std::sync::LazyLock::new(|| {
                            serde_json::from_str(#json)
                                .expect("Failed to parse const value")
                        });
                    jsonschema::ext::cmp::equal(instance, &*EXPECTED)
                }
            }
        }
    }
}

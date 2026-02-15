use proc_macro2::TokenStream;
use quote::quote;
use referencing::Draft;

use crate::codegen::symbols::EmitSymbols;

use super::{
    BackendAccessors, BackendIdentity, BackendMatchArms, BackendPatterns, BackendSymbols,
    BackendTypeChecks,
};

#[derive(Clone)]
pub(crate) struct SerdeJsonBackend;

impl BackendIdentity for SerdeJsonBackend {
    fn id(&self) -> &'static str {
        "serde_json"
    }
}

impl BackendSymbols for SerdeJsonBackend {
    fn emit_symbols(&self) -> EmitSymbols {
        EmitSymbols::serde_json()
    }
}

impl BackendTypeChecks for SerdeJsonBackend {
    fn instance_is_string(&self) -> TokenStream {
        quote! { instance.is_string() }
    }

    fn instance_is_number(&self) -> TokenStream {
        quote! { instance.is_number() }
    }

    fn instance_is_boolean(&self) -> TokenStream {
        quote! { instance.is_boolean() }
    }

    fn instance_is_null(&self) -> TokenStream {
        quote! { instance.is_null() }
    }

    fn instance_is_array(&self) -> TokenStream {
        quote! { instance.is_array() }
    }

    fn instance_is_object(&self) -> TokenStream {
        quote! { instance.is_object() }
    }

    fn instance_as_bool(&self) -> TokenStream {
        quote! { instance.as_bool() }
    }

    fn instance_as_str(&self) -> TokenStream {
        quote! { instance.as_str() }
    }

    fn integer_number_guard(&self, draft: Draft) -> TokenStream {
        if matches!(draft, Draft::Draft4) {
            quote! { n.is_i64() || n.is_u64() }
        } else {
            quote! { n.is_i64() || n.is_u64() || n.as_f64().is_some_and(|f| f.fract() == 0.0) }
        }
    }

    fn instance_is_integer(&self, draft: Draft) -> TokenStream {
        if matches!(draft, Draft::Draft4) {
            quote! {
                match instance {
                    serde_json::Value::Number(n) => n.is_i64() || n.is_u64(),
                    _ => false
                }
            }
        } else {
            quote! {
                match instance {
                    serde_json::Value::Number(n) => {
                        n.is_i64() || n.is_u64()
                            || n.as_f64().is_some_and(|f| f.fract() == 0.0)
                    }
                    _ => false
                }
            }
        }
    }
}

impl BackendMatchArms for SerdeJsonBackend {
    fn match_string_arm(&self, body: TokenStream) -> TokenStream {
        quote! { serde_json::Value::String(s) => { #body } }
    }

    fn match_number_arm(&self, body: TokenStream) -> TokenStream {
        quote! { serde_json::Value::Number(n) => { #body } }
    }

    fn match_boolean_arm(&self, body: TokenStream) -> TokenStream {
        quote! { serde_json::Value::Bool(b) => { #body } }
    }

    fn match_integer_arm(&self, guard: TokenStream, body: TokenStream) -> TokenStream {
        quote! { serde_json::Value::Number(n) if #guard => { #body } }
    }

    fn match_array_arm(&self, body: TokenStream) -> TokenStream {
        quote! { serde_json::Value::Array(arr) => { #body } }
    }

    fn match_object_arm(&self, body: TokenStream) -> TokenStream {
        quote! { serde_json::Value::Object(obj) => { #body } }
    }
}

impl BackendAccessors for SerdeJsonBackend {
    fn string_as_str(&self, string_expr: TokenStream) -> TokenStream {
        quote! { #string_expr.as_str() }
    }

    fn array_len(&self, array_expr: TokenStream) -> TokenStream {
        quote! { #array_expr.len() }
    }

    fn object_len(&self, object_expr: TokenStream) -> TokenStream {
        quote! { #object_expr.len() }
    }

    fn object_contains_key(&self, object_expr: TokenStream, key: &str) -> TokenStream {
        quote! { #object_expr.contains_key(#key) }
    }

    fn object_iter_all(&self, object_expr: TokenStream, body: TokenStream) -> TokenStream {
        quote! {
            #object_expr.iter().all(|(key, instance)| {
                #body
            })
        }
    }

    fn key_as_str(&self, key_expr: TokenStream) -> TokenStream {
        quote! { #key_expr.as_str() }
    }

    fn key_as_value_ref(&self, key_expr: TokenStream) -> TokenStream {
        quote! { &serde_json::Value::String(#key_expr.clone()) }
    }

    fn instance_object_property_as_str(&self, key: &str) -> TokenStream {
        quote! {
            match instance {
                serde_json::Value::Object(obj) => obj.get(#key).and_then(serde_json::Value::as_str),
                _ => None,
            }
        }
    }
}

impl BackendPatterns for SerdeJsonBackend {
    fn pattern_string(&self) -> TokenStream {
        quote! { serde_json::Value::String(_) }
    }

    fn pattern_number(&self) -> TokenStream {
        quote! { serde_json::Value::Number(_) }
    }

    fn pattern_number_binding(&self) -> TokenStream {
        quote! { serde_json::Value::Number(n) }
    }

    fn pattern_integer(&self, guard: TokenStream) -> TokenStream {
        quote! { serde_json::Value::Number(n) if #guard }
    }

    fn pattern_array(&self) -> TokenStream {
        quote! { serde_json::Value::Array(_) }
    }

    fn pattern_object(&self) -> TokenStream {
        quote! { serde_json::Value::Object(_) }
    }

    fn pattern_boolean(&self) -> TokenStream {
        quote! { serde_json::Value::Bool(_) }
    }

    fn pattern_null(&self) -> TokenStream {
        quote! { serde_json::Value::Null }
    }
}

//! `serde_json`-targeted token-stream emitters used by every keyword module.
#![allow(clippy::needless_pass_by_value)]

use proc_macro2::{Ident, TokenStream};
use quote::{quote, ToTokens};
use referencing::Draft;

use super::emit::ValueEmitter;

pub(crate) struct SerdeEmitter;

impl ValueEmitter for SerdeEmitter {
    #[inline]
    fn node_param(lifetime: Option<TokenStream>) -> TokenStream {
        if let Some(lifetime) = lifetime {
            quote! { &#lifetime __Value }
        } else {
            quote! { &__Value }
        }
    }

    fn map_param() -> TokenStream {
        quote! { &__Map }
    }

    fn array_param() -> TokenStream {
        quote! { &[__Value] }
    }

    fn instance_is_string() -> TokenStream {
        quote! { instance.is_string() }
    }

    fn instance_is_number() -> TokenStream {
        quote! { instance.is_number() }
    }

    fn instance_is_boolean() -> TokenStream {
        quote! { instance.is_boolean() }
    }

    fn instance_is_null() -> TokenStream {
        quote! { instance.is_null() }
    }

    fn instance_is_array() -> TokenStream {
        quote! { instance.is_array() }
    }

    fn instance_is_object() -> TokenStream {
        quote! { instance.is_object() }
    }

    fn instance_as_bool() -> TokenStream {
        quote! { instance.as_bool() }
    }

    fn instance_as_str() -> TokenStream {
        quote! { instance.as_str() }
    }

    // Integer checks delegate to runtime helpers: under `arbitrary-precision`,
    // integer-valued numbers outside the i64/u64/f64 range must classify exactly
    // like the runtime validator.
    fn integer_number_guard(draft: Draft) -> TokenStream {
        if matches!(draft, Draft::Draft4) {
            quote! { __types::is_integer_draft4(n) }
        } else {
            quote! { __types::is_integer(n) }
        }
    }

    fn instance_is_integer(draft: Draft) -> TokenStream {
        let guard = Self::integer_number_guard(draft);
        quote! {
            match instance {
                __Value::Number(n) => #guard,
                _ => false
            }
        }
    }

    fn match_string_arm(body: impl ToTokens) -> TokenStream {
        quote! { __Value::String(s) => { #body } }
    }

    fn match_number_arm(body: impl ToTokens) -> TokenStream {
        quote! { __Value::Number(n) => { #body } }
    }

    fn match_boolean_arm(body: impl ToTokens) -> TokenStream {
        quote! { __Value::Bool(b) => { #body } }
    }

    fn match_integer_arm(guard: impl ToTokens, body: impl ToTokens) -> TokenStream {
        quote! { __Value::Number(n) if #guard => { #body } }
    }

    fn match_array_arm(body: impl ToTokens) -> TokenStream {
        quote! { __Value::Array(arr) => { #body } }
    }

    fn match_object_arm(body: impl ToTokens) -> TokenStream {
        quote! { __Value::Object(obj) => { #body } }
    }

    fn string_as_str(string_expr: impl ToTokens) -> TokenStream {
        quote! { #string_expr.as_str() }
    }

    fn array_len(array_expr: impl ToTokens) -> TokenStream {
        quote! { #array_expr.len() }
    }

    fn array_get(array_expr: impl ToTokens, index: usize) -> TokenStream {
        quote! { #array_expr.get(#index) }
    }

    fn array_iter(array_expr: impl ToTokens) -> TokenStream {
        quote! { #array_expr.iter() }
    }

    fn object_len(object_expr: impl ToTokens) -> TokenStream {
        quote! { #object_expr.len() }
    }

    fn object_contains_key(object_expr: impl ToTokens, key: &str) -> TokenStream {
        let key = Self::declare_key(key);
        quote! { #object_expr.contains_key(#key) }
    }

    fn object_iter_all(object_expr: impl ToTokens, body: impl ToTokens) -> TokenStream {
        quote! {
            #object_expr.iter().all(|(key, instance)| {
                #body
            })
        }
    }

    fn object_get(object_expr: impl ToTokens, key: &str) -> TokenStream {
        let key = Self::declare_key(key);
        quote! { #object_expr.get(#key) }
    }

    fn object_iter_entries(object_expr: impl ToTokens) -> TokenStream {
        quote! { #object_expr.iter() }
    }

    fn object_keys_iter(object_expr: impl ToTokens) -> TokenStream {
        quote! { #object_expr.keys() }
    }

    fn object_keys_all_strings(keys_iter: impl ToTokens, body: impl ToTokens) -> TokenStream {
        quote! { #keys_iter.all(|s| { #body }) }
    }

    // `propertyNames` validates each name as an instance; serde has to build one.
    fn declare_key_node(key_expr: impl ToTokens) -> TokenStream {
        quote! { let __key_val: serde_json::Value = serde_json::Value::String(#key_expr.clone()); }
    }

    fn key_node_expr(_key_expr: impl ToTokens) -> TokenStream {
        quote! { &__key_val }
    }

    fn key_as_str(key_expr: impl ToTokens) -> TokenStream {
        quote! { #key_expr.as_str() }
    }

    fn key_as_value_ref(key_expr: impl ToTokens) -> TokenStream {
        quote! { &__Value::String(#key_expr.clone()) }
    }

    fn instance_object_property_as_str(key: &str) -> TokenStream {
        quote! {
            match instance {
                __Value::Object(obj) => obj.get(#key).and_then(__Value::as_str),
                _ => None,
            }
        }
    }

    fn instance_object_property_as_bool(key: &str) -> TokenStream {
        quote! {
            match instance {
                __Value::Object(obj) => obj.get(#key).and_then(__Value::as_bool),
                _ => None,
            }
        }
    }

    // `const: 1` matches `1.0`, so integral floats within i64 range must
    // normalize to the same discriminator value as their integer spelling.
    fn instance_object_property_as_i64(key: &str) -> TokenStream {
        quote! {
            match instance {
                __Value::Object(obj) => obj.get(#key).and_then(|value| {
                    value.as_i64().or_else(|| {
                        value.as_f64().and_then(|float| {
                            (float.fract() == 0.0
                                && float >= -9_223_372_036_854_775_808.0_f64
                                && float < 9_223_372_036_854_775_808.0_f64)
                                .then_some(float as i64)
                        })
                    })
                }),
                _ => None,
            }
        }
    }

    fn pattern_string() -> TokenStream {
        quote! { __Value::String(_) }
    }

    fn pattern_number() -> TokenStream {
        quote! { __Value::Number(_) }
    }

    fn pattern_number_binding() -> TokenStream {
        quote! { __Value::Number(n) }
    }

    fn pattern_integer(guard: impl ToTokens) -> TokenStream {
        quote! { __Value::Number(n) if #guard }
    }

    fn pattern_array() -> TokenStream {
        quote! { __Value::Array(_) }
    }

    fn pattern_object() -> TokenStream {
        quote! { __Value::Object(_) }
    }

    fn pattern_boolean() -> TokenStream {
        quote! { __Value::Bool(_) }
    }

    fn pattern_null() -> TokenStream {
        quote! { __Value::Null }
    }

    fn object_get_dynamic(object_expr: impl ToTokens, key_expr: impl ToTokens) -> TokenStream {
        quote! { #object_expr.get(#key_expr) }
    }

    fn object_is_empty(object_expr: impl ToTokens) -> TokenStream {
        quote! { #object_expr.is_empty() }
    }

    fn object_values_iter(object_expr: impl ToTokens) -> TokenStream {
        quote! { #object_expr.values() }
    }

    fn array_iter_ref(array_expr: impl ToTokens) -> TokenStream {
        array_expr.into_token_stream()
    }

    fn public_value_ty(_runtime_crate: &TokenStream, _lifetime: impl ToTokens) -> TokenStream {
        quote! { serde_json::Value }
    }

    fn entry_points(impl_mod_name: &Ident, runtime_crate: &TokenStream) -> TokenStream {
        let anonymous = Self::public_value_ty(runtime_crate, quote! { '_ });
        let borrowed = Self::public_value_ty(runtime_crate, quote! { '__i });
        quote! {
            pub fn is_valid(instance: &#anonymous) -> bool {
                #impl_mod_name::is_valid(instance)
            }

            pub fn validate<'__i>(
                instance: &'__i #borrowed,
            ) -> ::std::result::Result<(), #runtime_crate::ValidationError<'__i>> {
                match #impl_mod_name::validate(instance, &#runtime_crate::paths::LazyLocation::new()) {
                    Some(e) => Err(e),
                    None => Ok(()),
                }
            }

            pub fn iter_errors<'__i>(
                instance: &'__i #borrowed,
            ) -> #runtime_crate::ErrorIterator<'__i> {
                let mut errors = Vec::new();
                #impl_mod_name::collect_errors(instance, &#runtime_crate::paths::LazyLocation::new(), &mut errors);
                #runtime_crate::__private::error::iterator_from(errors)
            }
        }
    }

    fn declare_key(key: &str) -> TokenStream {
        quote! { #key }
    }

    fn key_to_owned(key_expr: impl ToTokens) -> TokenStream {
        quote! { #key_expr.clone() }
    }

    fn module_prelude() -> TokenStream {
        quote! {
            use serde_json::Value as __Value;
            use std::sync::LazyLock as __Lazy;
            type __Map = serde_json::Map<String, __Value>;
        }
    }

    fn function_prelude() -> TokenStream {
        TokenStream::new()
    }

    fn type_match(scrutinee: impl ToTokens, arms: Vec<TokenStream>) -> TokenStream {
        quote! { match #scrutinee { #(#arms)* } }
    }

    fn type_matches(patterns: Vec<TokenStream>) -> TokenStream {
        quote! { matches!(instance, #(#patterns)|*) }
    }

    fn array_is_unique(array_expr: impl ToTokens) -> TokenStream {
        quote! { jsonschema::__private::unique_items::is_unique(#array_expr) }
    }

    fn instance_equals_value(expected_expr: impl ToTokens) -> TokenStream {
        quote! { jsonschema::__private::cmp::equal(instance, #expected_expr) }
    }

    fn value_equals_instance(value_expr: impl ToTokens) -> TokenStream {
        quote! { jsonschema::__private::cmp::equal(#value_expr, instance) }
    }

    fn key_as_string_subject(key_expr: impl ToTokens) -> TokenStream {
        key_expr.into_token_stream()
    }

    fn if_object(instance_expr: impl ToTokens, body: impl ToTokens) -> TokenStream {
        quote! { if let __Value::Object(obj) = #instance_expr { #body } }
    }

    fn if_array(instance_expr: impl ToTokens, body: impl ToTokens) -> TokenStream {
        quote! { if let __Value::Array(arr) = #instance_expr { #body } }
    }

    fn node_to_json_string(instance_expr: impl ToTokens) -> TokenStream {
        quote! { #instance_expr.to_string() }
    }

    fn node_address(instance_expr: impl ToTokens) -> TokenStream {
        quote! { std::ptr::from_ref(#instance_expr) as usize }
    }

    fn err_instance(instance_expr: impl ToTokens) -> TokenStream {
        instance_expr.into_token_stream()
    }
}

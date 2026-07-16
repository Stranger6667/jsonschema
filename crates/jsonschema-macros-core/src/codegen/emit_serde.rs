//! `serde_json`-targeted token-stream emitters used by every keyword module.
#![allow(clippy::needless_pass_by_value)]

use proc_macro2::TokenStream;
use quote::quote;
use referencing::Draft;

#[inline]
pub(crate) fn value_ty() -> TokenStream {
    quote! { __Value }
}

pub(crate) fn map_ty() -> TokenStream {
    quote! { __Map }
}

pub(crate) fn value_slice_ty() -> TokenStream {
    quote! { [__Value] }
}

pub(crate) fn instance_is_string() -> TokenStream {
    quote! { instance.is_string() }
}

pub(crate) fn instance_is_number() -> TokenStream {
    quote! { instance.is_number() }
}

pub(crate) fn instance_is_boolean() -> TokenStream {
    quote! { instance.is_boolean() }
}

pub(crate) fn instance_is_null() -> TokenStream {
    quote! { instance.is_null() }
}

pub(crate) fn instance_is_array() -> TokenStream {
    quote! { instance.is_array() }
}

pub(crate) fn instance_is_object() -> TokenStream {
    quote! { instance.is_object() }
}

pub(crate) fn instance_as_bool() -> TokenStream {
    quote! { instance.as_bool() }
}

pub(crate) fn instance_as_str() -> TokenStream {
    quote! { instance.as_str() }
}

// Integer checks delegate to runtime helpers: under `arbitrary-precision`,
// integer-valued numbers outside the i64/u64/f64 range must classify exactly
// like the runtime validator.
pub(crate) fn integer_number_guard(draft: Draft) -> TokenStream {
    if matches!(draft, Draft::Draft4) {
        quote! { __types::is_integer_draft4(n) }
    } else {
        quote! { __types::is_integer(n) }
    }
}

pub(crate) fn instance_is_integer(draft: Draft) -> TokenStream {
    let guard = integer_number_guard(draft);
    quote! {
        match instance {
            __Value::Number(n) => #guard,
            _ => false
        }
    }
}

pub(crate) fn match_string_arm(body: TokenStream) -> TokenStream {
    quote! { __Value::String(s) => { #body } }
}

pub(crate) fn match_number_arm(body: TokenStream) -> TokenStream {
    quote! { __Value::Number(n) => { #body } }
}

pub(crate) fn match_boolean_arm(body: TokenStream) -> TokenStream {
    quote! { __Value::Bool(b) => { #body } }
}

pub(crate) fn match_integer_arm(guard: TokenStream, body: TokenStream) -> TokenStream {
    quote! { __Value::Number(n) if #guard => { #body } }
}

pub(crate) fn match_array_arm(body: TokenStream) -> TokenStream {
    quote! { __Value::Array(arr) => { #body } }
}

pub(crate) fn match_object_arm(body: TokenStream) -> TokenStream {
    quote! { __Value::Object(obj) => { #body } }
}

pub(crate) fn string_as_str(string_expr: TokenStream) -> TokenStream {
    quote! { #string_expr.as_str() }
}

pub(crate) fn array_len(array_expr: TokenStream) -> TokenStream {
    quote! { #array_expr.len() }
}

pub(crate) fn object_len(object_expr: TokenStream) -> TokenStream {
    quote! { #object_expr.len() }
}

pub(crate) fn object_contains_key(object_expr: TokenStream, key: &str) -> TokenStream {
    quote! { #object_expr.contains_key(#key) }
}

pub(crate) fn object_iter_all(object_expr: TokenStream, body: TokenStream) -> TokenStream {
    quote! {
        #object_expr.iter().all(|(key, instance)| {
            #body
        })
    }
}

pub(crate) fn key_as_str(key_expr: TokenStream) -> TokenStream {
    quote! { #key_expr.as_str() }
}

pub(crate) fn key_as_value_ref(key_expr: TokenStream) -> TokenStream {
    quote! { &__Value::String(#key_expr.clone()) }
}

pub(crate) fn instance_object_property_as_str(key: &str) -> TokenStream {
    quote! {
        match instance {
            __Value::Object(obj) => obj.get(#key).and_then(__Value::as_str),
            _ => None,
        }
    }
}

pub(crate) fn instance_object_property_as_bool(key: &str) -> TokenStream {
    quote! {
        match instance {
            __Value::Object(obj) => obj.get(#key).and_then(__Value::as_bool),
            _ => None,
        }
    }
}

pub(crate) fn instance_object_property_as_i64(key: &str) -> TokenStream {
    // `const: 1` matches `1.0`, so integral floats within i64 range must
    // normalize to the same discriminator value as their integer spelling.
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

pub(crate) fn pattern_string() -> TokenStream {
    quote! { __Value::String(_) }
}

pub(crate) fn pattern_number() -> TokenStream {
    quote! { __Value::Number(_) }
}

pub(crate) fn pattern_number_binding() -> TokenStream {
    quote! { __Value::Number(n) }
}

pub(crate) fn pattern_integer(guard: TokenStream) -> TokenStream {
    quote! { __Value::Number(n) if #guard }
}

pub(crate) fn pattern_array() -> TokenStream {
    quote! { __Value::Array(_) }
}

pub(crate) fn pattern_object() -> TokenStream {
    quote! { __Value::Object(_) }
}

pub(crate) fn pattern_boolean() -> TokenStream {
    quote! { __Value::Bool(_) }
}

pub(crate) fn pattern_null() -> TokenStream {
    quote! { __Value::Null }
}

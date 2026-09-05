//! PyO3-targeted token-stream emitters: emitted validators read native Python objects.
#![allow(clippy::needless_pass_by_value)]

use proc_macro2::{Ident, TokenStream};
use quote::{quote, ToTokens};
use referencing::Draft;

use super::emit::ValueEmitter;

pub(crate) struct Pyo3Emitter;

impl ValueEmitter for Pyo3Emitter {
    fn node_param(lifetime: Option<TokenStream>) -> TokenStream {
        if let Some(lifetime) = lifetime {
            quote! { __Value<#lifetime> }
        } else {
            quote! { __Value<'_> }
        }
    }

    fn map_param() -> TokenStream {
        quote! { __Map<'_> }
    }

    fn array_param() -> TokenStream {
        quote! { __Array<'_> }
    }

    fn instance_is_string() -> TokenStream {
        quote! { __NodeExt::is_string(&instance) }
    }

    fn instance_is_number() -> TokenStream {
        quote! { __NodeExt::is_number(&instance) }
    }

    fn instance_is_boolean() -> TokenStream {
        quote! { (__NodeExt::json_type(&instance) == __JT::Boolean) }
    }

    fn instance_is_null() -> TokenStream {
        quote! { __NodeExt::is_null(&instance) }
    }

    fn instance_is_array() -> TokenStream {
        quote! { (__NodeExt::json_type(&instance) == __JT::Array) }
    }

    fn instance_is_object() -> TokenStream {
        quote! { (__NodeExt::json_type(&instance) == __JT::Object) }
    }

    fn instance_as_bool() -> TokenStream {
        quote! { __NodeExt::as_boolean(&instance) }
    }

    fn instance_as_str() -> TokenStream {
        quote! { __NodeExt::as_string(&instance).as_deref() }
    }

    // Type dispatch matches on `JsonType`, so no number is bound in the pattern and the
    // guard has to read one itself.
    fn integer_number_guard(draft: Draft) -> TokenStream {
        if matches!(draft, Draft::Draft4) {
            quote! {
                __NodeExt::as_number(&instance)
                    .is_some_and(|__number| __NumberExt::is_written_as_integer(&__number))
            }
        } else {
            quote! {
                __NodeExt::as_number(&instance)
                    .is_some_and(|__number| __NumberExt::is_integer(&__number))
            }
        }
    }

    fn instance_is_integer(draft: Draft) -> TokenStream {
        Self::integer_number_guard(draft)
    }

    // Every arm rebinds the accessor's subject. An accessor can still decline where the type
    // says otherwise -- an unreadable string, an object of an unsupported type -- and records
    // that on the side, so the declining arm yields the type's zero and the recorded error
    // outranks it at the entry point.
    fn match_string_arm(body: impl ToTokens) -> TokenStream {
        quote! {
            __JT::String => match __NodeExt::as_string(&instance) {
                Some(__text) => { let s: &str = &__text; #body }
                None => Default::default(),
            }
        }
    }

    fn match_number_arm(body: impl ToTokens) -> TokenStream {
        quote! {
            __JT::Number => match __NodeExt::as_number(&instance) {
                Some(__number) => {
                    let __number = __NumberExt::to_number(&__number);
                    let n = &*__number;
                    #body
                }
                None => Default::default(),
            }
        }
    }

    fn match_boolean_arm(body: impl ToTokens) -> TokenStream {
        quote! {
            __JT::Boolean => match __NodeExt::as_boolean(&instance) {
                Some(__boolean) => { let b = &__boolean; #body }
                None => Default::default(),
            }
        }
    }

    fn match_integer_arm(guard: impl ToTokens, body: impl ToTokens) -> TokenStream {
        quote! {
            __JT::Number if #guard => match __NodeExt::as_number(&instance) {
                Some(__number) => {
                    let __number = __NumberExt::to_number(&__number);
                    let n = &*__number;
                    #body
                }
                None => Default::default(),
            }
        }
    }

    fn match_array_arm(body: impl ToTokens) -> TokenStream {
        quote! {
            __JT::Array => match __json::narrow_array(instance) {
                Some(arr) => { #body }
                None => Default::default(),
            }
        }
    }

    fn match_object_arm(body: impl ToTokens) -> TokenStream {
        quote! {
            __JT::Object => match __json::narrow_object(instance) {
                Some(obj) => { #body }
                None => Default::default(),
            }
        }
    }

    fn string_as_str(string_expr: impl ToTokens) -> TokenStream {
        quote! { (&*#string_expr) }
    }

    fn array_len(array_expr: impl ToTokens) -> TokenStream {
        quote! { __ArrayExt::len(&#array_expr) }
    }

    fn array_get(array_expr: impl ToTokens, index: usize) -> TokenStream {
        quote! { __ArrayExt::elements(&#array_expr).nth(#index) }
    }

    fn array_iter(array_expr: impl ToTokens) -> TokenStream {
        quote! { __ArrayExt::elements(&#array_expr) }
    }

    fn object_len(object_expr: impl ToTokens) -> TokenStream {
        quote! { __ObjectExt::len(&#object_expr) }
    }

    fn object_contains_key(object_expr: impl ToTokens, key: &str) -> TokenStream {
        let key = Self::declare_key(key);
        quote! { __ObjectExt::get(&#object_expr, #key).is_some() }
    }

    fn object_iter_all(object_expr: impl ToTokens, body: impl ToTokens) -> TokenStream {
        quote! {
            __ObjectExt::members(&#object_expr).all(|(key, instance)| {
                #body
            })
        }
    }

    fn object_get(object_expr: impl ToTokens, key: &str) -> TokenStream {
        let key = Self::declare_key(key);
        quote! { __ObjectExt::get(&#object_expr, #key) }
    }

    fn object_iter_entries(object_expr: impl ToTokens) -> TokenStream {
        quote! { __ObjectExt::members(&#object_expr) }
    }

    fn object_keys_iter(object_expr: impl ToTokens) -> TokenStream {
        quote! { __ObjectExt::members(&#object_expr).map(|(__name, _)| __name) }
    }

    fn object_keys_all_strings(keys_iter: impl ToTokens, body: impl ToTokens) -> TokenStream {
        quote! { #keys_iter.all(|__name| { let s: &str = &__name; #body }) }
    }

    fn declare_key_node(key_expr: impl ToTokens) -> TokenStream {
        quote! {
            let __key_val = __py3::PyString::new(__py, &#key_expr)
                .into_any();
        }
    }

    fn key_node_expr(_key_expr: impl ToTokens) -> TokenStream {
        quote! { __key_val.as_borrowed() }
    }

    fn key_as_str(key_expr: impl ToTokens) -> TokenStream {
        quote! { (&*#key_expr) }
    }

    fn key_as_value_ref(key_expr: impl ToTokens) -> TokenStream {
        quote! {
            __py3::PyString::new(__py, &#key_expr)
                .into_any()
                .as_borrowed()
        }
    }

    // A Python string node always borrows its text, so the borrowed arm is the whole domain
    // and a discriminator read this way is a `&str` for the lifetime of the instance.
    fn instance_object_property_as_str(key: &str) -> TokenStream {
        let key = Self::declare_key(key);
        quote! {
            __NodeExt::as_object(&instance)
                .and_then(|__object| __ObjectExt::get(&__object, #key))
                .and_then(|__value| match __NodeExt::as_string(&__value) {
                    Some(::std::borrow::Cow::Borrowed(__text)) => Some(__text),
                    _ => None,
                })
        }
    }

    fn instance_object_property_as_bool(key: &str) -> TokenStream {
        let key = Self::declare_key(key);
        quote! {
            __NodeExt::as_object(&instance)
                .and_then(|__object| __ObjectExt::get(&__object, #key))
                .and_then(|__value| __NodeExt::as_boolean(&__value))
        }
    }

    // `const: 1` matches `1.0`, so integral floats within i64 range must
    // normalize to the same discriminator value as their integer spelling.
    fn instance_object_property_as_i64(key: &str) -> TokenStream {
        let key = Self::declare_key(key);
        quote! {
            __NodeExt::as_object(&instance)
                .and_then(|__object| __ObjectExt::get(&__object, #key))
                .and_then(|__value| __NodeExt::as_number(&__value))
                .and_then(|__number| {
                    __NumberExt::as_i64(&__number).or_else(|| {
                        __NumberExt::as_f64(&__number).and_then(|float| {
                            (float.fract() == 0.0
                                && float >= -9_223_372_036_854_775_808.0_f64
                                && float < 9_223_372_036_854_775_808.0_f64)
                                .then_some(float as i64)
                        })
                    })
                })
        }
    }

    fn pattern_string() -> TokenStream {
        quote! { __JT::String }
    }

    fn pattern_number() -> TokenStream {
        quote! { __JT::Number }
    }

    fn pattern_number_binding() -> TokenStream {
        quote! { __JT::Number }
    }

    fn pattern_integer(guard: impl ToTokens) -> TokenStream {
        quote! { __JT::Number if #guard }
    }

    fn pattern_array() -> TokenStream {
        quote! { __JT::Array }
    }

    fn pattern_object() -> TokenStream {
        quote! { __JT::Object }
    }

    fn pattern_boolean() -> TokenStream {
        quote! { __JT::Boolean }
    }

    fn pattern_null() -> TokenStream {
        quote! { __JT::Null }
    }

    // Only a name held at compile time is interned; a name read from the instance is compared
    // as text.
    fn object_get_dynamic(object_expr: impl ToTokens, key_expr: impl ToTokens) -> TokenStream {
        quote! {
            __ObjectExt::members(&#object_expr)
                .find(|(__name, _)| &**__name == #key_expr)
                .map(|(_, __value)| __value)
        }
    }

    fn object_is_empty(object_expr: impl ToTokens) -> TokenStream {
        quote! { __ObjectExt::is_empty(&#object_expr) }
    }

    fn object_values_iter(object_expr: impl ToTokens) -> TokenStream {
        quote! { __json::object_values(#object_expr) }
    }

    fn array_iter_ref(array_expr: impl ToTokens) -> TokenStream {
        quote! { __ArrayExt::elements(&#array_expr) }
    }

    fn public_value_ty(runtime_crate: &TokenStream, lifetime: impl ToTokens) -> TokenStream {
        quote! {
            #runtime_crate::__private::pyo3::Bound<#lifetime, #runtime_crate::__private::pyo3::PyAny>
        }
    }

    // Reading a Python value can fail in ways the accessors cannot report, so each entry point
    // runs inside a scope that collects what was recorded. A recorded error outranks the run's
    // own result: building a validation error can itself reach an unreadable value, and that
    // error is the accurate one.
    fn entry_bodies() -> TokenStream {
        quote! {
            pub(super) fn entry_is_valid(instance: &__Bound<'_>) -> __py3::PyResult<bool> {
                let _scope = __json::PendingErrorScope::enter();
                __json::probe_root(instance.as_borrowed());
                if let Some(error) = __json::take_pending_error() {
                    return Err(error);
                }
                let result = is_valid(instance.as_borrowed());
                if let Some(error) = __json::take_pending_error() {
                    return Err(error);
                }
                Ok(result)
            }

            pub(super) fn entry_validate<'__i>(
                instance: &'__i __Bound<'__i>,
            ) -> __py3::PyResult<::std::result::Result<(), __VE<'__i>>> {
                let _scope = __json::PendingErrorScope::enter();
                __json::probe_root(instance.as_borrowed());
                if let Some(error) = __json::take_pending_error() {
                    return Err(error);
                }
                let result = match validate(
                    instance.as_borrowed(),
                    &__paths::LazyLocation::new(),
                ) {
                    Some(e) => Err(e),
                    None => Ok(()),
                };
                if let Some(error) = __json::take_pending_error() {
                    return Err(error);
                }
                Ok(result)
            }

            pub(super) fn entry_iter_errors<'__i>(
                instance: &'__i __Bound<'__i>,
            ) -> __py3::PyResult<__EI<'__i>> {
                let _scope = __json::PendingErrorScope::enter();
                __json::probe_root(instance.as_borrowed());
                if let Some(error) = __json::take_pending_error() {
                    return Err(error);
                }
                let mut errors = Vec::new();
                collect_errors(
                    instance.as_borrowed(),
                    &__paths::LazyLocation::new(),
                    &mut errors,
                );
                if let Some(error) = __json::take_pending_error() {
                    return Err(error);
                }
                Ok(__err::iterator_from(errors))
            }
        }
    }

    fn entry_points(impl_mod_name: &Ident, runtime_crate: &TokenStream) -> TokenStream {
        let anonymous = Self::public_value_ty(runtime_crate, quote! { '_ });
        let borrowed = Self::public_value_ty(runtime_crate, quote! { '__i });
        quote! {
            pub fn is_valid(
                instance: &#anonymous,
            ) -> #runtime_crate::__private::pyo3::PyResult<bool> {
                #impl_mod_name::entry_is_valid(instance)
            }

            pub fn validate<'__i>(
                instance: &'__i #borrowed,
            ) -> #runtime_crate::__private::pyo3::PyResult<
                ::std::result::Result<(), #runtime_crate::ValidationError<'__i>>,
            > {
                #impl_mod_name::entry_validate(instance)
            }

            pub fn iter_errors<'__i>(
                instance: &'__i #borrowed,
            ) -> #runtime_crate::__private::pyo3::PyResult<#runtime_crate::ErrorIterator<'__i>> {
                #impl_mod_name::entry_iter_errors(instance)
            }
        }
    }

    fn declare_key(key: &str) -> TokenStream {
        quote! { __py3::intern!(__py, #key).as_unbound() }
    }

    fn key_to_owned(key_expr: impl ToTokens) -> TokenStream {
        quote! { #key_expr.to_string() }
    }

    fn module_prelude() -> TokenStream {
        quote! {
            use jsonschema::__private::pyo3 as __py3;
            use jsonschema::json as __json;
            use jsonschema::json::{
                Array as __ArrayExt, JsonNumber as __NumberExt, Node as __NodeExt,
                Object as __ObjectExt,
            };
            use std::sync::LazyLock as __Lazy;
            type __Json = __json::Pyo3;
            type __Value<'py> = __py3::Borrowed<'py, 'py, __py3::PyAny>;
            type __Bound<'py> = __py3::Bound<'py, __py3::PyAny>;
            type __Map<'py> = <__Value<'py> as __NodeExt<'py, __Json>>::Object;
            type __Array<'py> = <__Value<'py> as __NodeExt<'py, __Json>>::Array;
        }
    }

    fn function_prelude() -> TokenStream {
        quote! { let __py = instance.py(); }
    }

    fn sole_array_match(body: TokenStream, fallback: TokenStream) -> TokenStream {
        quote! {
            match __json::narrow_array(instance) {
                Some(arr) => { #body }
                None => #fallback
            }
        }
    }

    fn sole_object_match(body: TokenStream, fallback: TokenStream) -> TokenStream {
        quote! {
            match __json::narrow_object(instance) {
                Some(obj) => { #body }
                None => #fallback
            }
        }
    }

    fn type_match(scrutinee: impl ToTokens, arms: Vec<TokenStream>) -> TokenStream {
        quote! { match __NodeExt::json_type(&#scrutinee) { #(#arms)* } }
    }

    fn type_matches(patterns: Vec<TokenStream>) -> TokenStream {
        quote! { matches!(__NodeExt::json_type(&instance), #(#patterns)|*) }
    }

    fn array_is_unique(array_expr: impl ToTokens) -> TokenStream {
        quote! { __ArrayExt::is_unique(&#array_expr) }
    }

    fn instance_equals_value(expected_expr: impl ToTokens) -> TokenStream {
        quote! { __NodeExt::equals_value(&instance, #expected_expr) }
    }

    fn value_equals_instance(value_expr: impl ToTokens) -> TokenStream {
        quote! { __NodeExt::equals_value(&instance, #value_expr) }
    }

    fn key_as_string_subject(key_expr: impl ToTokens) -> TokenStream {
        quote! { (&*#key_expr) }
    }

    fn if_object(instance_expr: impl ToTokens, body: impl ToTokens) -> TokenStream {
        quote! { if let Some(obj) = __NodeExt::as_object(&#instance_expr) { #body } }
    }

    fn if_array(instance_expr: impl ToTokens, body: impl ToTokens) -> TokenStream {
        quote! { if let Some(arr) = __NodeExt::as_array(&#instance_expr) { #body } }
    }

    fn node_to_json_string(instance_expr: impl ToTokens) -> TokenStream {
        quote! { __NodeExt::to_value(&#instance_expr).to_string() }
    }

    fn node_address(instance_expr: impl ToTokens) -> TokenStream {
        quote! { #instance_expr.as_ptr() as usize }
    }

    fn err_instance(instance_expr: impl ToTokens) -> TokenStream {
        quote! { __NodeExt::lazy_value(&#instance_expr) }
    }
}

//! Emitter for validators that read Ruby objects.
#![allow(clippy::needless_pass_by_value)]

use proc_macro2::{Ident, TokenStream};
use quote::{quote, ToTokens};
use referencing::Draft;

use super::emit::ValueEmitter;
use crate::context::MethodGates;

pub(crate) struct MagnusEmitter;

// A member name reaches its text through `AsRef<str>`.
fn name_str(name: impl ToTokens) -> TokenStream {
    quote! { ::std::convert::AsRef::<str>::as_ref(&#name) }
}

impl ValueEmitter for MagnusEmitter {
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

    // `JsonType` dispatch binds no number, so the guard reads one.
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

    // A declining accessor records an error and the arm yields the type's zero; the recorded
    // error wins at the entry point.
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

    // `json_type` already read the tag, so the accessor is one more tag test.
    fn match_array_arm(body: impl ToTokens) -> TokenStream {
        quote! {
            __JT::Array => match __NodeExt::as_array(&instance) {
                Some(arr) => { #body }
                None => Default::default(),
            }
        }
    }

    fn match_object_arm(body: impl ToTokens) -> TokenStream {
        quote! {
            __JT::Object => match __NodeExt::as_object(&instance) {
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
        let text = name_str(quote! { __name });
        quote! { #keys_iter.all(|__name| { let s: &str = #text; #body }) }
    }

    fn declare_key_node(key_expr: impl ToTokens) -> TokenStream {
        let text = name_str(key_expr);
        quote! { let __key_val = __json::magnus_string_node(#text); }
    }

    fn key_node_expr(_key_expr: impl ToTokens) -> TokenStream {
        quote! { __key_val }
    }

    fn key_as_str(key_expr: impl ToTokens) -> TokenStream {
        let text = name_str(key_expr);
        quote! { (#text) }
    }

    fn key_as_value_ref(key_expr: impl ToTokens) -> TokenStream {
        let text = name_str(key_expr);
        quote! { __json::magnus_string_node(#text) }
    }

    // Ruby strings borrow when their bytes are already UTF-8, so the discriminator lives as long
    // as the instance; anything else declines the fast path.
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

    // `const: 1` matches `1.0`: integral floats map to the integer they equal.
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

    fn object_get_dynamic(object_expr: impl ToTokens, key_expr: impl ToTokens) -> TokenStream {
        let text = name_str(quote! { __name });
        quote! {
            __ObjectExt::members(&#object_expr)
                .find(|(__name, _)| #text == #key_expr)
                .map(|(_, __value)| __value)
        }
    }

    fn object_is_empty(object_expr: impl ToTokens) -> TokenStream {
        quote! { __ObjectExt::is_empty(&#object_expr) }
    }

    fn object_values_iter(object_expr: impl ToTokens) -> TokenStream {
        quote! { __json::magnus_object_values(#object_expr) }
    }

    fn array_iter_ref(array_expr: impl ToTokens) -> TokenStream {
        quote! { __ArrayExt::elements(&#array_expr) }
    }

    // A `Value` carries no lifetime; the entry points borrow it to anchor the errors.
    fn public_value_ty(runtime_crate: &TokenStream, _lifetime: impl ToTokens) -> TokenStream {
        quote! { #runtime_crate::__private::magnus::Value }
    }

    // Accessors record read failures in a scope; a recorded error outranks the run's result. The
    // members snapshot is dropped first, since Ruby may have mutated the object since the last call.
    fn entry_bodies(methods: MethodGates) -> TokenStream {
        let is_valid = methods.is_valid.then(|| {
            quote! {
                pub(super) fn entry_is_valid(
                    instance: &__rb::Value,
                ) -> ::std::result::Result<bool, __rb::Error> {
                    let _scope = __json::MagnusPendingErrorScope::enter();
                    __json::magnus_invalidate_members_cache();
                    let node = __json::RbNode::new(__rb::AsRawValue::as_raw(*instance));
                    __json::magnus_probe_root(node);
                    if let Some(error) = __json::magnus_take_pending_error() {
                        return Err(error.into());
                    }
                    let result = is_valid(node);
                    if let Some(error) = __json::magnus_take_pending_error() {
                        return Err(error.into());
                    }
                    Ok(result)
                }
            }
        });
        let validate = methods.validate.then(|| {
            quote! {
                pub(super) fn entry_validate<'__i>(
                    instance: &'__i __rb::Value,
                ) -> ::std::result::Result<::std::result::Result<(), __VE<'__i>>, __rb::Error> {
                    let _scope = __json::MagnusPendingErrorScope::enter();
                    __json::magnus_invalidate_members_cache();
                    let node = __json::RbNode::new(__rb::AsRawValue::as_raw(*instance));
                    __json::magnus_probe_root(node);
                    if let Some(error) = __json::magnus_take_pending_error() {
                        return Err(error.into());
                    }
                    let result = match validate(node, &__paths::LazyLocation::new()) {
                        Some(e) => Err(e),
                        None => Ok(()),
                    };
                    if let Some(error) = __json::magnus_take_pending_error() {
                        return Err(error.into());
                    }
                    Ok(result)
                }
            }
        });
        let iter_errors = methods.iter_errors.then(|| {
            quote! {
                pub(super) fn entry_iter_errors<'__i>(
                    instance: &'__i __rb::Value,
                ) -> ::std::result::Result<__EI<'__i>, __rb::Error> {
                    let _scope = __json::MagnusPendingErrorScope::enter();
                    __json::magnus_invalidate_members_cache();
                    let node = __json::RbNode::new(__rb::AsRawValue::as_raw(*instance));
                    __json::magnus_probe_root(node);
                    if let Some(error) = __json::magnus_take_pending_error() {
                        return Err(error.into());
                    }
                    let mut errors = Vec::new();
                    collect_errors(node, &__paths::LazyLocation::new(), &mut errors);
                    if let Some(error) = __json::magnus_take_pending_error() {
                        return Err(error.into());
                    }
                    Ok(__err::iterator_from(errors))
                }
            }
        });
        quote! {
            #is_valid
            #validate
            #iter_errors
        }
    }

    fn entry_points(
        impl_mod_name: &Ident,
        runtime_crate: &TokenStream,
        methods: MethodGates,
    ) -> TokenStream {
        let value = Self::public_value_ty(runtime_crate, quote! { '_ });
        let is_valid = methods.is_valid.then(|| {
            quote! {
                pub fn is_valid(
                    instance: &#value,
                ) -> ::std::result::Result<bool, #runtime_crate::__private::magnus::Error> {
                    #impl_mod_name::entry_is_valid(instance)
                }
            }
        });
        let validate = methods.validate.then(|| {
            quote! {
                pub fn validate<'__i>(
                    instance: &'__i #value,
                ) -> ::std::result::Result<
                    ::std::result::Result<(), #runtime_crate::ValidationError<'__i>>,
                    #runtime_crate::__private::magnus::Error,
                > {
                    #impl_mod_name::entry_validate(instance)
                }
            }
        });
        let iter_errors = methods.iter_errors.then(|| {
            quote! {
                pub fn iter_errors<'__i>(
                    instance: &'__i #value,
                ) -> ::std::result::Result<
                    #runtime_crate::ErrorIterator<'__i>,
                    #runtime_crate::__private::magnus::Error,
                > {
                    #impl_mod_name::entry_iter_errors(instance)
                }
            }
        });
        quote! {
            #is_valid
            #validate
            #iter_errors
        }
    }

    // One interned key per site; the global cache behind `prepare_key` is locked once.
    fn declare_key(key: &str) -> TokenStream {
        quote! {
            {
                static __KEY: __Lazy<<__Json as __JsonExt>::PreparedKey> =
                    __Lazy::new(|| <__Json as __JsonExt>::prepare_key(#key));
                &*__KEY
            }
        }
    }

    fn key_to_owned(key_expr: impl ToTokens) -> TokenStream {
        let text = name_str(key_expr);
        quote! { #text.to_owned() }
    }

    fn module_prelude() -> TokenStream {
        quote! {
            use jsonschema::__private::magnus as __rb;
            use jsonschema::json as __json;
            use jsonschema::json::{
                Array as __ArrayExt, Json as __JsonExt, JsonNumber as __NumberExt,
                Node as __NodeExt, Object as __ObjectExt,
            };
            use std::sync::LazyLock as __Lazy;
            type __Json = __json::Magnus;
            type __Value<'a> = __json::RbNode<'a>;
            type __Map<'a> = <__Value<'a> as __NodeExt<'a, __Json>>::Object;
            type __Array<'a> = <__Value<'a> as __NodeExt<'a, __Json>>::Array;
        }
    }

    fn function_prelude() -> TokenStream {
        quote! {}
    }

    fn sole_array_match(body: TokenStream, fallback: TokenStream) -> TokenStream {
        quote! {
            match __NodeExt::as_array(&instance) {
                Some(arr) => { #body }
                None => #fallback
            }
        }
    }

    fn sole_object_match(body: TokenStream, fallback: TokenStream) -> TokenStream {
        quote! {
            match __NodeExt::as_object(&instance) {
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
        let text = name_str(key_expr);
        quote! { (#text) }
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
        quote! { (__json::RbNode::raw(#instance_expr) as usize) }
    }

    fn err_instance(instance_expr: impl ToTokens) -> TokenStream {
        quote! { __NodeExt::lazy_value(&#instance_expr) }
    }

    fn json_representation() -> TokenStream {
        quote! { __Json }
    }
}

//! Vocabulary for turning IR pieces into token streams, one method per JSON representation
//! detail a keyword module needs. Each JSON backend implements this once.
use proc_macro2::{Ident, TokenStream};
use quote::ToTokens;
use referencing::Draft;

pub(crate) trait ValueEmitter {
    /// The type a node is passed by. `serde_json` passes a reference into the document; a
    /// representation whose handle is itself a borrow passes that handle by value.
    fn node_param(lifetime: Option<TokenStream>) -> TokenStream;
    fn map_param() -> TokenStream;
    fn array_param() -> TokenStream;
    fn instance_is_string() -> TokenStream;
    fn instance_is_number() -> TokenStream;
    fn instance_is_boolean() -> TokenStream;
    fn instance_is_null() -> TokenStream;
    fn instance_is_array() -> TokenStream;
    fn instance_is_object() -> TokenStream;
    fn instance_as_bool() -> TokenStream;
    fn instance_as_str() -> TokenStream;
    fn integer_number_guard(draft: Draft) -> TokenStream;
    fn instance_is_integer(draft: Draft) -> TokenStream;
    fn match_string_arm(body: impl ToTokens) -> TokenStream;
    fn match_number_arm(body: impl ToTokens) -> TokenStream;
    fn match_boolean_arm(body: impl ToTokens) -> TokenStream;
    fn match_integer_arm(guard: impl ToTokens, body: impl ToTokens) -> TokenStream;
    fn match_array_arm(body: impl ToTokens) -> TokenStream;
    fn match_object_arm(body: impl ToTokens) -> TokenStream;
    fn string_as_str(string_expr: impl ToTokens) -> TokenStream;
    fn array_len(array_expr: impl ToTokens) -> TokenStream;
    fn array_get(array_expr: impl ToTokens, index: usize) -> TokenStream;
    fn array_iter(array_expr: impl ToTokens) -> TokenStream;
    fn object_len(object_expr: impl ToTokens) -> TokenStream;
    fn object_contains_key(object_expr: impl ToTokens, key: &str) -> TokenStream;
    fn object_iter_all(object_expr: impl ToTokens, body: impl ToTokens) -> TokenStream;
    fn object_get(object_expr: impl ToTokens, key: &str) -> TokenStream;
    fn object_iter_entries(object_expr: impl ToTokens) -> TokenStream;
    fn object_keys_iter(object_expr: impl ToTokens) -> TokenStream;
    /// Whether `body` holds for every property name, with the name bound as `s`.
    fn object_keys_all_strings(keys_iter: impl ToTokens, body: impl ToTokens) -> TokenStream;
    fn declare_key_node(key_expr: impl ToTokens) -> TokenStream;
    fn key_node_expr(key_expr: impl ToTokens) -> TokenStream;
    fn key_as_str(key_expr: impl ToTokens) -> TokenStream;
    fn key_as_value_ref(key_expr: impl ToTokens) -> TokenStream;
    fn instance_object_property_as_str(key: &str) -> TokenStream;
    fn instance_object_property_as_bool(key: &str) -> TokenStream;
    fn instance_object_property_as_i64(key: &str) -> TokenStream;
    fn pattern_string() -> TokenStream;
    fn pattern_number() -> TokenStream;
    fn pattern_number_binding() -> TokenStream;
    fn pattern_integer(guard: impl ToTokens) -> TokenStream;
    fn pattern_array() -> TokenStream;
    fn pattern_object() -> TokenStream;
    fn pattern_boolean() -> TokenStream;
    fn pattern_null() -> TokenStream;
    fn object_get_dynamic(object_expr: impl ToTokens, key_expr: impl ToTokens) -> TokenStream;
    fn object_is_empty(object_expr: impl ToTokens) -> TokenStream;
    fn object_values_iter(object_expr: impl ToTokens) -> TokenStream;
    fn array_iter_ref(array_expr: impl ToTokens) -> TokenStream;
    fn public_value_ty(runtime_crate: &TokenStream, lifetime: impl ToTokens) -> TokenStream;
    /// The `is_valid`/`validate`/`iter_errors` methods on the validator struct. A representation
    /// that records unreadable values out of band wraps each one and returns them.
    fn entry_points(impl_mod_name: &Ident, runtime_crate: &TokenStream) -> TokenStream;
    fn declare_key(key: &str) -> TokenStream;
    fn key_to_owned(key_expr: impl ToTokens) -> TokenStream;
    /// Imports and aliases the emitted module opens with, and the statement every emitted
    /// function starts from.
    fn module_prelude() -> TokenStream;
    fn function_prelude() -> TokenStream;
    fn type_match(scrutinee: impl ToTokens, arms: Vec<TokenStream>) -> TokenStream;
    /// Type dispatch with no per-type body: whether the instance has one of `patterns`' types.
    fn type_matches(patterns: Vec<TokenStream>) -> TokenStream;
    fn array_is_unique(array_expr: impl ToTokens) -> TokenStream;
    /// Whether the instance equals a value fixed by the schema.
    fn instance_equals_value(expected_expr: impl ToTokens) -> TokenStream;
    /// The same comparison with the schema's value as the left operand; JSON equality is
    /// symmetric, and the two call sites read in opposite directions.
    fn value_equals_instance(value_expr: impl ToTokens) -> TokenStream;
    /// A property name bound as the `s` subject the string keywords read.
    fn key_as_string_subject(key_expr: impl ToTokens) -> TokenStream;
    /// `body` with the instance narrowed to an object bound as `obj`, or nothing.
    fn if_object(instance_expr: impl ToTokens, body: impl ToTokens) -> TokenStream;
    /// `body` with the instance narrowed to an array bound as `arr`, or nothing.
    fn if_array(instance_expr: impl ToTokens, body: impl ToTokens) -> TokenStream;
    /// A node rendered as JSON text, for the names an error lists.
    fn node_to_json_string(instance_expr: impl ToTokens) -> TokenStream;
    /// What tells one live node from another, for the `$ref` cycle marks.
    fn node_address(instance_expr: impl ToTokens) -> TokenStream;
    fn err_instance(instance_expr: impl ToTokens) -> TokenStream;
}

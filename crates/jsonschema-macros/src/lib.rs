use proc_macro::TokenStream;

/// Generates a compile-time JSON Schema validator.
///
/// Reads and compiles the schema at compile time, generating inherent methods on the
/// annotated struct:
///
/// - `pub fn is_valid(instance: &serde_json::Value) -> bool`
/// - `pub fn validate(instance: &Value) -> Result<(), jsonschema::ValidationError<'_>>`
/// - `pub fn iter_errors(instance: &Value) -> jsonschema::ErrorIterator<'_>`
/// - `pub fn evaluate(instance: &Value) -> jsonschema::Evaluation`
///
/// Schema errors are emitted as regular Rust compile errors.
///
/// # Required attribute
///
/// Exactly one schema source must be provided:
///
/// - `path = "relative/or/absolute/path/to/schema.json"`
/// - `schema = r#"{"type":"string"}"#`
///
/// # Optional attributes
///
/// - `draft = Draft4|Draft6|Draft7|Draft201909|Draft202012` (a qualified path like
///   `jsonschema::Draft::Draft7` is accepted; only the final segment is inspected)
/// - `base_uri = "json-schema:///root/main.json"`
/// - `resources = { "uri" => { schema = r#"..."# }, "uri2" => { path = "..." } }`
/// - `validate_formats = true|false`
/// - `ignore_unknown_formats = true|false` (default: `true`)
/// - `formats = { "name" => crate::path::to::format_fn }`
/// - `keywords = { "name" => crate::path::to::keyword_factory_fn }` (same factory signature as
///   `ValidationOptions::with_keyword`; a failing factory panics at the validator's first use,
///   and overriding built-in keywords is not supported)
/// - `content_media_types = { "name" => crate::path::to::check_fn }`
/// - `content_encodings = { "name" => { check = crate::path::to::check_fn, convert = crate::path::to::convert_fn } }`
///   (same fn signatures as `ValidationOptions::{with_content_media_type,with_content_encoding}`)
/// - `email_options = { ... }`
/// - `pattern_options = { ... }`
///
/// `email_options` keys:
///
/// - `minimum_sub_domains = <usize>`
/// - `no_minimum_sub_domains = <bool>`
/// - `required_tld = <bool>`
/// - `allow_domain_literal = <bool>`
/// - `allow_display_text = <bool>`
///
/// `pattern_options` keys:
///
/// - `engine = fancy_regex|regex`
/// - `backtrack_limit = <usize>` (only for `fancy_regex`)
/// - `size_limit = <usize>`
/// - `dfa_size_limit = <usize>`
///
/// # Example
///
/// ```ignore
/// // Inline schema
/// #[jsonschema::validator(schema = r#"{"maxLength": 5}"#)]
/// struct Short;
///
/// // Or load it from a file:
/// // #[jsonschema::validator(path = "schema.json")]
///
/// let instance = serde_json::json!("value");
/// assert!(Short::is_valid(&instance));
/// Short::validate(&instance)?;
/// ```
///
/// With options:
///
/// ```ignore
/// #[jsonschema::validator(
///     path = "schema.json",
///     draft = Draft202012,
///     validate_formats = true,
///     formats = {
///         "currency" => crate::formats::is_currency,
///     },
///     pattern_options = {
///         engine = regex,
///         size_limit = 1_000_000,
///         dfa_size_limit = 2_000_000,
///     }
/// )]
/// struct Validator;
/// ```
#[proc_macro_attribute]
pub fn validator(attr: TokenStream, item: TokenStream) -> TokenStream {
    jsonschema_macros_core::expand(attr.into(), item.into()).into()
}

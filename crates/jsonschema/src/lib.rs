#![allow(clippy::unnecessary_wraps)]
// Runtime helpers stay `pub` for the macros-gated `__private` re-exports; without `macros` they are crate-internal.
#![cfg_attr(not(feature = "macros"), allow(unreachable_pub))]
//! A high-performance JSON Schema validator for Rust.
//!
//! - 📚 Support for popular JSON Schema drafts
//! - 🔧 Custom keywords and format validators
//! - ⚡ [Compile-time validators](#compile-time-validator-macro), also for
//!   [Python](#python-extension-modules) and [Ruby](#ruby-extension-modules) extension modules
//! - 🌐 Blocking & non-blocking remote reference fetching (network/file)
//! - 🎨 Structured Output v1 reports (flag/list/hierarchical)
//! - ✨ Meta-schema validation for schema documents
//! - 🧮 Schema canonicalization (experimental; see the [`canonical`] module)
//! - 🧩 Validation of custom in-memory JSON representations
//! - 🚀 WebAssembly support
//!
//! ## Supported drafts
//!
//! - ![Draft 2020-12](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft2020-12.json)
//! - ![Draft 2019-09](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft2019-09.json)
//! - ![Draft 7](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft7.json)
//! - ![Draft 6](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft6.json)
//! - ![Draft 4](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft4.json)
//!
//! # Validation
//!
//! The `jsonschema` crate offers two main approaches to validation: one-off validation and reusable validators.
//! When external references are involved, the validator can be constructed using either blocking or non-blocking I/O.
//!
//!
//! For simple use cases where you need to validate an instance against a schema once, use [`is_valid`] or [`validate`] functions:
//!
//! ```rust
//! use serde_json::json;
//!
//! let schema = json!({"type": "string"});
//! let instance = json!("Hello, world!");
//!
//! assert!(jsonschema::is_valid(&schema, &instance));
//! assert!(jsonschema::validate(&schema, &instance).is_ok());
//! ```
//!
//! Build a validator once to validate many instances; with external references, build it
//! blocking or non-blocking:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use serde_json::json;
//!
//! let schema = json!({"type": "string"});
//! // Blocking construction - will fetch external references synchronously
//! let validator = jsonschema::validator_for(&schema)?;
//! // Non-blocking construction - will fetch external references asynchronously
//! # #[cfg(feature = "resolve-async")]
//! # async fn async_example() -> Result<(), Box<dyn std::error::Error>> {
//! # let schema = json!({"type": "string"});
//! let validator = jsonschema::async_validator_for(&schema).await?;
//! # Ok(())
//! # }
//!
//!  // Once constructed, validation is always synchronous as it works with in-memory data
//! assert!(validator.is_valid(&json!("Hello, world!")));
//! assert!(!validator.is_valid(&json!(42)));
//! assert!(validator.validate(&json!(42)).is_err());
//!
//! // Iterate over all errors
//! let instance = json!(42);
//! for error in validator.iter_errors(&instance) {
//!     eprintln!("Error: {}", error);
//!     eprintln!("Location: {}", error.instance_path());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Note on `format` keyword
//!
//! Drafts 4, 6 and 7 check `format` by default; 2019-09 and 2020-12 only annotate. To check it in
//! any draft:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # use serde_json::json;
//! #
//! # let schema = json!({"type": "string"});
//! let validator = jsonschema::draft202012::options()
//!     .should_validate_formats(true)
//!     .build(&schema)?;
//! # Ok(())
//! # }
//! ```
//!
//! The validator then checks every known `format`, using the chosen draft's definitions.
//!
//! `idn-hostname` and `idn-email` need the `idna` feature, which is on by default. Without it they
//! are treated as unknown formats.
//!
//! # Reading errors
//!
//! [`ErrorIterator::into_errors`] collects the whole set into a [`ValidationErrors`], which
//! implements `Display` and [`std::error::Error`], so reporting every failure takes no formatting
//! loop of its own:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use serde_json::json;
//!
//! let validator = jsonschema::validator_for(&json!({
//!     "type": "object",
//!     "properties": {"n": {"minimum": 5}},
//!     "required": ["name"]
//! }))?;
//! let instance = json!({"n": 1});
//! let errors = validator.iter_errors(&instance).into_errors();
//!
//! assert_eq!(errors.len(), 2);
//! // Validation errors:
//! // 01: "name" is a required property
//! // 02: /n: 1 is less than the minimum of 5
//! println!("{errors}");
//! # Ok(())
//! # }
//! ```
//!
//! [`ValidationError::kind`] carries the failed keyword's operands, so a caller does not need to
//! re-read the schema to describe what was expected:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use serde_json::json;
//! use jsonschema::error::ValidationErrorKind;
//!
//! let validator = jsonschema::validator_for(&json!({"minimum": 5}))?;
//! let instance = json!(1);
//! let error = validator.validate(&instance).expect_err("must be invalid");
//!
//! match error.kind() {
//!     ValidationErrorKind::Minimum { limit } => assert_eq!(limit, &json!(5)),
//!     other => panic!("unexpected kind: {other:?}"),
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [`ValidationError::absolute_keyword_location`] gives the resolvable URI of the keyword that
//! produced the error, and [`ValidationError::masked`] hides the instance value in the message.
//!
//! # Compile-Time Validator Macro
//!
//! The `validator` attribute macro (`macros` feature) compiles a schema at build time into a
//! validator that runs faster than a runtime `Validator`; prefer it when the schema is known at
//! build time.
//!
//! ```ignore
//! // Inline schema
//! #[jsonschema::validator(schema = r#"{"maxLength": 5}"#)]
//! struct Short;
//!
//! // Or load it from a file:
//! // #[jsonschema::validator(path = "schema.json")]
//!
//! let instance = serde_json::json!("value");
//! assert!(Short::is_valid(&instance));
//! assert!(Short::validate(&instance).is_ok());
//! assert!(Short::evaluate(&instance).is_valid());
//! ```
//!
//! Supported macro attributes; each mirrors the matching [`ValidationOptions`] builder method,
//! which documents its behavior and defaults:
//!
//! - schema source (exactly one required): `path = "..."` (file) or `schema = r#"..."#` (inline)
//! - `backend = SerdeJson|Pyo3|Magnus` (default: `SerdeJson`), the representation the generated
//!   validator reads; `Pyo3` needs the `pyo3` feature (see
//!   [Python Extension Modules](#python-extension-modules)) and `Magnus` the `magnus` feature (see
//!   [Ruby Extension Modules](#ruby-extension-modules))
//! - `draft = Draft202012` (or another variant) -> [`ValidationOptions::with_draft`]
//! - `base_uri = "..."` -> [`ValidationOptions::with_base_uri`]
//! - `resources = { "<uri>" => { schema = r#"..."# } | { path = "..." } }`
//! - `vocabularies = ["<uri>"]` -> [`ValidationOptions::with_vocabulary`]
//! - `validate_formats = true|false` -> [`ValidationOptions::should_validate_formats`]
//! - `ignore_unknown_formats = true|false` -> [`ValidationOptions::should_ignore_unknown_formats`]
//! - `formats = { "name" => crate::path::to::fn }` -> [`ValidationOptions::with_format`]
//! - `keywords = { "name" => crate::path::to::factory }` -> [`ValidationOptions::with_keyword`]
//! - `content_media_types = { "type" => crate::path::to::fn }` -> [`ValidationOptions::with_content_media_type`]
//! - `content_encodings = { "name" => { check = ..., convert = ... } }` -> [`ValidationOptions::with_content_encoding`]
//! - `pattern_options = { ... }` -> [`PatternOptions`]
//! - `email_options = { ... }` -> [`EmailOptions`]
//! - `methods = { is_valid = true|false, validate = ..., iter_errors = ..., evaluate = ... }`,
//!   which methods to generate (all default to `true`; at least one must stay enabled)
//!
//! ## Python Extension Modules
//!
//! With `backend = Pyo3`, the generated validator reads Python objects in place, for extension
//! modules that know their schemas at build time. The methods take `&Bound<'_, PyAny>` and return
//! `PyResult<...>`, which is an error when the instance holds a value with no JSON counterpart,
//! such as a `set`:
//!
//! ```ignore
//! use pyo3::prelude::*;
//!
//! #[jsonschema::validator(path = "event.json", backend = Pyo3)]
//! struct Event;
//!
//! #[pyfunction]
//! fn is_valid_event(instance: &Bound<'_, PyAny>) -> PyResult<bool> {
//!     Event::is_valid(instance)
//! }
//! ```
//!
//! A `keywords` factory for this backend returns `Box<dyn for<'i> Keyword<'i, json::Pyo3>>`. A
//! complete extension with its build and test commands lives in
//! [`examples/pyo3-extension`](https://github.com/Stranger6667/jsonschema/tree/master/examples/pyo3-extension).
//!
//! Building the extension with `PyO3`'s `abi3` features limits it to the stable Python API, where
//! list and tuple elements are read through function calls; a build per Python version reads them
//! in place, which is faster on array-heavy instances.
//!
//! ## Ruby Extension Modules
//!
//! With `backend = Magnus`, the generated validator reads Ruby objects in place, for extension
//! modules that know their schemas at build time. The methods take `&magnus::Value` and return
//! `Result<..., magnus::Error>`, which is an error when the instance holds a value with no JSON
//! counterpart, such as a `Regexp`. They must run on a thread holding the GVL, as any Ruby call:
//!
//! ```ignore
//! use magnus::{function, Error, Ruby, Value};
//!
//! #[jsonschema::validator(path = "event.json", backend = Magnus)]
//! struct Event;
//!
//! fn valid_event(instance: Value) -> Result<bool, Error> {
//!     Event::is_valid(&instance)
//! }
//!
//! #[magnus::init]
//! fn init(ruby: &Ruby) -> Result<(), Error> {
//!     ruby.define_global_function("valid_event?", function!(valid_event, 1));
//!     Ok(())
//! }
//! ```
//!
//! A `keywords` factory for this backend returns `Box<dyn for<'i> Keyword<'i, json::Magnus>>`. A
//! complete extension with its build and test commands lives in
//! [`examples/magnus-extension`](https://github.com/Stranger6667/jsonschema/tree/master/examples/magnus-extension).
//!
//! ## Limitations
//!
//! - A `keywords` entry cannot take a built-in keyword's name: the macro rejects it, while
//!   [`ValidationOptions::with_keyword`] replaces the built-in.
//!
//! # Structured Output
//!
//! The `evaluate()` method provides access to structured validation output formats defined by
//! [JSON Schema Output v1](https://github.com/json-schema-org/json-schema-spec/blob/main/specs/output/jsonschema-validation-output-machines.md).
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use serde_json::json;
//!
//! let schema = json!({
//!     "type": "object",
//!     "properties": {
//!         "name": {"type": "string"},
//!         "age": {"type": "number", "minimum": 0}
//!     },
//!     "required": ["name"]
//! });
//!
//! let validator = jsonschema::validator_for(&schema)?;
//! let instance = json!({"name": "Alice", "age": 30});
//!
//! // Evaluate the instance
//! let evaluation = validator.evaluate(&instance);
//!
//! // Flag format: Simple boolean validity
//! let flag = evaluation.flag();
//! assert!(flag.valid);
//!
//! // List format: Flat list of all evaluation steps
//! let list_output = serde_json::to_value(evaluation.list())?;
//! println!("List output: {}", serde_json::to_string_pretty(&list_output)?);
//!
//! // Hierarchical format: Nested tree structure
//! let hierarchical_output = serde_json::to_value(evaluation.hierarchical())?;
//! println!(
//!     "Hierarchical output: {}",
//!     serde_json::to_string_pretty(&hierarchical_output)?
//! );
//!
//! // Iterate over annotations collected during validation
//! for annotation in evaluation.iter_annotations() {
//!     println!("Annotation at {}: {:?}",
//!         annotation.instance_location,
//!         annotation.annotations
//!     );
//! }
//!
//! // Iterate over errors (if any)
//! for error in evaluation.iter_errors() {
//!     println!("Error: {}", error.error);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [`Evaluation::list`] shows the full list output for an example schema.
//!
//! ## Output Formats
//!
//! ### Flag Format
//!
//! The simplest format, containing only a boolean validity indicator:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # use serde_json::json;
//! # let schema = json!({"type": "string"});
//! # let validator = jsonschema::validator_for(&schema)?;
//! let evaluation = validator.evaluate(&json!("hello"));
//! let flag = evaluation.flag();
//!
//! let output = serde_json::to_value(flag)?;
//! // Output: {"valid": true}
//! # Ok(())
//! # }
//! ```
//!
//! ### List Format
//!
//! A flat list of all evaluation units, where each unit describes a validation step:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # use serde_json::json;
//! let schema = json!({
//!     "allOf": [
//!         {"type": "number"},
//!         {"minimum": 0}
//!     ]
//! });
//! let validator = jsonschema::validator_for(&schema)?;
//! let evaluation = validator.evaluate(&json!(42));
//!
//! let list = evaluation.list();
//! let output = serde_json::to_value(list)?;
//! // Output includes all evaluation steps in a flat array
//! # Ok(())
//! # }
//! ```
//!
//! ### Hierarchical Format
//!
//! A nested tree structure that mirrors the schema's logical structure:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # use serde_json::json;
//! let schema = json!({
//!     "allOf": [
//!         {"type": "number"},
//!         {"minimum": 0}
//!     ]
//! });
//! let validator = jsonschema::validator_for(&schema)?;
//! let evaluation = validator.evaluate(&json!(42));
//!
//! let hierarchical = evaluation.hierarchical();
//! let output = serde_json::to_value(hierarchical)?;
//! // Output has nested "details" arrays for sub-schema evaluations
//! # Ok(())
//! # }
//! ```
//!
//! # Meta-Schema Validation
//!
//! [`meta`] checks a schema document against its meta-schema:
//!
//! ```rust
//! use serde_json::json;
//!
//! let schema = json!({
//!     "type": "object",
//!     "properties": {
//!         "name": {"type": "string"},
//!         "age": {"type": "integer", "minimum": 0}
//!     }
//! });
//!
//! // Validate schema with automatic draft detection
//! assert!(jsonschema::meta::is_valid(&schema));
//! assert!(jsonschema::meta::validate(&schema).is_ok());
//!
//! // Invalid schema example
//! let invalid_schema = json!({
//!     "type": "invalid_type",  // must be one of the valid JSON Schema types
//!     "minimum": "not_a_number"
//! });
//! assert!(!jsonschema::meta::is_valid(&invalid_schema));
//! assert!(jsonschema::meta::validate(&invalid_schema).is_err());
//! ```
//!
//! # Configuration
//!
//! ## Draft-specific Modules
//!
//! The library offers modules for specific JSON Schema draft versions:
//!
//! - [`draft4`]
//! - [`draft6`]
//! - [`draft7`]
//! - [`draft201909`]
//! - [`draft202012`]
//!
//! Each module provides:
//! - A `new` function to create a validator
//! - An `is_valid` function for validation with a boolean result
//! - An `validate` function for getting the first validation error
//! - An `options` function to create a draft-specific configuration builder
//! - A `meta` module for draft-specific meta-schema validation
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use serde_json::json;
//!
//! let schema = json!({"type": "string"});
//!
//! // Instance validation
//! let validator = jsonschema::draft7::new(&schema)?;
//! assert!(validator.is_valid(&json!("Hello")));
//!
//! // Meta-schema validation
//! assert!(jsonschema::draft7::meta::is_valid(&schema));
//! # Ok(())
//! # }
//! ```
//!
//! Each draft module also has one-off `is_valid` and `validate`:
//!
//! ```rust
//! use serde_json::json;
//!
//! let schema = json!({"type": "number", "minimum": 0});
//! let instance = json!(42);
//!
//! assert!(jsonschema::draft202012::is_valid(&schema, &instance));
//! assert!(jsonschema::draft202012::validate(&schema, &instance).is_ok());
//! ```
//!
//! For more advanced configuration, you can use the draft-specific `options` function:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use serde_json::json;
//!
//! let schema = json!({"type": "string", "format": "ends-with-42"});
//! let validator = jsonschema::draft202012::options()
//!     .with_format("ends-with-42", |s| s.ends_with("42"))
//!     .should_validate_formats(true)
//!     .build(&schema)?;
//!
//! assert!(validator.is_valid(&json!("Hello 42")));
//! assert!(!validator.is_valid(&json!("No!")));
//! # Ok(())
//! # }
//! ```
//!
//! ## General Configuration
//!
//! [`options()`] returns the same builder with the draft detected from `$schema`:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use serde_json::json;
//!
//! let schema = json!({"type": "string"});
//! let validator = jsonschema::options()
//!     // Add configuration options here
//!     .build(&schema)?;
//!
//! assert!(validator.is_valid(&json!("Hello")));
//! # Ok(())
//! # }
//! ```
//!
//! [`ValidationOptions`] lists every option.
//!
//! ## Automatic Draft Detection
//!
//! If you don't need to specify a particular draft version, you can use `jsonschema::validator_for`
//! which automatically detects the appropriate draft:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use serde_json::json;
//!
//! let schema = json!({"$schema": "http://json-schema.org/draft-07/schema#", "type": "string"});
//! let validator = jsonschema::validator_for(&schema)?;
//!
//! assert!(validator.is_valid(&json!("Hello")));
//! # Ok(())
//! # }
//! ```
//!
//! # External References
//!
//! By default, `jsonschema` resolves HTTP references using `reqwest` and file references from the local file system.
//! Both blocking and non-blocking retrieval is supported during validator construction.
//!
//! ```rust
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use serde_json::json;
//!
//! let schema = json!({"$schema": "http://json-schema.org/draft-07/schema#", "type": "string"});
//!
//! // Building a validator with blocking retrieval (default)
//! let validator = jsonschema::validator_for(&schema)?;
//!
//! // Building a validator with non-blocking retrieval (requires `resolve-async` feature)
//! # #[cfg(feature = "resolve-async")]
//! let validator = jsonschema::async_validator_for(&schema).await?;
//!
//! // Validation is always synchronous
//! assert!(validator.is_valid(&json!("Hello")));
//! # Ok(())
//! # }
//! ```
//!
//! You can disable the default behavior using crate features:
//!
//! - Disable HTTP resolving: `default-features = false, features = ["resolve-file", "idna"]`
//! - Disable file resolving:
//!   `default-features = false, features = ["resolve-http", "tls-aws-lc-rs", "idna"]`
//! - Enable async resolution: `features = ["resolve-async"]`
//! - Disable all resolving: `default-features = false, features = ["idna"]`
//!
//! ## Custom retrievers
//!
//! A retriever fetches documents on demand, from a database or a cache. When the set of documents
//! is known up front, put them in a [`Registry`] instead.
//!
//! You can implement custom retrievers for both blocking and non-blocking retrieval:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use std::{collections::HashMap, sync::Arc};
//! use jsonschema::{Retrieve, Uri};
//! use serde_json::{json, Value};
//!
//! struct InMemoryRetriever {
//!     schemas: HashMap<String, Value>,
//! }
//!
//! impl Retrieve for InMemoryRetriever {
//!
//!    fn retrieve(
//!        &self,
//!        uri: &Uri<String>,
//!    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
//!         self.schemas
//!             .get(uri.as_str())
//!             .cloned()
//!             .ok_or_else(|| format!("Schema not found: {uri}").into())
//!     }
//! }
//!
//! let mut schemas = HashMap::new();
//! schemas.insert(
//!     "https://example.com/person.json".to_string(),
//!     json!({
//!         "type": "object",
//!         "properties": {
//!             "name": { "type": "string" },
//!             "age": { "type": "integer" }
//!         },
//!         "required": ["name", "age"]
//!     }),
//! );
//!
//! let retriever = InMemoryRetriever { schemas };
//!
//! let schema = json!({
//!     "$ref": "https://example.com/person.json"
//! });
//!
//! let validator = jsonschema::options()
//!     .with_retriever(retriever)
//!     .build(&schema)?;
//!
//! assert!(validator.is_valid(&json!({
//!     "name": "Alice",
//!     "age": 30
//! })));
//!
//! assert!(!validator.is_valid(&json!({
//!     "name": "Bob"
//! })));
//! #    Ok(())
//! # }
//! ```
//!
//! And non-blocking version with the `resolve-async` feature enabled:
//!
//! ```rust,no_run
//! # #[cfg(all(
//! #     feature = "resolve-async",
//! #     any(
//! #         not(target_arch = "wasm32"),
//! #         all(target_arch = "wasm32", any(target_os = "unknown", target_os = "none")),
//! #     ),
//! # ))]
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use jsonschema::{AsyncRetrieve, Uri};
//! use serde_json::{Value, json};
//!
//! struct HttpRetriever;
//!
//! #[cfg_attr(target_family = "wasm", async_trait::async_trait(?Send))]
//! #[cfg_attr(not(target_family = "wasm"), async_trait::async_trait)]
//! impl AsyncRetrieve for HttpRetriever {
//!     async fn retrieve(
//!         &self,
//!         uri: &Uri<String>,
//!     ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
//!         reqwest::get(uri.as_str())
//!             .await?
//!             .json()
//!             .await
//!             .map_err(Into::into)
//!     }
//! }
//!
//! // Then use it to build a validator
//! let validator = jsonschema::async_options()
//!     .with_retriever(HttpRetriever)
//!     .build(&json!({"$ref": "https://example.com/user.json"}))
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! On `wasm32` targets, use `async_trait::async_trait(?Send)` so your retriever can rely on `Rc`, `JsFuture`, or other non-thread-safe types.
//!
//! ## Validating against schema definitions
//!
//! To validate against one definition of a larger schema, take it from [`validator_map_for`] by
//! pointer, or register the root schema and reference the definition from a wrapper schema:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use serde_json::json;
//! use jsonschema::{Registry, Resource};
//!
//! // Root schema with multiple definitions
//! let root_schema = json!({
//!     "$id": "https://example.com/root",
//!     "definitions": {
//!         "User": {
//!             "type": "object",
//!             "properties": {
//!                 "name": {"type": "string"},
//!                 "age": {"type": "integer", "minimum": 0}
//!             },
//!             "required": ["name"]
//!         },
//!         "Product": {
//!             "type": "object",
//!             "properties": {
//!                 "id": {"type": "integer"},
//!                 "title": {"type": "string"}
//!             },
//!             "required": ["id", "title"]
//!         }
//!     }
//! });
//!
//! // Create a schema that references the specific definition you want to validate against
//! let user_schema = json!({"$ref": "https://example.com/root#/definitions/User"});
//!
//! let registry = Registry::new()
//!     .add("https://example.com/root", root_schema)?
//!     .prepare()?;
//!
//! // Build validator for the specific definition via the shared prepared registry
//! let validator = jsonschema::options()
//!     .with_registry(&registry)
//!     .build(&user_schema)?;
//!
//! // Now validate data against just the User definition
//! assert!(validator.is_valid(&json!({"name": "Alice", "age": 30})));
//! assert!(!validator.is_valid(&json!({"age": 25})));  // Missing required "name"
//! # Ok(())
//! # }
//! ```
//!
//! ## Offline validation
//!
//! [`ValidationOptions::offline`] disables retrieval, so a reference that is not already in the
//! registry fails at construction instead of being fetched:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use serde_json::json;
//!
//! let schema = json!({"$ref": "https://example.com/schema.json"});
//! assert!(jsonschema::options().offline().build(&schema).is_err());
//! # Ok(())
//! # }
//! ```
//!
//! ## Bundling and dereferencing
//!
//! [`bundle`] embeds every referenced resource into the schema under `$defs` (`definitions` for
//! Draft 4/6/7), keeping each `$id` and leaving `$ref` values unchanged. The result is a Compound
//! Schema Document that needs no retrieval.
//!
//! [`dereference`] instead replaces each `$ref` with the schema it points to, for consumers that
//! do not resolve references. Circular references are left in place.
//!
//! # Regular Expression Configuration
//!
//! [`PatternOptions`] picks the engine for `pattern` and `patternProperties`.
//!
//! By default, the crate uses [`fancy-regex`](https://docs.rs/fancy-regex), which supports advanced
//! regular expression features such as lookaround and backreferences.
//!
//! The `regex` engine matches in linear time, so a hostile pattern in a user-provided schema
//! cannot stall validation; it lacks lookaround and backreferences.
//!
//! ### Example: Configure `fancy-regex` with Backtracking Limit
//!
//! ```rust
//! use serde_json::json;
//! use jsonschema::PatternOptions;
//!
//! let schema = json!({
//!     "type": "string",
//!     "pattern": "^(a+)+$"
//! });
//!
//! let validator = jsonschema::options()
//!     .with_pattern_options(
//!         PatternOptions::fancy_regex()
//!             .backtrack_limit(10_000)
//!     )
//!     .build(&schema)
//!     .expect("A valid schema");
//! ```
//!
//! ### Example: Use the `regex` Engine Instead
//!
//! ```rust
//! use serde_json::json;
//! use jsonschema::PatternOptions;
//!
//! let schema = json!({
//!     "type": "string",
//!     "pattern": "^a+$"
//! });
//!
//! let validator = jsonschema::options()
//!     .with_pattern_options(PatternOptions::regex())
//!     .build(&schema)
//!     .expect("A valid schema");
//! ```
//!
//! ### Example: Patterns in the Engine's Own Syntax
//!
//! Patterns are read as ECMA 262 and translated for the engine: `\d` matches `0-9` only. With
//! [`PatternSyntax::Engine`] they reach the engine unchanged, so `\d` matches every Unicode
//! decimal digit, as it does in the `regex` crate.
//!
//! ```rust
//! use serde_json::json;
//! use jsonschema::{PatternOptions, PatternSyntax};
//!
//! let schema = json!({"pattern": r"^\d+$"});
//!
//! let validator = jsonschema::options()
//!     .with_pattern_options(PatternOptions::regex().syntax(PatternSyntax::Engine))
//!     .build(&schema)
//!     .expect("A valid schema");
//!
//! assert!(validator.is_valid(&json!("٣")));
//! ```
//!
//! ### Notes
//!
//! - If no syntax is set, patterns are ECMA 262.
//! - Regular expressions that rely on advanced features like `(?<=...)` (lookbehind) or backreferences (`\1`) will fail with the `regex` engine.
//!
//! # Custom Keywords
//!
//! A custom keyword adds a check the standard keywords lack.
//! Keywords are generic over the instance representation ([`json`] module).
//!
//! To implement a custom keyword, you need to:
//! 1. Create a struct that implements the [`Keyword`] trait
//! 2. Create a factory function or closure that produces instances of your custom keyword
//! 3. Register the factory with [`ValidationOptions::with_keyword`]
//!
//! ```rust
//! use jsonschema::{paths::Location, Keyword, ValidationError};
//! use serde_json::{json, Map, Value};
//!
//! struct EvenNumberValidator;
//!
//! impl<'i> Keyword<'i> for EvenNumberValidator {
//!     fn validate(&self, instance: &'i Value) -> Result<(), ValidationError<'i>> {
//!         if let Some(n) = instance.as_i64() {
//!             if n % 2 == 0 {
//!                 return Ok(());
//!             }
//!         }
//!         Err(ValidationError::custom("value must be an even integer"))
//!     }
//!
//!     fn is_valid(&self, instance: &'i Value) -> bool {
//!         instance.as_i64().map_or(false, |n| n % 2 == 0)
//!     }
//! }
//!
//! fn even_number_factory<'a>(
//!     _parent: &'a Map<String, Value>,
//!     value: &'a Value,
//!     _path: Location,
//! ) -> Result<Box<dyn for<'i> Keyword<'i>>, ValidationError<'a>> {
//!     if value.as_bool() == Some(true) {
//!         Ok(Box::new(EvenNumberValidator))
//!     } else {
//!         Err(ValidationError::schema("The 'even-number' keyword must be set to true"))
//!     }
//! }
//!
//! let schema = json!({"even-number": true, "type": "integer"});
//! let validator = jsonschema::options()
//!     .with_keyword("even-number", even_number_factory)
//!     .build(&schema)
//!     .expect("Invalid schema");
//!
//! assert!(validator.is_valid(&json!(2)));
//! assert!(!validator.is_valid(&json!(3)));
//! assert!(!validator.is_valid(&json!("not a number")));
//! ```
//!
//! You can also use a closure instead of a factory function for simpler cases:
//!
//! ```rust
//! # use jsonschema::{paths::Location, Keyword, ValidationError};
//! # use serde_json::{json, Map, Value};
//! #
//! # struct EvenNumberValidator;
//! #
//! # impl<'i> Keyword<'i> for EvenNumberValidator {
//! #     fn validate(&self, instance: &'i Value) -> Result<(), ValidationError<'i>> {
//! #         Ok(())
//! #     }
//! #
//! #     fn is_valid(&self, instance: &'i Value) -> bool {
//! #         true
//! #     }
//! # }
//! let schema = json!({"even-number": true, "type": "integer"});
//! let validator = jsonschema::options()
//!     .with_keyword("even-number", |_, _, _| {
//!         Ok(Box::new(EvenNumberValidator))
//!     })
//!     .build(&schema)
//!     .expect("Invalid schema");
//! ```
//!
//! # Custom Formats
//!
//! [`ValidationOptions::with_format`] adds a check for a `format` name the built-ins do not cover.
//! Register a `fn(&str) -> bool` or a closure; on Draft 2019-09 and 2020-12 also call
//! `should_validate_formats(true)`:
//!
//! ```rust
//! use serde_json::json;
//!
//! // Step 1: Define the custom format validator function
//! fn ends_with_42(s: &str) -> bool {
//!     s.ends_with("42")
//! }
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Step 2: Create a schema using the custom format
//! let schema = json!({
//!     "type": "string",
//!     "format": "ends-with-42"
//! });
//!
//! // Step 3: Build the validator with the custom format
//! let validator = jsonschema::options()
//!     .with_format("ends-with-42", ends_with_42)
//!     .with_format("ends-with-43", |s| s.ends_with("43"))
//!     .should_validate_formats(true)
//!     .build(&schema)?;
//!
//! // Step 4: Validate instances
//! assert!(validator.is_valid(&json!("Hello42")));
//! assert!(!validator.is_valid(&json!("Hello43")));
//! assert!(!validator.is_valid(&json!(42))); // Not a string
//! #    Ok(())
//! # }
//! ```
//!
//! ### Notes on Custom Format Validators
//!
//! - Custom format validators are only called for string instances.
//! - In Draft 2019-09 and 2020-12, `format` checks nothing unless you call
//!   `.should_validate_formats(true)`.
//!
//! # Arbitrary Precision Numbers
//!
//! Enable the `arbitrary-precision` feature for exact validation of numbers beyond standard numeric ranges:
//!
//! ```toml
//! jsonschema = { version = "x.y.z", features = ["arbitrary-precision"] }
//! ```
//!
//! This provides:
//! - Arbitrarily large integers (e.g., `18446744073709551616`)
//! - Exact decimal precision without `f64` rounding (e.g., `0.1`, `0.3`)
//!
//! **Important**: precision survives only when `serde_json` parses JSON text; a float literal in
//! `json!()` is an `f64` first.
//!
//! ```rust
//! # use jsonschema::Validator;
//! // Precision preserved - parsed from JSON string
//! let schema = serde_json::from_str(r#"{"minimum": 0.1}"#)?;
//! let instance = serde_json::from_str("0.3")?;
//! let validator = Validator::new(&schema)?;
//! assert!(validator.is_valid(&instance));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Custom JSON Representations
//!
//! Validators can accept instances in any in-memory JSON representation, not just
//! `serde_json::Value`. Implement the traits in the [`json`] module for your representation and
//! build validators with [`options_for`]:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # type MyJson = jsonschema::json::SerdeJson;
//! # let schema = serde_json::json!({"type": "string"});
//! # let my_instance = &serde_json::json!("data");
//! let validator = jsonschema::options_for::<MyJson>().build(&schema)?;
//! assert!(validator.is_valid(my_instance));
//! # Ok(())
//! # }
//! ```
//!
//! See the [`json`] module for the full trait contract, a worked example, and how to verify an
//! implementation. The Python bindings use this mechanism to validate Python objects directly.
//!
//! # WebAssembly support
//!
//! `jsonschema` supports WebAssembly with different capabilities based on the target platform:
//!
//! ## Browser/JavaScript (`wasm32-unknown-unknown`)
//!
//! On this target `resolve-file` and `resolve-http` do not compile: there is no filesystem and
//! no blocking HTTP. Disable default features:
//!
//! ```toml
//! jsonschema = { version = "x.y.z", default-features = false }
//! ```
//!
//! For external references in browser environments, implement a custom retriever that uses
//! browser APIs (like `fetch`). See the [External References](#external-references) section.
//!
//! ## WASI (`wasm32-wasip1` / `wasm32-wasip2`)
//!
//! WASI environments (preview 1 and preview 2) can compile schemas and run validators, but the bundled
//! HTTP retriever depends on `reqwest`’s blocking client, which isn't available on these targets. Use
//! file access and custom retrievers instead.
//!
//! **Supported:**
//! - Blocking file resolution (`resolve-file` feature)
//! - Custom blocking retrievers (including wrapping async operations)
//! - Custom async retrievers via the `resolve-async` feature (for example, `jsonschema::async_options`
//!   together with your own async runtime)
//!
//! **Not Supported:**
//! - The bundled HTTP retriever (depends on `reqwest`’s blocking client)
//!
//! ```toml
//! jsonschema = { version = "x.y.z", default-features = false, features = ["resolve-file"] }
//! ```
//!
//! **HTTP:** implement [`Retrieve`], or `AsyncRetrieve` with `resolve-async`, over your own HTTP
//! client.

#[cfg(all(
    target_arch = "wasm32",
    target_os = "unknown",
    any(feature = "resolve-file", feature = "resolve-http")
))]
compile_error!(
    "Features 'resolve-http' and 'resolve-file' are not supported on wasm32-unknown-unknown"
);
#[cfg(all(
    not(target_arch = "wasm32"),
    feature = "resolve-http",
    not(any(feature = "tls-aws-lc-rs", feature = "tls-ring"))
))]
compile_error!(
    "Feature `resolve-http` requires a TLS provider: enable `tls-aws-lc-rs` \
(default) or `tls-ring`."
);

pub(crate) mod bundler;
pub mod canonical;
pub(crate) mod compiler;
mod content_encoding;
mod content_media_type;
pub(crate) mod dereferencer;
pub mod error;
mod evaluation;
pub(crate) use jsonschema_value::{
    cmp, numeric, unique, Array, Json, LazyInstance, Node, NodeIdentity, Object, SerdeJson,
};
/// Validating instances in a custom in-memory JSON representation.
///
/// Implement [`Json`], [`Node`], [`Object`], [`Array`], and [`JsonNumber`](json::JsonNumber)
/// for your representation, then build validators with [`options_for`]. Schemas are always
/// `serde_json::Value`; only instances use the custom representation. [`SerdeJson`] is the
/// built-in representation behind [`validator_for`] and the crate-level convenience functions.
///
/// Enable the `jsonb` feature for `json::Jsonb`, which reads a Postgres `jsonb` value in
/// place. It takes the container bytes without the varlena header, so detoast first:
///
/// ```rust,ignore
/// let detoasted = unsafe { pgrx::pg_sys::pg_detoast_datum_packed(datum.cast_mut_ptr()) };
/// let bytes = unsafe { pgrx::varlena_to_byte_slice(detoasted) };
/// validator.is_valid(Jsonb::root(bytes))
/// ```
///
/// The slice lives until the memory context resets. The bytes are in the server's native byte
/// order, and keys and strings are read as UTF-8. An error's instance is built on first access;
/// past 128 levels of nesting it holds `null`, and `json::jsonb::take_pending_error` returns
/// the message.
///
/// The accessors are infallible, so every node must map to a JSON value: reject nodes with no
/// JSON meaning (tags, foreign objects) before validation, or track them on a side channel of
/// the representation.
///
/// # Example
///
/// ```rust
/// use std::borrow::Cow;
///
/// use jsonschema::{
///     json::{Array, Json, JsonNumber, Node, NodeIdentity, Object},
///     JsonType,
/// };
/// use serde_json::Value;
///
/// #[derive(Default)]
/// enum ToyValue {
///     #[default]
///     Null,
///     Boolean(bool),
///     Number(f64),
///     String(String),
///     Array(Vec<ToyValue>),
///     Object(Vec<(String, ToyValue)>),
/// }
///
/// struct ToyJson;
///
/// impl Json for ToyJson {
///     // A cheap-to-`Clone` handle; `&ToyValue` plays the role `&serde_json::Value` does
///     // for the built-in representation.
///     type Node<'a> = &'a ToyValue;
///     // Property names are prepared once at schema compile time.
///     type PreparedKey = String;
///     // Scratch storage for nodes made from property names (`propertyNames`).
///     type StringBuffer = ToyValue;
///
///     fn prepare_key(key: &str) -> String {
///         key.to_owned()
///     }
///
///     fn with_string_node<T>(
///         buffer: &mut ToyValue,
///         string: &str,
///         f: impl FnOnce(&ToyValue) -> T,
///     ) -> T {
///         *buffer = ToyValue::String(string.to_owned());
///         f(buffer)
///     }
/// }
///
/// struct ToyNumber(f64);
///
/// impl JsonNumber for ToyNumber {
///     fn as_u64(&self) -> Option<u64> {
///         (self.0.fract() == 0.0 && self.0 >= 0.0).then_some(self.0 as u64)
///     }
///     fn as_i64(&self) -> Option<i64> {
///         (self.0.fract() == 0.0).then_some(self.0 as i64)
///     }
///     fn as_f64(&self) -> Option<f64> {
///         Some(self.0)
///     }
///     fn as_str(&self) -> Cow<'_, str> {
///         Cow::Owned(self.0.to_string())
///     }
///     fn to_number(&self) -> Cow<'_, serde_json::Number> {
///         Cow::Owned(serde_json::Number::from_f64(self.0).expect("finite"))
///     }
/// }
///
/// impl<'a> Node<'a, ToyJson> for &'a ToyValue {
///     type Object = &'a [(String, ToyValue)];
///     type Array = &'a [ToyValue];
///     type Number = ToyNumber;
///
///     fn as_object(&self) -> Option<&'a [(String, ToyValue)]> {
///         match self {
///             ToyValue::Object(members) => Some(members),
///             _ => None,
///         }
///     }
///     fn as_array(&self) -> Option<&'a [ToyValue]> {
///         match self {
///             ToyValue::Array(items) => Some(items),
///             _ => None,
///         }
///     }
///     fn as_string(&self) -> Option<Cow<'a, str>> {
///         match self {
///             ToyValue::String(string) => Some(Cow::Borrowed(string)),
///             _ => None,
///         }
///     }
///     fn as_number(&self) -> Option<ToyNumber> {
///         match self {
///             ToyValue::Number(number) => Some(ToyNumber(*number)),
///             _ => None,
///         }
///     }
///     fn as_boolean(&self) -> Option<bool> {
///         match self {
///             ToyValue::Boolean(boolean) => Some(*boolean),
///             _ => None,
///         }
///     }
///     fn is_null(&self) -> bool {
///         matches!(self, ToyValue::Null)
///     }
///     fn json_type(&self) -> JsonType {
///         match self {
///             ToyValue::Null => JsonType::Null,
///             ToyValue::Boolean(_) => JsonType::Boolean,
///             ToyValue::Number(_) => JsonType::Number,
///             ToyValue::String(_) => JsonType::String,
///             ToyValue::Array(_) => JsonType::Array,
///             ToyValue::Object(_) => JsonType::Object,
///         }
///     }
///     // Cold paths only: error messages, `const`/`enum` comparisons, `uniqueItems`.
///     fn to_value(&self) -> Cow<'a, Value> {
///         Cow::Owned(match self {
///             ToyValue::Null => Value::Null,
///             ToyValue::Boolean(boolean) => Value::Bool(*boolean),
///             ToyValue::Number(number) => serde_json::Number::from_f64(*number)
///                 .map(Value::Number)
///                 .expect("finite"),
///             ToyValue::String(string) => Value::String(string.clone()),
///             ToyValue::Array(items) => Value::Array(
///                 items.iter().map(|item| item.to_value().into_owned()).collect(),
///             ),
///             ToyValue::Object(members) => Value::Object(
///                 members
///                     .iter()
///                     .map(|(name, value)| (name.clone(), value.to_value().into_owned()))
///                     .collect(),
///             ),
///         })
///     }
///     // Stable per live node; see the trait docs for the exact contract.
///     fn identity(&self) -> Option<NodeIdentity> {
///         Some(NodeIdentity::new(std::ptr::from_ref::<ToyValue>(*self) as usize))
///     }
/// }
///
/// impl<'a> Object<'a, ToyJson> for &'a [(String, ToyValue)] {
///     type Node = &'a ToyValue;
///     type MemberName = &'a str;
///     type MembersIter = ToyMembersIter<'a>;
///
///     fn len(&self) -> usize {
///         <[(String, ToyValue)]>::len(self)
///     }
///     fn get(&self, key: &String) -> Option<&'a ToyValue> {
///         self.iter().find(|(name, _)| name == key).map(|(_, value)| value)
///     }
///     fn members(&self) -> ToyMembersIter<'a> {
///         ToyMembersIter(self.iter())
///     }
/// }
///
/// struct ToyMembersIter<'a>(std::slice::Iter<'a, (String, ToyValue)>);
///
/// impl<'a> Iterator for ToyMembersIter<'a> {
///     type Item = (&'a str, &'a ToyValue);
///     fn next(&mut self) -> Option<Self::Item> {
///         self.0.next().map(|(name, value)| (name.as_str(), value))
///     }
/// }
///
/// impl<'a> Array<'a, ToyJson> for &'a [ToyValue] {
///     type Node = &'a ToyValue;
///     type ElementsIter = std::slice::Iter<'a, ToyValue>;
///
///     fn len(&self) -> usize {
///         <[ToyValue]>::len(self)
///     }
///     fn elements(&self) -> std::slice::Iter<'a, ToyValue> {
///         self.iter()
///     }
/// }
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let schema = serde_json::json!({
///     "type": "object",
///     "properties": {"name": {"type": "string", "minLength": 2}},
///     "required": ["name"]
/// });
/// let validator = jsonschema::options_for::<ToyJson>().build(&schema)?;
/// # let _ = format!("{:?}", jsonschema::options_for::<ToyJson>());
///
/// let valid = ToyValue::Object(vec![("name".into(), ToyValue::String("bob".into()))]);
/// let invalid = ToyValue::Object(vec![]);
/// assert!(validator.is_valid(&valid));
/// let error = validator.validate(&invalid).expect_err("missing required");
/// assert_eq!(error.to_string(), "\"name\" is a required property");
/// # Ok(())
/// # }
/// ```
///
/// # Verifying an implementation
///
/// Enable the `conformance` feature, encode the `conformance::document()` JSON document in your
/// representation, and run `conformance::assert_conformance` on it; it checks the accessor
/// contract the validator relies on, including the subtle parts (code-point string lengths,
/// mathematical number equality, node identity stability).
///
/// # Custom keywords
///
/// [`Keyword`] is generic over the representation and operates on `F::Node` directly; register
/// implementations with [`ValidationOptions::with_keyword`](crate::ValidationOptions::with_keyword).
pub mod json {
    #[cfg(feature = "conformance")]
    pub use jsonschema_value::conformance;
    pub use jsonschema_value::{
        cmp, unique, Array, Json, JsonNumber, Node, NodeIdentity, Object, SerdeJson,
    };
    #[cfg(feature = "pyo3")]
    pub use jsonschema_value::{
        inspect, narrow_array, narrow_object, object_values, stock_enum_value, Pyo3,
    };
    #[cfg(feature = "jsonb")]
    pub use jsonschema_value::{jsonb, Jsonb, JsonbNode};
    #[cfg(feature = "magnus")]
    pub use jsonschema_value::{
        magnus_child, magnus_invalidate_members_cache, magnus_is_object, magnus_object_values,
        magnus_probe_root, magnus_string_node, magnus_take_pending_error, Magnus,
        MagnusPendingErrorScope, PendingError, RbNode,
    };
}
mod http;
mod keywords;
#[cfg(all(feature = "macros", not(target_family = "wasm")))]
mod meta_codegen;
mod node;
mod options;
pub mod output;
pub mod paths;
pub(crate) mod properties;
pub(crate) mod regex;
mod retriever;
mod unfinished_matches;
pub mod types {
    pub use jsonschema_value::types::{JsonType, JsonTypeSet, JsonTypeSetIterator};
}
mod validator;

pub use canonical::CanonicalizationError;
pub use error::{
    ErrorIterator, MaskedValidationError, ValidationError, ValidationErrorParts, ValidationErrors,
};
pub use evaluation::{
    AnnotationEntry, ErrorEntry, Evaluation, FlagOutput, HierarchicalOutput, ListOutput,
    OutputUnit, OutputUnitChildren,
};
pub use http::HttpOptions;
#[doc(inline)]
#[cfg(feature = "macros")]
pub use jsonschema_macros::validator;
pub use keywords::custom::Keyword;
pub use options::{
    EmailOptions, FancyRegex, PatternCache, PatternOptions, PatternSyntax, Regex, ValidationOptions,
};
pub use referencing::{
    uri, Draft, Error as ReferencingError, Registry, RegistryBuilder, Resource, Retrieve, Uri,
};
#[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
pub use retriever::{HttpRetriever, HttpRetrieverError};
pub use types::{JsonType, JsonTypeSet, JsonTypeSetIterator};
pub use validator::{ValidationContext, Validator, ValidatorMap};

#[cfg(feature = "resolve-async")]
pub use referencing::AsyncRetrieve;
#[cfg(all(
    feature = "resolve-http",
    feature = "resolve-async",
    not(target_arch = "wasm32")
))]
pub use retriever::AsyncHttpRetriever;

use serde_json::Value;

/// Whether `schema` accepts `instance`. The draft is detected from `$schema`.
///
/// # Examples
///
/// ```rust
/// use serde_json::json;
///
/// let schema = json!({"maxLength": 5});
/// let instance = json!("foo");
/// assert!(jsonschema::is_valid(&schema, &instance));
/// ```
///
/// # Panics
///
/// Panics if the schema is invalid or a reference cannot be resolved.
///
/// This function **must not** be called from within an async runtime if the schema contains
/// external references that require network requests, or it will panic when attempting to block.
/// Use `async_validator_for` for async contexts, or run this in a separate blocking thread
/// via `tokio::task::spawn_blocking`.
#[must_use]
#[inline]
pub fn is_valid(schema: &Value, instance: &Value) -> bool {
    validator_for(schema)
        .expect("Invalid schema")
        .is_valid(instance)
}

/// Validate `instance` against `schema` and return the first error if any. Draft is detected automatically.
///
/// # Examples
///
/// ```rust
/// use serde_json::json;
///
/// let schema = json!({"maxLength": 5});
/// let instance = json!("foo");
/// assert!(jsonschema::validate(&schema, &instance).is_ok());
/// ```
///
/// # Errors
///
/// Returns the first [`ValidationError`] encountered when `instance` violates `schema`.
///
/// # Panics
///
/// Panics if the schema is invalid or a reference cannot be resolved.
///
/// This function **must not** be called from within an async runtime if the schema contains
/// external references that require network requests, or it will panic when attempting to block.
/// Use `async_validator_for` for async contexts, or run this in a separate blocking thread
/// via `tokio::task::spawn_blocking`.
#[inline]
pub fn validate<'i>(schema: &Value, instance: &'i Value) -> Result<(), ValidationError<'i>> {
    validator_for(schema)
        .expect("Invalid schema")
        .validate(instance)
}

/// Evaluate `instance` against `schema` and return structured validation output. Draft is detected automatically.
///
/// Returns an [`Evaluation`] containing detailed validation results in JSON Schema Output v1 format,
/// including annotations and errors across the entire validation tree.
///
/// # Examples
///
/// ```rust
/// use serde_json::json;
///
/// let schema = json!({"type": "string", "minLength": 3});
/// let instance = json!("foo");
/// let evaluation = jsonschema::evaluate(&schema, &instance);
/// assert!(evaluation.flag().valid);
/// ```
///
/// # Panics
///
/// Panics if the schema is invalid or a reference cannot be resolved.
///
/// This function **must not** be called from within an async runtime if the schema contains
/// external references that require network requests, or it will panic when attempting to block.
/// Use `async_validator_for` for async contexts, or run this in a separate blocking thread
/// via `tokio::task::spawn_blocking`.
#[must_use]
#[inline]
pub fn evaluate(schema: &Value, instance: &Value) -> Evaluation {
    validator_for(schema)
        .expect("Invalid schema")
        .evaluate(instance)
}

/// Create a validator for the input schema with automatic draft detection and default options.
///
/// # Examples
///
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({"minimum": 5});
/// let instance = json!(42);
///
/// let validator = jsonschema::validator_for(&schema)?;
/// assert!(validator.is_valid(&instance));
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if the schema is invalid or external references cannot be resolved.
///
/// # Panics
///
/// This function **must not** be called from within an async runtime if the schema contains
/// external references that require network requests, or it will panic when attempting to block.
/// Use `async_validator_for` for async contexts, or run this in a separate blocking thread
/// via `tokio::task::spawn_blocking`.
pub fn validator_for(schema: &Value) -> Result<Validator, ValidationError<'static>> {
    Validator::new(schema)
}

/// Create a [`ValidatorMap`] from the input schema using automatic draft detection and
/// default options.
///
/// Every reachable subschema is compiled eagerly. The root schema is always present
/// under the key `"#"`. Subschemas that fail to compile (e.g. unresolvable `$ref`) are
/// silently omitted.
///
/// # Examples
///
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({
///     "$defs": {
///         "User": {"type": "object", "required": ["name"]}
///     }
/// });
///
/// let map = jsonschema::validator_map_for(&schema)?;
///
/// let user_validator = map.get("#/$defs/User").unwrap();
/// assert!(user_validator.is_valid(&json!({"name": "Alice"})));
/// assert!(!user_validator.is_valid(&json!({})));
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if the schema is invalid or external references cannot be resolved.
///
/// # Panics
///
/// This function **must not** be called from within an async runtime if the schema contains
/// external references that require network requests, or it will panic when attempting to block.
/// Use `async_validator_map_for` for async contexts, or run this in a separate blocking thread
/// via `tokio::task::spawn_blocking`.
pub fn validator_map_for(schema: &Value) -> Result<ValidatorMap, ValidationError<'static>> {
    options().build_map(schema)
}

/// Embed all external `$ref` targets into a draft-appropriate container,
/// producing a Compound Schema Document that validates identically to the original.
/// Draft 4/6/7 use `definitions`; Draft 2019-09/2020-12 use `$defs`.
/// `$ref` values are preserved unchanged.
/// An embedded resource of another draft gets both `id` and `$id`, so the root's draft finds
/// it too.
///
/// **Limitation:** only `$ref` is followed; `$dynamicRef` and `$recursiveRef` targets are not
/// embedded.
///
/// For custom resources or retrievers, use [`options()`] and call `.bundle()`.
///
/// # Errors
///
/// Returns an error if the meta-schema named in `$schema`, an `$id` against its base URI, or
/// a `$ref` cannot be resolved.
///
/// # Panics
///
/// This function **must not** be called from within an async runtime if the schema contains
/// external references that require network requests, or it will panic when attempting to block.
/// Use `async_options().bundle()` for async contexts, or run this in a separate blocking thread
/// via `tokio::task::spawn_blocking`.
///
/// # Examples
///
/// ```rust
/// use serde_json::json;
///
/// let schema = json!({"type": "string"});
/// let bundled = jsonschema::bundle(&schema).expect("bundling failed");
/// assert_eq!(bundled, schema); // no external refs, returned unchanged
/// ```
pub fn bundle(schema: &Value) -> Result<Value, ReferencingError> {
    options().bundle(schema)
}

/// Dereference a JSON Schema by recursively replacing all `$ref` values
/// with the schemas they point to.
///
/// Circular references are left in place as `$ref` strings.
///
/// For custom retriever, registry, draft, or base URI, use [`options()`] and call `.dereference()`.
///
/// # Errors
///
/// Returns an error if any `$ref` cannot be resolved (e.g. points to an
/// external URI not present in the schema and no retriever is available).
///
/// # Example
///
/// ```rust
/// use serde_json::json;
///
/// let schema = json!({
///     "$defs": {"tag": {"type": "string"}},
///     "properties": {"name": {"$ref": "#/$defs/tag"}}
/// });
/// let result = jsonschema::dereference(&schema).expect("dereference failed");
/// assert_eq!(result["properties"]["name"]["type"], "string");
/// ```
pub fn dereference(schema: &Value) -> Result<Value, ReferencingError> {
    options().dereference(schema)
}

/// Bundle a JSON Schema into a Compound Schema Document,
/// using async retrieval for external references.
///
/// Async counterpart to [`bundle`]. For custom resources or retrievers,
/// use [`async_options()`] and call `.bundle()`.
///
/// # Errors
///
/// Returns an error if the meta-schema named in `$schema`, an `$id` against its base URI, or
/// a `$ref` cannot be resolved.
#[cfg(feature = "resolve-async")]
pub async fn async_bundle(schema: &Value) -> Result<Value, ReferencingError> {
    async_options().bundle(schema).await
}

/// Dereference a JSON Schema asynchronously.
///
/// Async counterpart to [`dereference`]. For custom resources or retrievers,
/// use [`async_options()`] and call `.dereference()`.
///
/// # Errors
///
/// Returns an error if any `$ref` cannot be resolved.
#[cfg(feature = "resolve-async")]
pub async fn async_dereference(schema: &Value) -> Result<Value, ReferencingError> {
    async_options().dereference(schema).await
}

/// Create a validator for the input schema with automatic draft detection and default options,
/// using non-blocking retrieval for external references.
///
/// This is the async counterpart to [`validator_for`]. Only construction is asynchronous;
/// validation is synchronous.
///
/// # Examples
///
/// ```rust
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({
///     "type": "object",
///     "properties": {
///         "user": { "$ref": "https://example.com/user.json" }
///     }
/// });
///
/// let validator = jsonschema::async_validator_for(&schema).await?;
/// assert!(validator.is_valid(&json!({"user": {"name": "Alice"}})));
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if the schema is invalid or external references cannot be resolved.
#[cfg(feature = "resolve-async")]
pub async fn async_validator_for(schema: &Value) -> Result<Validator, ValidationError<'static>> {
    Validator::async_new(schema).await
}

/// Create a [`ValidatorMap`] from the input schema using async retrieval for external references.
///
/// Async counterpart to [`validator_map_for`]. Note that only construction is asynchronous —
/// validation itself is always synchronous.
///
/// # Examples
///
/// ```rust
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({"$defs": {"User": {"type": "object", "required": ["name"]}}});
/// let map = jsonschema::async_validator_map_for(&schema).await?;
/// assert!(map["#/$defs/User"].is_valid(&json!({"name": "Bob"})));
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if the schema is invalid or external references cannot be resolved.
#[cfg(feature = "resolve-async")]
pub async fn async_validator_map_for(
    schema: &Value,
) -> Result<ValidatorMap, ValidationError<'static>> {
    async_options().build_map(schema).await
}

/// Reduce a JSON Schema to a normal form that accepts the same values.
///
/// Experimental: keyword coverage is incomplete and the API may change in minor releases.
///
/// Use [`canonical::options`](fn@canonical::options) to configure canonicalization.
///
/// A schema the canonical form cannot model exactly still succeeds, as a `Raw` result holding the
/// original document; see [Unsupported schemas](canonical#unsupported-schemas).
///
/// # Errors
///
/// Returns [`CanonicalizationError`] when the input is not a valid JSON Schema document or a reference cannot be resolved.
pub fn canonicalize(value: &Value) -> Result<canonical::CanonicalSchema, CanonicalizationError> {
    canonical::options().canonicalize(value)
}

/// Create a builder for configuring JSON Schema validation options.
///
/// If [`with_draft`](ValidationOptions::with_draft) is not called, the draft is
/// auto-detected from the schema's `$schema` field, as in [`validator_for`].
///
/// **Note:** [`ValidationOptions::build`] panics inside an async runtime when the schema has
/// references that need network requests; use `async_options` there.
///
/// # Examples
///
/// Basic usage with draft specification:
///
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
/// use jsonschema::Draft;
///
/// let schema = json!({"type": "string"});
/// let validator = jsonschema::options()
///     .with_draft(Draft::Draft7)
///     .build(&schema)?;
///
/// assert!(validator.is_valid(&json!("Hello")));
/// # Ok(())
/// # }
/// ```
///
/// Advanced configuration:
///
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({"type": "string", "format": "custom"});
/// let validator = jsonschema::options()
///     .with_format("custom", |value| value.len() == 3)
///     .should_validate_formats(true)
///     .build(&schema)?;
///
/// assert!(validator.is_valid(&json!("abc")));
/// assert!(!validator.is_valid(&json!("abcd")));
/// # Ok(())
/// # }
/// ```
///
/// See [`ValidationOptions`] for all available configuration options.
#[must_use]
pub fn options<'i>() -> ValidationOptions<'i> {
    Validator::options()
}

/// Create a builder whose validators accept instances in the JSON representation `F`.
///
/// Same configuration surface as [`options()`]; `build` yields a `Validator<F>`.
#[must_use]
pub fn options_for<'i, F: Json>() -> ValidationOptions<'i, std::sync::Arc<dyn Retrieve>, F> {
    ValidationOptions::default()
}

/// Create a builder whose validators accept instances in the JSON representation `F`, with
/// async retrieval of external references.
///
/// Same configuration surface as [`async_options()`]; `build` yields a `Validator<F>`.
#[cfg(feature = "resolve-async")]
#[must_use]
pub fn async_options_for<'i, F: Json>(
) -> ValidationOptions<'i, std::sync::Arc<dyn AsyncRetrieve>, F> {
    ValidationOptions::default()
}

/// Create a builder whose validators retrieve external references with non-blocking I/O.
///
/// # Examples
///
/// Basic usage with external references:
///
/// ```rust
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({
///     "$ref": "https://example.com/user.json"
/// });
///
/// let validator = jsonschema::async_options()
///     .build(&schema)
///     .await?;
///
/// assert!(validator.is_valid(&json!({"name": "Alice"})));
/// # Ok(())
/// # }
/// ```
///
/// Advanced configuration:
///
/// ```rust
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::{Value, json};
/// use jsonschema::{Draft, AsyncRetrieve, Uri};
///
/// // Custom async retriever
/// struct MyRetriever;
///
/// #[cfg_attr(target_family = "wasm", async_trait::async_trait(?Send))]
/// #[cfg_attr(not(target_family = "wasm"), async_trait::async_trait)]
/// impl AsyncRetrieve for MyRetriever {
///     async fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
///         // Custom retrieval logic
///         Ok(json!({}))
///     }
/// }
///
/// let schema = json!({
///     "$ref": "https://example.com/user.json"
/// });
/// let validator = jsonschema::async_options()
///     .with_draft(Draft::Draft202012)
///     .with_retriever(MyRetriever)
///     .build(&schema)
///     .await?;
/// # Ok(())
/// # }
/// ```
///
/// On `wasm` targets the trait is declared with `async_trait(?Send)`, and your implementation
/// must use it too.
///
/// See [`ValidationOptions`] for all available configuration options.
#[cfg(feature = "resolve-async")]
#[must_use]
pub fn async_options<'i>() -> ValidationOptions<'i, std::sync::Arc<dyn AsyncRetrieve>> {
    Validator::async_options()
}

/// Validating schema documents against their meta-schemas.
pub mod meta {
    use crate::{error::ValidationError, Draft, Json, Node, Object, Registry, Validator};
    use ahash::AHashSet;
    use referencing::Retrieve;
    use serde_json::Value;
    use std::{
        any::{Any, TypeId},
        borrow::Cow,
        sync::{OnceLock, RwLock},
    };

    pub use validator_handle::MetaValidator;

    /// Meta-schema validators that read a schema held as a Python object.
    ///
    /// The bundled drafts are compiled in, so a schema already in Python form is checked without
    /// being converted to [`serde_json::Value`]. A `$schema` outside them is reached through
    /// [`is_valid_for`] / [`validate_for`] instead.
    ///
    /// Needs the `macros` feature; without it, use [`is_valid_for`] / [`validate_for`] with
    /// [`json::Pyo3`](crate::json::Pyo3).
    #[cfg(all(feature = "macros", feature = "pyo3", not(target_family = "wasm")))]
    pub mod pyo3 {
        use crate::Draft;
        use ::pyo3::{types::PyAny, Borrowed};

        pub use crate::meta_codegen::pyo3::{is_valid_fn, validate_fn, IsValidFn, ValidateFn};

        /// The draft whose meta-schema `schema` names in `$schema`.
        ///
        /// `Draft::Unknown` means a URI outside the bundled drafts, which [`is_valid_fn`] and
        /// [`validate_fn`] cannot check; no `$schema` gives Draft 2020-12.
        #[must_use]
        pub fn draft_of(schema: Borrowed<'_, '_, PyAny>) -> Draft {
            super::meta_cache::<crate::json::Pyo3>().draft_of(&schema)
        }
    }

    /// Meta-schema validators that read a schema held as a Ruby object.
    ///
    /// The bundled drafts are compiled in, so a schema already in Ruby form is checked without
    /// being converted to [`serde_json::Value`]. A `$schema` outside them is reached through
    /// [`is_valid_for`] / [`validate_for`] instead.
    ///
    /// Needs the `macros` feature; without it, use [`is_valid_for`] / [`validate_for`] with
    /// [`json::Magnus`](crate::json::Magnus).
    #[cfg(all(feature = "macros", feature = "magnus", not(target_family = "wasm")))]
    pub mod magnus {
        use crate::{json::RbNode, Draft};

        pub use crate::meta_codegen::magnus::{is_valid_fn, validate_fn, IsValidFn, ValidateFn};

        /// The draft whose meta-schema `schema` names in `$schema`.
        ///
        /// `Draft::Unknown` means a URI outside the bundled drafts, which [`is_valid_fn`] and
        /// [`validate_fn`] cannot check; no `$schema` gives Draft 2020-12.
        #[must_use]
        pub fn draft_of(schema: RbNode<'_>) -> Draft {
            super::meta_cache::<crate::json::Magnus>().draft_of(&schema)
        }
    }

    /// Create a meta-validation options builder.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use serde_json::json;
    /// use jsonschema::{Registry, Resource};
    ///
    /// let custom_meta = Resource::from_contents(json!({
    ///     "$schema": "https://json-schema.org/draft/2020-12/schema",
    ///     "type": "object"
    /// }));
    ///
    /// let registry = Registry::new()
    ///     .add("http://example.com/meta", custom_meta)
    ///     .unwrap()
    ///     .prepare()
    ///     .unwrap();
    ///
    /// let schema = json!({
    ///     "$schema": "http://example.com/meta",
    ///     "type": "string"
    /// });
    ///
    /// assert!(jsonschema::meta::options()
    ///     .with_registry(&registry)
    ///     .is_valid(&schema));
    /// ```
    #[must_use]
    pub fn options<'a>() -> MetaSchemaOptions<'a> {
        MetaSchemaOptions::default()
    }

    /// Options for meta-schema validation.
    #[derive(Clone, Default)]
    pub struct MetaSchemaOptions<'a> {
        registry: Option<&'a Registry<'a>>,
    }

    impl<'a> MetaSchemaOptions<'a> {
        /// Use a registry for resolving custom meta-schemas.
        ///
        /// # Examples
        ///
        /// ```rust
        /// use serde_json::json;
        /// use jsonschema::{Registry, Resource};
        ///
        /// let custom_meta = Resource::from_contents(json!({
        ///     "$schema": "https://json-schema.org/draft/2020-12/schema",
        ///     "type": "object"
        /// }));
        ///
        /// let registry = Registry::new()
        ///     .add("http://example.com/meta", custom_meta)
        ///     .unwrap()
        ///     .prepare()
        ///     .unwrap();
        ///
        /// let options = jsonschema::meta::options()
        ///     .with_registry(&registry);
        /// ```
        #[must_use]
        pub fn with_registry(mut self, registry: &'a Registry<'a>) -> Self {
            self.registry = Some(registry);
            self
        }

        /// Check if a schema is valid according to its meta-schema.
        ///
        /// # Panics
        ///
        /// Panics if the meta-schema cannot be resolved.
        #[must_use]
        pub fn is_valid(&self, schema: &Value) -> bool {
            match try_meta_validator_for(schema, self.registry) {
                Ok(validator) => validator.is_valid(schema),
                Err(e) => panic!("Failed to resolve meta-schema: {e}"),
            }
        }

        /// Validate a schema according to its meta-schema.
        ///
        /// # Errors
        ///
        /// Returns [`ValidationError`] if the schema is invalid or if the meta-schema cannot be resolved.
        pub fn validate<'schema>(
            &self,
            schema: &'schema Value,
        ) -> Result<(), ValidationError<'schema>> {
            let validator = try_meta_validator_for(schema, self.registry)?;
            validator.validate(schema)
        }
    }

    mod validator_handle {
        use crate::{ValidationError, Validator};
        use serde_json::Value;
        use std::{marker::PhantomData, ops::Deref};

        /// Handle to a meta-schema [`Validator`].
        pub struct MetaValidator<'a>(MetaValidatorInner<'a>);

        // Native builds borrow cached validators or own one built for a custom meta-schema; wasm
        // targets always own them because `Validator` does not implement `Sync` there.
        // Under `macros`, native builds dispatch to compile-time generated validators; the cached
        // runtime validator is resolved lazily by draft only for the `evaluate` path (`AsRef`).
        enum MetaValidatorInner<'a> {
            #[cfg(all(not(feature = "macros"), not(target_family = "wasm")))]
            Borrowed(&'a Validator),
            Owned(Box<Validator>, PhantomData<&'a Validator>),
            #[cfg(all(feature = "macros", not(target_family = "wasm")))]
            Generated {
                draft: crate::Draft,
                is_valid: fn(&Value) -> bool,
                validate: crate::meta_codegen::ValidateFn,
                iter_errors: crate::meta_codegen::IterErrorsFn,
            },
        }

        // `borrowed` is the only method using `'a`; it is absent on wasm and under `macros`.
        #[cfg_attr(
            any(target_family = "wasm", feature = "macros"),
            allow(clippy::elidable_lifetime_names)
        )]
        impl<'a> MetaValidator<'a> {
            #[cfg(all(not(feature = "macros"), not(target_family = "wasm")))]
            pub(crate) fn borrowed(validator: &'a Validator) -> Self {
                Self(MetaValidatorInner::Borrowed(validator))
            }

            pub(crate) fn owned(validator: Validator) -> Self {
                Self(MetaValidatorInner::Owned(Box::new(validator), PhantomData))
            }

            #[cfg(all(feature = "macros", not(target_family = "wasm")))]
            pub(crate) fn generated(draft: crate::Draft) -> Self {
                Self(MetaValidatorInner::Generated {
                    draft,
                    is_valid: crate::meta_codegen::is_valid_fn(draft),
                    validate: crate::meta_codegen::validate_fn(draft),
                    iter_errors: crate::meta_codegen::iter_errors_fn(draft),
                })
            }

            /// Validate `schema` against the meta-schema, returning `true` if valid.
            #[must_use]
            pub fn is_valid(&self, schema: &Value) -> bool {
                match &self.0 {
                    #[cfg(all(feature = "macros", not(target_family = "wasm")))]
                    MetaValidatorInner::Generated { is_valid, .. } => is_valid(schema),
                    _ => self.as_ref().is_valid(schema),
                }
            }

            /// Validate `schema` against the meta-schema, returning the first error if any.
            ///
            /// # Errors
            ///
            /// Returns the first [`ValidationError`] describing why the schema violates the meta-schema.
            pub fn validate<'i>(&self, schema: &'i Value) -> Result<(), ValidationError<'i>> {
                match &self.0 {
                    #[cfg(all(feature = "macros", not(target_family = "wasm")))]
                    MetaValidatorInner::Generated { validate, .. } => validate(schema),
                    _ => self.as_ref().validate(schema),
                }
            }

            /// Validate `schema` against the meta-schema, yielding every error.
            #[must_use]
            pub fn iter_errors<'i>(&'i self, schema: &'i Value) -> crate::ErrorIterator<'i> {
                match &self.0 {
                    #[cfg(all(feature = "macros", not(target_family = "wasm")))]
                    MetaValidatorInner::Generated { iter_errors, .. } => iter_errors(schema),
                    _ => self.as_ref().iter_errors(schema),
                }
            }
        }

        impl AsRef<Validator> for MetaValidator<'_> {
            fn as_ref(&self) -> &Validator {
                match &self.0 {
                    #[cfg(all(not(feature = "macros"), not(target_family = "wasm")))]
                    MetaValidatorInner::Borrowed(validator) => validator,
                    MetaValidatorInner::Owned(validator, _) => validator,
                    // `evaluate` has no generated counterpart; fall back to the runtime validator.
                    #[cfg(all(feature = "macros", not(target_family = "wasm")))]
                    MetaValidatorInner::Generated { draft, .. } => {
                        crate::meta::validators::runtime_validator_for_draft(*draft)
                    }
                }
            }
        }

        impl Deref for MetaValidator<'_> {
            type Target = Validator;

            fn deref(&self) -> &Self::Target {
                self.as_ref()
            }
        }
    }

    pub(crate) mod validators {
        use crate::Validator;
        #[cfg(not(target_family = "wasm"))]
        use std::sync::LazyLock;

        fn build_validator(schema: &serde_json::Value) -> Validator {
            crate::options()
                .without_schema_validation()
                .build(schema)
                .expect("Meta-schema should be valid")
        }

        #[cfg(not(target_family = "wasm"))]
        pub(crate) static DRAFT4_META_VALIDATOR: LazyLock<Validator> =
            LazyLock::new(|| build_validator(&referencing::meta::DRAFT4));
        #[cfg(target_family = "wasm")]
        pub(crate) fn draft4_meta_validator() -> Validator {
            build_validator(&referencing::meta::DRAFT4)
        }

        #[cfg(not(target_family = "wasm"))]
        pub(crate) static DRAFT6_META_VALIDATOR: LazyLock<Validator> =
            LazyLock::new(|| build_validator(&referencing::meta::DRAFT6));
        #[cfg(target_family = "wasm")]
        pub(crate) fn draft6_meta_validator() -> Validator {
            build_validator(&referencing::meta::DRAFT6)
        }

        #[cfg(not(target_family = "wasm"))]
        pub(crate) static DRAFT7_META_VALIDATOR: LazyLock<Validator> =
            LazyLock::new(|| build_validator(&referencing::meta::DRAFT7));
        #[cfg(target_family = "wasm")]
        pub(crate) fn draft7_meta_validator() -> Validator {
            build_validator(&referencing::meta::DRAFT7)
        }

        #[cfg(not(target_family = "wasm"))]
        pub(crate) static DRAFT201909_META_VALIDATOR: LazyLock<Validator> =
            LazyLock::new(|| build_validator(&referencing::meta::DRAFT201909));
        #[cfg(target_family = "wasm")]
        pub(crate) fn draft201909_meta_validator() -> Validator {
            build_validator(&referencing::meta::DRAFT201909)
        }

        #[cfg(not(target_family = "wasm"))]
        pub(crate) static DRAFT202012_META_VALIDATOR: LazyLock<Validator> =
            LazyLock::new(|| build_validator(&referencing::meta::DRAFT202012));
        #[cfg(target_family = "wasm")]
        pub(crate) fn draft202012_meta_validator() -> Validator {
            build_validator(&referencing::meta::DRAFT202012)
        }

        // Backs `evaluate` and the other `Deref` methods of a generated meta validator.
        #[cfg(all(feature = "macros", not(target_family = "wasm")))]
        pub(crate) fn runtime_validator_for_draft(draft: crate::Draft) -> &'static Validator {
            use crate::Draft;
            match draft {
                Draft::Draft4 => &DRAFT4_META_VALIDATOR,
                Draft::Draft6 => &DRAFT6_META_VALIDATOR,
                Draft::Draft7 => &DRAFT7_META_VALIDATOR,
                Draft::Draft201909 => &DRAFT201909_META_VALIDATOR,
                _ => &DRAFT202012_META_VALIDATOR,
            }
        }
    }

    pub(crate) fn validator_for_draft(draft: Draft) -> MetaValidator<'static> {
        #[cfg(all(feature = "macros", not(target_family = "wasm")))]
        {
            MetaValidator::generated(draft)
        }
        #[cfg(all(not(feature = "macros"), not(target_family = "wasm")))]
        {
            match draft {
                Draft::Draft4 => MetaValidator::borrowed(&validators::DRAFT4_META_VALIDATOR),
                Draft::Draft6 => MetaValidator::borrowed(&validators::DRAFT6_META_VALIDATOR),
                Draft::Draft7 => MetaValidator::borrowed(&validators::DRAFT7_META_VALIDATOR),
                Draft::Draft201909 => {
                    MetaValidator::borrowed(&validators::DRAFT201909_META_VALIDATOR)
                }
                // Draft202012, Unknown, or any future draft variants
                _ => MetaValidator::borrowed(&validators::DRAFT202012_META_VALIDATOR),
            }
        }
        #[cfg(target_family = "wasm")]
        {
            let validator = match draft {
                Draft::Draft4 => validators::draft4_meta_validator(),
                Draft::Draft6 => validators::draft6_meta_validator(),
                Draft::Draft7 => validators::draft7_meta_validator(),
                Draft::Draft201909 => validators::draft201909_meta_validator(),
                // Draft202012, Unknown, or any future draft variants
                _ => validators::draft202012_meta_validator(),
            };
            MetaValidator::owned(validator)
        }
    }

    /// Whether its meta-schema accepts `schema`. The draft is detected from `$schema`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use serde_json::json;
    ///
    /// let schema = json!({
    ///     "type": "string",
    ///     "maxLength": 5
    /// });
    /// assert!(jsonschema::meta::is_valid(&schema));
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the meta-schema cannot be resolved.
    ///
    /// # Note
    ///
    /// A custom `$schema` is fetched with the default retriever; to supply the meta-schema
    /// yourself, use [`meta::options().with_registry(...)`](crate::meta::options).
    #[must_use]
    pub fn is_valid(schema: &Value) -> bool {
        match try_meta_validator_for(schema, None) {
            Ok(validator) => validator.is_valid(schema),
            Err(error) => panic!("Failed to resolve meta-schema: {error}"),
        }
    }
    /// Validate a JSON Schema document against its meta-schema and return the first error if any.
    /// Draft version is detected automatically.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use serde_json::json;
    ///
    /// let schema = json!({
    ///     "type": "string",
    ///     "maxLength": 5
    /// });
    /// assert!(jsonschema::meta::validate(&schema).is_ok());
    ///
    /// // Invalid schema
    /// let invalid_schema = json!({
    ///     "type": "invalid_type"
    /// });
    /// assert!(jsonschema::meta::validate(&invalid_schema).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the first [`ValidationError`] describing why the schema violates the detected
    /// meta-schema, or an error if the meta-schema cannot be resolved.
    ///
    /// # Note
    ///
    /// A custom `$schema` is fetched with the default retriever; to supply the meta-schema
    /// yourself, use [`meta::options().with_registry(...)`](crate::meta::options).
    pub fn validate(schema: &Value) -> Result<(), ValidationError<'_>> {
        let validator = try_meta_validator_for(schema, None)?;
        validator.validate(schema)
    }

    /// Build a validator for a JSON Schema's meta-schema.
    /// Draft version is detected automatically.
    ///
    /// Returns a [`MetaValidator`] that can be used to validate the schema or access
    /// structured validation output via the evaluate API.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use serde_json::json;
    ///
    /// let schema = json!({
    ///     "type": "string",
    ///     "maxLength": 5
    /// });
    ///
    /// let validator = jsonschema::meta::validator_for(&schema)
    ///     .expect("Valid meta-schema");
    ///
    /// // Use evaluate API for structured output
    /// let evaluation = validator.evaluate(&schema);
    /// assert!(evaluation.flag().valid);
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] if the meta-schema cannot be resolved or built.
    ///
    /// # Note
    ///
    /// A custom `$schema` is fetched with the default retriever; to supply the meta-schema
    /// yourself, use [`meta::options().with_registry(...)`](crate::meta::options).
    pub fn validator_for(
        schema: &Value,
    ) -> Result<MetaValidator<'static>, ValidationError<'static>> {
        try_meta_validator_for(schema, None)
    }

    /// Validate a schema document held in a custom JSON representation ([`json`](crate::json))
    /// against its meta-schema.
    /// Draft version is detected automatically.
    ///
    /// # Errors
    ///
    /// Returns the first [`ValidationError`], or a referencing error if the meta-schema cannot be
    /// resolved.
    ///
    /// # Panics
    ///
    /// Panics if a bundled meta-schema fails to build.
    pub fn validate_for<F: Json>(schema: F::Node<'_>) -> Result<(), ValidationError<'_>> {
        let cache = meta_cache::<F>();
        match cache.draft_of(&schema) {
            Draft::Unknown => custom_meta_validator(cache, &schema)?.validate(schema),
            draft => cache.validator(draft).validate(schema),
        }
    }

    /// Check a schema document held in a custom JSON representation against its meta-schema.
    /// Draft version is detected automatically.
    ///
    /// # Errors
    ///
    /// Returns a referencing error if the meta-schema cannot be resolved.
    ///
    /// # Panics
    ///
    /// Panics if a bundled meta-schema fails to build.
    pub fn is_valid_for<F: Json>(schema: F::Node<'_>) -> Result<bool, ValidationError<'static>> {
        let cache = meta_cache::<F>();
        Ok(match cache.draft_of(&schema) {
            Draft::Unknown => custom_meta_validator(cache, &schema)?.is_valid(schema),
            draft => cache.validator(draft).is_valid(schema),
        })
    }

    /// Meta-schemas outside the bundled drafts are resolved through the `$schema` chain.
    fn custom_meta_validator<F: Json>(
        cache: &MetaCache<F>,
        schema: &F::Node<'_>,
    ) -> Result<Validator<F>, ValidationError<'static>> {
        let uri = cache
            .meta_schema_uri(schema)
            .expect("`$schema` must exist when draft is Unknown")
            .into_owned();
        let (custom_meta_schema, resolved_draft) = resolve_meta_schema_chain(&uri)?;
        crate::options_for::<F>()
            .with_draft(resolved_draft)
            .without_schema_validation()
            .build(&custom_meta_schema)
    }

    /// A `Validator<F>` only accepts nodes of the representation it was built for, so meta-schema
    /// state is shared per `F` rather than globally.
    struct MetaCache<F: Json> {
        schema_key: F::PreparedKey,
        validators: [OnceLock<Validator<F>>; 5],
    }

    impl<F: Json> MetaCache<F> {
        fn meta_schema_uri<'a>(&self, schema: &F::Node<'a>) -> Option<Cow<'a, str>> {
            schema.as_object()?.get(&self.schema_key)?.as_string()
        }

        fn draft_of(&self, schema: &F::Node<'_>) -> Draft {
            self.meta_schema_uri(schema)
                .map_or_else(Draft::default, |uri| Draft::from_schema_uri(&uri))
        }

        fn validator(&self, draft: Draft) -> &Validator<F> {
            let (index, meta_schema) = match draft {
                Draft::Draft4 => (0, &referencing::meta::DRAFT4),
                Draft::Draft6 => (1, &referencing::meta::DRAFT6),
                Draft::Draft7 => (2, &referencing::meta::DRAFT7),
                Draft::Draft201909 => (3, &referencing::meta::DRAFT201909),
                // Draft202012, Unknown, or any future draft variants
                _ => (4, &referencing::meta::DRAFT202012),
            };
            self.validators[index].get_or_init(|| {
                crate::options_for::<F>()
                    .without_schema_validation()
                    .build(meta_schema)
                    .expect("Meta-schema should be valid")
            })
        }
    }

    /// Statics cannot be generic over `F`, so entries are found by type. There is one per
    /// representation reaching this code, which a linear scan covers.
    fn meta_cache<F: Json>() -> &'static MetaCache<F> {
        static CACHE: RwLock<Vec<(TypeId, &'static (dyn Any + Send + Sync))>> =
            RwLock::new(Vec::new());

        fn find<F: Json>(
            entries: &[(TypeId, &'static (dyn Any + Send + Sync))],
        ) -> Option<&'static MetaCache<F>> {
            let type_id = TypeId::of::<F>();
            entries.iter().find(|(id, _)| *id == type_id).map(|(_, e)| {
                e.downcast_ref()
                    .expect("Entries are found by representation type")
            })
        }

        if let Some(cache) = find::<F>(&CACHE.read().expect("Meta-validator cache is poisoned")) {
            return cache;
        }
        let mut entries = CACHE.write().expect("Meta-validator cache is poisoned");
        if let Some(cache) = find::<F>(&entries) {
            return cache;
        }
        let cache: &'static MetaCache<F> = Box::leak(Box::new(MetaCache {
            schema_key: F::prepare_key("$schema"),
            validators: Default::default(),
        }));
        entries.push((TypeId::of::<F>(), cache));
        cache
    }

    fn try_meta_validator_for<'a>(
        schema: &Value,
        registry: Option<&'a Registry<'a>>,
    ) -> Result<MetaValidator<'a>, ValidationError<'static>> {
        let draft = Draft::default().detect(schema);

        // For custom meta-schemas (Draft::Unknown), attempt to resolve the meta-schema
        if draft == Draft::Unknown {
            if let Some(meta_schema_uri) = schema
                .as_object()
                .and_then(|obj| obj.get("$schema"))
                .and_then(|s| s.as_str())
            {
                // A registry, when given, is the only source of the meta-schema
                if let Some(registry) = registry {
                    let (custom_meta_schema, resolved_draft) =
                        resolve_meta_schema_with_registry(meta_schema_uri, registry)?;
                    let validator = crate::options()
                        .with_draft(resolved_draft)
                        .with_registry(registry)
                        .with_base_uri(meta_schema_uri.trim_end_matches('#'))
                        .without_schema_validation()
                        .build(&custom_meta_schema)?;
                    return Ok(MetaValidator::owned(validator));
                }

                // Use default retriever
                let (custom_meta_schema, resolved_draft) =
                    resolve_meta_schema_chain(meta_schema_uri)?;
                let validator = crate::options()
                    .with_draft(resolved_draft)
                    .without_schema_validation()
                    .build(&custom_meta_schema)?;
                return Ok(MetaValidator::owned(validator));
            }
        }

        Ok(validator_for_draft(draft))
    }

    fn resolve_meta_schema_with_registry(
        uri: &str,
        registry: &Registry<'_>,
    ) -> Result<(Value, Draft), ValidationError<'static>> {
        let resolver = registry.resolver(referencing::uri::from_str(uri)?);
        let first_resolved = resolver.lookup("")?;
        let first_meta_schema = first_resolved.contents().clone();

        let draft = walk_meta_schema_chain(uri, |current_uri| {
            let resolver = registry.resolver(referencing::uri::from_str(current_uri)?);
            let resolved = resolver.lookup("")?;
            Ok(resolved.contents().clone())
        })?;

        Ok((first_meta_schema, draft))
    }

    fn resolve_meta_schema_chain(uri: &str) -> Result<(Value, Draft), ValidationError<'static>> {
        let retriever = crate::retriever::DefaultRetriever;
        let first_meta_uri = referencing::uri::from_str(uri)?;
        let first_meta_schema = retriever
            .retrieve(&first_meta_uri)
            .map_err(|e| referencing::Error::unretrievable(uri, e))?;

        let draft = walk_meta_schema_chain(uri, |current_uri| {
            let meta_uri = referencing::uri::from_str(current_uri)?;
            retriever
                .retrieve(&meta_uri)
                .map_err(|e| referencing::Error::unretrievable(current_uri, e))
        })?;

        Ok((first_meta_schema, draft))
    }

    pub(crate) fn walk_meta_schema_chain(
        start_uri: &str,
        mut fetch: impl FnMut(&str) -> Result<Value, referencing::Error>,
    ) -> Result<Draft, referencing::Error> {
        let mut visited = AHashSet::new();
        let mut current_uri = start_uri.to_string();

        loop {
            if !visited.insert(current_uri.clone()) {
                return Err(referencing::Error::circular_metaschema(current_uri));
            }

            let meta_schema = fetch(&current_uri)?;
            let draft = Draft::default().detect(&meta_schema);

            if draft != Draft::Unknown {
                return Ok(draft);
            }

            current_uri = meta_schema
                .get("$schema")
                .and_then(|s| s.as_str())
                .expect("`$schema` must exist when draft is Unknown")
                .to_string();
        }
    }
}

/// Validators for JSON Schema Draft 4.
///
/// [![Draft 4](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft4.json)](https://bowtie.report/#/implementations/rust-jsonschema)
///
/// # Examples
///
/// ```rust
/// use serde_json::json;
///
/// let schema = json!({"type": "number", "multipleOf": 2});
/// let instance = json!(4);
///
/// assert!(jsonschema::draft4::is_valid(&schema, &instance));
/// ```
pub mod draft4 {
    use super::{Draft, ValidationError, ValidationOptions, Validator, Value};

    /// Create a new JSON Schema validator using Draft 4 specifications.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({"minimum": 5});
    /// let instance = json!(42);
    ///
    /// let validator = jsonschema::draft4::new(&schema)?;
    /// assert!(validator.is_valid(&instance));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the schema is not a valid Draft 4 document or if referenced resources
    /// cannot be resolved.
    pub fn new(schema: &Value) -> Result<Validator, ValidationError<'static>> {
        options().build(schema)
    }
    /// Whether `schema` accepts `instance` under Draft 4, building a validator per call.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use serde_json::json;
    ///
    /// let schema = json!({"minimum": 5});
    /// let valid = json!(42);
    /// let invalid = json!(3);
    ///
    /// assert!(jsonschema::draft4::is_valid(&schema, &valid));
    /// assert!(!jsonschema::draft4::is_valid(&schema, &invalid));
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `schema` cannot be compiled into a Draft 4 validator.
    #[must_use]
    pub fn is_valid(schema: &Value, instance: &Value) -> bool {
        new(schema).expect("Invalid schema").is_valid(instance)
    }
    /// Validate `instance` against `schema` under Draft 4 and return the first error,
    /// building a validator per call.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use serde_json::json;
    ///
    /// let schema = json!({"minimum": 5});
    /// let valid = json!(42);
    /// let invalid = json!(3);
    ///
    /// assert!(jsonschema::draft4::validate(&schema, &valid).is_ok());
    /// assert!(jsonschema::draft4::validate(&schema, &invalid).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the first [`ValidationError`] when `instance` violates the schema.
    ///
    /// # Panics
    ///
    /// Panics if `schema` cannot be compiled into a Draft 4 validator.
    pub fn validate<'i>(schema: &Value, instance: &'i Value) -> Result<(), ValidationError<'i>> {
        new(schema).expect("Invalid schema").validate(instance)
    }
    /// Creates a [`ValidationOptions`] builder pre-configured for JSON Schema Draft 4.
    ///
    /// This function provides a shorthand for `jsonschema::options().with_draft(Draft::Draft4)`.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({"type": "string", "format": "ends-with-42"});
    /// let validator = jsonschema::draft4::options()
    ///     .with_format("ends-with-42", |s| s.ends_with("42"))
    ///     .should_validate_formats(true)
    ///     .build(&schema)?;
    ///
    /// assert!(validator.is_valid(&json!("Hello 42")));
    /// assert!(!validator.is_valid(&json!("No!")));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See [`ValidationOptions`] for all available configuration options.
    #[must_use]
    pub fn options<'i>() -> ValidationOptions<'i> {
        crate::options().with_draft(Draft::Draft4)
    }

    /// Checking schemas against the Draft 4 meta-schema.
    pub mod meta {
        use crate::{meta::MetaValidator, ValidationError};
        use serde_json::Value;

        /// Returns a handle to the Draft 4 meta-schema validator.
        #[must_use]
        pub fn validator() -> MetaValidator<'static> {
            crate::meta::validator_for_draft(super::Draft::Draft4)
        }

        /// Whether the Draft 4 meta-schema accepts `schema`.
        ///
        /// # Examples
        ///
        /// ```rust
        /// use serde_json::json;
        ///
        /// let schema = json!({
        ///     "type": "string",
        ///     "maxLength": 5
        /// });
        /// assert!(jsonschema::draft4::meta::is_valid(&schema));
        /// ```
        #[must_use]
        #[inline]
        pub fn is_valid(schema: &Value) -> bool {
            validator().is_valid(schema)
        }

        /// Validate a JSON Schema document against Draft 4 meta-schema and return the first error if any.
        ///
        /// # Examples
        ///
        /// ```rust
        /// use serde_json::json;
        ///
        /// let schema = json!({
        ///     "type": "string",
        ///     "maxLength": 5
        /// });
        /// assert!(jsonschema::draft4::meta::validate(&schema).is_ok());
        ///
        /// // Invalid schema
        /// let invalid_schema = json!({
        ///     "type": "invalid_type"
        /// });
        /// assert!(jsonschema::draft4::meta::validate(&invalid_schema).is_err());
        /// ```
        ///
        /// # Errors
        ///
        /// Returns the first [`ValidationError`] describing why the schema violates the Draft 4 meta-schema.
        #[inline]
        pub fn validate(schema: &Value) -> Result<(), ValidationError<'_>> {
            validator().validate(schema)
        }
    }
}

/// Validators for JSON Schema Draft 6.
///
/// [![Draft 6](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft6.json)](https://bowtie.report/#/implementations/rust-jsonschema)
///
/// # Examples
///
/// ```rust
/// use serde_json::json;
///
/// let schema = json!({"type": "string", "format": "uri"});
/// let instance = json!("https://www.example.com");
///
/// assert!(jsonschema::draft6::is_valid(&schema, &instance));
/// ```
pub mod draft6 {
    use super::{Draft, ValidationError, ValidationOptions, Validator, Value};

    /// Create a new JSON Schema validator using Draft 6 specifications.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({"minimum": 5});
    /// let instance = json!(42);
    ///
    /// let validator = jsonschema::draft6::new(&schema)?;
    /// assert!(validator.is_valid(&instance));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the schema is not a valid Draft 6 document or if referenced resources
    /// cannot be resolved.
    pub fn new(schema: &Value) -> Result<Validator, ValidationError<'static>> {
        options().build(schema)
    }
    /// Whether `schema` accepts `instance` under Draft 6, building a validator per call.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use serde_json::json;
    ///
    /// let schema = json!({"minimum": 5});
    /// let valid = json!(42);
    /// let invalid = json!(3);
    ///
    /// assert!(jsonschema::draft6::is_valid(&schema, &valid));
    /// assert!(!jsonschema::draft6::is_valid(&schema, &invalid));
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `schema` cannot be compiled into a Draft 6 validator.
    #[must_use]
    pub fn is_valid(schema: &Value, instance: &Value) -> bool {
        new(schema).expect("Invalid schema").is_valid(instance)
    }
    /// Validate `instance` against `schema` under Draft 6 and return the first error,
    /// building a validator per call.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use serde_json::json;
    ///
    /// let schema = json!({"minimum": 5});
    /// let valid = json!(42);
    /// let invalid = json!(3);
    ///
    /// assert!(jsonschema::draft6::validate(&schema, &valid).is_ok());
    /// assert!(jsonschema::draft6::validate(&schema, &invalid).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the first [`ValidationError`] when `instance` violates the schema.
    ///
    /// # Panics
    ///
    /// Panics if `schema` cannot be compiled into a Draft 6 validator.
    pub fn validate<'i>(schema: &Value, instance: &'i Value) -> Result<(), ValidationError<'i>> {
        new(schema).expect("Invalid schema").validate(instance)
    }
    /// Creates a [`ValidationOptions`] builder pre-configured for JSON Schema Draft 6.
    ///
    /// This function provides a shorthand for `jsonschema::options().with_draft(Draft::Draft6)`.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({"type": "string", "format": "ends-with-42"});
    /// let validator = jsonschema::draft6::options()
    ///     .with_format("ends-with-42", |s| s.ends_with("42"))
    ///     .should_validate_formats(true)
    ///     .build(&schema)?;
    ///
    /// assert!(validator.is_valid(&json!("Hello 42")));
    /// assert!(!validator.is_valid(&json!("No!")));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See [`ValidationOptions`] for all available configuration options.
    #[must_use]
    pub fn options<'i>() -> ValidationOptions<'i> {
        crate::options().with_draft(Draft::Draft6)
    }

    /// Checking schemas against the Draft 6 meta-schema.
    pub mod meta {
        use crate::{meta::MetaValidator, ValidationError};
        use serde_json::Value;

        /// Returns a handle to the Draft 6 meta-schema validator.
        #[must_use]
        pub fn validator() -> MetaValidator<'static> {
            crate::meta::validator_for_draft(super::Draft::Draft6)
        }

        /// Whether the Draft 6 meta-schema accepts `schema`.
        ///
        /// # Examples
        ///
        /// ```rust
        /// use serde_json::json;
        ///
        /// let schema = json!({
        ///     "type": "string",
        ///     "maxLength": 5
        /// });
        /// assert!(jsonschema::draft6::meta::is_valid(&schema));
        /// ```
        #[must_use]
        #[inline]
        pub fn is_valid(schema: &Value) -> bool {
            validator().is_valid(schema)
        }

        /// Validate a JSON Schema document against Draft 6 meta-schema and return the first error if any.
        ///
        /// # Examples
        ///
        /// ```rust
        /// use serde_json::json;
        ///
        /// let schema = json!({
        ///     "type": "string",
        ///     "maxLength": 5
        /// });
        /// assert!(jsonschema::draft6::meta::validate(&schema).is_ok());
        ///
        /// // Invalid schema
        /// let invalid_schema = json!({
        ///     "type": "invalid_type"
        /// });
        /// assert!(jsonschema::draft6::meta::validate(&invalid_schema).is_err());
        /// ```
        ///
        /// # Errors
        ///
        /// Returns the first [`ValidationError`] describing why the schema violates the Draft 6 meta-schema.
        #[inline]
        pub fn validate(schema: &Value) -> Result<(), ValidationError<'_>> {
            validator().validate(schema)
        }
    }
}

/// Validators for JSON Schema Draft 7.
///
/// [![Draft 7](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft7.json)](https://bowtie.report/#/implementations/rust-jsonschema)
///
/// # Examples
///
/// ```rust
/// use serde_json::json;
///
/// let schema = json!({"type": "string", "pattern": "^[a-zA-Z0-9]+$"});
/// let instance = json!("abc123");
///
/// assert!(jsonschema::draft7::is_valid(&schema, &instance));
/// ```
pub mod draft7 {
    use super::{Draft, ValidationError, ValidationOptions, Validator, Value};

    /// Create a new JSON Schema validator using Draft 7 specifications.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({"minimum": 5});
    /// let instance = json!(42);
    ///
    /// let validator = jsonschema::draft7::new(&schema)?;
    /// assert!(validator.is_valid(&instance));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the schema is not a valid Draft 7 document or if referenced resources
    /// cannot be resolved.
    pub fn new(schema: &Value) -> Result<Validator, ValidationError<'static>> {
        options().build(schema)
    }
    /// Whether `schema` accepts `instance` under Draft 7, building a validator per call.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use serde_json::json;
    ///
    /// let schema = json!({"minimum": 5});
    /// let valid = json!(42);
    /// let invalid = json!(3);
    ///
    /// assert!(jsonschema::draft7::is_valid(&schema, &valid));
    /// assert!(!jsonschema::draft7::is_valid(&schema, &invalid));
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `schema` cannot be compiled into a Draft 7 validator.
    #[must_use]
    pub fn is_valid(schema: &Value, instance: &Value) -> bool {
        new(schema).expect("Invalid schema").is_valid(instance)
    }
    /// Validate `instance` against `schema` under Draft 7 and return the first error,
    /// building a validator per call.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use serde_json::json;
    ///
    /// let schema = json!({"minimum": 5});
    /// let valid = json!(42);
    /// let invalid = json!(3);
    ///
    /// assert!(jsonschema::draft7::validate(&schema, &valid).is_ok());
    /// assert!(jsonschema::draft7::validate(&schema, &invalid).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the first [`ValidationError`] when `instance` violates the schema.
    ///
    /// # Panics
    ///
    /// Panics if `schema` cannot be compiled into a Draft 7 validator.
    pub fn validate<'i>(schema: &Value, instance: &'i Value) -> Result<(), ValidationError<'i>> {
        new(schema).expect("Invalid schema").validate(instance)
    }
    /// Creates a [`ValidationOptions`] builder pre-configured for JSON Schema Draft 7.
    ///
    /// This function provides a shorthand for `jsonschema::options().with_draft(Draft::Draft7)`.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({"type": "string", "format": "ends-with-42"});
    /// let validator = jsonschema::draft7::options()
    ///     .with_format("ends-with-42", |s| s.ends_with("42"))
    ///     .should_validate_formats(true)
    ///     .build(&schema)?;
    ///
    /// assert!(validator.is_valid(&json!("Hello 42")));
    /// assert!(!validator.is_valid(&json!("No!")));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See [`ValidationOptions`] for all available configuration options.
    #[must_use]
    pub fn options<'i>() -> ValidationOptions<'i> {
        crate::options().with_draft(Draft::Draft7)
    }

    /// Checking schemas against the Draft 7 meta-schema.
    pub mod meta {
        use crate::{meta::MetaValidator, ValidationError};
        use serde_json::Value;

        /// Returns a handle to the Draft 7 meta-schema validator.
        #[must_use]
        pub fn validator() -> MetaValidator<'static> {
            crate::meta::validator_for_draft(super::Draft::Draft7)
        }

        /// Whether the Draft 7 meta-schema accepts `schema`.
        ///
        /// # Examples
        ///
        /// ```rust
        /// use serde_json::json;
        ///
        /// let schema = json!({
        ///     "type": "string",
        ///     "maxLength": 5
        /// });
        /// assert!(jsonschema::draft7::meta::is_valid(&schema));
        /// ```
        #[must_use]
        #[inline]
        pub fn is_valid(schema: &Value) -> bool {
            validator().is_valid(schema)
        }

        /// Validate a JSON Schema document against Draft 7 meta-schema and return the first error if any.
        ///
        /// # Examples
        ///
        /// ```rust
        /// use serde_json::json;
        ///
        /// let schema = json!({
        ///     "type": "string",
        ///     "maxLength": 5
        /// });
        /// assert!(jsonschema::draft7::meta::validate(&schema).is_ok());
        ///
        /// // Invalid schema
        /// let invalid_schema = json!({
        ///     "type": "invalid_type"
        /// });
        /// assert!(jsonschema::draft7::meta::validate(&invalid_schema).is_err());
        /// ```
        ///
        /// # Errors
        ///
        /// Returns the first [`ValidationError`] describing why the schema violates the Draft 7 meta-schema.
        #[inline]
        pub fn validate(schema: &Value) -> Result<(), ValidationError<'_>> {
            validator().validate(schema)
        }
    }
}

/// Validators for JSON Schema Draft 2019-09.
///
/// [![Draft 2019-09](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft2019-09.json)](https://bowtie.report/#/implementations/rust-jsonschema)
///
/// # Examples
///
/// ```rust
/// use serde_json::json;
///
/// let schema = json!({"type": "array", "minItems": 2, "uniqueItems": true});
/// let instance = json!([1, 2]);
///
/// assert!(jsonschema::draft201909::is_valid(&schema, &instance));
/// ```
pub mod draft201909 {
    use super::{Draft, ValidationError, ValidationOptions, Validator, Value};

    /// Create a new JSON Schema validator using Draft 2019-09 specifications.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({"minimum": 5});
    /// let instance = json!(42);
    ///
    /// let validator = jsonschema::draft201909::new(&schema)?;
    /// assert!(validator.is_valid(&instance));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the schema is not a valid Draft 2019-09 document or if referenced resources
    /// cannot be resolved.
    pub fn new(schema: &Value) -> Result<Validator, ValidationError<'static>> {
        options().build(schema)
    }
    /// Whether `schema` accepts `instance` under Draft 2019-09, building a validator per call.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use serde_json::json;
    ///
    /// let schema = json!({"minimum": 5});
    /// let valid = json!(42);
    /// let invalid = json!(3);
    ///
    /// assert!(jsonschema::draft201909::is_valid(&schema, &valid));
    /// assert!(!jsonschema::draft201909::is_valid(&schema, &invalid));
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `schema` cannot be compiled into a Draft 2019-09 validator.
    #[must_use]
    pub fn is_valid(schema: &Value, instance: &Value) -> bool {
        new(schema).expect("Invalid schema").is_valid(instance)
    }
    /// Validate `instance` against `schema` under Draft 2019-09 and return the first error,
    /// building a validator per call.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use serde_json::json;
    ///
    /// let schema = json!({"minimum": 5});
    /// let valid = json!(42);
    /// let invalid = json!(3);
    ///
    /// assert!(jsonschema::draft201909::validate(&schema, &valid).is_ok());
    /// assert!(jsonschema::draft201909::validate(&schema, &invalid).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the first [`ValidationError`] when `instance` violates the schema.
    ///
    /// # Panics
    ///
    /// Panics if `schema` cannot be compiled into a Draft 2019-09 validator.
    pub fn validate<'i>(schema: &Value, instance: &'i Value) -> Result<(), ValidationError<'i>> {
        new(schema).expect("Invalid schema").validate(instance)
    }
    /// Creates a [`ValidationOptions`] builder pre-configured for JSON Schema Draft 2019-09.
    ///
    /// This function provides a shorthand for `jsonschema::options().with_draft(Draft::Draft201909)`.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({"type": "string", "format": "ends-with-42"});
    /// let validator = jsonschema::draft201909::options()
    ///     .with_format("ends-with-42", |s| s.ends_with("42"))
    ///     .should_validate_formats(true)
    ///     .build(&schema)?;
    ///
    /// assert!(validator.is_valid(&json!("Hello 42")));
    /// assert!(!validator.is_valid(&json!("No!")));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See [`ValidationOptions`] for all available configuration options.
    #[must_use]
    pub fn options<'i>() -> ValidationOptions<'i> {
        crate::options().with_draft(Draft::Draft201909)
    }

    /// Checking schemas against the Draft 2019-09 meta-schema.
    pub mod meta {
        use crate::{meta::MetaValidator, ValidationError};
        use serde_json::Value;

        /// Returns a handle to the Draft 2019-09 meta-schema validator.
        #[must_use]
        pub fn validator() -> MetaValidator<'static> {
            crate::meta::validator_for_draft(super::Draft::Draft201909)
        }
        /// Whether the Draft 2019-09 meta-schema accepts `schema`.
        ///
        /// # Examples
        ///
        /// ```rust
        /// use serde_json::json;
        ///
        /// let schema = json!({
        ///     "type": "string",
        ///     "maxLength": 5
        /// });
        /// assert!(jsonschema::draft201909::meta::is_valid(&schema));
        /// ```
        #[must_use]
        #[inline]
        pub fn is_valid(schema: &Value) -> bool {
            validator().is_valid(schema)
        }

        /// Validate a JSON Schema document against Draft 2019-09 meta-schema and return the first error if any.
        ///
        /// # Examples
        ///
        /// ```rust
        /// use serde_json::json;
        ///
        /// let schema = json!({
        ///     "type": "string",
        ///     "maxLength": 5
        /// });
        /// assert!(jsonschema::draft201909::meta::validate(&schema).is_ok());
        ///
        /// // Invalid schema
        /// let invalid_schema = json!({
        ///     "type": "invalid_type"
        /// });
        /// assert!(jsonschema::draft201909::meta::validate(&invalid_schema).is_err());
        /// ```
        ///
        /// # Errors
        ///
        /// Returns the first [`ValidationError`] describing why the schema violates the Draft 2019-09 meta-schema.
        #[inline]
        pub fn validate(schema: &Value) -> Result<(), ValidationError<'_>> {
            validator().validate(schema)
        }
    }
}

/// Validators for JSON Schema Draft 2020-12.
///
/// [![Draft 2020-12](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft2020-12.json)](https://bowtie.report/#/implementations/rust-jsonschema)
///
/// # Examples
///
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]});
/// let instance = json!({"name": "John Doe"});
///
/// assert!(jsonschema::draft202012::is_valid(&schema, &instance));
/// # Ok(())
/// # }
/// ```
pub mod draft202012 {
    use super::{Draft, ValidationError, ValidationOptions, Validator, Value};

    /// Create a new JSON Schema validator using Draft 2020-12 specifications.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({"minimum": 5});
    /// let instance = json!(42);
    ///
    /// let validator = jsonschema::draft202012::new(&schema)?;
    /// assert!(validator.is_valid(&instance));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the schema is not a valid Draft 2020-12 document or if referenced resources
    /// cannot be resolved.
    pub fn new(schema: &Value) -> Result<Validator, ValidationError<'static>> {
        options().build(schema)
    }
    /// Whether `schema` accepts `instance` under Draft 2020-12, building a validator per call.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use serde_json::json;
    ///
    /// let schema = json!({"minimum": 5});
    /// let valid = json!(42);
    /// let invalid = json!(3);
    ///
    /// assert!(jsonschema::draft202012::is_valid(&schema, &valid));
    /// assert!(!jsonschema::draft202012::is_valid(&schema, &invalid));
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `schema` cannot be compiled into a Draft 2020-12 validator.
    #[must_use]
    pub fn is_valid(schema: &Value, instance: &Value) -> bool {
        new(schema).expect("Invalid schema").is_valid(instance)
    }
    /// Validate `instance` against `schema` under Draft 2020-12 and return the first error,
    /// building a validator per call.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use serde_json::json;
    ///
    /// let schema = json!({"minimum": 5});
    /// let valid = json!(42);
    /// let invalid = json!(3);
    ///
    /// assert!(jsonschema::draft202012::validate(&schema, &valid).is_ok());
    /// assert!(jsonschema::draft202012::validate(&schema, &invalid).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the first [`ValidationError`] when `instance` violates the schema.
    ///
    /// # Panics
    ///
    /// Panics if `schema` cannot be compiled into a Draft 2020-12 validator.
    pub fn validate<'i>(schema: &Value, instance: &'i Value) -> Result<(), ValidationError<'i>> {
        new(schema).expect("Invalid schema").validate(instance)
    }
    /// Creates a [`ValidationOptions`] builder pre-configured for JSON Schema Draft 2020-12.
    ///
    /// This function provides a shorthand for `jsonschema::options().with_draft(Draft::Draft202012)`.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({"type": "string", "format": "ends-with-42"});
    /// let validator = jsonschema::draft202012::options()
    ///     .with_format("ends-with-42", |s| s.ends_with("42"))
    ///     .should_validate_formats(true)
    ///     .build(&schema)?;
    ///
    /// assert!(validator.is_valid(&json!("Hello 42")));
    /// assert!(!validator.is_valid(&json!("No!")));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See [`ValidationOptions`] for all available configuration options.
    #[must_use]
    pub fn options<'i>() -> ValidationOptions<'i> {
        crate::options().with_draft(Draft::Draft202012)
    }

    /// Checking schemas against the Draft 2020-12 meta-schema.
    pub mod meta {
        use crate::{meta::MetaValidator, ValidationError};
        use serde_json::Value;

        /// Returns a handle to the Draft 2020-12 meta-schema validator.
        #[must_use]
        pub fn validator() -> MetaValidator<'static> {
            crate::meta::validator_for_draft(super::Draft::Draft202012)
        }

        /// Whether the Draft 2020-12 meta-schema accepts `schema`.
        ///
        /// # Examples
        ///
        /// ```rust
        /// use serde_json::json;
        ///
        /// let schema = json!({
        ///     "type": "string",
        ///     "maxLength": 5
        /// });
        /// assert!(jsonschema::draft202012::meta::is_valid(&schema));
        /// ```
        #[must_use]
        #[inline]
        pub fn is_valid(schema: &Value) -> bool {
            validator().is_valid(schema)
        }

        /// Validate a JSON Schema document against Draft 2020-12 meta-schema and return the first error if any.
        ///
        /// # Examples
        ///
        /// ```rust
        /// use serde_json::json;
        ///
        /// let schema = json!({
        ///     "type": "string",
        ///     "maxLength": 5
        /// });
        /// assert!(jsonschema::draft202012::meta::validate(&schema).is_ok());
        ///
        /// // Invalid schema
        /// let invalid_schema = json!({
        ///     "type": "invalid_type"
        /// });
        /// assert!(jsonschema::draft202012::meta::validate(&invalid_schema).is_err());
        /// ```
        ///
        /// # Errors
        ///
        /// Returns the first [`ValidationError`] describing why the schema violates the Draft 2020-12 meta-schema.
        #[inline]
        pub fn validate(schema: &Value) -> Result<(), ValidationError<'_>> {
            validator().validate(schema)
        }
    }
}

#[cfg(feature = "macros")]
#[doc(hidden)]
pub mod __private {
    #[cfg(feature = "pyo3")]
    pub mod pyo3 {
        pub use pyo3::{intern, types::PyString, Borrowed, Bound, PyAny, PyResult};
    }

    // Expands to the generated `backend = Pyo3` code, or without the `pyo3` feature to one
    // `compile_error!` naming it.
    #[cfg(feature = "pyo3")]
    #[doc(hidden)]
    #[macro_export]
    macro_rules! __pyo3_backend {
        ($($generated:tt)*) => { $($generated)* };
    }

    #[cfg(not(feature = "pyo3"))]
    #[doc(hidden)]
    #[macro_export]
    macro_rules! __pyo3_backend {
        ($($generated:tt)*) => {
            ::core::compile_error!("`backend = Pyo3` needs the `pyo3` feature of `jsonschema`");
        };
    }

    pub use crate::__pyo3_backend as pyo3_backend;

    #[cfg(feature = "magnus")]
    pub mod magnus {
        pub use ::magnus::{rb_sys::AsRawValue, Error, Value};
    }

    // Expands to the generated `backend = Magnus` code, or without the `magnus` feature to one
    // `compile_error!` naming it.
    #[cfg(feature = "magnus")]
    #[doc(hidden)]
    #[macro_export]
    macro_rules! __magnus_backend {
        ($($generated:tt)*) => { $($generated)* };
    }

    #[cfg(not(feature = "magnus"))]
    #[doc(hidden)]
    #[macro_export]
    macro_rules! __magnus_backend {
        ($($generated:tt)*) => {
            ::core::compile_error!("`backend = Magnus` needs the `magnus` feature of `jsonschema`");
        };
    }

    pub use crate::__magnus_backend as magnus_backend;

    pub use ::serde_json;

    pub mod fancy_regex {
        pub use fancy_regex::{Regex, RegexBuilder};
    }
    pub mod regex {
        pub use jsonschema_regex::{contains_ecma_whitespace, RepeatedClass};
        pub use regex::{Regex, RegexBuilder};
    }
    /// Regex matching for generated code: a match the engine cannot finish is resolved by running
    /// validation under each of its outcomes, as the runtime validator does.
    pub mod unfinished_matches {
        use std::cell::{Cell, RefCell};

        use crate::{
            paths::LazyLocation,
            regex::FancyRegexError,
            unfinished_matches::{
                assume_each_outcome, errors, validity, MatchAssumptions, MatchSite, Outcomes,
            },
            LazyInstance, ValidationError,
        };

        std::thread_local! {
            static MATCHES: RefCell<Option<Box<MatchAssumptions>>> = const { RefCell::new(None) };
            /// Whether `MATCHES` holds a record, readable without borrowing it.
            static MET: Cell<bool> = const { Cell::new(false) };
        }

        /// Whether a run on this thread met a regex match the engine could not finish.
        #[inline]
        #[must_use]
        pub fn met() -> bool {
            MET.get()
        }

        /// Whether `regex` matches `subject`. A match the engine cannot finish is recorded at
        /// `schema_path` with the instance location when the caller tracks one.
        #[inline]
        #[must_use]
        pub fn fancy_is_match(
            regex: &fancy_regex::Regex,
            subject: &str,
            schema_path: &str,
            instance_path: Option<&LazyLocation>,
        ) -> bool {
            if MET.get() {
                if let Some(matches) =
                    resolve_unfinished(regex, subject, schema_path, instance_path)
                {
                    return matches;
                }
            }
            match regex.is_match(subject) {
                Ok(matches) => matches,
                Err(error) => resolve(regex, subject, error, schema_path, instance_path),
            }
        }

        /// The outcome for a match a run could not finish, without the engine, which would only
        /// fail again.
        #[cold]
        fn resolve_unfinished(
            regex: &fancy_regex::Regex,
            subject: &str,
            schema_path: &str,
            instance_path: Option<&LazyLocation>,
        ) -> Option<bool> {
            MATCHES.with_borrow_mut(|matches| {
                let matches = matches.as_mut()?;
                matches.is_unfinished(regex.as_str(), subject).then(|| {
                    matches.resolve_unfinished(regex.as_str(), subject, || {
                        site(schema_path, instance_path)
                    })
                })
            })
        }

        #[cold]
        fn resolve(
            regex: &fancy_regex::Regex,
            subject: &str,
            error: fancy_regex::Error,
            schema_path: &str,
            instance_path: Option<&LazyLocation>,
        ) -> bool {
            let error = FancyRegexError {
                error,
                pattern: regex.as_str().into(),
            };
            MET.set(true);
            MATCHES.with_borrow_mut(|matches| {
                matches
                    .get_or_insert_with(Box::default)
                    .resolve(subject, error, || site(schema_path, instance_path))
            })
        }

        fn site(schema_path: &str, instance_path: Option<&LazyLocation>) -> MatchSite {
            MatchSite::escaped(schema_path, instance_path.map(Into::into))
        }

        fn swap(matches: &mut Option<Box<MatchAssumptions>>) {
            MATCHES.with_borrow_mut(|current| std::mem::swap(current, matches));
            MET.set(MATCHES.with_borrow(Option::is_some));
        }

        /// Runs `run` under each combination of outcomes for the unfinished matches it meets,
        /// starting from a state of its own: the record the entry's run left may belong to an
        /// enclosing run. `Err` carries the result of a run that met none.
        #[cold]
        fn under_each_outcome<T>(mut run: impl FnMut() -> T) -> Result<Outcomes<T>, T> {
            let mut swapped = |matches: &mut Option<Box<MatchAssumptions>>| {
                swap(matches);
                let result = run();
                swap(matches);
                result
            };
            let mut first = None;
            let result = swapped(&mut first);
            match first {
                Some(first) => Ok(assume_each_outcome(swapped, *first)),
                None => Err(result),
            }
        }

        /// `is_valid` for an entry whose run met an unfinished match. `run` is the generated
        /// function itself, taken by pointer so the entry's own call to it stays inlinable.
        #[cold]
        #[must_use]
        pub fn is_valid<N: Copy>(run: fn(N) -> bool, node: N) -> bool {
            match under_each_outcome(|| run(node)) {
                Ok(outcomes) => validity(outcomes),
                Err(valid) => valid,
            }
        }

        /// `validate` for an entry whose run met an unfinished match.
        #[cold]
        #[must_use]
        pub fn validate<'i, N: Copy>(
            instance: impl FnOnce() -> LazyInstance<'i>,
            collect: fn(N, &LazyLocation, &mut super::error::Errors<'i>) -> bool,
            node: N,
        ) -> Option<ValidationError<'i>> {
            collect_errors(instance, collect, node).into_iter().next()
        }

        /// `collect_errors` for an entry whose run met an unfinished match.
        #[cold]
        #[must_use]
        pub fn collect_errors<'i, N: Copy>(
            instance: impl FnOnce() -> LazyInstance<'i>,
            collect: fn(N, &LazyLocation, &mut super::error::Errors<'i>) -> bool,
            node: N,
        ) -> Vec<ValidationError<'i>> {
            let run = || {
                let mut errors = super::error::Errors::new(false);
                let _ = collect(node, &LazyLocation::new(), &mut errors);
                errors.into_vec()
            };
            match under_each_outcome(run) {
                Ok(outcomes) => errors(outcomes, instance),
                Err(errors) => errors,
            }
        }
    }
    /// Results of `$ref` cycles that reach deeper instances, so the keywords that reach one
    /// instance through the same cycle validate it once per run.
    pub mod ref_memo {
        use std::{
            cell::{Cell, RefCell},
            hash::Hash,
            thread::LocalKey,
        };

        use ahash::AHashMap;

        use crate::{
            json::{Json, Node, NodeIdentity},
            validator::MEMO_MIN_VISITS,
        };

        type Results<K> = RefCell<Option<AHashMap<K, bool>>>;

        std::thread_local! {
            /// Targets computed so far; a call's share of it decides whether its result is kept.
            static VISITS: Cell<u32> = const { Cell::new(0) };
            static RESULTS: Results<(usize, NodeIdentity)> = const { RefCell::new(None) };
            /// Results under a dynamic scope, by the scope's entry in `SCOPES`.
            static SCOPED_RESULTS: Results<(usize, usize, NodeIdentity)> =
                const { RefCell::new(None) };
            /// The dynamic scopes seen on this thread. Code addresses stay fixed, so an identity
            /// holds across runs.
            static SCOPES: RefCell<Option<Scopes>> = const { RefCell::new(None) };
        }

        /// Each dynamic scope's identity, by the code it binds the looked-up names to.
        #[derive(Default)]
        struct Scopes {
            identities: AHashMap<Box<[usize]>, usize>,
            /// The bindings of each identity.
            bindings: Vec<Box<[usize]>>,
            /// The identity given last, which nested calls in one scope ask for again.
            last: usize,
        }

        /// Drops the results of the previous run on this thread.
        #[inline]
        pub fn clear() {
            RESULTS.with_borrow_mut(Option::take);
        }

        /// Drops the results [`is_valid_in_scope`] kept in the previous run on this thread.
        #[inline]
        pub fn clear_in_scope() {
            SCOPED_RESULTS.with_borrow_mut(Option::take);
        }

        /// `target(instance)`, computed once per container once it costs enough to keep.
        #[inline]
        pub fn is_valid<'a, F: Json>(
            target: fn(F::Node<'a>) -> bool,
            instance: F::Node<'a>,
        ) -> bool {
            let Some(identity) = instance.container_identity() else {
                return target(instance);
            };
            kept(&RESULTS, (target as usize, identity), || target(instance))
        }

        /// [`is_valid`] for a target whose result depends on the dynamic scope: `scope` gives the
        /// scope's identity, so a result is kept for the scope it was computed in.
        #[inline]
        pub fn is_valid_in_scope<'a, F: Json>(
            target: fn(F::Node<'a>) -> bool,
            instance: F::Node<'a>,
            scope: fn() -> usize,
        ) -> bool {
            let Some(identity) = instance.container_identity() else {
                return target(instance);
            };
            kept(
                &SCOPED_RESULTS,
                (target as usize, scope(), identity),
                || target(instance),
            )
        }

        /// The identity of the dynamic scope given by `bindings`: for each name a dynamic
        /// reference looks up, the address of the code bound to it, or `0` when unbound.
        #[must_use]
        #[inline]
        pub fn scope<const N: usize>(bindings: &[usize; N]) -> usize {
            SCOPES.with_borrow_mut(|scopes| {
                let scopes = scopes.get_or_insert_with(Scopes::default);
                if scopes.bindings.get(scopes.last).is_some_and(|last| {
                    last.len() == N && last.iter().zip(bindings).all(|(last, bound)| last == bound)
                }) {
                    return scopes.last;
                }
                let scope = if let Some(&scope) = scopes.identities.get(&bindings[..]) {
                    scope
                } else {
                    let scope = scopes.bindings.len();
                    scopes.identities.insert(bindings[..].into(), scope);
                    scopes.bindings.push(bindings[..].into());
                    scope
                };
                scopes.last = scope;
                scope
            })
        }

        /// The kept result under `key`, or `compute`'s, kept once it costs enough.
        #[inline]
        fn kept<K: Hash + Eq>(
            results: &'static LocalKey<Results<K>>,
            key: K,
            compute: impl FnOnce() -> bool,
        ) -> bool {
            if let Some(valid) = results.with_borrow(|results| results.as_ref()?.get(&key).copied())
            {
                return valid;
            }
            let visit = VISITS.get();
            VISITS.set(visit.wrapping_add(1));
            let valid = compute();
            if VISITS.get().wrapping_sub(visit) >= MEMO_MIN_VISITS {
                results.with_borrow_mut(|results| {
                    results.get_or_insert_with(AHashMap::new).insert(key, valid);
                });
            }
            valid
        }
    }
    pub mod unique_items {
        pub use crate::unique::is_unique;
    }
    pub mod cmp {
        pub use crate::cmp::{equal, equal_numbers};
    }
    pub mod custom {
        use crate::{
            json::{Json, Node},
            paths::Location,
        };

        #[must_use]
        pub fn location(pointer: &str) -> Location {
            Location::from_escaped(pointer)
        }

        /// Run a custom keyword and fill in error context exactly like the runtime validator's `CustomKeyword` wrapper.
        pub fn validate<'i, F: Json>(
            keyword: &dyn crate::Keyword<'i, F>,
            instance: &F::Node<'i>,
            instance_path: Location,
            schema_path: &'static str,
            keyword_name: &str,
        ) -> Option<crate::ValidationError<'i>> {
            match keyword.validate(instance.clone()) {
                Ok(()) => None,
                Err(error) => Some(error.with_generated_context(
                    instance.to_value(),
                    instance_path,
                    super::schema_location(schema_path),
                    keyword_name,
                )),
            }
        }

        /// Report a custom keyword's errors: its `validate` error when only the first is wanted,
        /// its `iter_errors` otherwise, with context filled in exactly like [`validate`] does.
        /// `true` when the caller must stop.
        pub fn errors<'i, F: Json>(
            keyword: &dyn crate::Keyword<'i, F>,
            instance: &F::Node<'i>,
            instance_path: Location,
            schema_path: &'static str,
            keyword_name: &str,
            errors: &mut super::error::Errors<'i>,
        ) -> bool {
            if errors.first_only() {
                return validate(keyword, instance, instance_path, schema_path, keyword_name)
                    .is_some_and(|error| errors.push(error));
            }
            let mut found = keyword.iter_errors(instance.clone()).peekable();
            if found.peek().is_none() {
                return false;
            }
            let value = instance.to_value();
            for error in found {
                let _ = errors.push(error.with_generated_context(
                    value.clone(),
                    instance_path.clone(),
                    super::schema_location(schema_path),
                    keyword_name,
                ));
            }
            false
        }
    }
    pub mod types {
        /// Integer check per drafts 6+: floats with zero fractional part count.
        #[must_use]
        pub fn is_integer(n: &serde_json::Number) -> bool {
            jsonschema_value::types::number_is_integer(n)
        }
        /// Integer check per draft 4: numbers written with a decimal point are not integers.
        #[must_use]
        pub fn is_integer_draft4(n: &serde_json::Number) -> bool {
            crate::keywords::legacy::type_draft_4::is_integer(n)
        }
    }
    pub mod format {
        #[cfg(feature = "idna")]
        pub use crate::keywords::format::is_valid_idn_hostname;
        pub use crate::keywords::format::{
            is_valid_date, is_valid_datetime, is_valid_duration, is_valid_fancy_regex_engine_regex,
            is_valid_hostname, is_valid_hostname_rfc1034, is_valid_ipv4, is_valid_ipv6,
            is_valid_iri, is_valid_iri_reference, is_valid_json_pointer, is_valid_regex,
            is_valid_regex_engine_regex, is_valid_relative_json_pointer, is_valid_time,
            is_valid_uri, is_valid_uri_reference, is_valid_uri_template, is_valid_uuid,
        };

        // Without `idna` the format is unknown, and an unknown format accepts every instance.
        #[cfg(not(feature = "idna"))]
        #[must_use]
        pub fn is_valid_idn_hostname(_: &str) -> bool {
            true
        }

        /// Per-call memoization cache for URI/IRI `format` checks in generated validators.
        pub type Cache = std::collections::HashMap<Box<str>, bool, ahash::RandomState>;

        /// Validate an email format using configured [`crate::EmailOptions`].
        #[must_use]
        pub fn is_valid_email_with_options(
            value: &str,
            options: Option<&crate::EmailOptions>,
        ) -> bool {
            crate::keywords::format::is_valid_email(value, options.map(|opts| &opts.inner))
        }

        /// Validate an IDN email format using configured [`crate::EmailOptions`].
        #[cfg(feature = "idna")]
        #[must_use]
        pub fn is_valid_idn_email_with_options(
            value: &str,
            options: Option<&crate::EmailOptions>,
        ) -> bool {
            crate::keywords::format::is_valid_idn_email(value, options.map(|opts| &opts.inner))
        }

        // Without `idna` the format is unknown, and an unknown format accepts every instance.
        #[cfg(not(feature = "idna"))]
        #[must_use]
        pub fn is_valid_idn_email_with_options(_: &str, _: Option<&crate::EmailOptions>) -> bool {
            true
        }
    }
    pub mod content {
        pub use crate::{
            content_encoding::{
                from_base16, from_base32, from_base32hex, from_base64, from_base64url, is_base16,
                is_base32, is_base32hex, is_base64, is_base64url,
            },
            content_media_type::is_json,
        };
    }
    pub mod numeric {
        /// Compare `value` >= `limit` using runtime numeric semantics.
        pub fn ge<T>(value: &serde_json::Number, limit: T) -> bool
        where
            T: Copy + num_traits::ToPrimitive,
            u64: num_cmp::NumCmp<T>,
            i64: num_cmp::NumCmp<T>,
            f64: num_cmp::NumCmp<T>,
        {
            crate::numeric::ge(value, limit)
        }

        /// Compare `value` <= `limit` using runtime numeric semantics.
        pub fn le<T>(value: &serde_json::Number, limit: T) -> bool
        where
            T: Copy + num_traits::ToPrimitive,
            u64: num_cmp::NumCmp<T>,
            i64: num_cmp::NumCmp<T>,
            f64: num_cmp::NumCmp<T>,
        {
            crate::numeric::le(value, limit)
        }

        pub fn eq<T>(value: &serde_json::Number, limit: T) -> bool
        where
            T: Copy + num_traits::ToPrimitive,
            u64: num_cmp::NumCmp<T>,
            i64: num_cmp::NumCmp<T>,
            f64: num_cmp::NumCmp<T>,
        {
            crate::numeric::eq(value, limit)
        }

        /// Compare `value` > `limit` using runtime numeric semantics.
        pub fn gt<T>(value: &serde_json::Number, limit: T) -> bool
        where
            T: Copy + num_traits::ToPrimitive,
            u64: num_cmp::NumCmp<T>,
            i64: num_cmp::NumCmp<T>,
            f64: num_cmp::NumCmp<T>,
        {
            crate::numeric::gt(value, limit)
        }

        /// Compare `value` < `limit` using runtime numeric semantics.
        pub fn lt<T>(value: &serde_json::Number, limit: T) -> bool
        where
            T: Copy + num_traits::ToPrimitive,
            u64: num_cmp::NumCmp<T>,
            i64: num_cmp::NumCmp<T>,
            f64: num_cmp::NumCmp<T>,
        {
            crate::numeric::lt(value, limit)
        }

        /// Check `multipleOf` with integer divisors using runtime numeric semantics.
        #[must_use]
        pub fn is_multiple_of_integer(value: &serde_json::Number, multiple: f64) -> bool {
            crate::numeric::is_multiple_of_integer(value, multiple)
        }

        /// Check `multipleOf` with fractional divisors using runtime numeric semantics.
        #[must_use]
        pub fn is_multiple_of_float(value: &serde_json::Number, multiple: f64) -> bool {
            crate::numeric::is_multiple_of_float(value, multiple)
        }

        /// Check numeric bounds with a compiled descriptor for arbitrary-precision schemas.
        #[cfg(feature = "arbitrary-precision")]
        #[must_use]
        pub fn check_compiled_bound(
            value: &serde_json::Number,
            op: u8,
            limit_literal: &'static str,
        ) -> bool {
            use jsonschema_value::numeric_check::{
                check_bound, compile_bound, BoundOp, CompiledBound,
            };

            #[inline]
            fn literal_key(literal: &'static str) -> (usize, usize) {
                (literal.as_ptr() as usize, literal.len())
            }

            type BoundCache =
                std::cell::RefCell<ahash::AHashMap<((usize, usize), u8), CompiledBound>>;
            std::thread_local! {
                static CACHE: BoundCache = std::cell::RefCell::new(ahash::AHashMap::default());
            }

            let op_tag = op;
            let Some(op) = BoundOp::from_u8(op_tag) else {
                unreachable!("codegen emits only bound op tags 0..=3")
            };
            let key = (literal_key(limit_literal), op_tag);

            CACHE.with(|cache| {
                let mut cache = cache.borrow_mut();
                let compiled = cache.entry(key).or_insert_with(|| {
                    let limit = serde_json::from_str::<serde_json::Number>(limit_literal)
                        .expect("Codegen emitted an invalid numeric literal");
                    compile_bound(op, &limit)
                });
                check_bound(compiled, value)
            })
        }

        /// Check `multipleOf` with a compiled descriptor for arbitrary-precision schemas.
        #[cfg(feature = "arbitrary-precision")]
        #[must_use]
        pub fn check_compiled_multiple_of(
            value: &serde_json::Number,
            limit_literal: &'static str,
        ) -> bool {
            use jsonschema_value::numeric_check::{
                check_multiple_of, compile_multiple_of, CompiledMultipleOf,
            };

            #[inline]
            fn literal_key(literal: &'static str) -> (usize, usize) {
                (literal.as_ptr() as usize, literal.len())
            }

            std::thread_local! {
                static CACHE: std::cell::RefCell<ahash::AHashMap<(usize, usize), CompiledMultipleOf>> =
                    std::cell::RefCell::new(ahash::AHashMap::default());
            }
            let key = literal_key(limit_literal);

            CACHE.with(|cache| {
                let mut cache = cache.borrow_mut();
                let compiled = cache.entry(key).or_insert_with(|| {
                    let limit = serde_json::from_str::<serde_json::Number>(limit_literal)
                        .expect("Codegen emitted an invalid numeric literal");
                    compile_multiple_of(&limit)
                });
                check_multiple_of(compiled, value)
            })
        }
    }

    pub use crate::evaluation::generated as evaluation;

    /// The schema path of an error a generated validator reports, shared by every error at that
    /// site on this thread instead of allocated per error.
    fn schema_location(path: &'static str) -> crate::paths::Location {
        /// Every site's location, and the last one asked for, as errors at one site come in runs.
        #[derive(Default)]
        struct Locations {
            last: Option<((usize, usize), crate::paths::Location)>,
            all: ahash::AHashMap<(usize, usize), crate::paths::Location>,
        }
        std::thread_local! {
            static LOCATIONS: std::cell::RefCell<Locations> = std::cell::RefCell::default();
        }
        if path.is_empty() {
            return crate::paths::Location::new();
        }
        let key = (path.as_ptr().addr(), path.len());
        // During thread teardown the cache is gone, and the path is built as it would be without it.
        LOCATIONS
            .try_with(|locations| {
                let mut locations = locations.borrow_mut();
                if let Some((last, location)) = &locations.last {
                    if *last == key {
                        return location.clone();
                    }
                }
                let location = locations
                    .all
                    .entry(key)
                    .or_insert_with(|| crate::paths::Location::from_escaped(path))
                    .clone();
                locations.last = Some((key, location.clone()));
                location
            })
            .unwrap_or_else(|_| crate::paths::Location::from_escaped(path))
    }

    pub mod error {
        use serde_json::Value;

        use crate::{
            paths::Location,
            types::{JsonType, JsonTypeSet},
            validator::LazyEvaluationPath,
            ErrorIterator, LazyInstance, ValidationError,
        };

        /// Wrap errors collected by generated `collect_errors` code into an [`ErrorIterator`].
        #[inline]
        #[must_use]
        pub fn iterator_from(errors: Vec<ValidationError<'_>>) -> ErrorIterator<'_> {
            ErrorIterator::from_iterator(errors.into_iter())
        }

        /// Errors reported by generated code: all of them for `iter_errors`, or only the first
        /// one for `validate`.
        pub enum Errors<'i> {
            First(Option<ValidationError<'i>>),
            All(Vec<ValidationError<'i>>),
        }

        impl<'i> Errors<'i> {
            #[inline]
            #[must_use]
            pub fn new(first_only: bool) -> Self {
                if first_only {
                    Self::First(None)
                } else {
                    Self::All(Vec::new())
                }
            }

            /// Records `error`; `true` when the caller must stop because only the first error
            /// is wanted.
            #[inline]
            #[must_use]
            pub fn push(&mut self, error: ValidationError<'i>) -> bool {
                match self {
                    Self::First(first) => {
                        *first = Some(error);
                        true
                    }
                    Self::All(errors) => {
                        errors.push(error);
                        false
                    }
                }
            }

            #[inline]
            #[must_use]
            pub fn first_only(&self) -> bool {
                matches!(self, Self::First(_))
            }

            /// Where the errors recorded next start.
            #[inline]
            #[must_use]
            pub fn mark(&self) -> usize {
                match self {
                    Self::First(first) => usize::from(first.is_some()),
                    Self::All(errors) => errors.len(),
                }
            }

            fn since(&mut self, start: usize) -> &mut [ValidationError<'i>] {
                match self {
                    Self::First(first) => {
                        let first = first.as_mut().map(std::slice::from_mut).unwrap_or_default();
                        let start = start.min(first.len());
                        &mut first[start..]
                    }
                    Self::All(errors) => &mut errors[start..],
                }
            }

            /// The errors recorded since `start` come from the subschema at `pointer`, reached
            /// through the reference at `site`: their evaluation paths move from `pointer` to
            /// `site`. With `base`, errors not yet located are placed in that resource, which
            /// starts `offset` bytes into their schema paths.
            pub fn reached(
                &mut self,
                start: usize,
                pointer: &str,
                base: Option<&str>,
                offset: usize,
                site: &str,
            ) {
                let errors = self.since(start);
                if errors.is_empty() {
                    return;
                }
                let base = base.map(|base| {
                    referencing::uri::from_str(base).expect("the generator writes a URI base")
                });
                let mut buffer = referencing::uri::EncodedBuffer::new();
                let moves = !pointer.is_empty() || !site.is_empty();
                for error in errors {
                    error.edit_all(&mut |error| {
                        if let Some(base) = &base {
                            if error.is_unlocated() {
                                error.locate_in(Some(base), offset, &mut buffer);
                            }
                        }
                        if moves {
                            error.move_evaluation_path(pointer, site);
                        }
                    });
                }
            }

            /// The errors recorded since `start` come from the subschema in place at `site`, through
            /// the function that reports them relative to that subschema.
            pub fn inside(&mut self, start: usize, site: &str) {
                for error in self.since(start) {
                    error.edit_all(&mut |error| error.nest_under(site));
                }
            }

            /// Places the errors recorded since `start` and not yet located in the resource at
            /// `base` (the default base for `None`), which starts `offset` bytes into their schema
            /// paths.
            pub fn within(&mut self, start: usize, base: Option<&str>, offset: usize) {
                let base = base.map(|base| {
                    referencing::uri::from_str(base).expect("the generator writes a URI base")
                });
                let mut buffer = referencing::uri::EncodedBuffer::new();
                for error in self.since(start) {
                    error.edit_all(&mut |error| {
                        if error.is_unlocated() {
                            error.locate_in(base.as_ref(), offset, &mut buffer);
                        }
                    });
                }
            }

            #[inline]
            #[must_use]
            pub fn into_first(self) -> Option<ValidationError<'i>> {
                match self {
                    Self::First(first) => first,
                    Self::All(errors) => errors.into_iter().next(),
                }
            }

            #[inline]
            #[must_use]
            pub fn into_vec(self) -> Vec<ValidationError<'i>> {
                match self {
                    Self::First(first) => first.into_iter().collect(),
                    Self::All(errors) => errors,
                }
            }
        }

        /// `contentEncoding` keyword violation.
        #[inline]
        pub fn content_encoding<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            encoding: &str,
        ) -> ValidationError<'i> {
            ValidationError::content_encoding(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                encoding,
            )
        }

        /// A failure to convert the decoded content, reported at the `contentEncoding` location
        /// as the runtime does.
        #[inline]
        pub fn content_conversion<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            error: ValidationError<'_>,
        ) -> ValidationError<'i> {
            ValidationError::new(
                instance.into(),
                error.into_parts().kind,
                instance_path,
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
            )
        }

        /// `contentMediaType` keyword violation.
        #[inline]
        pub fn content_media_type<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            media_type: &str,
        ) -> ValidationError<'i> {
            ValidationError::content_media_type(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                media_type,
            )
        }

        /// `minLength` keyword violation: string is shorter than `limit` characters.
        #[inline]
        pub fn min_length<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            limit: u64,
        ) -> ValidationError<'i> {
            ValidationError::min_length(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                limit,
            )
        }

        /// `maxLength` keyword violation: string exceeds `limit` characters.
        #[inline]
        pub fn max_length<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            limit: u64,
        ) -> ValidationError<'i> {
            ValidationError::max_length(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                limit,
            )
        }

        /// `pattern` keyword violation.
        #[inline]
        pub fn pattern<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            pattern: &str,
        ) -> ValidationError<'i> {
            ValidationError::pattern(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                pattern.to_string(),
            )
        }

        /// `format` keyword violation.
        #[inline]
        pub fn format<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            format: &str,
        ) -> ValidationError<'i> {
            ValidationError::format(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                format,
            )
        }

        /// `type` keyword violation for one expected type; [`multiple_types`] takes several.
        #[inline]
        pub fn single_type<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            ty: JsonType,
        ) -> ValidationError<'i> {
            ValidationError::single_type_error(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                ty,
            )
        }

        /// `type` keyword violation for a union of expected types.
        #[inline]
        pub fn multiple_types<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            types: JsonTypeSet,
        ) -> ValidationError<'i> {
            ValidationError::multiple_type_error(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                types,
            )
        }

        /// `minimum` keyword violation: value is less than `limit`.
        #[inline]
        pub fn minimum<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            limit: Value,
        ) -> ValidationError<'i> {
            ValidationError::minimum(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                limit,
            )
        }

        /// `maximum` keyword violation: value exceeds `limit`.
        #[inline]
        pub fn maximum<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            limit: Value,
        ) -> ValidationError<'i> {
            ValidationError::maximum(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                limit,
            )
        }

        /// `exclusiveMinimum` keyword violation: value is not strictly greater than `limit`.
        #[inline]
        pub fn exclusive_minimum<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            limit: Value,
        ) -> ValidationError<'i> {
            ValidationError::exclusive_minimum(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                limit,
            )
        }

        /// `exclusiveMaximum` keyword violation: value is not strictly less than `limit`.
        #[inline]
        pub fn exclusive_maximum<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            limit: Value,
        ) -> ValidationError<'i> {
            ValidationError::exclusive_maximum(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                limit,
            )
        }

        /// `multipleOf` keyword violation (non-arbitrary-precision).
        #[cfg(not(feature = "arbitrary-precision"))]
        #[inline]
        pub fn multiple_of<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            multiple_of: f64,
        ) -> ValidationError<'i> {
            ValidationError::multiple_of(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                multiple_of,
            )
        }

        /// `multipleOf` keyword violation (arbitrary-precision).
        #[cfg(feature = "arbitrary-precision")]
        #[inline]
        pub fn multiple_of<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            multiple_of: Value,
        ) -> ValidationError<'i> {
            ValidationError::multiple_of(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                multiple_of,
            )
        }

        /// `minItems` keyword violation: array has fewer items than `limit`.
        #[inline]
        pub fn min_items<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            limit: u64,
        ) -> ValidationError<'i> {
            ValidationError::min_items(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                limit,
            )
        }

        /// `maxItems` keyword violation: array exceeds `limit` items.
        #[inline]
        pub fn max_items<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            limit: u64,
        ) -> ValidationError<'i> {
            ValidationError::max_items(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                limit,
            )
        }

        /// `additionalItems` keyword violation (drafts 4 to 2019-09, array form of `items`).
        #[inline]
        pub fn additional_items<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            limit: usize,
        ) -> ValidationError<'i> {
            ValidationError::additional_items(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                limit,
            )
        }

        /// `uniqueItems` keyword violation: array contains duplicate elements.
        #[inline]
        pub fn unique_items<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
        ) -> ValidationError<'i> {
            ValidationError::unique_items(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
            )
        }

        /// `contains` keyword violation: no array item matched the `contains` schema.
        #[inline]
        pub fn contains<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
        ) -> ValidationError<'i> {
            ValidationError::contains(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
            )
        }

        /// `minContains` keyword violation: fewer than `limit` items match `contains`.
        #[inline]
        pub fn min_contains<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            limit: u64,
        ) -> ValidationError<'i> {
            ValidationError::min_contains(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                limit,
            )
        }

        /// `maxContains` keyword violation: more than `limit` items match `contains`.
        #[inline]
        pub fn max_contains<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            limit: u64,
        ) -> ValidationError<'i> {
            ValidationError::max_contains(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                limit,
            )
        }

        /// `minProperties` keyword violation: object has fewer properties than `limit`.
        #[inline]
        pub fn min_properties<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            limit: u64,
        ) -> ValidationError<'i> {
            ValidationError::min_properties(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                limit,
            )
        }

        /// `maxProperties` keyword violation: object exceeds `limit` properties.
        #[inline]
        pub fn max_properties<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            limit: u64,
        ) -> ValidationError<'i> {
            ValidationError::max_properties(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                limit,
            )
        }

        /// `propertyNames` keyword violation: `error` is why the subschema rejects one property
        /// name.
        #[inline]
        pub fn property_names<'i>(
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            error: ValidationError<'_>,
        ) -> ValidationError<'i> {
            ValidationError::property_names(instance_path, instance.into(), error.to_owned())
        }

        /// `required` keyword violation: object is missing `property`.
        #[inline]
        pub fn required<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            property: &str,
        ) -> ValidationError<'i> {
            ValidationError::required(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                Value::String(property.to_owned()),
            )
        }

        /// `additionalProperties` keyword violation (the `false` schema form).
        #[inline]
        pub fn additional_properties<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            unexpected: Vec<String>,
        ) -> ValidationError<'i> {
            ValidationError::additional_properties(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                unexpected,
            )
        }

        /// `const` keyword violation: instance does not equal the expected constant.
        #[inline]
        pub fn constant<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            expected: &std::sync::Arc<Value>,
        ) -> ValidationError<'i> {
            ValidationError::new(
                instance.into(),
                crate::error::ValidationErrorKind::Constant {
                    expected_value: std::sync::Arc::clone(expected),
                },
                instance_path,
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
            )
        }

        /// `enum` keyword violation: instance does not match any of the allowed values.
        #[inline]
        pub fn enumeration<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            options: &std::sync::Arc<Value>,
        ) -> ValidationError<'i> {
            ValidationError::enumeration(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                options,
            )
        }

        /// `false` schema violation: nothing is valid against a `false` schema.
        #[inline]
        pub fn false_schema<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
        ) -> ValidationError<'i> {
            ValidationError::false_schema(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
            )
        }

        /// `not` keyword violation: instance is valid against the negated schema.
        #[inline]
        pub fn not<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            schema: Value,
        ) -> ValidationError<'i> {
            ValidationError::not(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                schema,
            )
        }

        /// `anyOf` keyword violation: instance is not valid under any of the listed schemas.
        #[inline]
        pub fn any_of<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            context: Vec<Vec<ValidationError<'i>>>,
        ) -> ValidationError<'i> {
            ValidationError::any_of(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                context,
            )
        }

        /// `oneOf` keyword violation: instance is not valid under any of the listed schemas.
        #[inline]
        pub fn one_of_not_valid<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            context: Vec<Vec<ValidationError<'i>>>,
        ) -> ValidationError<'i> {
            ValidationError::one_of_not_valid(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                context,
            )
        }

        /// `oneOf` keyword violation: instance is valid under more than one of the listed schemas.
        #[inline]
        pub fn one_of_multiple_valid<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            context: Vec<Vec<ValidationError<'i>>>,
        ) -> ValidationError<'i> {
            ValidationError::one_of_multiple_valid(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                context,
            )
        }

        /// `unevaluatedProperties` keyword violation.
        #[inline]
        pub fn unevaluated_properties<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            unexpected: Vec<String>,
        ) -> ValidationError<'i> {
            ValidationError::unevaluated_properties(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                unexpected,
            )
        }

        /// `unevaluatedItems` keyword violation.
        #[inline]
        pub fn unevaluated_items<'i>(
            schema_path: &'static str,
            instance_path: Location,
            instance: impl Into<LazyInstance<'i>>,
            unexpected: Vec<String>,
        ) -> ValidationError<'i> {
            ValidationError::unevaluated_items(
                super::schema_location(schema_path),
                LazyEvaluationPath::SameAsSchemaPath,
                instance_path,
                instance.into(),
                unexpected,
            )
        }
    }
}

#[cfg(test)]
pub(crate) mod tests_util {
    use super::Validator;
    use crate::ValidationError;
    use serde_json::{json, Value};
    use std::fmt::Write;

    /// `{path} @{instance location} ok|FAIL` per unit of the hierarchical output, children
    /// indented below their parent.
    pub(crate) fn evaluation_outline(schema: &Value, instance: &Value) -> Vec<String> {
        let evaluation = crate::validator_for(schema)
            .expect("valid schema")
            .evaluate(instance);
        let mut lines = Vec::new();
        outline(
            &serde_json::to_value(evaluation.hierarchical()).expect("hierarchical output"),
            0,
            &mut lines,
        );
        lines
    }

    fn outline(node: &Value, depth: usize, lines: &mut Vec<String>) {
        let mut line = format!(
            "{}{} @{} {}",
            "  ".repeat(depth),
            node["evaluationPath"].as_str().expect("path"),
            node["instanceLocation"].as_str().expect("location"),
            if node["valid"] == json!(true) {
                "ok"
            } else {
                "FAIL"
            },
        );
        if let Some(annotations) = node.get("annotations") {
            write!(line, " ann={annotations}").expect("writes to a string");
        }
        if let Some(annotations) = node.get("droppedAnnotations") {
            write!(line, " dropped={annotations}").expect("writes to a string");
        }
        if let Some(errors) = node.get("errors").and_then(Value::as_object) {
            let keywords: Vec<_> = errors.keys().map(String::as_str).collect();
            write!(line, " err={}", keywords.join(",")).expect("writes to a string");
        }
        lines.push(line);
        for child in node["details"].as_array().into_iter().flatten() {
            outline(child, depth + 1, lines);
        }
    }

    #[track_caller]
    pub(crate) fn is_not_valid_with(validator: &Validator, instance: &Value) {
        assert!(
            !validator.is_valid(instance),
            "{instance} should not be valid (via is_valid)",
        );
        assert!(
            validator.validate(instance).is_err(),
            "{instance} should not be valid (via validate)",
        );
        assert!(
            validator.iter_errors(instance).next().is_some(),
            "{instance} should not be valid (via validate)",
        );
        let evaluation = validator.evaluate(instance);
        assert!(
            !evaluation.flag().valid,
            "{instance} should not be valid (via evaluate)",
        );
    }

    /// `is_valid` and every error as `(schema path, message)` per instance, under a 2020-12
    /// meta-schema that enables only `vocabulary` beside core.
    pub(crate) fn outcomes_with_only_vocabulary(
        vocabulary: &str,
        schema: &Value,
        instances: &[Value],
    ) -> Vec<(bool, Vec<(String, String)>)> {
        let meta_id = format!("json-schema:///meta/only-{vocabulary}");
        let meta = serde_json::json!({
            "$id": meta_id,
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$vocabulary": {
                "https://json-schema.org/draft/2020-12/vocab/core": true,
                format!("https://json-schema.org/draft/2020-12/vocab/{vocabulary}"): true
            }
        });
        let registry = crate::Registry::new()
            .add(meta_id.as_str(), &meta)
            .expect("meta-schema registers")
            .prepare()
            .expect("registry prepares");
        let mut schema = schema.clone();
        schema["$schema"] = Value::String(meta_id);
        let validator = crate::options()
            .with_registry(&registry)
            .build(&schema)
            .expect("schema compiles");
        instances
            .iter()
            .map(|instance| {
                let errors = validator
                    .iter_errors(instance)
                    .map(|error| (error.schema_path().as_str().to_string(), error.to_string()))
                    .collect();
                (validator.is_valid(instance), errors)
            })
            .collect()
    }

    #[track_caller]
    pub(crate) fn is_not_valid(schema: &Value, instance: &Value) {
        let validator = crate::options()
            .should_validate_formats(true)
            .build(schema)
            .expect("Invalid schema");
        is_not_valid_with(&validator, instance);
    }

    pub(crate) fn expect_errors(schema: &Value, instance: &Value, errors: &[&str]) {
        let mut actual = crate::validator_for(schema)
            .expect("Should be a valid schema")
            .iter_errors(instance)
            .map(|e| e.to_string())
            .collect::<Vec<String>>();
        actual.sort();
        assert_eq!(actual, errors);
    }

    #[track_caller]
    pub(crate) fn is_valid_with(validator: &Validator, instance: &Value) {
        if let Some(first) = validator.iter_errors(instance).next() {
            panic!(
                "{} should be valid (via validate). Error: {} at {}",
                instance,
                first,
                first.instance_path()
            );
        }
        assert!(
            validator.is_valid(instance),
            "{instance} should be valid (via is_valid)",
        );
        assert!(
            validator.validate(instance).is_ok(),
            "{instance} should be valid (via is_valid)",
        );
        let evaluation = validator.evaluate(instance);
        assert!(
            evaluation.flag().valid,
            "{instance} should be valid (via evaluate)",
        );
    }

    #[track_caller]
    pub(crate) fn is_valid(schema: &Value, instance: &Value) {
        let validator = crate::options()
            .should_validate_formats(true)
            .build(schema)
            .expect("Invalid schema");
        is_valid_with(&validator, instance);
    }

    #[track_caller]
    pub(crate) fn validate(schema: &Value, instance: &Value) -> ValidationError<'static> {
        let validator = crate::options()
            .should_validate_formats(true)
            .build(schema)
            .expect("Invalid schema");
        let err = validator
            .validate(instance)
            .expect_err("Should be an error")
            .to_owned();
        err
    }

    #[track_caller]
    pub(crate) fn assert_schema_location(schema: &Value, instance: &Value, expected: &str) {
        let error = validate(schema, instance);
        assert_eq!(error.schema_path().as_str(), expected);
    }

    #[track_caller]
    pub(crate) fn assert_absolute_keyword_locations(
        schema: &Value,
        instance: &Value,
        expected: &[(&str, &str)],
    ) {
        let validator = crate::validator_for(schema).expect("Invalid schema");
        let actual: Vec<(String, String)> = validator
            .iter_errors(instance)
            .map(|error| {
                (
                    error.kind().keyword().to_string(),
                    error
                        .absolute_keyword_location()
                        .expect("Absolute keyword location")
                        .to_string(),
                )
            })
            .collect();
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(keyword, location)| ((*keyword).to_string(), (*location).to_string()))
            .collect();
        assert_eq!(actual, expected);
    }

    /// Message, instance path, schema path and absolute keyword location of an error.
    type ErrorLocations = (String, String, String, Option<String>);

    fn error_locations(error: &ValidationError<'_>) -> ErrorLocations {
        (
            error.to_string(),
            error.instance_path().as_str().to_owned(),
            error.schema_path().as_str().to_owned(),
            error.absolute_keyword_location().map(ToString::to_string),
        )
    }

    /// Asserts the errors from `iter_errors` and the one from `validate` against `expected`
    /// (message, instance path, schema path), with `schema` both at the root of a resource and
    /// behind a `$ref`. The absolute keyword location must name the schema path.
    #[track_caller]
    pub(crate) fn assert_error_locations(
        schema: &Value,
        instance: &Value,
        expected: &[(&str, &str, &str)],
    ) {
        const BASE: &str = "https://example.com/s.json";
        let mut root = schema.clone();
        root["$id"] = Value::from(BASE);
        let behind_ref =
            serde_json::json!({"$id": BASE, "$defs": {"t": schema}, "$ref": "#/$defs/t"});
        for (schema, prefix) in [(root, ""), (behind_ref, "/$defs/t")] {
            let validator = crate::validator_for(&schema).expect("Invalid schema");
            let expected: Vec<ErrorLocations> = expected
                .iter()
                .map(|(message, instance_path, schema_path)| {
                    (
                        (*message).to_owned(),
                        (*instance_path).to_owned(),
                        format!("{prefix}{schema_path}"),
                        Some(format!("{BASE}#{prefix}{schema_path}")),
                    )
                })
                .collect();
            let actual: Vec<ErrorLocations> = validator
                .iter_errors(instance)
                .map(|error| error_locations(&error))
                .collect();
            assert_eq!(actual, expected, "iter_errors for {schema}");
            let first = validator
                .validate(instance)
                .expect_err("Should be an error");
            assert_eq!(
                Some(error_locations(&first)),
                expected.first().cloned(),
                "validate for {schema}"
            );
        }
    }

    #[track_caller]
    pub(crate) fn assert_evaluation_path(schema: &Value, instance: &Value, expected: &str) {
        let error = validate(schema, instance);
        assert_eq!(error.evaluation_path().as_str(), expected);
    }

    #[track_caller]
    pub(crate) fn assert_locations(schema: &Value, instance: &Value, expected: &[&str]) {
        let validator = crate::validator_for(schema).unwrap();
        let mut errors: Vec<_> = validator
            .iter_errors(instance)
            .map(|error| error.schema_path().as_str().to_string())
            .collect();
        errors.sort();
        for (error, location) in errors.into_iter().zip(expected) {
            assert_eq!(error, *location);
        }
    }

    #[track_caller]
    pub(crate) fn assert_keyword_location(
        validator: &Validator,
        instance: &Value,
        instance_pointer: &str,
        keyword_pointer: &str,
    ) {
        fn pointer_from_schema_location(location: &str) -> &str {
            location
                .split_once('#')
                .map_or(location, |(_, fragment)| fragment)
        }

        let evaluation = validator.evaluate(instance);
        let serialized =
            serde_json::to_value(evaluation.list()).expect("List output should be serializable");
        let details = serialized
            .get("details")
            .and_then(|value| value.as_array())
            .expect("List output must contain details");
        let mut available = Vec::new();
        for entry in details {
            let Some(instance_location) = entry
                .get("instanceLocation")
                .and_then(|value| value.as_str())
            else {
                continue;
            };
            if instance_location != instance_pointer {
                continue;
            }
            let schema_location = entry
                .get("schemaLocation")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let pointer = pointer_from_schema_location(schema_location);
            if pointer == keyword_pointer {
                return;
            }
            available.push(pointer.to_string());
        }

        panic!(
            "No annotation for instance pointer `{instance_pointer}` with keyword location `{keyword_pointer}`. Available keyword locations for pointer: {available:?}"
        );
    }

    #[track_caller]
    pub(crate) fn is_valid_with_draft4(schema: &Value, instance: &Value) {
        let validator = crate::options()
            .with_draft(crate::Draft::Draft4)
            .should_validate_formats(true)
            .build(schema)
            .expect("Invalid schema");
        is_valid_with(&validator, instance);
    }

    #[track_caller]
    pub(crate) fn is_not_valid_with_draft4(schema: &Value, instance: &Value) {
        let validator = crate::options()
            .with_draft(crate::Draft::Draft4)
            .should_validate_formats(true)
            .build(schema)
            .expect("Invalid schema");
        is_not_valid_with(&validator, instance);
    }

    /// Compiles `subschema` behind a `$ref`: the meta-schema never sees a subschema under an
    /// unknown keyword, so malformed keyword values reach the keyword compilers. `location` is
    /// relative to `subschema`.
    #[track_caller]
    pub(crate) fn assert_compile_error_with(
        options: &crate::ValidationOptions,
        subschema: &Value,
        message: &str,
        location: &str,
    ) {
        let error = options
            .build(&serde_json::json!({"$ref": "#/x", "x": subschema}))
            .expect_err("Should fail to compile");
        let location = format!("/x{location}");
        assert_eq!(
            (
                error.to_string(),
                error.instance_path().as_str(),
                error.schema_path().as_str()
            ),
            (message.to_string(), location.as_str(), location.as_str())
        );
    }

    #[track_caller]
    pub(crate) fn assert_compile_error(subschema: &Value, message: &str, location: &str) {
        assert_compile_error_with(&crate::options(), subschema, message, location);
    }
}

#[cfg(test)]
mod tests {
    use crate::{validator_for, CanonicalizationError, Registry, SerdeJson, ValidationError};

    use super::Draft;
    use serde_json::{json, Value};
    use test_case::test_case;

    // A build error judges the schema itself, so its instance path is the location that failed
    // to compile.
    #[test_case(&json!({"type": "object", "properties": {"sku": {"type": "string", "pattern": "([unclosed"}}}), "/properties/sku/pattern")]
    #[test_case(&json!({"patternProperties": {"([unclosed": {"type": "string"}}}), "/patternProperties/([unclosed")]
    fn build_error_instance_path(schema: &Value, expected: &str) {
        let error = validator_for(schema).expect_err("Should fail to compile");
        assert_eq!(error.instance_path().as_str(), expected);
    }

    // A subschema's `$schema` switches the draft for that subschema, whatever the root draft.
    #[test_case(Draft::Draft4, &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "string"}]}), false ; "2020-12 under draft 4")]
    #[test_case(Draft::Draft4, &json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "items": [{"type": "string"}]}), false ; "2019-09 under draft 4")]
    #[test_case(Draft::Draft7, &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "string"}]}), false ; "2020-12 under draft 7")]
    #[test_case(Draft::Draft4, &json!({"$schema": "http://json-schema.org/draft-07/schema#", "items": [{"type": "string"}]}), false ; "draft 7 under draft 4")]
    #[test_case(Draft::Draft201909, &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "string"}]}), false ; "2020-12 under 2019-09")]
    #[test_case(Draft::Draft202012, &json!({"$schema": "http://json-schema.org/draft-04/schema#", "prefixItems": [{"type": "string"}]}), true ; "draft 4 under 2020-12")]
    fn nested_schema_switches_draft(root: Draft, pair: &Value, expected: bool) {
        let validator = crate::options()
            .with_draft(root)
            .build(&json!({"properties": {"pair": pair}}))
            .expect("Should build validator");
        assert_eq!(validator.is_valid(&json!({"pair": [1]})), expected);
    }

    // A subschema's `$schema` picks its draft whether evaluation reaches it inline or through a
    // `$ref`, in either compilation order.
    #[test_case(DRAFT202012, &json!({"$schema": DRAFT4, "type": "integer"}), &json!(1.0), false ; "draft 4 type under 2020-12")]
    #[test_case(DRAFT201909, &json!({"$schema": DRAFT4, "type": "integer"}), &json!(1.0), false ; "draft 4 type under 2019-09")]
    #[test_case(DRAFT7, &json!({"$schema": DRAFT4, "type": "integer"}), &json!(1.0), false ; "draft 4 type under draft 7")]
    #[test_case(DRAFT202012, &json!({"$schema": DRAFT4, "maximum": 5, "exclusiveMaximum": true}), &json!(5), false ; "draft 4 boolean exclusiveMaximum under 2020-12")]
    #[test_case(DRAFT201909, &json!({"$schema": DRAFT4, "maximum": 5, "exclusiveMaximum": true}), &json!(5), false ; "draft 4 boolean exclusiveMaximum under 2019-09")]
    #[test_case(DRAFT7, &json!({"$schema": DRAFT4, "maximum": 5, "exclusiveMaximum": true}), &json!(5), false ; "draft 4 boolean exclusiveMaximum under draft 7")]
    #[test_case(DRAFT7, &json!({"$schema": DRAFT201909, "dependentRequired": {"foo": ["bar"]}}), &json!({"foo": 1}), false ; "2019-09 dependentRequired under draft 7")]
    #[test_case(DRAFT202012, &json!({"$schema": DRAFT201909, "prefixItems": [{"type": "string"}]}), &json!([1]), true ; "2019-09 prefixItems under 2020-12")]
    #[test_case(DRAFT202012, &json!({"$schema": DRAFT7, "prefixItems": [{"type": "string"}]}), &json!([1]), true ; "draft 7 prefixItems under 2020-12")]
    #[test_case(DRAFT201909, &json!({"$schema": DRAFT202012, "prefixItems": [{"type": "string"}]}), &json!([1]), false ; "2020-12 prefixItems under 2019-09")]
    #[test_case(DRAFT7, &json!({"$schema": DRAFT202012, "prefixItems": [{"type": "string"}]}), &json!([1]), false ; "2020-12 prefixItems under draft 7")]
    #[test_case(DRAFT202012, &json!({"$schema": DRAFT201909, "items": [{"type": "string"}]}), &json!([1]), false ; "2019-09 array items under 2020-12")]
    #[test_case(DRAFT202012, &json!({"$schema": DRAFT7, "items": [{"type": "string"}]}), &json!([1]), false ; "draft 7 array items under 2020-12")]
    #[test_case(DRAFT202012, &json!({"$schema": DRAFT7, "allOf": [{"$ref": "https://example.com/any", "type": "integer"}]}), &json!("x"), true ; "draft 7 keyword beside ref under 2020-12")]
    #[test_case(DRAFT201909, &json!({"$schema": DRAFT7, "allOf": [{"$ref": "https://example.com/any", "type": "integer"}]}), &json!("x"), true ; "draft 7 keyword beside ref under 2019-09")]
    #[test_case(DRAFT7, &json!({"$schema": DRAFT201909, "allOf": [{"$ref": "https://example.com/any", "type": "integer"}]}), &json!("x"), false ; "2019-09 keyword beside ref under draft 7")]
    #[test_case(DRAFT7, &json!({"$schema": DRAFT202012, "allOf": [{"$ref": "https://example.com/any", "type": "integer"}]}), &json!("x"), false ; "2020-12 keyword beside ref under draft 7")]
    #[test_case(DRAFT202012, &json!({"$id": "https://example.com/x", "id": "https://example.com/x", "$schema": DRAFT4, "type": "integer"}), &json!(1.0), false ; "draft 4 resource under 2020-12")]
    #[test_case(DRAFT7, &json!({"$id": "https://example.com/x", "$schema": DRAFT201909, "dependentRequired": {"foo": ["bar"]}}), &json!({"foo": 1}), false ; "2019-09 resource under draft 7")]
    #[test_case(DRAFT7, &json!({"$id": "https://example.com/x", "$schema": DRAFT202012, "prefixItems": [{"type": "string"}]}), &json!([1]), false ; "2020-12 resource under draft 7")]
    fn nested_schema_applies_inline_and_through_ref(
        root: &str,
        body: &Value,
        instance: &Value,
        expected: bool,
    ) {
        let any = json!({});
        let registry = Registry::new()
            .add("https://example.com/any", &any)
            .expect("Valid resource")
            .prepare()
            .expect("Registry builds");
        let verdict = |schema: Value, property: &str| {
            crate::options()
                .with_registry(&registry)
                .build(&schema)
                .expect("Should build validator")
                .is_valid(&json!({property: instance}))
        };
        let inline_first =
            json!({"$schema": root, "properties": {"a": body, "b": {"$ref": "#/properties/a"}}});
        let reference_first =
            json!({"$schema": root, "properties": {"a": {"$ref": "#/properties/b"}, "b": body}});
        assert_eq!(
            [
                verdict(json!({"$schema": root, "properties": {"a": body}}), "a"),
                verdict(
                    json!({"$schema": root, "properties": {"a": {"$ref": "#/definitions/x"}}, "definitions": {"x": body}}),
                    "a"
                ),
                verdict(inline_first.clone(), "a"),
                verdict(inline_first, "b"),
                verdict(reference_first.clone(), "a"),
                verdict(reference_first, "b"),
            ],
            [expected; 6]
        );
    }

    // A subschema below a `$schema` is in that draft too, and a `$ref` target is in the draft of
    // where it is written, whatever draft the `$ref` sits in.
    #[test_case(&json!({"properties": {"a": {"$schema": DRAFT4, "allOf": [{"type": "integer"}]}}}), &json!({"a": 1.0}), false ; "child inline")]
    #[test_case(&json!({"properties": {"a": {"$ref": "#/$defs/x/allOf/0"}}, "$defs": {"x": {"$schema": DRAFT4, "allOf": [{"type": "integer"}]}}}), &json!({"a": 1.0}), false ; "child through ref")]
    #[test_case(&json!({"properties": {"a": {"$ref": "#/$defs/x/allOf/0"}}, "$defs": {"x": {"$id": "https://example.com/x", "id": "https://example.com/x", "$schema": DRAFT4, "allOf": [{"type": "integer"}]}}}), &json!({"a": 1.0}), false ; "child of a resource through ref")]
    #[test_case(&json!({"properties": {"a": {"$schema": DRAFT4, "properties": {"n": {"$ref": "#/$defs/t"}}}, "b": {"$ref": "#/$defs/t"}}, "$defs": {"t": {"type": "integer"}}}), &json!({"a": {"n": 1.0}}), true ; "target in root draft from draft 4 first")]
    #[test_case(&json!({"properties": {"a": {"$schema": DRAFT4, "properties": {"n": {"$ref": "#/$defs/t"}}}, "b": {"$ref": "#/$defs/t"}}, "$defs": {"t": {"type": "integer"}}}), &json!({"b": 1.0}), true ; "target in root draft from root draft second")]
    #[test_case(&json!({"properties": {"a": {"$ref": "#/$defs/t"}, "b": {"$schema": DRAFT4, "properties": {"n": {"$ref": "#/$defs/t"}}}}, "$defs": {"t": {"type": "integer"}}}), &json!({"b": {"n": 1.0}}), true ; "target in root draft from draft 4 second")]
    #[test_case(&json!({"properties": {"a": {"$ref": "#/$defs/t"}, "b": {"$schema": DRAFT4, "properties": {"n": {"$ref": "#/$defs/t"}}}}, "$defs": {"t": {"$schema": DRAFT4, "type": "integer"}}}), &json!({"a": 1.0}), false ; "target in draft 4 from root draft first")]
    #[test_case(&json!({"properties": {"a": {"$ref": "#/$defs/t"}, "b": {"$schema": DRAFT4, "properties": {"n": {"$ref": "#/$defs/t"}}}}, "$defs": {"t": {"$schema": DRAFT4, "type": "integer"}}}), &json!({"b": {"n": 1.0}}), false ; "target in draft 4 from draft 4 second")]
    // Draft 4 names a resource with `id`, so the pointer enters it and its `$ref`s resolve there.
    #[test_case(&json!({"properties": {"a": {"$ref": "#/$defs/x"}}, "$defs": {"x": {"$schema": DRAFT4, "id": "https://example.com/x", "properties": {"n": {"$ref": "#/definitions/y"}}, "definitions": {"y": {"type": "integer"}}}}}), &json!({"a": {"n": 1.0}}), false ; "draft 4 id through ref")]
    #[test_case(&json!({"properties": {"a": {"$ref": "#/$defs/x/definitions/y"}}, "$defs": {"x": {"$schema": DRAFT4, "definitions": {"y": {"id": "https://example.com/y", "properties": {"n": {"$ref": "#/definitions/z"}}, "definitions": {"z": {"type": "integer"}}}}}}}), &json!({"a": {"n": 1.0}}), false ; "draft 4 id below a draft 4 subschema through ref")]
    fn nested_schema_draft_through_ref(schema: &Value, instance: &Value, expected: bool) {
        let validator = validator_for(schema).expect("Should build validator");
        assert_eq!(validator.is_valid(instance), expected);
    }

    const NO_VALIDATION: &str = "json-schema:///meta/no-validation";

    // The vocabularies a subschema's `$schema` declares hold for its children, inline and
    // through a `$ref`: the meta-schema turns `minimum` off.
    #[test_case(&json!({"properties": {"a": {"$schema": NO_VALIDATION, "allOf": [{"minimum": 5}]}}}), true ; "custom inline")]
    #[test_case(&json!({"properties": {"a": {"$ref": "#/$defs/x"}}, "$defs": {"x": {"$schema": NO_VALIDATION, "allOf": [{"minimum": 5}]}}}), true ; "custom through ref")]
    #[test_case(&json!({"properties": {"a": {"$ref": "#/$defs/x/allOf/0"}}, "$defs": {"x": {"$schema": NO_VALIDATION, "allOf": [{"minimum": 5}]}}}), true ; "custom child through ref")]
    #[test_case(&json!({"$schema": NO_VALIDATION, "properties": {"a": {"$schema": DRAFT201909, "allOf": [{"minimum": 5}]}}}), false ; "standard under custom inline")]
    #[test_case(&json!({"$schema": NO_VALIDATION, "properties": {"a": {"$ref": "#/$defs/x/allOf/0"}}, "$defs": {"x": {"$schema": DRAFT201909, "allOf": [{"minimum": 5}]}}}), false ; "standard under custom child through ref")]
    #[test_case(&json!({"properties": {"a": {"$ref": "#c"}}, "$defs": {"x": {"$schema": NO_VALIDATION, "allOf": [{"$anchor": "c", "minimum": 5}]}}}), true ; "custom child through anchor")]
    #[test_case(&json!({"properties": {"a": {"$dynamicRef": "#c"}}, "$defs": {"x": {"$schema": NO_VALIDATION, "allOf": [{"$dynamicAnchor": "c", "minimum": 5}]}}}), true ; "custom child through dynamic anchor")]
    #[test_case(&json!({"$schema": NO_VALIDATION, "properties": {"a": {"$ref": "#c"}}, "$defs": {"x": {"$schema": DRAFT202012, "allOf": [{"$anchor": "c", "minimum": 5}]}}}), false ; "standard under custom child through anchor")]
    #[test_case(&json!({"$schema": NO_VALIDATION, "properties": {"a": {"$id": "https://example.com/r", "allOf": [{"minimum": 5}]}}}), true ; "resource under custom inline")]
    #[test_case(&json!({"$schema": NO_VALIDATION, "properties": {"a": {"$ref": "https://example.com/r"}}, "$defs": {"r": {"$id": "https://example.com/r", "allOf": [{"minimum": 5}]}}}), true ; "resource under custom through ref")]
    #[test_case(&json!({"properties": {"a": {"$ref": "https://example.com/r"}}, "$defs": {"x": {"$schema": NO_VALIDATION, "allOf": [{"$id": "https://example.com/r", "minimum": 5}]}}}), true ; "resource below custom through ref")]
    fn nested_schema_vocabularies_through_ref(schema: &Value, expected: bool) {
        let registry = no_validation_registry(None);
        let validator = crate::options()
            .with_registry(&registry)
            .build(schema)
            .expect("Should build validator");
        assert_eq!(validator.is_valid(&json!({"a": 1})), expected);
    }

    fn no_validation_registry(deep: Option<Value>) -> Registry<'static> {
        let meta_schema = json!({
            "$id": NO_VALIDATION,
            "$schema": DRAFT202012,
            "$vocabulary": {
                "https://json-schema.org/draft/2020-12/vocab/core": true,
                "https://json-schema.org/draft/2020-12/vocab/applicator": true,
                "https://json-schema.org/draft/2020-12/vocab/validation": false
            }
        });
        let registry = Registry::new()
            .add(NO_VALIDATION, meta_schema)
            .expect("Valid resource");
        match deep {
            Some(document) => registry.add("https://example.com/deep", document),
            None => Ok(registry),
        }
        .expect("Valid resource")
        .prepare()
        .expect("Registry builds")
    }

    // An anchor nested deeper than the registry crawls recursively, below a subschema whose
    // `$schema` turns `minimum` off, in the document itself and in an owned resource.
    #[test_case(false ; "borrowed")]
    #[test_case(true ; "owned")]
    fn nested_schema_vocabularies_deep_below(owned: bool) {
        let mut nested = json!({"$anchor": "c", "minimum": 5});
        for _ in 0..200 {
            nested = json!({"allOf": [nested]});
        }
        let document =
            json!({"$defs": {"x": {"$schema": NO_VALIDATION, "allOf": [nested]}}, "$ref": "#c"});
        let (registry, schema) = if owned {
            (
                no_validation_registry(Some(document)),
                json!({"$ref": "https://example.com/deep#c"}),
            )
        } else {
            (no_validation_registry(None), document)
        };
        let validator = crate::options()
            .with_registry(&registry)
            .build(&schema)
            .expect("Should build validator");
        assert!(validator.is_valid(&json!(1)));
    }

    // The fragment of an absolute keyword location is a JSON Pointer from the root of the
    // resource that holds the keyword, not from the root of the document embedding it.
    #[test_case(
        &json!({"properties": {"tree": {"$id": "https://example.com/tree", "type": "object", "properties": {"value": {"type": "string"}}}}}),
        &json!({"tree": {"value": 1}}),
        &["https://example.com/tree#/properties/value/type"];
        "draft 2020-12"
    )]
    #[test_case(
        &json!({"properties": {"tree": {"$id": "https://example.com/tree", "type": "object"}}}),
        &json!({"tree": 1}),
        &["https://example.com/tree#/type"];
        "keyword at resource root"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "properties": {"tree": {"$id": "https://example.com/tree", "properties": {"value": {"type": "string"}}}}}),
        &json!({"tree": {"value": 1}}),
        &["https://example.com/tree#/properties/value/type"];
        "draft 2019-09"
    )]
    #[test_case(
        &json!({"$schema": "http://json-schema.org/draft-07/schema#", "properties": {"tree": {"$id": "https://example.com/tree", "properties": {"value": {"type": "string"}}}}}),
        &json!({"tree": {"value": 1}}),
        &["https://example.com/tree#/properties/value/type"];
        "draft 7"
    )]
    #[test_case(
        &json!({"$schema": "http://json-schema.org/draft-06/schema#", "properties": {"tree": {"$id": "https://example.com/tree", "properties": {"value": {"type": "string"}}}}}),
        &json!({"tree": {"value": 1}}),
        &["https://example.com/tree#/properties/value/type"];
        "draft 6"
    )]
    #[test_case(
        &json!({"$schema": "http://json-schema.org/draft-04/schema#", "properties": {"tree": {"id": "https://example.com/tree", "properties": {"value": {"type": "string"}}}}}),
        &json!({"tree": {"value": 1}}),
        &["https://example.com/tree#/properties/value/type"];
        "draft 4"
    )]
    #[test_case(
        &json!({"$id": "https://example.com/root.json", "properties": {"tree": {"$id": "tree.json", "properties": {"value": {"type": "string"}}}}}),
        &json!({"tree": {"value": 1}}),
        &["https://example.com/tree.json#/properties/value/type"];
        "relative id"
    )]
    #[test_case(
        &json!({
            "$id": "https://example.com/root.json",
            "properties": {
                "outer": {
                    "$id": "outer/",
                    "properties": {
                        "inner": {"$id": "inner.json", "properties": {"value": {"type": "string"}}},
                        "count": {"type": "integer"}
                    }
                },
                "name": {"type": "string"}
            }
        }),
        &json!({"outer": {"inner": {"value": 1}, "count": "x"}, "name": 1}),
        &[
            "https://example.com/outer/#/properties/count/type",
            "https://example.com/outer/inner.json#/properties/value/type",
            "https://example.com/root.json#/properties/name/type",
        ];
        "id nested in id"
    )]
    #[test_case(
        &json!({"$schema": "http://json-schema.org/draft-07/schema#", "$id": "https://example.com/root", "properties": {"tree": {"$id": "#tree", "properties": {"value": {"type": "string"}}}}}),
        &json!({"tree": {"value": 1}}),
        &["https://example.com/root#/properties/tree/properties/value/type"];
        "draft 7 fragment-only id names no resource"
    )]
    #[test_case(
        &json!({"$schema": "http://json-schema.org/draft-07/schema#", "$id": "https://example.com/root", "properties": {"tree": {"$id": "https://example.com/tree", "$ref": "#/definitions/leaf"}}, "definitions": {"leaf": {"type": "string"}}}),
        &json!({"tree": 1}),
        &["https://example.com/root#/definitions/leaf/type"];
        "draft 7 id beside ref names no resource"
    )]
    fn absolute_keyword_location_in_nested_resource(
        schema: &Value,
        instance: &Value,
        expected: &[&str],
    ) {
        let validator = validator_for(schema).expect("Should build validator");
        let mut locations: Vec<String> = validator
            .iter_errors(instance)
            .map(|error| {
                error
                    .absolute_keyword_location()
                    .expect("Absolute keyword location")
                    .to_string()
            })
            .collect();
        locations.sort();
        assert_eq!(locations, expected);
        let evaluation = validator.evaluate(instance);
        let mut locations: Vec<String> = evaluation
            .iter_errors()
            .map(|entry| {
                entry
                    .absolute_keyword_location
                    .expect("Absolute keyword location")
                    .to_string()
            })
            .collect();
        locations.sort();
        assert_eq!(locations, expected);
    }

    #[test_case(
        &json!({"properties": {"tree": {"$id": "https://example.com/tree", "properties": {"value": false}}}}),
        &json!({"tree": {"value": 1}}),
        &[("https://example.com/tree#/properties/value", Some("https://example.com/tree#/properties/value"))],
        &[Some("https://example.com/tree#/properties/value")];
        "false subschema in nested resource"
    )]
    #[test_case(
        &json!({"properties": {"tree": {"$id": "https://example.com/tree", "unevaluatedProperties": false}}}),
        &json!({"tree": {"a": 1, "b": 2}}),
        &[
            ("https://example.com/tree#/unevaluatedProperties", Some("https://example.com/tree#/unevaluatedProperties")),
            ("https://example.com/tree#/unevaluatedProperties", Some("https://example.com/tree#/unevaluatedProperties")),
            ("https://example.com/tree#/unevaluatedProperties", Some("https://example.com/tree#/unevaluatedProperties")),
        ],
        &[Some("https://example.com/tree#/unevaluatedProperties")];
        "unevaluatedProperties false in nested resource"
    )]
    #[test_case(
        &json!({"$id": "https://example.com/root", "$ref": "#/$defs/never", "$defs": {"never": false}}),
        &json!(1),
        &[("https://example.com/root#/$defs/never", Some("https://example.com/root#/$defs/never"))],
        &[Some("https://example.com/root#/$defs/never")];
        "ref to false in same resource"
    )]
    #[test_case(
        &json!({"$id": "https://example.com/root", "$ref": "other#/$defs/never", "$defs": {"other": {"$id": "other", "$defs": {"never": false}}}}),
        &json!(1),
        &[("https://example.com/other#/$defs/never", Some("https://example.com/other#/$defs/never"))],
        &[Some("https://example.com/other#/$defs/never")];
        "ref to false in other resource"
    )]
    #[test_case(
        &json!({"properties": {"value": false}}),
        &json!({"value": 1}),
        &[("/properties/value", None)],
        &[None];
        "false subschema without id"
    )]
    fn false_schema_error_locations(
        schema: &Value,
        instance: &Value,
        evaluate: &[(&str, Option<&str>)],
        iter_errors: &[Option<&str>],
    ) {
        let validator = validator_for(schema).expect("Should build validator");
        let evaluation = validator.evaluate(instance);
        let locations: Vec<(&str, Option<String>)> = evaluation
            .iter_errors()
            .map(|entry| {
                (
                    entry.schema_location,
                    entry.absolute_keyword_location.map(ToString::to_string),
                )
            })
            .collect();
        let expected: Vec<(&str, Option<String>)> = evaluate
            .iter()
            .map(|(schema_location, absolute)| (*schema_location, absolute.map(str::to_string)))
            .collect();
        assert_eq!(locations, expected);
        let locations: Vec<Option<String>> = validator
            .iter_errors(instance)
            .map(|error| error.absolute_keyword_location().map(ToString::to_string))
            .collect();
        let expected: Vec<Option<String>> = iter_errors
            .iter()
            .map(|absolute| absolute.map(str::to_string))
            .collect();
        assert_eq!(locations, expected);
    }

    #[test_case(
        &json!({"$id": "https://example.com/tree", "properties": {"a": {"type": "string"}, "b": {"type": "string"}}, "required": ["a", "b"]}),
        &json!({}),
        &["", "/properties", "https://example.com/tree#", "https://example.com/tree#/required", "https://example.com/tree#/properties"];
        "required"
    )]
    #[test_case(
        &json!({"$id": "https://example.com/tree", "minLength": 1, "maxLength": 2}),
        &json!("abc"),
        &["", "/properties", "https://example.com/tree#", "https://example.com/tree#/minLength", "https://example.com/tree#/maxLength"];
        "maxLength beside minLength"
    )]
    #[test_case(
        &json!({"$id": "https://example.com/tree", "type": "array", "minItems": 2, "maxItems": 0, "items": {"type": "string"}}),
        &json!([1]),
        &["", "/properties", "https://example.com/tree#", "https://example.com/tree#/type", "https://example.com/tree#/minItems", "https://example.com/tree#/maxItems", "https://example.com/tree#/items", "https://example.com/tree#/items", "https://example.com/tree#/items/type"];
        "array bounds beside items"
    )]
    #[test_case(
        &json!({"$id": "https://example.com/tree", "type": "array", "items": {"type": "string"}}),
        &json!(1),
        &["", "/properties", "https://example.com/tree#", "https://example.com/tree#/type", "https://example.com/tree#/items"];
        "type beside items"
    )]
    #[test_case(
        &json!({"$id": "https://example.com/tree", "patternProperties": {"^a": {}}, "additionalProperties": false}),
        &json!({"a": 1, "b": 1}),
        &["", "/properties", "https://example.com/tree#", "https://example.com/tree#/patternProperties", "https://example.com/tree#/patternProperties/%5Ea", "https://example.com/tree#/additionalProperties", "https://example.com/tree#/additionalProperties"];
        "patternProperties beside additionalProperties"
    )]
    fn list_schema_locations_in_nested_resource(tree: &Value, instance: &Value, expected: &[&str]) {
        let validator =
            validator_for(&json!({"properties": {"tree": tree}})).expect("Should build validator");
        let evaluation = validator.evaluate(&json!({"tree": instance}));
        let list = serde_json::to_value(evaluation.list()).expect("List output");
        let locations: Vec<&str> = list["details"]
            .as_array()
            .expect("List output details")
            .iter()
            .map(|entry| entry["schemaLocation"].as_str().expect("Schema location"))
            .collect();
        assert_eq!(locations, expected);
    }

    #[test]
    fn output_locations_in_nested_resource() {
        let validator = validator_for(&json!({
            "properties": {
                "tree": {"$id": "https://example.com/tree", "properties": {"value": {"type": "string"}}}
            }
        }))
        .expect("Should build validator");

        let evaluation = validator.evaluate(&json!({"tree": {"value": 1}}));
        let list = serde_json::to_value(evaluation.list()).expect("List output");
        let locations: Vec<&str> = list["details"]
            .as_array()
            .expect("List output details")
            .iter()
            .map(|entry| entry["schemaLocation"].as_str().expect("Schema location"))
            .collect();
        assert_eq!(
            locations,
            [
                "",
                "/properties",
                "https://example.com/tree#",
                "https://example.com/tree#/properties",
                "https://example.com/tree#/properties/value",
                "https://example.com/tree#/properties/value/type",
            ]
        );

        let evaluation = validator.evaluate(&json!({"tree": {"value": "leaf"}}));
        let annotations: Vec<Option<String>> = evaluation
            .iter_annotations()
            .map(|entry| entry.absolute_keyword_location.map(ToString::to_string))
            .collect();
        assert_eq!(
            annotations,
            [
                None,
                Some("https://example.com/tree#/properties".to_string())
            ]
        );
    }

    #[test_case(
        &json!({"$id": "https://example.com/root", "patternProperties": {"^a": {}}, "additionalProperties": false}),
        &json!({"a": 1}),
        &[("https://example.com/root#/patternProperties", Some("https://example.com/root#/patternProperties"))];
        "patternProperties beside additionalProperties at resource root"
    )]
    #[test_case(
        &json!({"properties": {"tree": {"$id": "https://example.com/tree", "patternProperties": {"^a": {}}, "additionalProperties": false}}}),
        &json!({"tree": {"a": 1}}),
        &[
            ("/properties", None),
            ("https://example.com/tree#/patternProperties", Some("https://example.com/tree#/patternProperties")),
        ];
        "patternProperties beside additionalProperties in nested resource"
    )]
    #[test_case(
        &json!({"properties": {"tree": {"$id": "https://example.com/tree", "unknown": 1}}}),
        &json!({"tree": 1}),
        &[
            ("/properties", None),
            ("https://example.com/tree#", Some("https://example.com/tree#")),
        ];
        "unknown keyword at nested resource root"
    )]
    fn annotation_locations(schema: &Value, instance: &Value, expected: &[(&str, Option<&str>)]) {
        let validator = validator_for(schema).expect("Should build validator");
        let evaluation = validator.evaluate(instance);
        let locations: Vec<(&str, Option<String>)> = evaluation
            .iter_annotations()
            .map(|entry| {
                (
                    entry.schema_location,
                    entry.absolute_keyword_location.map(ToString::to_string),
                )
            })
            .collect();
        let expected: Vec<(&str, Option<String>)> = expected
            .iter()
            .map(|(schema_location, absolute)| (*schema_location, absolute.map(str::to_string)))
            .collect();
        assert_eq!(locations, expected);
    }

    fn property_with_space() -> crate::Validator {
        validator_for(&json!({
            "$id": "https://example.com/root",
            "properties": {"s p": {"type": "string"}}
        }))
        .expect("Should build validator")
    }

    #[test]
    fn list_output_encodes_schema_locations() {
        let validator = property_with_space();
        let evaluation = validator.evaluate(&json!({"s p": 1}));
        assert_eq!(
            serde_json::to_value(evaluation.list()).expect("List output"),
            json!({
                "valid": false,
                "details": [
                    {
                        "valid": false,
                        "evaluationPath": "",
                        "schemaLocation": "https://example.com/root#",
                        "instanceLocation": ""
                    },
                    {
                        "valid": false,
                        "evaluationPath": "/properties",
                        "schemaLocation": "https://example.com/root#/properties",
                        "instanceLocation": "",
                        "droppedAnnotations": ["s p"]
                    },
                    {
                        "valid": false,
                        "evaluationPath": "/properties/s p",
                        "schemaLocation": "https://example.com/root#/properties/s%20p",
                        "instanceLocation": "/s p"
                    },
                    {
                        "valid": false,
                        "evaluationPath": "/properties/s p/type",
                        "schemaLocation": "https://example.com/root#/properties/s%20p/type",
                        "instanceLocation": "/s p",
                        "errors": {"type": "1 is not of type \"string\""}
                    }
                ]
            })
        );
    }

    #[test]
    fn hierarchical_output_encodes_schema_locations() {
        let validator = property_with_space();
        let evaluation = validator.evaluate(&json!({"s p": 1}));
        assert_eq!(
            serde_json::to_value(evaluation.hierarchical()).expect("Hierarchical output"),
            json!({
                "valid": false,
                "evaluationPath": "",
                "schemaLocation": "https://example.com/root#",
                "instanceLocation": "",
                "details": [
                    {
                        "valid": false,
                        "evaluationPath": "/properties",
                        "schemaLocation": "https://example.com/root#/properties",
                        "instanceLocation": "",
                        "droppedAnnotations": ["s p"],
                        "details": [
                            {
                                "valid": false,
                                "evaluationPath": "/properties/s p",
                                "schemaLocation": "https://example.com/root#/properties/s%20p",
                                "instanceLocation": "/s p",
                                "details": [
                                    {
                                        "valid": false,
                                        "evaluationPath": "/properties/s p/type",
                                        "schemaLocation": "https://example.com/root#/properties/s%20p/type",
                                        "instanceLocation": "/s p",
                                        "errors": {"type": "1 is not of type \"string\""}
                                    }
                                ]
                            }
                        ]
                    }
                ]
            })
        );
    }

    #[test]
    fn annotation_locations_are_encoded() {
        let validator = validator_for(&json!({
            "$id": "https://example.com/root",
            "properties": {"s p": {"properties": {"a b": true}}}
        }))
        .expect("Should build validator");
        let evaluation = validator.evaluate(&json!({"s p": {"a b": 1}}));
        let locations: Vec<(&str, Option<String>, &str)> = evaluation
            .iter_annotations()
            .map(|entry| {
                (
                    entry.schema_location,
                    entry.absolute_keyword_location.map(ToString::to_string),
                    entry.instance_location.as_str(),
                )
            })
            .collect();
        assert_eq!(
            locations,
            [
                (
                    "https://example.com/root#/properties",
                    Some("https://example.com/root#/properties".to_string()),
                    ""
                ),
                (
                    "https://example.com/root#/properties/s%20p/properties",
                    Some("https://example.com/root#/properties/s%20p/properties".to_string()),
                    "/s p"
                ),
            ]
        );
        assert_eq!(
            serde_json::to_value(evaluation.list()).expect("List output"),
            json!({
                "valid": true,
                "details": [
                    {
                        "valid": true,
                        "evaluationPath": "",
                        "schemaLocation": "https://example.com/root#",
                        "instanceLocation": ""
                    },
                    {
                        "valid": true,
                        "evaluationPath": "/properties",
                        "schemaLocation": "https://example.com/root#/properties",
                        "instanceLocation": "",
                        "annotations": ["s p"]
                    },
                    {
                        "valid": true,
                        "evaluationPath": "/properties/s p",
                        "schemaLocation": "https://example.com/root#/properties/s%20p",
                        "instanceLocation": "/s p"
                    },
                    {
                        "valid": true,
                        "evaluationPath": "/properties/s p/properties",
                        "schemaLocation": "https://example.com/root#/properties/s%20p/properties",
                        "instanceLocation": "/s p",
                        "annotations": ["a b"]
                    },
                    {
                        "valid": true,
                        "evaluationPath": "/properties/s p/properties/a b",
                        "schemaLocation": "https://example.com/root#/properties/s%20p/properties/a%20b",
                        "instanceLocation": "/s p/a b"
                    }
                ]
            })
        );
    }

    #[test_case("s p", "s p", "s%20p")]
    #[test_case("a%b", "a%b", "a%25b")]
    #[test_case("q\"t", "q\"t", "q%22t")]
    #[test_case("c^d", "c^d", "c%5Ed")]
    #[test_case("[x]", "[x]", "%5Bx%5D")]
    #[test_case("{y}", "{y}", "%7By%7D")]
    #[test_case("p|q", "p|q", "p%7Cq")]
    #[test_case("b\\s", "b\\s", "b%5Cs")]
    #[test_case("<>", "<>", "%3C%3E")]
    #[test_case("`b", "`b", "%60b")]
    #[test_case("#h", "#h", "%23h")]
    #[test_case("é", "é", "%C3%A9")]
    #[test_case("t~e", "t~0e", "t~0e")]
    #[test_case("s/l", "s~1l", "s~1l")]
    fn error_locations_agree_on_encoding(name: &str, pointer: &str, encoded: &str) {
        let validator = validator_for(&json!({
            "$id": "https://example.com/root",
            "properties": {name: {"type": "string"}}
        }))
        .expect("Should build validator");
        let instance = json!({name: 1});
        let expected = vec![(
            format!("/properties/{pointer}/type"),
            format!("https://example.com/root#/properties/{encoded}/type"),
            format!("/{pointer}"),
        )];

        let from_iter_errors: Vec<(String, String, String)> = validator
            .iter_errors(&instance)
            .map(|error| {
                (
                    error.schema_path().to_string(),
                    error
                        .absolute_keyword_location()
                        .expect("Absolute keyword location")
                        .to_string(),
                    error.instance_path().to_string(),
                )
            })
            .collect();
        assert_eq!(from_iter_errors, expected);

        let evaluation = validator.evaluate(&instance);
        let from_entries: Vec<(String, Option<String>, String)> = evaluation
            .iter_errors()
            .map(|entry| {
                (
                    entry.schema_location.to_string(),
                    entry.absolute_keyword_location.map(ToString::to_string),
                    entry.instance_location.to_string(),
                )
            })
            .collect();
        assert_eq!(
            from_entries,
            [(
                expected[0].1.clone(),
                Some(expected[0].1.clone()),
                expected[0].2.clone()
            )]
        );

        let list = serde_json::to_value(evaluation.list()).expect("List output");
        let from_list: Vec<(String, String, String)> = list["details"]
            .as_array()
            .expect("List output details")
            .iter()
            .filter(|entry| entry.get("errors").is_some())
            .map(|entry| {
                let field = |key: &str| entry[key].as_str().expect("Location").to_string();
                (
                    field("evaluationPath"),
                    field("schemaLocation"),
                    field("instanceLocation"),
                )
            })
            .collect();
        assert_eq!(from_list, expected);
    }

    #[test]
    fn ref_target_location_is_encoded() {
        let validator = validator_for(&json!({
            "$id": "https://example.com/root",
            "$ref": "#/$defs/s%20p",
            "$defs": {"s p": {"type": "string"}}
        }))
        .expect("Should build validator");
        let evaluation = validator.evaluate(&json!(1));
        let list = serde_json::to_value(evaluation.list()).expect("List output");
        let locations: Vec<(&str, &str)> = list["details"]
            .as_array()
            .expect("List output details")
            .iter()
            .map(|entry| {
                (
                    entry["evaluationPath"].as_str().expect("Evaluation path"),
                    entry["schemaLocation"].as_str().expect("Schema location"),
                )
            })
            .collect();
        assert_eq!(
            locations,
            [
                ("", "https://example.com/root#"),
                ("/$ref", "https://example.com/root#/$defs/s%20p"),
                ("/$ref/type", "https://example.com/root#/$defs/s%20p/type"),
            ]
        );
        let absolute: Vec<Option<String>> = validator
            .iter_errors(&json!(1))
            .map(|error| error.absolute_keyword_location().map(ToString::to_string))
            .collect();
        assert_eq!(
            absolute,
            [Some(
                "https://example.com/root#/$defs/s%20p/type".to_string()
            )]
        );
    }

    const DRAFT4: &str = "http://json-schema.org/draft-04/schema#";
    const DRAFT6: &str = "http://json-schema.org/draft-06/schema#";
    const DRAFT7: &str = "http://json-schema.org/draft-07/schema#";
    const DRAFT202012: &str = "https://json-schema.org/draft/2020-12/schema";

    fn with_subschema(root: &str, subschema: &Value) -> Value {
        json!({"$schema": root, "properties": {"a": subschema}})
    }

    // A subschema is checked against the meta-schema of its own `$schema`, not the root's.
    #[test_case(DRAFT202012, &json!({"$id": "https://example.com/a", "$schema": DRAFT4, "items": [{"type": "string"}]}), &json!(["x"]), &json!([1]) ; "draft 4 tuple items under 2020-12")]
    #[test_case(DRAFT202012, &json!({"$schema": DRAFT4, "items": [{"type": "string"}]}), &json!(["x"]), &json!([1]) ; "draft 4 tuple items without id under 2020-12")]
    #[test_case(DRAFT7, &json!({"$schema": DRAFT4, "minimum": 5, "exclusiveMinimum": true}), &json!(6), &json!(5) ; "draft 4 boolean exclusiveMinimum without id under draft 7")]
    #[test_case(DRAFT202012, &json!({"$id": "https://example.com/a", "$schema": DRAFT4, "minimum": 5, "exclusiveMinimum": true}), &json!(6), &json!(5) ; "draft 4 boolean exclusiveMinimum under 2020-12")]
    #[test_case(DRAFT6, &json!({"$id": "https://example.com/a", "$schema": DRAFT4, "minimum": 5, "exclusiveMinimum": true}), &json!(6), &json!(5) ; "draft 4 boolean exclusiveMinimum under draft 6")]
    #[test_case(DRAFT4, &json!({"$id": "https://example.com/a", "$schema": DRAFT202012, "exclusiveMinimum": 5}), &json!(6), &json!(5) ; "2020-12 numeric exclusiveMinimum under draft 4")]
    fn subschema_follows_own_meta_schema(
        root: &str,
        subschema: &Value,
        valid: &Value,
        invalid: &Value,
    ) {
        let validator =
            validator_for(&with_subschema(root, subschema)).expect("Should build validator");
        assert_eq!(
            (
                validator.is_valid(&json!({"a": valid})),
                validator.is_valid(&json!({"a": invalid}))
            ),
            (true, false)
        );
    }

    // The error points at the offending keyword inside the subschema.
    #[test_case(DRAFT7, &json!({"$id": "https://example.com/a", "$schema": DRAFT202012, "minContains": -1}), "/properties/a/minContains" ; "2020-12 under draft 7")]
    #[test_case(DRAFT202012, &json!({"$id": "https://example.com/a", "$schema": DRAFT4, "minimum": 1, "exclusiveMinimum": 5}), "/properties/a/exclusiveMinimum" ; "draft 4 under 2020-12")]
    #[test_case(DRAFT202012, &json!({"$id": "https://example.com/a", "$schema": DRAFT4, "properties": {"b": {"$id": "https://example.com/b", "$schema": DRAFT202012, "minContains": -1}}}), "/properties/a/properties/b/minContains" ; "2020-12 under draft 4 under 2020-12")]
    #[test_case(DRAFT202012, &json!({"$schema": DRAFT4, "exclusiveMaximum": 5}), "/properties/a/exclusiveMaximum" ; "draft 4 without id under 2020-12")]
    #[test_case(DRAFT201909, &json!({"$schema": DRAFT4, "exclusiveMaximum": 5}), "/properties/a/exclusiveMaximum" ; "draft 4 without id under 2019-09")]
    #[test_case(DRAFT7, &json!({"$schema": DRAFT4, "exclusiveMaximum": 5}), "/properties/a/exclusiveMaximum" ; "draft 4 without id under draft 7")]
    fn subschema_rejected_by_own_meta_schema(root: &str, subschema: &Value, expected: &str) {
        let error = validator_for(&with_subschema(root, subschema))
            .expect_err("Should reject the subschema");
        assert_eq!(error.instance_path().as_str(), expected);
    }

    const DRAFT201909: &str = "https://json-schema.org/draft/2019-09/schema";

    fn build_error(schema: &Value) -> ValidationError<'static> {
        validator_for(schema).expect_err("Should reject the schema")
    }

    fn build_error_with_draft4(schema: &Value) -> ValidationError<'static> {
        crate::options()
            .with_draft(Draft::Draft4)
            .build(schema)
            .expect_err("Should reject the schema")
    }

    fn build_error_asserting_formats(schema: &Value) -> ValidationError<'static> {
        crate::options()
            .should_validate_formats(true)
            .build(schema)
            .expect_err("Should reject the schema")
    }

    fn build_error_ignoring_formats(schema: &Value) -> ValidationError<'static> {
        crate::options()
            .should_validate_formats(false)
            .build(schema)
            .expect_err("Should reject the schema")
    }

    // `$schema` names a registered meta-schema written against Draft 7.
    fn build_error_with_custom_meta_schema(schema: &Value) -> ValidationError<'static> {
        let registry = Registry::new()
            .add(
                "https://example.com/meta",
                json!({"$schema": DRAFT7, "type": "object"}),
            )
            .expect("Should accept the meta-schema")
            .prepare()
            .expect("Should create registry");
        crate::options()
            .with_registry(&registry)
            .build(schema)
            .expect_err("Should reject the schema")
    }

    fn build_map_error(schema: &Value) -> ValidationError<'static> {
        crate::options()
            .build_map(schema)
            .expect_err("Should reject the schema")
    }

    fn canonical_prepare_error(schema: &Value) -> ValidationError<'static> {
        let Err(CanonicalizationError::ValidationError(error)) =
            crate::canonical::options().prepare(schema)
        else {
            panic!("Should reject the schema against its meta-schema");
        };
        error
    }

    // A schema its meta-schema rejects fails to build with the first error that meta-schema reports.
    #[test_case(build_error, &json!({"$schema": DRAFT4, "minimum": "foo"}), "\"foo\" is not of type \"number\"", "/minimum", "/properties/minimum/type", "/properties/minimum/type", "http://json-schema.org/draft-04/schema#/properties/minimum/type", "type" ; "draft 4")]
    #[test_case(build_error_with_draft4, &json!({"minimum": "foo"}), "\"foo\" is not of type \"number\"", "/minimum", "/properties/minimum/type", "/properties/minimum/type", "http://json-schema.org/draft-04/schema#/properties/minimum/type", "type" ; "explicit draft 4")]
    #[test_case(build_error, &json!({"$schema": DRAFT6, "exclusiveMinimum": true}), "true is not of type \"number\"", "/exclusiveMinimum", "/properties/exclusiveMinimum/type", "/properties/exclusiveMinimum/type", "http://json-schema.org/draft-06/schema#/properties/exclusiveMinimum/type", "type" ; "draft 6")]
    #[test_case(build_error, &json!({"$schema": DRAFT7, "properties": {"a": {"type": "invalid_type"}}}), "\"invalid_type\" is not valid under any of the schemas listed in the 'anyOf' keyword", "/properties/a/type", "/properties/type/anyOf", "/properties/properties/additionalProperties/$ref/properties/type/anyOf", "http://json-schema.org/draft-07/schema#/properties/type/anyOf", "anyOf" ; "draft 7 subschema")]
    #[test_case(build_error, &json!({"$schema": DRAFT7, "pattern": "["}), "\"[\" is not a \"regex\"", "/pattern", "/properties/pattern/format", "/properties/pattern/format", "http://json-schema.org/draft-07/schema#/properties/pattern/format", "format" ; "draft 7 regex format")]
    #[test_case(build_error_ignoring_formats, &json!({"$schema": DRAFT7, "pattern": "["}), "\"[\" is not a \"regex\"", "/pattern", "/properties/pattern/format", "/properties/pattern/format", "http://json-schema.org/draft-07/schema#/properties/pattern/format", "format" ; "draft 7 regex format with formats ignored")]
    #[test_case(build_error, &json!({"$schema": DRAFT201909, "items": {"minItems": -1}}), "{\"minItems\":-1} is not valid under any of the schemas listed in the 'anyOf' keyword", "/items", "/properties/items/anyOf", "/allOf/1/$ref/properties/items/anyOf", "https://json-schema.org/draft/2019-09/meta/applicator#/properties/items/anyOf", "anyOf" ; "2019-09 subschema")]
    #[test_case(build_error, &json!({"properties": {"a": {"$defs": {"b": {"required": "x"}}}}}), "\"x\" is not of type \"array\"", "/properties/a/$defs/b/required", "/$defs/stringArray/type", "/allOf/1/$ref/properties/properties/additionalProperties/$dynamicRef/allOf/0/$ref/properties/$defs/additionalProperties/$dynamicRef/allOf/3/$ref/properties/required/$ref/type", "https://json-schema.org/draft/2020-12/meta/validation#/$defs/stringArray/type", "type" ; "2020-12 subschema")]
    #[test_case(build_error_asserting_formats, &json!({"properties": {"a": {"$defs": {"b": {"required": "x"}}}}}), "\"x\" is not of type \"array\"", "/properties/a/$defs/b/required", "/$defs/stringArray/type", "/allOf/1/$ref/properties/properties/additionalProperties/$dynamicRef/allOf/0/$ref/properties/$defs/additionalProperties/$dynamicRef/allOf/3/$ref/properties/required/$ref/type", "https://json-schema.org/draft/2020-12/meta/validation#/$defs/stringArray/type", "type" ; "2020-12 subschema with formats asserted")]
    #[test_case(build_error, &json!({"type": "invalid_type", "minimum": "not_a_number", "required": true}), "\"not_a_number\" is not of type \"number\"", "/minimum", "/properties/minimum/type", "/allOf/3/$ref/properties/minimum/type", "https://json-schema.org/draft/2020-12/meta/validation#/properties/minimum/type", "type" ; "several violations")]
    #[test_case(build_error_with_custom_meta_schema, &json!({"$schema": "https://example.com/meta", "minLength": -1}), "-1 is less than the minimum of 0", "/minLength", "/definitions/nonNegativeInteger/minimum", "/properties/minLength/$ref/allOf/0/$ref/minimum", "http://json-schema.org/draft-07/schema#/definitions/nonNegativeInteger/minimum", "minimum" ; "registered custom meta-schema")]
    #[test_case(build_error, &json!({"$schema": "https://example.com/unknown", "minLength": -1}), "-1 is less than the minimum of 0", "/minLength", "/$defs/nonNegativeInteger/minimum", "/allOf/3/$ref/properties/minLength/$ref/$ref/minimum", "https://json-schema.org/draft/2020-12/meta/validation#/$defs/nonNegativeInteger/minimum", "minimum" ; "unregistered meta-schema")]
    #[test_case(build_error, &json!({"$schema": DRAFT7, "properties": {"a": {"$id": "https://example.com/a", "$schema": DRAFT202012, "minContains": -1}}}), "-1 is less than the minimum of 0", "/properties/a/minContains", "/$defs/nonNegativeInteger/minimum", "/allOf/3/$ref/properties/minContains/$ref/minimum", "https://json-schema.org/draft/2020-12/meta/validation#/$defs/nonNegativeInteger/minimum", "minimum" ; "embedded resource")]
    #[test_case(build_error, &json!({"$schema": DRAFT202012, "minLength": -1, "properties": {"a": {"$id": "https://example.com/a", "$schema": DRAFT4, "items": [{}]}}}), "-1 is less than the minimum of 0", "/minLength", "/$defs/nonNegativeInteger/minimum", "/allOf/3/$ref/properties/minLength/$ref/$ref/minimum", "https://json-schema.org/draft/2020-12/meta/validation#/$defs/nonNegativeInteger/minimum", "minimum" ; "resource enclosing an embedded one")]
    #[test_case(build_map_error, &json!({"properties": {"a": {"$defs": {"b": {"required": "x"}}}}}), "\"x\" is not of type \"array\"", "/properties/a/$defs/b/required", "/$defs/stringArray/type", "/allOf/1/$ref/properties/properties/additionalProperties/$dynamicRef/allOf/0/$ref/properties/$defs/additionalProperties/$dynamicRef/allOf/3/$ref/properties/required/$ref/type", "https://json-schema.org/draft/2020-12/meta/validation#/$defs/stringArray/type", "type" ; "validator map")]
    #[test_case(canonical_prepare_error, &json!({"$schema": DRAFT4, "minimum": "foo"}), "\"foo\" is not of type \"number\"", "/minimum", "/properties/minimum/type", "/properties/minimum/type", "http://json-schema.org/draft-04/schema#/properties/minimum/type", "type" ; "canonicalization")]
    fn meta_schema_rejection(
        reject: fn(&Value) -> ValidationError<'static>,
        schema: &Value,
        message: &str,
        instance_path: &str,
        schema_path: &str,
        evaluation_path: &str,
        absolute_keyword_location: &str,
        keyword: &str,
    ) {
        let error = reject(schema);
        assert_eq!(
            (
                error.to_string(),
                error.instance_path().as_str(),
                error.schema_path().as_str(),
                error.evaluation_path().as_str(),
                error
                    .absolute_keyword_location()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                error.kind().keyword(),
            ),
            (
                message.to_string(),
                instance_path,
                schema_path,
                evaluation_path,
                absolute_keyword_location.to_string(),
                keyword,
            )
        );
    }

    #[test_case(crate::is_valid ; "autodetect")]
    #[test_case(crate::draft4::is_valid ; "draft4")]
    #[test_case(crate::draft6::is_valid ; "draft6")]
    #[test_case(crate::draft7::is_valid ; "draft7")]
    #[test_case(crate::draft201909::is_valid ; "draft201909")]
    #[test_case(crate::draft202012::is_valid ; "draft202012")]
    fn test_is_valid(is_valid_fn: fn(&serde_json::Value, &serde_json::Value) -> bool) {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer", "minimum": 0}
            },
            "required": ["name"]
        });

        let valid_instance = json!({
            "name": "John Doe",
            "age": 30
        });

        let invalid_instance = json!({
            "age": -5
        });

        assert!(is_valid_fn(&schema, &valid_instance));
        assert!(!is_valid_fn(&schema, &invalid_instance));
    }

    #[test_case(crate::validate ; "autodetect")]
    #[test_case(crate::draft4::validate ; "draft4")]
    #[test_case(crate::draft6::validate ; "draft6")]
    #[test_case(crate::draft7::validate ; "draft7")]
    #[test_case(crate::draft201909::validate ; "draft201909")]
    #[test_case(crate::draft202012::validate ; "draft202012")]
    fn test_validate(
        validate_fn: for<'i> fn(
            &serde_json::Value,
            &'i serde_json::Value,
        ) -> Result<(), ValidationError<'i>>,
    ) {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer", "minimum": 0}
            },
            "required": ["name"]
        });

        let valid_instance = json!({
            "name": "John Doe",
            "age": 30
        });

        let invalid_instance = json!({
            "age": -5
        });

        assert!(validate_fn(&schema, &valid_instance).is_ok());
        assert!(validate_fn(&schema, &invalid_instance).is_err());
    }

    #[test]
    fn test_evaluate() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer", "minimum": 0}
            },
            "required": ["name"]
        });

        let valid_instance = json!({
            "name": "John Doe",
            "age": 30
        });

        let invalid_instance = json!({
            "age": -5
        });

        let valid_eval = crate::evaluate(&schema, &valid_instance);
        assert!(valid_eval.flag().valid);

        let invalid_eval = crate::evaluate(&schema, &invalid_instance);
        assert!(!invalid_eval.flag().valid);
        let errors: Vec<_> = invalid_eval.iter_errors().collect();
        assert!(!errors.is_empty());
    }

    #[test_case(crate::meta::validate, crate::meta::is_valid ; "autodetect")]
    #[test_case(crate::draft4::meta::validate, crate::draft4::meta::is_valid ; "draft4")]
    #[test_case(crate::draft6::meta::validate, crate::draft6::meta::is_valid ; "draft6")]
    #[test_case(crate::draft7::meta::validate, crate::draft7::meta::is_valid ; "draft7")]
    #[test_case(crate::draft201909::meta::validate, crate::draft201909::meta::is_valid ; "draft201909")]
    #[test_case(crate::draft202012::meta::validate, crate::draft202012::meta::is_valid ; "draft202012")]
    fn test_meta_validation(
        validate_fn: fn(&serde_json::Value) -> Result<(), ValidationError>,
        is_valid_fn: fn(&serde_json::Value) -> bool,
    ) {
        let valid = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer", "minimum": 0}
            },
            "required": ["name"]
        });

        let invalid = json!({
            "type": "invalid_type",
            "minimum": "not_a_number",
            "required": true  // should be an array
        });

        assert!(validate_fn(&valid).is_ok());
        assert!(validate_fn(&invalid).is_err());
        assert!(is_valid_fn(&valid));
        assert!(!is_valid_fn(&invalid));
    }

    #[test_case(&json!({"type": "object", "required": ["name"]}), true ; "valid")]
    #[test_case(&json!({"type": "invalid_type", "required": true}), false ; "invalid")]
    #[test_case(&json!(true), true ; "boolean schema")]
    #[test_case(&json!({"$schema": "http://json-schema.org/draft-04/schema#", "minimum": 5, "exclusiveMinimum": true}), true ; "draft4 boolean exclusive minimum")]
    #[test_case(&json!({"$schema": "http://json-schema.org/draft-04/schema#", "exclusiveMinimum": 5}), false ; "draft4 numeric exclusive minimum")]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "exclusiveMinimum": 5}), true ; "draft2020-12 numeric exclusive minimum")]
    fn test_meta_validate_for(schema: &serde_json::Value, expected: bool) {
        let result = crate::meta::validate_for::<SerdeJson>(schema);
        assert_eq!(result.is_ok(), expected);
        assert_eq!(result.is_ok(), crate::meta::validate(schema).is_ok());
        assert_eq!(
            crate::meta::is_valid_for::<SerdeJson>(schema).expect("Meta-schema is resolvable"),
            expected
        );
    }

    #[test]
    fn test_meta_validate_for_unresolvable_meta_schema() {
        let schema = json!({"$schema": "htt://json-schema.org/draft-07/schema"});
        let error = crate::meta::validate_for::<SerdeJson>(&schema)
            .expect_err("Unresolvable meta-schema should fail");
        assert!(matches!(
            error.kind(),
            crate::error::ValidationErrorKind::Referencing(_)
        ));
        let error = crate::meta::is_valid_for::<SerdeJson>(&schema)
            .expect_err("Unresolvable meta-schema should fail");
        assert!(matches!(
            error.kind(),
            crate::error::ValidationErrorKind::Referencing(_)
        ));
    }

    #[test]
    fn test_exclusive_minimum_across_drafts() {
        // In Draft 4, exclusiveMinimum is a boolean modifier for minimum
        let draft4_schema = json!({
            "$schema": "http://json-schema.org/draft-04/schema#",
            "minimum": 5,
            "exclusiveMinimum": true
        });
        assert!(crate::meta::is_valid(&draft4_schema));
        assert!(crate::meta::validate(&draft4_schema).is_ok());

        // This is invalid in Draft 4 (exclusiveMinimum must be boolean)
        let invalid_draft4 = json!({
            "$schema": "http://json-schema.org/draft-04/schema#",
            "exclusiveMinimum": 5
        });
        assert!(!crate::meta::is_valid(&invalid_draft4));
        assert!(crate::meta::validate(&invalid_draft4).is_err());

        // In Draft 6 and later, exclusiveMinimum is a numeric value
        let drafts = [
            "http://json-schema.org/draft-06/schema#",
            "http://json-schema.org/draft-07/schema#",
            "https://json-schema.org/draft/2019-09/schema",
            "https://json-schema.org/draft/2020-12/schema",
        ];

        for uri in drafts {
            // Valid in Draft 6+ (numeric exclusiveMinimum)
            let valid_schema = json!({
                "$schema": uri,
                "exclusiveMinimum": 5
            });
            assert!(
                crate::meta::is_valid(&valid_schema),
                "Schema should be valid for {uri}"
            );
            assert!(
                crate::meta::validate(&valid_schema).is_ok(),
                "Schema validation should succeed for {uri}",
            );

            // Invalid in Draft 6+ (can't use boolean with minimum)
            let invalid_schema = json!({
                "$schema": uri,
                "minimum": 5,
                "exclusiveMinimum": true
            });
            assert!(
                !crate::meta::is_valid(&invalid_schema),
                "Schema should be invalid for {uri}",
            );
            assert!(
                crate::meta::validate(&invalid_schema).is_err(),
                "Schema validation should fail for {uri}",
            );
        }
    }

    #[test_case(
        "http://json-schema.org/draft-04/schema#",
        true,
        5,
        true ; "draft4 valid"
    )]
    #[test_case(
        "http://json-schema.org/draft-04/schema#",
        5,
        true,
        false ; "draft4 invalid"
    )]
    #[test_case(
        "http://json-schema.org/draft-06/schema#",
        5,
        true,
        false ; "draft6 invalid"
    )]
    #[test_case(
        "http://json-schema.org/draft-07/schema#",
        5,
        true,
        false ; "draft7 invalid"
    )]
    #[test_case(
        "https://json-schema.org/draft/2019-09/schema",
        5,
        true,
        false ; "draft2019-09 invalid"
    )]
    #[test_case(
        "https://json-schema.org/draft/2020-12/schema",
        5,
        true,
        false ; "draft2020-12 invalid"
    )]
    fn test_exclusive_minimum_detection(
        schema_uri: &str,
        exclusive_minimum: impl Into<serde_json::Value>,
        minimum: impl Into<serde_json::Value>,
        expected: bool,
    ) {
        let schema = json!({
            "$schema": schema_uri,
            "minimum": minimum.into(),
            "exclusiveMinimum": exclusive_minimum.into()
        });

        let is_valid_result = crate::meta::is_valid(&schema);
        assert_eq!(is_valid_result, expected);

        let validate_result = crate::meta::validate(&schema);
        assert_eq!(validate_result.is_ok(), expected);
    }

    #[test]
    fn test_invalid_schema_uri() {
        let schema = json!({
            "$schema": "invalid-uri",
            "type": "string"
        });

        let result = crate::options().without_schema_validation().build(&schema);

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.to_string().contains("Unknown meta-schema"));
        assert!(error.to_string().contains("invalid-uri"));
    }

    #[test]
    fn test_invalid_schema_keyword() {
        let schema = json!({
            // Note `htt`, not `http`
            "$schema": "htt://json-schema.org/draft-07/schema",
            "type": "string"
        });

        // Without registering the meta-schema, this should fail
        let result = crate::options().without_schema_validation().build(&schema);

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.to_string().contains("Unknown meta-schema"));
        assert!(error
            .to_string()
            .contains("htt://json-schema.org/draft-07/schema"));
    }

    #[test_case(Draft::Draft4)]
    #[test_case(Draft::Draft6)]
    #[test_case(Draft::Draft7)]
    fn meta_schemas(draft: Draft) {
        // The bundled meta-schemas accept an empty `enum` and one with equal members.
        for schema in [json!({"enum": [0, 0.0]}), json!({"enum": []})] {
            assert!(crate::options().with_draft(draft).build(&schema).is_ok());
        }
    }

    #[test]
    fn incomplete_escape_in_pattern() {
        // An incomplete `\u` escape in `pattern` fails to build.
        let schema = json!({"pattern": "\\u"});
        assert!(crate::validator_for(&schema).is_err());
    }

    #[test]
    fn validation_error_propagation() {
        fn foo() -> Result<(), Box<dyn std::error::Error>> {
            let schema = json!({});
            let validator = validator_for(&schema)?;
            let _ = validator.is_valid(&json!({}));
            Ok(())
        }
        let _ = foo();
    }

    #[test]
    fn test_meta_validation_with_unknown_schema() {
        let schema = json!({
            "$schema": "json-schema:///custom",
            "type": "string"
        });

        // Meta-validation errors when the meta-schema cannot be retrieved
        assert!(crate::meta::validate(&schema).is_err());

        // Building a validator also fails without registration
        let result = crate::validator_for(&schema);
        assert!(result.is_err());
    }

    #[test]
    #[cfg(all(not(target_arch = "wasm32"), feature = "resolve-file"))]
    fn test_meta_validation_respects_metaschema_draft() {
        use std::io::Write;

        let mut temp_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
        let meta_schema_draft7 = json!({
            "$id": "http://example.com/meta/draft7",
            "$schema": "http://json-schema.org/draft-07/schema",
            "type": ["object", "boolean"],
            "properties": {
                "$schema": { "type": "string" },
                "type": {},
                "properties": { "type": "object" }
            },
            "additionalProperties": false
        });
        write!(temp_file, "{meta_schema_draft7}").expect("Failed to write to temp file");

        let uri = crate::retriever::path_to_uri(temp_file.path());

        let schema_using_draft7_meta = json!({
            "$schema": uri,
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "unevaluatedProperties": false
        });

        let schema_valid_for_draft7_meta = json!({
            "$schema": uri,
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            }
        });

        assert!(crate::meta::is_valid(&meta_schema_draft7));
        assert!(!crate::meta::is_valid(&schema_using_draft7_meta));
        assert!(crate::meta::is_valid(&schema_valid_for_draft7_meta));
    }

    #[test]
    #[cfg(all(not(target_arch = "wasm32"), feature = "resolve-file"))]
    fn test_meta_schema_chain_resolution() {
        use std::io::Write;

        // Create intermediate meta-schema pointing to Draft 2020-12
        let mut intermediate_file =
            tempfile::NamedTempFile::new().expect("Failed to create temp file");
        let intermediate_meta = json!({
            "$id": "http://example.com/meta/intermediate",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object"
        });
        write!(intermediate_file, "{intermediate_meta}").expect("Failed to write to temp file");
        let intermediate_uri = crate::retriever::path_to_uri(intermediate_file.path());

        // Create custom meta-schema with unknown draft that points to intermediate
        // This triggers the chain resolution code path in resolve_meta_schema_chain
        let mut custom_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
        let custom_meta = json!({
            "$id": "http://example.com/meta/custom",
            "$schema": intermediate_uri,
            "type": "object"
        });
        write!(custom_file, "{custom_meta}").expect("Failed to write to temp file");
        let custom_uri = crate::retriever::path_to_uri(custom_file.path());

        let schema = json!({
            "$schema": custom_uri,
            "type": "string"
        });

        // Should successfully resolve through the chain and detect Draft 2020-12
        assert!(crate::meta::is_valid(&schema));
    }

    #[test]
    #[cfg(all(not(target_arch = "wasm32"), feature = "resolve-file"))]
    fn test_circular_meta_schema_reference() {
        use std::io::Write;

        // Create meta-schema A pointing to meta-schema B
        let mut meta_a_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
        let meta_a_uri = crate::retriever::path_to_uri(meta_a_file.path());

        // Create meta-schema B pointing back to meta-schema A
        let mut meta_b_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
        let meta_b_uri = crate::retriever::path_to_uri(meta_b_file.path());

        let meta_a = json!({
            "$id": "http://example.com/meta/a",
            "$schema": &meta_b_uri,
            "type": "object"
        });
        write!(meta_a_file, "{meta_a}").expect("Failed to write to temp file");

        let meta_b = json!({
            "$id": "http://example.com/meta/b",
            "$schema": &meta_a_uri,
            "type": "object"
        });
        write!(meta_b_file, "{meta_b}").expect("Failed to write to temp file");

        let schema = json!({
            "$schema": meta_a_uri.clone(),
            "type": "string"
        });

        // Should return a circular meta-schema error
        let result = crate::meta::options().validate(&schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Circular meta-schema reference"));
    }

    #[test]
    fn simple_schema_with_unknown_draft() {
        let meta_schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "http://custom.example.com/schema",
            "$vocabulary": {
                "https://json-schema.org/draft/2020-12/vocab/core": true,
                "https://json-schema.org/draft/2020-12/vocab/applicator": true,
                "https://json-schema.org/draft/2020-12/vocab/validation": true,
            }
        });

        let schema = json!({
            "$schema": "http://custom.example.com/schema",
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            }
        });

        let registry = Registry::new()
            .add("http://custom.example.com/schema", meta_schema)
            .expect("Should accept meta-schema")
            .prepare()
            .expect("Should create registry");
        let validator = crate::options()
            .without_schema_validation()
            .with_registry(&registry)
            .build(&schema)
            .expect("Should build validator");

        assert!(validator.is_valid(&json!({"name": "test"})));
        assert!(!validator.is_valid(&json!({"name": 123})));
        assert!(!validator.is_valid(&json!("not an object")));
    }

    #[test]
    fn custom_meta_schema_support() {
        // Define a custom meta-schema that extends Draft 2020-12
        let meta_schema = json!({
            "$id": "http://example.com/meta/schema",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "Core schema definition",
            "type": "object",
            "allOf": [
                {
                    "$ref": "#/$defs/editable"
                },
                {
                    "$ref": "#/$defs/core"
                }
            ],
            "properties": {
                "properties": {
                    "type": "object",
                    "patternProperties": {
                        ".*": {
                            "type": "object",
                            "properties": {
                                "type": {
                                    "type": "string",
                                    "enum": [
                                        "array",
                                        "boolean",
                                        "integer",
                                        "number",
                                        "object",
                                        "string",
                                        "null"
                                    ]
                                }
                            }
                        }
                    },
                    "propertyNames": {
                        "type": "string",
                        "pattern": "^[A-Za-z_][A-Za-z0-9_]*$"
                    }
                }
            },
            "unevaluatedProperties": false,
            "required": [
                "properties"
            ],
            "$defs": {
                "core": {
                    "type": "object",
                    "properties": {
                        "$id": {
                            "type": "string"
                        },
                        "$schema": {
                            "type": "string"
                        },
                        "type": {
                            "const": "object"
                        },
                        "title": {
                            "type": "string"
                        },
                        "description": {
                            "type": "string"
                        },
                        "additionalProperties": {
                            "type": "boolean",
                            "const": false
                        }
                    },
                    "required": [
                        "$id",
                        "$schema",
                        "type"
                    ]
                },
                "editable": {
                    "type": "object",
                    "properties": {
                        "creationDate": {
                            "type": "string",
                            "format": "date-time"
                        },
                        "updateDate": {
                            "type": "string",
                            "format": "date-time"
                        }
                    },
                    "required": [
                        "creationDate"
                    ]
                }
            }
        });

        // A schema that uses the custom meta-schema
        let element_schema = json!({
            "$schema": "http://example.com/meta/schema",
            "$id": "http://example.com/schemas/element",
            "title": "Element",
            "description": "An element",
            "creationDate": "2024-12-31T12:31:53+01:00",
            "properties": {
                "value": {
                    "type": "string"
                }
            },
            "type": "object"
        });

        let registry = Registry::new()
            .add("http://example.com/meta/schema", meta_schema)
            .expect("Should accept meta-schema")
            .prepare()
            .expect("Should create registry");
        let validator = crate::options()
            .without_schema_validation()
            .with_registry(&registry)
            .build(&element_schema)
            .expect("Should successfully build validator with custom meta-schema");

        let valid_instance = json!({
            "value": "test string"
        });
        assert!(validator.is_valid(&valid_instance));

        let invalid_instance = json!({
            "value": 123
        });
        assert!(!validator.is_valid(&invalid_instance));
    }

    #[test]
    fn custom_meta_schema_with_fragment_finds_vocabularies() {
        // Custom meta-schema URIs with trailing # should be found in registry
        let custom_meta = json!({
            "$id": "http://example.com/custom-with-unevaluated",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$vocabulary": {
                "https://json-schema.org/draft/2020-12/vocab/core": true,
                "https://json-schema.org/draft/2020-12/vocab/applicator": true,
                "https://json-schema.org/draft/2020-12/vocab/validation": true,
                "https://json-schema.org/draft/2020-12/vocab/unevaluated": true
            }
        });

        let registry = Registry::new()
            .add("http://example.com/custom-with-unevaluated", custom_meta)
            .expect("Should accept meta-schema")
            .prepare()
            .expect("Should create registry");

        let schema = json!({
            "$id": "http://example.com/subject",
            "$schema": "http://example.com/custom-with-unevaluated#",
            "type": "object",
            "properties": {
                "foo": { "type": "string" }
            },
            "unevaluatedProperties": false
        });

        let validator = crate::options()
            .without_schema_validation()
            .with_registry(&registry)
            .build(&schema)
            .expect("Should build validator");

        assert!(validator.is_valid(&json!({"foo": "bar"})));
        assert!(!validator.is_valid(&json!({"foo": "bar", "extra": "value"})));
    }

    fn dialect_registry(vocabularies: &Value) -> Registry<'static> {
        Registry::new()
            .add(
                "https://example.com/dialect",
                json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "$id": "https://example.com/dialect",
                    "$vocabulary": vocabularies,
                }),
            )
            .expect("Should accept meta-schema")
            .prepare()
            .expect("Should create registry")
    }

    fn format_assertion_dialect(required: bool) -> Value {
        json!({
            "https://json-schema.org/draft/2020-12/vocab/core": true,
            "https://json-schema.org/draft/2020-12/vocab/validation": true,
            "https://json-schema.org/draft/2020-12/vocab/format-assertion": required,
        })
    }

    // The boolean only steers implementations that do not know the vocabulary.
    #[test_case(true)]
    #[test_case(false)]
    fn format_assertion_vocabulary_asserts_without_opt_in(required: bool) {
        let registry = dialect_registry(&format_assertion_dialect(required));
        let validator = crate::options()
            .with_registry(&registry)
            .build(&json!({
                "$schema": "https://example.com/dialect",
                "type": "string",
                "format": "date-time"
            }))
            .expect("Should build validator");

        assert!(validator.is_valid(&json!("2026-08-22T12:00:00Z")));
        assert!(!validator.is_valid(&json!("not-a-date")));
    }

    #[test_case(false)]
    #[test_case(true)]
    fn format_assertion_vocabulary_rejects_unknown_format(ignore_unknown_formats: bool) {
        let registry = dialect_registry(&format_assertion_dialect(true));
        let error = crate::options()
            .with_registry(&registry)
            .should_ignore_unknown_formats(ignore_unknown_formats)
            .build(&json!({
                "$schema": "https://example.com/dialect",
                "format": "totally-made-up"
            }))
            .expect_err("Should reject unknown format");

        assert_eq!(
            error.to_string(),
            "Unknown format: 'totally-made-up'. The meta-schema asserts formats, so unrecognized ones cannot be ignored. Register a check for it or disable format validation"
        );
    }

    #[test]
    fn format_assertion_vocabulary_honors_explicit_opt_out() {
        let registry = dialect_registry(&format_assertion_dialect(true));
        let validator = crate::options()
            .with_registry(&registry)
            .should_validate_formats(false)
            .build(&json!({
                "$schema": "https://example.com/dialect",
                "format": "date-time"
            }))
            .expect("Should build validator");

        assert!(validator.is_valid(&json!("not-a-date")));
    }

    #[test_case(true, false ; "required")]
    #[test_case(false, true ; "optional")]
    fn draft201909_format_vocabulary_asserts_when_required(required: bool, expected: bool) {
        let registry = Registry::new()
            .add(
                "https://example.com/dialect",
                json!({
                    "$schema": "https://json-schema.org/draft/2019-09/schema",
                    "$id": "https://example.com/dialect",
                    "$vocabulary": {
                        "https://json-schema.org/draft/2019-09/vocab/core": true,
                        "https://json-schema.org/draft/2019-09/vocab/validation": true,
                        "https://json-schema.org/draft/2019-09/vocab/format": required,
                    },
                }),
            )
            .expect("Should accept meta-schema")
            .prepare()
            .expect("Should create registry");
        let validator = crate::options()
            .with_registry(&registry)
            .build(&json!({
                "$schema": "https://example.com/dialect",
                "format": "ipv4"
            }))
            .expect("Should build validator");

        assert_eq!(validator.is_valid(&json!("not-an-ipv4")), expected);
    }

    #[test]
    fn dialect_without_id_declares_vocabularies() {
        let registry = Registry::new()
            .add(
                "https://example.com/dialect",
                json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "$vocabulary": format_assertion_dialect(true),
                }),
            )
            .expect("Should accept meta-schema")
            .prepare()
            .expect("Should create registry");
        let validator = crate::options()
            .with_registry(&registry)
            .build(&json!({
                "$schema": "https://example.com/dialect",
                "format": "ipv4"
            }))
            .expect("Should build validator");

        assert!(!validator.is_valid(&json!("not-an-ipv4")));
    }

    #[test]
    fn nested_dialect_declares_vocabularies() {
        let registry = dialect_registry(&format_assertion_dialect(true));
        let validator = crate::options()
            .with_draft(Draft::Draft4)
            .with_registry(&registry)
            .build(&json!({
                "properties": {
                    "address": {"$schema": "https://example.com/dialect", "format": "ipv4"}
                }
            }))
            .expect("Should build validator");

        assert!(!validator.is_valid(&json!({"address": "not-an-ipv4"})));
    }

    #[test_case("https://json-schema.org/draft/2020-12/meta/format-assertion" ; "2020-12 format-assertion")]
    #[test_case("https://json-schema.org/draft/2019-09/meta/format" ; "2019-09 format")]
    fn bundled_format_meta_schema_asserts(meta_schema: &str) {
        let validator = crate::options()
            .build(&json!({
                "$schema": meta_schema,
                "format": "ipv4"
            }))
            .expect("Should build validator");

        assert!(validator.is_valid(&json!("127.0.0.1")));
        assert!(!validator.is_valid(&json!("not-an-ipv4")));
    }

    #[test]
    fn format_annotation_vocabulary_does_not_assert() {
        let registry = dialect_registry(&json!({
            "https://json-schema.org/draft/2020-12/vocab/core": true,
            "https://json-schema.org/draft/2020-12/vocab/format-annotation": true,
        }));
        let validator = crate::options()
            .with_registry(&registry)
            .build(&json!({
                "$schema": "https://example.com/dialect",
                "format": "date-time"
            }))
            .expect("Should build validator");

        assert!(validator.is_valid(&json!("not-a-date")));
    }

    const UNKNOWN_VOCABULARY_ERROR: &str = "Unknown vocabulary: 'https://example.com/vocab/made-up' is required by the meta-schema. Adjust configuration to declare support for it";

    #[test_case("https://example.com/vocab/made-up", true, false, Some(UNKNOWN_VOCABULARY_ERROR) ; "required undeclared")]
    #[test_case("https://example.com/vocab/made-up", false, false, None ; "optional undeclared")]
    #[test_case("https://example.com/vocab/made-up", true, true, None ; "required declared")]
    #[test_case("https://Example.COM/vocab/made-up", true, true, None ; "declared uppercase host")]
    #[test_case("https://example.com/vocab/./made-up", true, true, None ; "declared dot segment")]
    #[test_case("https://example.com/vocab/made%2Dup", true, true, None ; "declared percent-encoded unreserved")]
    fn unknown_vocabulary(uri: &str, required: bool, declared: bool, expected: Option<&str>) {
        let registry = dialect_registry(&json!({
            "https://json-schema.org/draft/2020-12/vocab/core": true,
            uri: required,
        }));
        let mut options = crate::options().with_registry(&registry);
        if declared {
            options = options.with_vocabulary(uri);
        }

        // `Clone` keeps the declarations.
        let error = options
            .clone()
            .build(&json!({"$schema": "https://example.com/dialect"}))
            .err()
            .map(|error| error.to_string());

        assert_eq!(error.as_deref(), expected);
    }

    #[test_case("$ref", "https://example.com/inner" ; "ref to resource")]
    #[test_case("$ref", "https://example.com/inner#/$defs/leaf" ; "ref into resource")]
    #[test_case("$dynamicRef", "https://example.com/inner" ; "dynamic ref to resource")]
    fn required_unknown_vocabulary_is_rejected_behind_reference(keyword: &str, target: &str) {
        let registry = Registry::new()
            .add(
                "https://example.com/dialect",
                json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "$id": "https://example.com/dialect",
                    "$vocabulary": {
                        "https://json-schema.org/draft/2020-12/vocab/core": true,
                        "https://example.com/vocab/made-up": true,
                    },
                }),
            )
            .expect("Should accept meta-schema")
            .add(
                "https://example.com/inner",
                json!({
                    "$schema": "https://example.com/dialect",
                    "$id": "https://example.com/inner",
                    "type": "string",
                    "$defs": {"leaf": {"type": "string"}},
                }),
            )
            .expect("Should accept inner schema")
            .prepare()
            .expect("Should create registry");
        let error = crate::options()
            .with_registry(&registry)
            .build(&json!({keyword: target}))
            .expect_err("Should reject required unknown vocabulary");

        assert_eq!(error.to_string(), UNKNOWN_VOCABULARY_ERROR);
    }

    #[test]
    fn strict_meta_schema_catches_typos() {
        // A meta-schema with `unevaluatedProperties: false` rejects unknown keywords, top-level
        // and nested.

        let strict_meta = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://json-schema.org/draft/2020-12/strict",
            "$dynamicAnchor": "meta",
            "$ref": "https://json-schema.org/draft/2020-12/schema",
            "unevaluatedProperties": false
        });

        let registry = Registry::new()
            .add("https://json-schema.org/draft/2020-12/strict", strict_meta)
            .expect("Should accept strict meta-schema")
            .prepare()
            .expect("Should create registry");

        // Valid schema - all keywords are recognized
        let valid_schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/strict",
            "type": "object",
            "properties": {
                "name": {"type": "string", "minLength": 1}
            }
        });

        assert!(crate::meta::options()
            .with_registry(&registry)
            .is_valid(&valid_schema));

        // Invalid schema - top-level typo "typ" instead of "type"
        let invalid_schema_top_level = json!({
            "$schema": "https://json-schema.org/draft/2020-12/strict",
            "typ": "string"  // Typo
        });

        assert!(!crate::meta::options()
            .with_registry(&registry)
            .is_valid(&invalid_schema_top_level));

        // Invalid schema - nested invalid keyword "minSize" (not a real JSON Schema keyword)
        let invalid_schema_nested = json!({
            "$schema": "https://json-schema.org/draft/2020-12/strict",
            "type": "object",
            "properties": {
                "name": {"type": "string", "minSize": 1}  // Invalid keyword in nested schema
            }
        });

        assert!(!crate::meta::options()
            .with_registry(&registry)
            .is_valid(&invalid_schema_nested));
    }

    #[test]
    fn custom_meta_schema_preserves_underlying_draft_behavior() {
        // A custom meta-schema written against Draft 7 keeps Draft 7 rules: `$ref` siblings are
        // ignored.

        let custom_meta_draft7 = json!({
            "$id": "http://example.com/meta/draft7-custom",
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "properties": {
                "customKeyword": {"type": "string"}
            }
        });

        let registry = Registry::new()
            .add("http://example.com/meta/draft7-custom", custom_meta_draft7)
            .expect("Should accept meta-schema")
            .prepare()
            .expect("Should create registry");

        let schema = json!({
            "$id": "http://example.com/subject",
            "$schema": "http://example.com/meta/draft7-custom",
            "$ref": "#/$defs/positiveNumber",
            "maximum": 5,
            "$defs": {
                "positiveNumber": {
                    "type": "number",
                    "minimum": 0
                }
            }
        });

        let validator = crate::options()
            .without_schema_validation()
            .with_registry(&registry)
            .build(&schema)
            .expect("Should build validator");

        // In Draft 7: siblings of $ref are ignored, so maximum: 5 has no effect
        // In Draft 2020-12: siblings are evaluated, so maximum: 5 would apply
        assert!(validator.is_valid(&json!(10)));
    }

    mod meta_options_tests {
        use super::*;
        use crate::Registry;

        #[test]
        fn test_meta_options_with_registry_valid_schema() {
            let custom_meta = json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {
                    "$schema": { "type": "string" },
                    "type": { "type": "string" },
                    "maxLength": { "type": "integer" }
                },
                "additionalProperties": false
            });

            let registry = Registry::new()
                .add("http://example.com/meta", custom_meta)
                .unwrap()
                .prepare()
                .unwrap();

            let schema = json!({
                "$schema": "http://example.com/meta",
                "type": "string",
                "maxLength": 10
            });

            assert!(crate::meta::options()
                .with_registry(&registry)
                .is_valid(&schema));

            assert!(crate::meta::options()
                .with_registry(&registry)
                .validate(&schema)
                .is_ok());
        }

        #[test]
        fn test_meta_options_with_registry_invalid_schema() {
            let custom_meta = json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {
                    "type": { "type": "string" }
                },
                "additionalProperties": false
            });

            let registry = Registry::new()
                .add("http://example.com/meta", custom_meta)
                .unwrap()
                .prepare()
                .unwrap();

            // Schema has disallowed property
            let schema = json!({
                "$schema": "http://example.com/meta",
                "type": "string",
                "maxLength": 10  // Not allowed by custom meta-schema
            });

            assert!(!crate::meta::options()
                .with_registry(&registry)
                .is_valid(&schema));

            assert!(crate::meta::options()
                .with_registry(&registry)
                .validate(&schema)
                .is_err());
        }

        #[test]
        fn test_meta_options_with_registry_chain() {
            // Create a chain: custom-meta -> draft2020-12
            let custom_meta = json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object"
            });

            let registry = Registry::new()
                .add("http://example.com/custom", custom_meta)
                .unwrap()
                .prepare()
                .unwrap();

            let schema = json!({
                "$schema": "http://example.com/custom",
                "type": "string"
            });

            assert!(crate::meta::options()
                .with_registry(&registry)
                .is_valid(&schema));
        }

        #[test]
        fn test_meta_options_with_registry_multi_level_chain() {
            // Create chain: schema -> meta-level-2 -> meta-level-1 -> draft2020-12
            let meta_level_1 = json!({
                "$id": "http://example.com/meta/level1",
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {
                    "customProp": { "type": "boolean" }
                }
            });

            let meta_level_2 = json!({
                "$id": "http://example.com/meta/level2",
                "$schema": "http://example.com/meta/level1",
                "type": "object",
                "customProp": true
            });

            let registry = Registry::new()
                .extend([
                    ("http://example.com/meta/level1", meta_level_1),
                    ("http://example.com/meta/level2", meta_level_2),
                ])
                .unwrap()
                .prepare()
                .unwrap();

            let schema = json!({
                "$schema": "http://example.com/meta/level2",
                "type": "string",
                "customProp": true
            });

            assert!(crate::meta::options()
                .with_registry(&registry)
                .is_valid(&schema));
        }

        #[test]
        fn test_meta_options_with_registry_multi_document_meta_schema() {
            let shared_constraints = json!({
                "$id": "http://example.com/meta/shared",
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {
                    "maxLength": { "type": "integer", "minimum": 0 }
                }
            });

            let root_meta = json!({
                "$id": "http://example.com/meta/root",
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {
                    "$schema": { "type": "string" },
                    "type": { "type": "string" }
                },
                "allOf": [
                    { "$ref": "http://example.com/meta/shared" }
                ]
            });

            let registry = Registry::new()
                .extend([
                    ("http://example.com/meta/root", root_meta),
                    ("http://example.com/meta/shared", shared_constraints),
                ])
                .unwrap()
                .prepare()
                .unwrap();

            let schema = json!({
                "$schema": "http://example.com/meta/root",
                "type": "string",
                "maxLength": 5
            });

            let result = crate::meta::options()
                .with_registry(&registry)
                .validate(&schema);

            assert!(
                result.is_ok(),
                "meta validation failed even though registry contains all meta-schemas: {}",
                result.unwrap_err()
            );

            assert!(crate::meta::options()
                .with_registry(&registry)
                .is_valid(&schema));
        }

        #[test]
        fn test_meta_options_without_registry_unknown_metaschema() {
            let schema = json!({
                "$schema": "http://0.0.0.0/nonexistent",
                "type": "string"
            });

            // Without registry, should fail to resolve
            let result = crate::meta::options().validate(&schema);
            assert!(result.is_err());
        }

        #[test]
        #[should_panic(expected = "Failed to resolve meta-schema")]
        fn test_meta_options_is_valid_panics_on_missing_metaschema() {
            let schema = json!({
                "$schema": "http://0.0.0.0/nonexistent",
                "type": "string"
            });

            // is_valid() should panic if meta-schema cannot be resolved
            let _ = crate::meta::options().is_valid(&schema);
        }

        #[test]
        fn test_meta_options_with_registry_missing_metaschema() {
            let custom_meta = json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object"
            });

            let registry = Registry::new()
                .add("http://example.com/meta1", custom_meta)
                .unwrap()
                .prepare()
                .unwrap();

            // Schema references a different meta-schema not in registry
            let schema = json!({
                "$schema": "http://example.com/meta2",
                "type": "string"
            });

            let result = crate::meta::options()
                .with_registry(&registry)
                .validate(&schema);

            assert!(result.is_err());
        }

        #[test]
        fn test_meta_options_circular_reference_detection() {
            // Create a circular reference: meta1 -> meta2 -> meta1
            let meta1 = json!({
                "$id": "http://example.com/meta1",
                "$schema": "http://example.com/meta2",
                "type": "object"
            });

            let meta2 = json!({
                "$id": "http://example.com/meta2",
                "$schema": "http://example.com/meta1",
                "type": "object"
            });

            let registry = Registry::new()
                .extend([
                    ("http://example.com/meta1", meta1),
                    ("http://example.com/meta2", meta2),
                ])
                .unwrap()
                .prepare()
                .unwrap();

            let schema = json!({
                "$schema": "http://example.com/meta1",
                "type": "string"
            });

            let result = crate::meta::options()
                .with_registry(&registry)
                .validate(&schema);

            assert!(result.is_err());
            // Check it's specifically a circular error
            let err = result.unwrap_err();
            assert!(err.to_string().contains("Circular"));
        }

        #[test]
        fn test_meta_options_standard_drafts_without_registry() {
            // Standard drafts should work without registry
            let schemas = vec![
                json!({ "$schema": "http://json-schema.org/draft-04/schema#", "type": "string" }),
                json!({ "$schema": "http://json-schema.org/draft-06/schema#", "type": "string" }),
                json!({ "$schema": "http://json-schema.org/draft-07/schema#", "type": "string" }),
                json!({ "$schema": "https://json-schema.org/draft/2019-09/schema", "type": "string" }),
                json!({ "$schema": "https://json-schema.org/draft/2020-12/schema", "type": "string" }),
            ];

            for schema in schemas {
                assert!(
                    crate::meta::options().is_valid(&schema),
                    "Failed for schema: {schema}"
                );
            }
        }

        #[test]
        fn test_meta_options_validate_returns_specific_errors() {
            let custom_meta = json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "required": ["type"]
            });

            let registry = Registry::new()
                .add("http://example.com/meta", custom_meta)
                .unwrap()
                .prepare()
                .unwrap();

            // Schema missing required property
            let schema = json!({
                "$schema": "http://example.com/meta",
                "properties": {
                    "name": { "type": "string" }
                }
            });

            let result = crate::meta::options()
                .with_registry(&registry)
                .validate(&schema);

            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.to_string().contains("required") || err.to_string().contains("type"));
        }

        #[test]
        fn test_meta_options_builds_validator_with_resolved_draft() {
            let custom_meta = json!({
                "$id": "http://example.com/meta/draft7-based",
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "object",
                "properties": {
                    "$schema": { "type": "string" },
                    "type": { "type": "string" },
                    "minLength": { "type": "integer" }
                },
                "additionalProperties": false
            });

            let registry = Registry::new()
                .add("http://example.com/meta/draft7-based", custom_meta)
                .unwrap()
                .prepare()
                .unwrap();

            let schema = json!({
                "$schema": "http://example.com/meta/draft7-based",
                "type": "string",
                "minLength": 5
            });

            let result = crate::meta::options()
                .with_registry(&registry)
                .validate(&schema);

            assert!(result.is_ok());
        }

        #[test]
        fn test_meta_options_validator_uses_correct_draft() {
            let custom_meta_draft6 = json!({
                "$id": "http://example.com/meta/draft6-based",
                "$schema": "http://json-schema.org/draft-06/schema#",
                "type": "object",
                "properties": {
                    "$schema": { "type": "string" },
                    "type": { "type": "string" },
                    "exclusiveMinimum": { "type": "number" }
                },
                "additionalProperties": false
            });

            let registry = Registry::new()
                .add("http://example.com/meta/draft6-based", custom_meta_draft6)
                .unwrap()
                .prepare()
                .unwrap();

            let schema_valid_for_draft6 = json!({
                "$schema": "http://example.com/meta/draft6-based",
                "type": "number",
                "exclusiveMinimum": 0
            });

            let result = crate::meta::options()
                .with_registry(&registry)
                .validate(&schema_valid_for_draft6);

            assert!(result.is_ok());
        }

        #[test]
        fn test_meta_options_without_schema_validation_in_built_validator() {
            let custom_meta = json!({
                "$id": "http://example.com/meta/custom",
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {
                    "$schema": { "type": "string" },
                    "type": { "type": "string" }
                },
                "additionalProperties": false
            });

            let registry = Registry::new()
                .add("http://example.com/meta/custom", custom_meta)
                .unwrap()
                .prepare()
                .unwrap();

            let schema = json!({
                "$schema": "http://example.com/meta/custom",
                "type": "string"
            });

            let result = crate::meta::options()
                .with_registry(&registry)
                .validate(&schema);

            assert!(result.is_ok());
        }

        #[test]
        fn test_meta_validation_uses_resolved_draft_from_chain() {
            // Chain: user-schema -> custom-meta -> Draft 4
            // Validator should use Draft 4 rules to validate the schema
            let custom_meta = json!({
                "$id": "http://example.com/meta/draft4-based",
                "$schema": "http://json-schema.org/draft-04/schema#",
                "type": "object",
                "properties": {
                    "$schema": { "type": "string" },
                    "type": { "type": "string" },
                    "enum": { "type": "array" },
                    "const": { "type": "string" }
                },
                "additionalProperties": false
            });

            let registry = Registry::new()
                .add("http://example.com/meta/draft4-based", custom_meta)
                .unwrap()
                .prepare()
                .unwrap();

            let schema = json!({
                "$schema": "http://example.com/meta/draft4-based",
                "type": "string",
                "const": "foo"
            });

            let result = crate::meta::options()
                .with_registry(&registry)
                .validate(&schema);

            assert!(result.is_ok());
        }

        #[test]
        fn test_meta_validation_multi_level_chain_uses_resolved_draft() {
            // Multi-level chain: user-schema -> meta-2 -> meta-1 -> Draft 4
            let meta_level_1 = json!({
                "$id": "http://example.com/meta/level1",
                "$schema": "http://json-schema.org/draft-04/schema#",
                "type": "object",
                "properties": {
                    "customKeyword": { "type": "boolean" }
                }
            });

            let meta_level_2 = json!({
                "$id": "http://example.com/meta/level2",
                "$schema": "http://example.com/meta/level1",
                "type": "object",
                "properties": {
                    "$schema": { "type": "string" },
                    "type": { "type": "string" },
                    "minimum": { "type": "number" },
                    "exclusiveMinimum": { "type": "boolean" }
                },
                "customKeyword": true,
                "additionalProperties": false
            });

            let registry = Registry::new()
                .extend([
                    ("http://example.com/meta/level1", meta_level_1),
                    ("http://example.com/meta/level2", meta_level_2),
                ])
                .unwrap()
                .prepare()
                .unwrap();

            let schema = json!({
                "$schema": "http://example.com/meta/level2",
                "type": "number",
                "minimum": 5,
                "exclusiveMinimum": true
            });

            let result = crate::meta::options()
                .with_registry(&registry)
                .validate(&schema);

            assert!(result.is_ok());
        }
    }

    #[test]
    fn test_meta_validator_for_valid_schema() {
        let schema = json!({
            "type": "string",
            "maxLength": 5
        });

        let validator = crate::meta::validator_for(&schema).expect("Valid meta-schema");
        assert!(validator.is_valid(&schema));
    }

    #[test]
    fn test_meta_validator_for_invalid_schema() {
        let schema = json!({
            "type": "invalid_type"
        });

        let validator = crate::meta::validator_for(&schema).expect("Valid meta-schema");
        assert!(!validator.is_valid(&schema));
    }

    #[test]
    fn test_meta_validator_for_evaluate_api() {
        let schema = json!({
            "type": "string",
            "maxLength": 5
        });

        let validator = crate::meta::validator_for(&schema).expect("Valid meta-schema");
        let evaluation = validator.evaluate(&schema);

        let flag = evaluation.flag();
        assert!(flag.valid);
    }

    #[test]
    fn test_meta_validator_for_evaluate_api_invalid() {
        let schema = json!({
            "type": "invalid_type",
            "minimum": "not a number"
        });

        let validator = crate::meta::validator_for(&schema).expect("Valid meta-schema");
        let evaluation = validator.evaluate(&schema);

        let flag = evaluation.flag();
        assert!(!flag.valid);
    }

    #[test]
    fn test_meta_validator_for_all_drafts() {
        let schemas = vec![
            json!({ "$schema": "http://json-schema.org/draft-04/schema#", "type": "string" }),
            json!({ "$schema": "http://json-schema.org/draft-06/schema#", "type": "string" }),
            json!({ "$schema": "http://json-schema.org/draft-07/schema#", "type": "string" }),
            json!({ "$schema": "https://json-schema.org/draft/2019-09/schema", "type": "string" }),
            json!({ "$schema": "https://json-schema.org/draft/2020-12/schema", "type": "string" }),
        ];

        for schema in schemas {
            let validator = crate::meta::validator_for(&schema).unwrap();
            assert!(validator.is_valid(&schema));
        }
    }

    #[test]
    fn test_meta_validator_for_iter_errors() {
        let schema = json!({
            "type": "invalid_type",
            "minimum": "not a number"
        });

        let validator = crate::meta::validator_for(&schema).expect("Valid meta-schema");
        let errors: Vec<_> = validator.iter_errors(&schema).collect();
        assert!(!errors.is_empty());
    }

    #[test_case(
        "http://json-schema.org/draft-04/schema#",
        "http://json-schema.org/draft-04/schema#/definitions/positiveInteger/minimum",
        "/definitions/positiveInteger/minimum";
        "draft 4"
    )]
    #[test_case(
        "http://json-schema.org/draft-06/schema#",
        "http://json-schema.org/draft-06/schema#/definitions/nonNegativeInteger/minimum",
        "/definitions/nonNegativeInteger/minimum";
        "draft 6"
    )]
    #[test_case(
        "http://json-schema.org/draft-07/schema#",
        "http://json-schema.org/draft-07/schema#/definitions/nonNegativeInteger/minimum",
        "/definitions/nonNegativeInteger/minimum";
        "draft 7"
    )]
    #[test_case(
        "https://json-schema.org/draft/2019-09/schema",
        "https://json-schema.org/draft/2019-09/meta/validation#/$defs/nonNegativeInteger/minimum",
        "/$defs/nonNegativeInteger/minimum";
        "draft 2019-09"
    )]
    #[test_case(
        "https://json-schema.org/draft/2020-12/schema",
        "https://json-schema.org/draft/2020-12/meta/validation#/$defs/nonNegativeInteger/minimum",
        "/$defs/nonNegativeInteger/minimum";
        "draft 2020-12"
    )]
    fn test_meta_validator_for_reports_negative_length(
        dialect: &str,
        schema_location: &str,
        schema_path: &str,
    ) {
        const NEGATIVE: &str = "-1 is less than the minimum of 0";
        let schema = json!({"$schema": dialect, "minLength": -1});
        let validator = crate::meta::validator_for(&schema).expect("Valid meta-schema");

        let evaluation = validator.evaluate(&schema);
        let evaluated: Vec<_> = evaluation
            .iter_errors()
            .map(|entry| {
                (
                    entry.instance_location.as_str().to_string(),
                    entry.schema_location.to_string(),
                    entry.error.to_string(),
                )
            })
            .collect();
        let errors: Vec<_> = validator
            .iter_errors(&schema)
            .map(|error| {
                (
                    error.to_string(),
                    error.instance_path().to_string(),
                    error.schema_path().to_string(),
                )
            })
            .collect();
        assert_eq!(
            (evaluation.flag().valid, evaluated, errors),
            (
                false,
                vec![(
                    "/minLength".to_string(),
                    schema_location.to_string(),
                    NEGATIVE.to_string()
                )],
                vec![(
                    NEGATIVE.to_string(),
                    "/minLength".to_string(),
                    schema_path.to_string()
                )]
            )
        );
    }
}

#[cfg(all(test, feature = "resolve-async", not(target_family = "wasm")))]
mod async_tests {
    use std::{collections::HashMap, sync::Arc};

    use serde_json::json;

    use crate::{AsyncRetrieve, Draft, Uri};

    #[derive(Clone)]
    struct TestRetriever {
        schemas: HashMap<String, serde_json::Value>,
    }

    impl TestRetriever {
        fn new() -> Self {
            let mut schemas = HashMap::new();
            schemas.insert(
                "https://example.com/user.json".to_string(),
                json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "age": {"type": "integer", "minimum": 0}
                    },
                    "required": ["name"]
                }),
            );
            Self { schemas }
        }
    }

    #[cfg_attr(target_family = "wasm", async_trait::async_trait(?Send))]
    #[cfg_attr(not(target_family = "wasm"), async_trait::async_trait)]
    impl AsyncRetrieve for TestRetriever {
        async fn retrieve(
            &self,
            uri: &Uri<String>,
        ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
            self.schemas
                .get(uri.as_str())
                .cloned()
                .ok_or_else(|| "Schema not found".into())
        }
    }

    #[tokio::test]
    async fn test_async_validator_for() {
        let schema = json!({
            "$ref": "https://example.com/user.json"
        });

        let validator = crate::async_options()
            .with_retriever(TestRetriever::new())
            .build(&schema)
            .await
            .unwrap();

        // Valid instance
        assert!(validator.is_valid(&json!({
            "name": "John Doe",
            "age": 30
        })));

        // Invalid instances
        assert!(!validator.is_valid(&json!({
            "age": -5
        })));
        assert!(!validator.is_valid(&json!({
            "name": 123,
            "age": 30
        })));
    }

    #[tokio::test]
    async fn test_async_options_with_draft() {
        let schema = json!({
            "$ref": "https://example.com/user.json"
        });

        let validator = crate::async_options()
            .with_draft(Draft::Draft202012)
            .with_retriever(TestRetriever::new())
            .build(&schema)
            .await
            .unwrap();

        assert!(validator.is_valid(&json!({
            "name": "John Doe",
            "age": 30
        })));
    }

    #[tokio::test]
    async fn test_async_retrieval_failure() {
        let schema = json!({
            "$ref": "https://example.com/nonexistent.json"
        });

        let result = crate::async_options()
            .with_retriever(TestRetriever::new())
            .build(&schema)
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Schema not found"));
    }

    #[tokio::test]
    async fn test_async_nested_references() {
        let mut retriever = TestRetriever::new();
        retriever.schemas.insert(
            "https://example.com/nested.json".to_string(),
            json!({
                "type": "object",
                "properties": {
                    "user": { "$ref": "https://example.com/user.json" }
                }
            }),
        );

        let schema = json!({
            "$ref": "https://example.com/nested.json"
        });

        let validator = crate::async_options()
            .with_retriever(retriever)
            .build(&schema)
            .await
            .unwrap();

        // Valid nested structure
        assert!(validator.is_valid(&json!({
            "user": {
                "name": "John Doe",
                "age": 30
            }
        })));

        // Invalid nested structure
        assert!(!validator.is_valid(&json!({
            "user": {
                "age": -5
            }
        })));
    }

    #[tokio::test]
    async fn test_async_with_registry_uses_async_retriever_for_inline_only_refs() {
        let registry = crate::Registry::new().prepare().unwrap();
        let schema = json!({
            "$ref": "https://example.com/user.json"
        });

        let validator = crate::async_options()
            .with_registry(&registry)
            .with_retriever(TestRetriever::new())
            .build(&schema)
            .await
            .unwrap();

        assert!(validator.is_valid(&json!({
            "name": "John Doe",
            "age": 30
        })));
        assert!(!validator.is_valid(&json!({
            "age": -5
        })));
    }

    #[tokio::test]
    async fn test_async_validator_for_basic() {
        let schema = json!({"type": "integer"});

        let validator = crate::async_validator_for(&schema).await.unwrap();

        assert!(validator.is_valid(&json!(42)));
        assert!(!validator.is_valid(&json!("abc")));
    }

    #[tokio::test]
    async fn test_async_build_future_is_send() {
        let schema = Arc::new(json!({
            "$ref": "https://example.com/user.json"
        }));
        let retriever = TestRetriever::new();

        let handle = tokio::spawn({
            let schema = Arc::clone(&schema);
            let retriever = retriever.clone();
            async move {
                crate::async_options()
                    .with_retriever(retriever)
                    .build(&schema)
                    .await
            }
        });

        let validator = handle.await.unwrap().unwrap();
        assert!(validator.is_valid(&json!({
            "name": "John Doe",
            "age": 30
        })));
    }
}

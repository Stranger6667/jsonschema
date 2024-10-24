//! A high-performance JSON Schema validator for Rust.
//!
//! - 📚 Support for popular JSON Schema drafts
//! - 🔧 Custom keywords and format validators
//! - 🌐 Remote reference fetching (network/file)
//! - 🎨 `Basic` output style as per JSON Schema spec
//! - 🚀 WebAssembly support
//!
//! ## Supported drafts
//!
//! Compliance levels vary across drafts, with newer versions having some unimplemented keywords.
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
//! For better performance, especially when validating multiple instances against the same schema, build a validator once and reuse it:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use serde_json::json;
//!
//! let schema = json!({"type": "string"});
//! let validator = jsonschema::validator_for(&schema)?;
//!
//! assert!(validator.is_valid(&json!("Hello, world!")));
//! assert!(!validator.is_valid(&json!(42)));
//! assert!(validator.validate(&json!(42)).is_err());
//!
//! // Iterate over all errors
//! let instance = json!(42);
//! for error in validator.iter_errors(&instance) {
//!     eprintln!("Error: {}", error);
//!     eprintln!("Location: {}", error.instance_path);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Configuration
//!
//! `jsonschema` provides several ways to configure and use JSON Schema validation.
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
//!
//! Here's how you can explicitly use a specific draft version:
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use serde_json::json;
//!
//! let schema = json!({"type": "string"});
//! let validator = jsonschema::draft7::new(&schema)?;
//!
//! assert!(validator.is_valid(&json!("Hello")));
//! assert!(validator.validate(&json!("Hello")).is_ok());
//! # Ok(())
//! # }
//! ```
//!
//! You can also use the convenience [`is_valid`] and [`validate`] functions:
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
//! For configuration options that are not draft-specific, `jsonschema` provides a builder via `jsonschema::options()`.
//!
//! Here's an example:
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
//! For a complete list of configuration options and their usage, please refer to the [`ValidationOptions`] struct.
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
//!
//! To enable HTTPS support, add the `rustls-tls` feature to `reqwest` in your `Cargo.toml`:
//!
//! ```toml
//! reqwest = { version = "*", features = ["rustls-tls"] }
//! ```
//!
//! You can disable the default behavior using crate features:
//!
//! - Disable HTTP resolving: `default-features = false, features = ["resolve-file"]`
//! - Disable file resolving: `default-features = false, features = ["resolve-http"]`
//! - Disable both: `default-features = false`
//!
//! You can implement a custom retriever to handle external references. Here's an example that uses a static map of schemas:
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
//!        uri: &Uri<&str>,
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
//! # Output Styles
//!
//! `jsonschema` supports the `basic` output style as defined in JSON Schema Draft 2019-09.
//! This styles allow you to serialize validation results in a standardized format using `serde`.
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use serde_json::json;
//!
//! let schema_json = json!({
//!     "title": "string value",
//!     "type": "string"
//! });
//! let instance = json!("some string");
//! let validator = jsonschema::validator_for(&schema_json)?;
//!
//! let output = validator.apply(&instance).basic();
//!
//! assert_eq!(
//!     serde_json::to_value(output)?,
//!     json!({
//!         "valid": true,
//!         "annotations": [
//!             {
//!                 "keywordLocation": "",
//!                 "instanceLocation": "",
//!                 "annotations": {
//!                     "title": "string value"
//!                 }
//!             }
//!         ]
//!     })
//! );
//! #    Ok(())
//! # }
//! ```
//!
//! # Custom Keywords
//!
//! `jsonschema` allows you to extend its functionality by implementing custom validation logic through custom keywords.
//! This feature is particularly useful when you need to validate against domain-specific rules that aren't covered by the standard JSON Schema keywords.
//!
//! To implement a custom keyword, you need to:
//! 1. Create a struct that implements the [`Keyword`] trait
//! 2. Create a factory function or closure that produces instances of your custom keyword
//! 3. Register the custom keyword with the [`Validator`] instance using the [`ValidationOptions::with_keyword`] method
//!
//! Here's a complete example:
//!
//! ```rust
//! use jsonschema::{
//!     paths::{LazyLocation, Location},
//!     Keyword, ValidationError,
//! };
//! use serde_json::{json, Map, Value};
//! use std::iter::once;
//!
//! // Step 1: Implement the Keyword trait
//! struct EvenNumberValidator;
//!
//! impl Keyword for EvenNumberValidator {
//!     fn validate<'i>(
//!         &self,
//!         instance: &'i Value,
//!         location: &LazyLocation,
//!     ) -> Result<(), ValidationError<'i>> {
//!         if let Value::Number(n) = instance {
//!             if n.as_u64().map_or(false, |n| n % 2 == 0) {
//!                 Ok(())
//!             } else {
//!                 return Err(ValidationError::custom(
//!                     Location::new(),
//!                     location.into(),
//!                     instance,
//!                     "Number must be even",
//!                 ));
//!             }
//!         } else {
//!             Err(ValidationError::custom(
//!                 Location::new(),
//!                 location.into(),
//!                 instance,
//!                 "Value must be a number",
//!             ))
//!         }
//!     }
//!
//!     fn is_valid(&self, instance: &Value) -> bool {
//!         instance.as_u64().map_or(false, |n| n % 2 == 0)
//!     }
//! }
//!
//! // Step 2: Create a factory function
//! fn even_number_validator_factory<'a>(
//!     _parent: &'a Map<String, Value>,
//!     value: &'a Value,
//!     path: Location,
//! ) -> Result<Box<dyn Keyword>, ValidationError<'a>> {
//!     // You can use the `value` parameter to configure your validator if needed
//!     if value.as_bool() == Some(true) {
//!         Ok(Box::new(EvenNumberValidator))
//!     } else {
//!         Err(ValidationError::custom(
//!             Location::new(),
//!             path,
//!             value,
//!             "The 'even-number' keyword must be set to true",
//!         ))
//!     }
//! }
//!
//! // Step 3: Use the custom keyword
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let schema = json!({"even-number": true, "type": "integer"});
//!     let validator = jsonschema::options()
//!         .with_keyword("even-number", even_number_validator_factory)
//!         .build(&schema)?;
//!
//!     assert!(validator.is_valid(&json!(2)));
//!     assert!(!validator.is_valid(&json!(3)));
//!     assert!(!validator.is_valid(&json!("not a number")));
//!
//!     Ok(())
//! }
//! ```
//!
//! In this example, we've created a custom `even-number` keyword that validates whether a number is even.
//! The `EvenNumberValidator` implements the actual validation logic, while the `even_number_validator_factory`
//! creates instances of the validator and allows for additional configuration based on the keyword's value in the schema.
//!
//! You can also use a closure instead of a factory function for simpler cases:
//!
//! ```rust
//! # use jsonschema::{
//! #     paths::LazyLocation,
//! #     Keyword, ValidationError,
//! # };
//! # use serde_json::{json, Map, Value};
//! # use std::iter::once;
//! #
//! # struct EvenNumberValidator;
//! #
//! # impl Keyword for EvenNumberValidator {
//! #     fn validate<'i>(
//! #         &self,
//! #         instance: &'i Value,
//! #         location: &LazyLocation,
//! #     ) -> Result<(), ValidationError<'i>> {
//! #         Ok(())
//! #     }
//! #
//! #     fn is_valid(&self, instance: &Value) -> bool {
//! #         true
//! #     }
//! # }
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let schema = json!({"even-number": true, "type": "integer"});
//! let validator = jsonschema::options()
//!     .with_keyword("even-number", |_, _, _| {
//!         Ok(Box::new(EvenNumberValidator))
//!     })
//!     .build(&schema)?;
//! # Ok(())
//! # }
//! ```
//!
//! # Custom Formats
//!
//! JSON Schema allows for format validation through the `format` keyword. While `jsonschema`
//! provides built-in validators for standard formats, you can also define custom format validators
//! for domain-specific string formats.
//!
//! To implement a custom format validator:
//!
//! 1. Define a function or a closure that takes a `&str` and returns a `bool`.
//! 2. Register the function with `jsonschema::options().with_format()`.
//!
//! ```rust
//! use serde_json::json;
//!
//! // Step 1: Define the custom format validator function
//! fn ends_with_42(s: &str) -> bool {
//!     s.ends_with("42!")
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
//!     .with_format("ends-with-43", |s| s.ends_with("43!"))
//!     .should_validate_formats(true)
//!     .build(&schema)?;
//!
//! // Step 4: Validate instances
//! assert!(validator.is_valid(&json!("Hello42!")));
//! assert!(!validator.is_valid(&json!("Hello43!")));
//! assert!(!validator.is_valid(&json!(42))); // Not a string
//! #    Ok(())
//! # }
//! ```
//!
//! ### Notes on Custom Format Validators
//!
//! - Custom format validators are only called for string instances.
//! - Format validation can be disabled globally or per-draft using [`ValidationOptions`].
//!   Ensure format validation is enabled if you're using custom formats.
//!
//! # WebAssembly support
//!
//! When using `jsonschema` in WASM environments, be aware that external references are
//! not supported by default due to WASM limitations:
//!    - No filesystem access (`resolve-file` feature)
//!    - No direct HTTP requests, at least right now (`resolve-http` feature)
//!
//! To use `jsonschema` in WASM, disable default features:
//!
//! ```toml
//! jsonschema = { version = "x.y.z", default-features = false }
//! ```
//!
//! For external references in WASM you may want to implement a custom retriever.
//! See the [External References](#external-references) section for implementation details.

pub(crate) mod compiler;
mod content_encoding;
mod content_media_type;
mod ecma;
pub mod error;
mod keywords;
mod node;
mod options;
pub mod output;
pub mod paths;
pub mod primitive_type;
pub(crate) mod properties;
mod retriever;
mod validator;

pub use error::{ErrorIterator, ValidationError};
pub use keywords::custom::Keyword;
pub use options::ValidationOptions;
pub use output::BasicOutput;
pub use referencing::{Draft, Error as ReferencingError, Resource, Retrieve, Uri};
pub use validator::Validator;

use serde_json::Value;

#[cfg(all(
    target_arch = "wasm32",
    any(feature = "resolve-http", feature = "resolve-file")
))]
compile_error!("Features 'resolve-http' and 'resolve-file' are not supported on WASM.");

/// Validate `instance` against `schema` and get a `true` if the instance is valid and `false`
/// otherwise. Draft is detected automatically.
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
/// This function panics if an invalid schema is passed.
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
/// # Panics
///
/// This function panics if an invalid schema is passed.
#[inline]
pub fn validate<'i>(schema: &Value, instance: &'i Value) -> Result<(), ValidationError<'i>> {
    validator_for(schema)
        .expect("Invalid schema")
        .validate(instance)
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
pub fn validator_for(schema: &Value) -> Result<Validator, ValidationError<'static>> {
    Validator::new(schema)
}

/// Create a builder for configuring JSON Schema validation options.
///
/// This function returns a [`ValidationOptions`] struct, which allows you to set various
/// options for JSON Schema validation. You can use this builder to specify
/// the draft version, set custom formats, and more.
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
pub fn options() -> ValidationOptions {
    Validator::options()
}

/// Functionality specific to JSON Schema Draft 4.
///
/// [![Draft 4](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft4.json)](https://bowtie.report/#/implementations/rust-jsonschema)
///
/// This module provides functions for creating validators and performing validation
/// according to the JSON Schema Draft 4 specification.
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
    use super::*;

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
    pub fn new(schema: &Value) -> Result<Validator, ValidationError<'static>> {
        options().build(schema)
    }
    /// Validate an instance against a schema using Draft 4 specifications without creating a validator.
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
    #[must_use]
    pub fn is_valid(schema: &Value, instance: &Value) -> bool {
        new(schema).expect("Invalid schema").is_valid(instance)
    }
    /// Validate an instance against a schema using Draft 4 specifications without creating a validator.
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
    pub fn options() -> ValidationOptions {
        let mut options = crate::options();
        options.with_draft(Draft::Draft4);
        options
    }
}

/// Functionality specific to JSON Schema Draft 6.
///
/// [![Draft 6](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft6.json)](https://bowtie.report/#/implementations/rust-jsonschema)
///
/// This module provides functions for creating validators and performing validation
/// according to the JSON Schema Draft 6 specification.
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
    use super::*;

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
    pub fn new(schema: &Value) -> Result<Validator, ValidationError<'static>> {
        options().build(schema)
    }
    /// Validate an instance against a schema using Draft 6 specifications without creating a validator.
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
    #[must_use]
    pub fn is_valid(schema: &Value, instance: &Value) -> bool {
        new(schema).expect("Invalid schema").is_valid(instance)
    }
    /// Validate an instance against a schema using Draft 6 specifications without creating a validator.
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
    pub fn options() -> ValidationOptions {
        let mut options = crate::options();
        options.with_draft(Draft::Draft6);
        options
    }
}

/// Functionality specific to JSON Schema Draft 7.
///
/// [![Draft 7](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft7.json)](https://bowtie.report/#/implementations/rust-jsonschema)
///
/// This module provides functions for creating validators and performing validation
/// according to the JSON Schema Draft 7 specification.
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
    use super::*;

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
    pub fn new(schema: &Value) -> Result<Validator, ValidationError<'static>> {
        options().build(schema)
    }
    /// Validate an instance against a schema using Draft 7 specifications without creating a validator.
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
    #[must_use]
    pub fn is_valid(schema: &Value, instance: &Value) -> bool {
        new(schema).expect("Invalid schema").is_valid(instance)
    }
    /// Validate an instance against a schema using Draft 7 specifications without creating a validator.
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
    pub fn options() -> ValidationOptions {
        let mut options = crate::options();
        options.with_draft(Draft::Draft7);
        options
    }
}

/// Functionality specific to JSON Schema Draft 2019-09.
///
/// [![Draft 2019-09](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft2019-09.json)](https://bowtie.report/#/implementations/rust-jsonschema)
///
/// This module provides functions for creating validators and performing validation
/// according to the JSON Schema Draft 2019-09 specification.
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
    use super::*;

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
    pub fn new(schema: &Value) -> Result<Validator, ValidationError<'static>> {
        options().build(schema)
    }
    /// Validate an instance against a schema using Draft 2019-09 specifications without creating a validator.
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
    #[must_use]
    pub fn is_valid(schema: &Value, instance: &Value) -> bool {
        new(schema).expect("Invalid schema").is_valid(instance)
    }
    /// Validate an instance against a schema using Draft 2019-09 specifications without creating a validator.
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
    pub fn options() -> ValidationOptions {
        let mut options = crate::options();
        options.with_draft(Draft::Draft201909);
        options
    }
}

/// Functionality specific to JSON Schema Draft 2020-12.
///
/// [![Draft 2020-12](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft2020-12.json)](https://bowtie.report/#/implementations/rust-jsonschema)
///
/// This module provides functions for creating validators and performing validation
/// according to the JSON Schema Draft 2020-12 specification.
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
    use super::*;

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
    pub fn new(schema: &Value) -> Result<Validator, ValidationError<'static>> {
        options().build(schema)
    }
    /// Validate an instance against a schema using Draft 2020-12 specifications without creating a validator.
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
    #[must_use]
    pub fn is_valid(schema: &Value, instance: &Value) -> bool {
        new(schema).expect("Invalid schema").is_valid(instance)
    }
    /// Validate an instance against a schema using Draft 2020-12 specifications without creating a validator.
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
    pub fn options() -> ValidationOptions {
        let mut options = crate::options();
        options.with_draft(Draft::Draft202012);
        options
    }
}

#[cfg(test)]
pub(crate) mod tests_util {
    use super::Validator;
    use crate::ValidationError;
    use serde_json::Value;

    #[track_caller]
    pub(crate) fn is_not_valid_with(validator: &Validator, instance: &Value) {
        assert!(
            !validator.is_valid(instance),
            "{} should not be valid (via is_valid)",
            instance
        );
        assert!(
            validator.validate(instance).is_err(),
            "{} should not be valid (via validate)",
            instance
        );
        assert!(
            validator.iter_errors(instance).next().is_some(),
            "{} should not be valid (via validate)",
            instance
        );
        assert!(
            !validator.apply(instance).basic().is_valid(),
            "{} should not be valid (via apply)",
            instance
        );
    }

    #[track_caller]
    pub(crate) fn is_not_valid(schema: &Value, instance: &Value) {
        let validator = crate::options()
            .should_validate_formats(true)
            .build(schema)
            .expect("Invalid schema");
        is_not_valid_with(&validator, instance)
    }

    #[track_caller]
    pub(crate) fn is_not_valid_with_draft(draft: crate::Draft, schema: &Value, instance: &Value) {
        let validator = crate::options()
            .should_validate_formats(true)
            .with_draft(draft)
            .build(schema)
            .expect("Invalid schema");
        is_not_valid_with(&validator, instance)
    }

    pub(crate) fn expect_errors(schema: &Value, instance: &Value, errors: &[&str]) {
        assert_eq!(
            crate::validator_for(schema)
                .expect("Should be a valid schema")
                .iter_errors(instance)
                .map(|e| e.to_string())
                .collect::<Vec<String>>(),
            errors
        )
    }

    #[track_caller]
    pub(crate) fn is_valid_with(validator: &Validator, instance: &Value) {
        if let Some(first) = validator.iter_errors(instance).next() {
            panic!(
                "{} should be valid (via validate). Error: {} at {}",
                instance, first, first.instance_path
            );
        }
        assert!(
            validator.is_valid(instance),
            "{} should be valid (via is_valid)",
            instance
        );
        assert!(
            validator.validate(instance).is_ok(),
            "{} should be valid (via is_valid)",
            instance
        );
        assert!(
            validator.apply(instance).basic().is_valid(),
            "{} should be valid (via apply)",
            instance
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
    pub(crate) fn is_valid_with_draft(draft: crate::Draft, schema: &Value, instance: &Value) {
        let validator = crate::options().with_draft(draft).build(schema).unwrap();
        is_valid_with(&validator, instance)
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
            .into_owned();
        err
    }

    #[track_caller]
    pub(crate) fn assert_schema_location(schema: &Value, instance: &Value, expected: &str) {
        let error = validate(schema, instance);
        assert_eq!(error.schema_path.as_str(), expected)
    }

    #[track_caller]
    pub(crate) fn assert_locations(schema: &Value, instance: &Value, expected: &[&str]) {
        let validator = crate::validator_for(schema).unwrap();
        let errors = validator.iter_errors(instance);
        for (error, location) in errors.zip(expected) {
            assert_eq!(error.schema_path.as_str(), *location)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{validator_for, ValidationError};

    use super::Draft;
    use serde_json::json;
    use test_case::test_case;

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

    #[test_case(Draft::Draft4)]
    #[test_case(Draft::Draft6)]
    #[test_case(Draft::Draft7)]
    fn meta_schemas(draft: Draft) {
        // See GH-258
        for schema in [json!({"enum": [0, 0.0]}), json!({"enum": []})] {
            assert!(crate::options().with_draft(draft).build(&schema).is_ok())
        }
    }

    #[test]
    fn incomplete_escape_in_pattern() {
        // See GH-253
        let schema = json!({"pattern": "\\u"});
        assert!(crate::validator_for(&schema).is_err())
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
}

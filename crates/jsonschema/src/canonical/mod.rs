//! Schema canonicalization: reduce a JSON Schema to a normal form.
//!
//! Schemas accepting the same value set reduce to the *same* canonical schema.
//!
//! # Examples
//!
//! ```
//! use jsonschema::canonicalize;
//! use serde_json::json;
//!
//! let schema = canonicalize(&json!({"type": "integer", "minimum": 0})).unwrap();
//! assert_eq!(
//!     schema.to_json_schema(),
//!     json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "integer", "minimum": 0})
//! );
//! ```
//!
//! # Entry points
//!
//! - [`canonicalize`](crate::canonicalize) / [`options`](fn@options) - canonicalize a schema (`options` configures
//!   the draft and registry).
//! - [`CanonicalSchema`] - the result: emit with [`to_json_schema`](CanonicalSchema::to_json_schema), inspect with
//!   [`view`](CanonicalSchema::view).
//! - [`CanonicalView`] - a total, match-once view of one canonical node for structural inspection.

pub mod json;

pub(crate) mod algebra;
pub(crate) mod context;
pub(crate) mod emit;
pub(crate) mod error;
pub(crate) mod ir;
pub(crate) mod options;
pub(crate) mod parse;
pub(crate) mod schema;
pub(crate) mod view;

pub use error::CanonicalizationError;
pub use options::{options, CanonicalizeOptions};
pub use schema::CanonicalSchema;
pub use view::{CanonicalKind, CanonicalView};

pub(crate) use schema::DefinitionMap;

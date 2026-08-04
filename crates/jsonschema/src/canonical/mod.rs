//! Schema canonicalization: reduce a JSON Schema to a normal form.
//!
//! <div class="warning">
//!
//! Experimental: the API may change in minor releases. Schemas that cannot be represented exactly
//! are preserved verbatim as [`CanonicalKind::Raw`].
//!
//! </div>
//!
//! Canonicalization rewrites schemas to a normal form without changing the accepted value set.
//! Equivalent supported schemas reduce to the same form and contradictions reduce to `false`.
//!
//! # Examples
//!
//! ```
//! use jsonschema::{canonicalize, canonical::{CanonicalKind, CanonicalView}};
//! use serde_json::json;
//!
//! // Equivalent schemas share one canonical form.
//! let interval = canonicalize(&json!({"type": "integer", "minimum": 1, "maximum": 1})).unwrap();
//! let constant = canonicalize(&json!({"const": 1, "type": "integer"})).unwrap();
//! assert_eq!(interval.to_json_schema(), constant.to_json_schema());
//!
//! // `allOf` folds into a single constraint set.
//! let folded = canonicalize(&json!({
//!     "allOf": [{"type": "integer", "minimum": 0}, {"type": "integer", "maximum": 10}]
//! })).unwrap();
//! assert_eq!(
//!     folded.to_json_schema(),
//!     json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "integer", "minimum": 0, "maximum": 10})
//! );
//!
//! // Contradictions collapse to `false`; `is_satisfiable` reports it.
//! let empty = canonicalize(&json!({"type": "integer", "minimum": 10, "maximum": 5})).unwrap();
//! assert!(!empty.is_satisfiable());
//!
//! // Inspect the result with a single `match` over a `CanonicalView`.
//! let deduped = canonicalize(&json!({"enum": [2, 1, 2, 9]})).unwrap();
//! match deduped.view() {
//!     CanonicalView::Enum(values) => assert_eq!(values, vec![json!(1), json!(2), json!(9)]),
//!     other => panic!("expected an enum, got {other:?}"),
//! }
//!
//! // Unsupported constructs keep the whole document as an opaque `Raw` pass-through.
//! let raw = canonicalize(&json!({"if": {}, "unevaluatedProperties": false})).unwrap();
//! assert_eq!(raw.kind(), CanonicalKind::Raw);
//! ```
//!
//! # How it works
//!
//! Canonicalization parses a schema into an internal representation, normalizes that
//! representation, then emits JSON Schema. Annotations that do not affect validation disappear.
//! The selected draft, format policy, and regular-expression configuration are part of the result's
//! semantics.
//!
//! # Unsupported schemas
//!
//! When exact normalization is unavailable, canonicalization succeeds with
//! [`CanonicalKind::Raw`] and preserves the original document unchanged. Unresolved references
//! remain errors. A reference whose target uses a different draft than the referring document is
//! also not yet modeled and falls back to `Raw` (future work).
//!
//! # Recursive schemas
//!
//! A schema demanding an infinite descent, such as `{"type": "object", "required": ["a"],
//! "properties": {"a": {"$ref": "#"}}}`, canonicalizes to `false`: every cycle passes a keyword
//! that consumes structure, so no finite value can satisfy it.
//!
//! A cycle closed entirely through in-place applicators (`allOf`, `anyOf`, `oneOf`, `not`) consumes
//! nothing, so no descent is forced. Carrying no assertion anywhere on it, such a cycle leaves
//! nothing for a value to violate and canonicalizes to `true`; otherwise it is left untouched.
//!
//! # Entry points
//!
//! - [`canonicalize`](crate::canonicalize) canonicalizes with defaults.
//! - [`options`](fn@options) configures canonicalization.
//! - [`CanonicalSchema`] emits, inspects, and checks the result.

#![deny(clippy::wildcard_enum_match_arm)]

pub mod json;

pub(crate) mod algebra;
pub(crate) mod context;
pub(crate) mod emit;
pub(crate) mod emptiness;
pub(crate) mod error;
pub(crate) mod ir;
pub(crate) mod negate;
pub(crate) mod options;
pub(crate) mod oracle;
pub(crate) mod parse;
pub(crate) mod schema;
pub(crate) mod view;

pub use error::{CanonicalizationError, OperandMismatch};
pub use options::{options, CanonicalizeOptions};
pub use schema::CanonicalSchema;
pub use view::{CanonicalKind, CanonicalView, ContainsView};

pub(crate) const CANONICAL_REFERENCE_PREFIX: &str = "urn:jsonschema:canonical:";

/// Names the document root in the definition key space. No minted key can spell it: a reference
/// resolving to the root short-circuits before a key is derived, and every derived key is either a
/// `#/$defs/`-style pointer or carries [`CANONICAL_REFERENCE_PREFIX`].
pub(crate) const ROOT_DEFINITION_KEY: &str = "#";

pub(crate) use schema::DefinitionMap;

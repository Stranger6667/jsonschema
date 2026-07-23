//! Schema canonicalization: reduce a JSON Schema to a normal form.
//!
//! <div class="warning">
//!
//! Experimental: keyword coverage is incomplete and the API may change in minor releases. A schema
//! using any unsupported construct canonicalizes to an opaque pass-through of the whole document;
//! see [Coverage](#coverage).
//!
//! </div>
//!
//! Schemas accepting the same value set reduce to the *same* canonical schema, unsatisfiable ones to
//! `false`. Canonicalization is **sound**: the canonical form accepts a value iff the original does -
//! it rewrites shape, never the accepted set. Equivalence becomes structural equality and
//! satisfiability a single check.
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
//! let raw = canonicalize(&json!({"properties": {"name": {"type": "string"}}})).unwrap();
//! assert_eq!(raw.kind(), CanonicalKind::Raw);
//! ```
//!
//! # How it works
//!
//! A schema is treated as the set of JSON values it accepts. Canonicalization picks one
//! representative schema per value set: it parses the document into an internal representation,
//! rewrites every equivalent spelling to that representative, and emits it back as JSON Schema.
//! Rewriting is driven by the value set alone, so two schemas accepting the same values come out
//! structurally identical no matter how differently they were written, and constraints admitting
//! no value at all reduce to the `false` schema, which
//! [`is_satisfiable`](CanonicalSchema::is_satisfiable) reports.
//!
//! Only constructs that constrain the accepted value set survive. Annotations such as `title` or
//! `description`, and keywords the draft does not define, leave no trace in the canonical form.
//! `format` follows the validator's draft policy: Draft 4/6/7 assert known formats and keep them,
//! while Draft 2019-09/2020-12 treat them as annotations and drop them;
//! [`CanonicalizeOptions::should_validate_formats`] overrides the default.
//!
//! # Coverage
//!
//! Canonicalization models a growing subset of JSON Schema - currently the type system,
//! `const`/`enum` value sets, the `allOf`/`anyOf` combinators, and numeric and string constraints.
//! Rather than relying on any keyword list, detect support per document: a schema using anything
//! outside the modeled subset (references, object or array keywords, ...) canonicalizes
//! *successfully* to an opaque pass-through of the whole document, [`CanonicalView::Raw`]. A `Raw`
//! schema is the original verbatim - equivalent but inert: nothing folds, and
//! [`is_satisfiable`](CanonicalSchema::is_satisfiable) stays conservatively `true`. Match on
//! [`CanonicalKind::Raw`](CanonicalKind) to tell the two outcomes apart.
//!
//! # Entry points
//!
//! - [`canonicalize`](crate::canonicalize) / [`options`](fn@options) - canonicalize a schema (`options` configures
//!   the draft, registry, format assertions, and pattern engine).
//! - [`CanonicalSchema`] - the result: emit with [`to_json_schema`](CanonicalSchema::to_json_schema), inspect with
//!   [`view`](CanonicalSchema::view), check [`is_satisfiable`](CanonicalSchema::is_satisfiable).
//! - [`CanonicalView`] - a total, match-once view of one canonical node for structural inspection.

#![deny(clippy::wildcard_enum_match_arm)]

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

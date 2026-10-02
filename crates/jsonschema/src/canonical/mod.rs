//! Schema canonicalization: reduce a JSON Schema to a normal form.
//!
//! <div class="warning">
//!
//! Experimental: the API may change in minor releases. Schemas that cannot be represented exactly
//! are preserved verbatim as [`CanonicalKind::Raw`].
//!
//! </div>
//!
//! Canonicalization rewrites a schema to a normal form that accepts the same values. Two schemas
//! accepting the same values reduce to the same form, and a schema proven to accept nothing
//! reduces to `false`. A schema the canonical form cannot model comes back unchanged as
//! [`CanonicalKind::Raw`]; see [Unsupported schemas](#unsupported-schemas).
//!
//! # Examples
//!
//! ```
//! use jsonschema::{canonicalize, canonical::{CanonicalKind, CanonicalView, Satisfiability}};
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
//! // Contradictions collapse to `false`; `satisfiability` reports it.
//! let empty = canonicalize(&json!({"type": "integer", "minimum": 10, "maximum": 5})).unwrap();
//! assert_eq!(empty.satisfiability(), Satisfiability::No);
//!
//! // Inspect the result with a single `match` over a `CanonicalView`.
//! let deduped = canonicalize(&json!({"enum": [2, 1, 2, 9]})).unwrap();
//! match deduped.view() {
//!     CanonicalView::Enum(values) => assert_eq!(values, vec![json!(1), json!(2), json!(9)]),
//!     other => panic!("expected an enum, got {other:?}"),
//! }
//!
//! // Unsupported constructs keep the whole document as an opaque `Raw` pass-through.
//! let raw = canonicalize(&json!({"dependencies": {}, "unevaluatedProperties": false})).unwrap();
//! assert_eq!(raw.kind(), CanonicalKind::Raw);
//! ```
//!
//! # How it works
//!
//! Canonicalization parses a schema into an internal representation, normalizes it, then emits
//! JSON Schema. Annotations such as `title` and `description` are dropped. The draft, whether
//! `format` asserts, and the regular-expression engine all change what a schema accepts, so each
//! result carries them, and set operations refuse operands that differ in any of them.
//!
//! # Comparing two versions of a schema
//!
//! [`subtract`](CanonicalSchema::subtract) tells you what an edit did to a schema.
//! `old.subtract(&new)` accepts exactly the values `old` accepts and `new` rejects, so it accepts
//! nothing when the edit lost nothing.
//!
//! ```
//! use jsonschema::{canonicalize, canonical::Satisfiability};
//! use serde_json::json;
//!
//! let old = canonicalize(&json!({"type": "string"}))?;
//! let new = canonicalize(&json!({"type": "string", "maxLength": 50}))?;
//!
//! // What `new` stopped accepting, as a schema: the strings longer than 50.
//! assert_eq!(
//!     old.subtract(&new)?.to_json_schema(),
//!     json!({
//!         "$schema": "https://json-schema.org/draft/2020-12/schema",
//!         "type": "string",
//!         "minLength": 51
//!     })
//! );
//! // Nothing is accepted that was not accepted before, so the change only narrows.
//! assert_eq!(new.subtract(&old)?.satisfiability(), Satisfiability::No);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! The direction to check depends on who sends the value:
//!
//! - For a request schema, check `old.subtract(&new)`. It accepts the payloads existing callers
//!   send that the new schema rejects.
//! - For a response schema, check `new.subtract(&old)`. It accepts the values the new schema lets
//!   a server return that callers never agreed to read.
//!
//! [`Satisfiability::No`] on the difference proves the edit safe in that direction.
//! [`Satisfiability::Unknown`] proves nothing either way. When the canonical form cannot express
//! the difference exactly, `subtract` returns [`CanonicalizationError::UnsupportedResult`].
//!
//! ## What the operations do with `$defs`
//!
//! A `$defs` key is private to the document that declares it, so two versions of a schema may give
//! one key different bodies, as they do whenever you edit a shared component. The operations merge
//! both maps into one and rename the clashing keys of one side, so a changed `$defs` body never
//! causes an error. The generated names show up in the result:
//!
//! ```text
//! old: {"$defs": {"User": {"type": "object"}}, "$ref": "#/$defs/User"}
//! new: {"$defs": {"User": {"type": "object", "minProperties": 1}}, "$ref": "#/$defs/User"}
//!      old.subtract(&new)  =>  {"const": {}}   (the empty object, which `new` no longer takes)
//! ```
//!
//! Operands that differ in draft, `format` assertion or regular-expression engine return
//! [`CanonicalizationError::IncompatibleOperands`], and so do two conflicts between documents:
//!
//! * The operands resolve **one external resource** to different schemas
//!   ([`OperandMismatch::Definitions`]). A URI names the same resource in every document, so the
//!   two registries disagree about it. Canonicalize both operands against one registry.
//! * Both operands read the **document root** `#`, and it names a different schema on each side
//!   ([`OperandMismatch::DocumentRoots`]). `#` means "this document" and has no key to rename.
//!   Write the recursion through a `$defs` entry, `{"$defs": {"Node": ...}, "$ref": "#/$defs/Node"}`,
//!   and the clash becomes a renamed key instead:
//!
//! ```text
//! old: {"type": "object", "properties": {"next": {"$ref": "#"}}}
//! new: {"type": "object", "properties": {"next": {"$ref": "#"}}, "minProperties": 1}
//!      => Err(IncompatibleOperands(DocumentRoots))
//! ```
//!
//! A result keeps the root its operands had, so chained calls such as `a.union(&b)?.covers(&a)`
//! work.
//!
//! # Unsupported schemas
//!
//! A schema the canonical form cannot model exactly still canonicalizes: the result has kind
//! [`CanonicalKind::Raw`] and holds the original document unchanged. [`CanonicalView::Raw`]
//! carries a [`RawReason`] naming what stopped the run and, when a single subschema is at fault,
//! its pointer. A reference into a document of another draft also yields `Raw`. A reference that
//! cannot be resolved is an error, [`CanonicalizationError::ReferenceResolution`].
//!
//! # Recursive schemas
//!
//! `{"type": "object", "required": ["a"], "properties": {"a": {"$ref": "#"}}}` canonicalizes to
//! `false`. Its cycle passes through `properties`, which steps into a child value, and `required`
//! makes every step mandatory, so only an infinitely nested value could satisfy it.
//!
//! A cycle made only of `$ref` and the applicators that stay on the same value (`allOf`, `anyOf`,
//! `oneOf`, `not`) never steps into a child. When no keyword on such a cycle asserts anything, as in
//! `{"$ref": "#"}`, nothing on it can reject a value and it canonicalizes to `true`. A cycle with
//! an assertion on it keeps its references.
//!
//! # Entry points
//!
//! - [`canonicalize`](crate::canonicalize) canonicalizes with defaults.
//! - [`options`](fn@options) configures canonicalization.
//! - [`CanonicalSchema`] emits, inspects, and checks the result. Its
//!   [`intersect`](CanonicalSchema::intersect), [`union`](CanonicalSchema::union),
//!   [`subtract`](CanonicalSchema::subtract), and [`negate`](CanonicalSchema::negate) combine two
//!   results as sets of values. `a.covers(&b)` asks whether `a` accepts every value `b` accepts,
//!   and [`satisfiability`](CanonicalSchema::satisfiability) asks whether a schema accepts any.
//!
//! # Reading the two questions
//!
//! `Unknown` means the canonicalizer could not decide. Read it as the answer that keeps you safe:
//!
//! - [`satisfiability`](CanonicalSchema::satisfiability) - test for `No`, treat `Unknown` like `Yes`.
//! - [`covers`](CanonicalSchema::covers) - test for `Yes`, treat `Unknown` like `No`.
//!
//! Neither method raises on `Unknown`, so a caller that reads it the other way acts on a conclusion
//! nobody proved.

#![deny(clippy::wildcard_enum_match_arm)]

pub mod json;

pub(crate) mod algebra;
mod candidates;
pub(crate) mod containment;
pub(crate) mod context;
pub(crate) mod emit;
pub(crate) mod emptiness;
pub(crate) mod error;
pub(crate) mod ir;
pub(crate) mod negate;
pub(crate) mod options;
pub(crate) mod parse;
pub(crate) mod refold;
pub(crate) mod rename;
pub(crate) mod schema;
pub(crate) mod view;

pub use error::{CanonicalizationError, OperandMismatch};
pub use options::{options, CanonicalizeOptions, Cause, PreparedDocument, UnsatisfiableReason};
pub use schema::{CanonicalSchema, Containment, Satisfiability};
pub use view::{
    ArrayView, CanonicalKind, CanonicalView, ContainsView, Distinctness, IntegerView, NumberView,
    ObjectView, ObjectViolationView, RawReason, RawView, StringView, TypedGroupView,
};

pub(crate) const CANONICAL_REFERENCE_PREFIX: &str = "urn:jsonschema:canonical:";

/// Names the document root in the definition key space. No generated key can collide with it: a
/// reference resolving to the root short-circuits before a key is derived, and every derived key
/// is either a `#/$defs/`-style pointer or carries [`CANONICAL_REFERENCE_PREFIX`].
pub(crate) const ROOT_DEFINITION_KEY: &str = "#";

pub(crate) use schema::DefinitionMap;

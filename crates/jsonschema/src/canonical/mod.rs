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
//! # Comparing two versions of a schema
//!
//! A canonical schema *is* the set of values it accepts, so the set operations answer what editing
//! a schema did to that set. [`subtract`](CanonicalSchema::subtract) is the one to reach for: the
//! difference accepts exactly the values the old schema took and the new one turns away, and it is
//! empty exactly when nothing was lost.
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
//! Compare a request schema new-against-old and a response schema old-against-new: narrowing a
//! request turns away payloads a caller used to send, widening a response returns values a caller
//! never agreed to read.
//!
//! [`Satisfiability::No`] on the difference proves nothing was lost. [`Satisfiability::Unknown`]
//! proves nothing either way. Where the canonical form cannot express the difference exactly, it
//! declines with [`CanonicalizationError::UnsupportedResult`] rather than guess.
//!
//! ## What the operations do with `$defs`
//!
//! A `$defs` key is a name private to the document that declares it, so two versions may give the
//! same key different bodies - editing a shared component is the commonest change there is. The
//! operations resolve both operands through one map, and rename the clashing keys of one side
//! apart to build it. Nothing is refused, and the generated names show up in the result:
//!
//! ```text
//! old: {"$defs": {"User": {"type": "object"}}, "$ref": "#/$defs/User"}
//! new: {"$defs": {"User": {"type": "object", "minProperties": 1}}, "$ref": "#/$defs/User"}
//!      old.subtract(&new)  =>  {"const": {}}   (the empty object, which `new` no longer takes)
//! ```
//!
//! Two cases still report [`CanonicalizationError::IncompatibleOperands`] rather than answering:
//!
//! * The operands resolve **one external resource** to different schemas
//!   ([`OperandMismatch::Definitions`]). A resource is named by its URI rather than by a private
//!   key, so two documents disagreeing about one disagree about the resource itself - which is a
//!   difference between their registries, not between the schemas being compared.
//! * Both operands read the **document root** `#`, and it points at different schemas
//!   ([`OperandMismatch::DocumentRoots`]). `#` means "this document", so it cannot be renamed the
//!   way a private key can, and two separately canonicalized recursive schemas bind it to two
//!   different documents:
//!
//! ```text
//! old: {"type": "object", "properties": {"next": {"$ref": "#"}}}
//! new: {"type": "object", "properties": {"next": {"$ref": "#"}}, "minProperties": 1}
//!      => Err(IncompatibleOperands(DocumentRoots))
//! ```
//!
//! Chaining is unaffected: a result keeps the root its operands had, so `a.union(&b)?.covers(&a)`
//! works.
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
//! - [`CanonicalSchema`] emits, inspects, and checks the result. Its
//!   [`intersect`](CanonicalSchema::intersect), [`union`](CanonicalSchema::union),
//!   [`subtract`](CanonicalSchema::subtract), and [`negate`](CanonicalSchema::negate) combine two
//!   results as sets of values, while [`covers`](CanonicalSchema::covers) and
//!   [`satisfiability`](CanonicalSchema::satisfiability) ask about them.
//!
//! # Reading the two questions
//!
//! `Unknown` means undecided, not negative, and the two have opposite safe readings:
//!
//! - [`satisfiability`](CanonicalSchema::satisfiability) - test for `No`, treat `Unknown` like `Yes`.
//! - [`covers`](CanonicalSchema::covers) - test for `Yes`, treat `Unknown` like `No`.
//!
//! Reading either the other way around is silent: nothing raises, and the caller acts on a
//! conclusion the canonical form never reached.

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
pub(crate) mod refold;
pub(crate) mod rename;
pub(crate) mod schema;
pub(crate) mod view;
mod witness;

pub use error::{CanonicalizationError, OperandMismatch};
pub use options::{options, CanonicalizeOptions};
pub use schema::{CanonicalSchema, Containment, Satisfiability};
pub use view::{CanonicalKind, CanonicalView, ContainsView, Distinctness, ObjectViolationView};

pub(crate) const CANONICAL_REFERENCE_PREFIX: &str = "urn:jsonschema:canonical:";

/// Names the document root in the definition key space. No minted key can collide with it: a
/// reference resolving to the root short-circuits before a key is derived, and every derived key
/// is either a `#/$defs/`-style pointer or carries [`CANONICAL_REFERENCE_PREFIX`].
pub(crate) const ROOT_DEFINITION_KEY: &str = "#";

pub(crate) use schema::DefinitionMap;

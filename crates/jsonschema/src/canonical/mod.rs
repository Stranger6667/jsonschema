//! Schema canonicalization: reduce a JSON Schema to a normal form.
//!
//! Schemas accepting the same value set reduce to the *same* canonical schema, unsatisfiable ones to `false`. It is
//! **sound**: the canonical form accepts a value iff the original does - it rewrites shape, never the accepted set.
//!
//! So equivalence becomes structural equality, satisfiability a single check, and schemas gain an algebra
//! ([`intersect`](CanonicalSchema::intersect), [`union`](CanonicalSchema::union),
//! [`negate`](CanonicalSchema::negate), [`subtract`](CanonicalSchema::subtract)).
//!
//! # Examples
//!
//! ```
//! use jsonschema::{canonicalize, canonical::CanonicalView};
//! use serde_json::json;
//!
//! // Equivalent schemas share one canonical form.
//! let closed_interval = canonicalize(&json!({"type": "integer", "minimum": 1, "maximum": 1})).unwrap();
//! let constant = canonicalize(&json!({"const": 1, "type": "integer"})).unwrap();
//! assert_eq!(closed_interval.to_json_schema(), constant.to_json_schema());
//!
//! // Contradictions collapse; `is_satisfiable` reports it.
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
//! // Schemas compose: the intersection validates iff both operands do, the union iff either does.
//! let positive = canonicalize(&json!({"type": "integer", "minimum": 0})).unwrap();
//! let small = canonicalize(&json!({"type": "integer", "maximum": 10})).unwrap();
//! assert!(positive.intersect(&small).is_satisfiable());
//! let integer = canonicalize(&json!({"type": "integer"})).unwrap();
//! let string = canonicalize(&json!({"type": "string"})).unwrap();
//! assert_eq!(
//!     integer.union(&string).to_json_schema(),
//!     canonicalize(&json!({"type": ["integer", "string"]})).unwrap().to_json_schema(),
//! );
//! ```
//!
//! # How it works
//!
//! [`canonicalize`](crate::canonicalize) parses the schema into an internal representation, then rewrites it to a
//! fixpoint by repeating one pass of ordered stages. The stages fall into two groups:
//!
//! - **canonicalization** - structural rewrites that pick one representative per equivalence class: fold value sets,
//!   desugar `if`/`then`/`else`, eliminate `not`, partition `allOf` by type.
//! - **normalization** - tighten leaves and detect emptiness: snap numeric bounds, cap array lengths, close object
//!   requirements, map contradictions to `false`.
//!
//! Two operations back the stages: intersection computes `A and B`, negation the complement. Both feed their result
//! back through the pipeline, so the canonical-form invariant always holds.
//!
//! Format handling follows the validator's draft policy: Draft 4/6/7 assert known formats; Draft 2019-09/2020-12 treat
//! them as annotations unless [`CanonicalizeOptions::should_validate_formats`] is set. With assertions on, incompatible
//! formats (e.g. `date`/`uuid`) collapse to `false`; otherwise both are preserved. Emit preserves the assertion policy
//! under draft defaults where expressible. Combining schemas from different drafts emits under the newer draft, keeps
//! the receiver's pattern settings, and never reinterprets annotation-only formats as assertions.
//!
//! # Glossary
//!
//! The model that ties the module together, then the vocabulary as roles within it.
//!
//! ## The model
//!
//! - **Value-set semantics** - a schema *is* the set of JSON values it accepts; canonicalization picks one
//!   representative per set, so equivalent schemas become equal and the empty set becomes `false`.
//! - **Schema algebra** - set operations on those value-sets: `intersect` = intersection, `union` = union,
//!   `negate` = complement, `subtract` = difference, `is_subschema_of` = subset. `true`/`false` are full/empty.
//! - **Sound but incomplete** - the rewriting ops (`canonicalize`, `intersect`, `union`, `negate`) are *exact*
//!   (preserve the accepted set). The decision queries ([`is_satisfiable`](CanonicalSchema::is_satisfiable),
//!   [`is_subschema_of`](CanonicalSchema::is_subschema_of)) are *conservative*: a positive verdict is a guarantee,
//!   anything else is "unknown" - a first-class outcome, since regex relations and recursion are undecidable.
//!
//! ## Building blocks
//!
//! - **Leaf** - the per-type atom carrying that type's constraints (integer, number, string, array, object, plus the
//!   bare `null`/`boolean`/`const`/`enum` forms). All reasoning bottoms out in leaf-versus-leaf comparisons.
//! - **Facet** - one constraint on a leaf, mirroring a keyword: `bounds`, `multiple_of`, `pattern`, `length`, ...
//! - **Structure** - the logical layer over leaves: `AllOf` (and), `AnyOf` (or), `OneOf` (exactly one), `Not`, and
//!   `IfThenElse` (a guarded union, kept as a node only when its condition is opaque - else it desugars).
//! - **`MultiType`** - a pure type union (`type: [...]`) with no other constraint.
//! - **Typed leaf vs guard** - a leaf may *pin* a type (`TypedGroup`: "is a number *and* B") or only *imply* one
//!   (`TypeGuard`: "*if* a number then B; other types pass") - how a kind-specific keyword used without `type` behaves.
//! - **Negation-closure facets** (`not_multiple_of`, `not_patterns`, `repeated_items`) - let a leaf's complement stay
//!   one leaf instead of escaping to `Not`; e.g. `not_multiple_of: [1]` on a number means "non-integer".
//! - **Opaque nodes** (`Reference`, `Recursive`, `DynamicRef`, `Raw`) - semantics the set-model cannot express;
//!   carried verbatim and excluded from the algebra.
//!
//! ## Object scope
//!
//! - **Constraint (universal) vs Requirement (existential)** - *every* property matching a matcher satisfies `S`
//!   (`properties`, `additionalProperties`) versus *at least one* does (`required`, dependents, count bounds).
//! - **Matcher** - which keys a clause governs: a name, a pattern, or the additional-name set. A requirement matcher
//!   is absolute; a constraint catch-all is exemption-scoped - mixing up this scope is the classic unsoundness source.
//!
//! ## Reasoning
//!
//! - **Three oracles** - every decision reduces to three sound questions: *membership* (`admits`), *containment*
//!   (`covers`), *emptiness*. Each answers exactly when it can and "unknown" otherwise.
//! - **Residual** - what `subtract` leaves behind. `A` is contained in `B` iff `A` minus `B` is empty, so containment
//!   is proven by the residual collapsing to `false`; a non-single-leaf residual is kept as an `AllOf`.
//! - **Verdict** - the oracle answer: `Proven` (a guarantee) or `Unknown` (decides nothing), propagated
//!   conservatively. No `Refuted` - sound non-containment is the `is_subschema_of` pipeline's judgment, not a leaf one.
//! - **Prover** - decides containment across recursion: a recursive `$ref` is handled coinductively, assuming the
//!   goal while proving it (greatest-fixpoint), so cyclic schemas terminate.
//!
//! Implementation note: nodes are interned and `Arc`-shared with a kind-mask plus structural hash so passes skip
//! clean subtrees cheaply, and the prover caps its search with a fuel budget - exhaustion yields `Unknown`.
//!
//! # Entry points
//!
//! - [`canonicalize`](crate::canonicalize) / [`options`](fn@options) - run the pipeline (`options` configures draft,
//!   registry, and retrievers).
//! - [`CanonicalSchema`] - the result: emit with [`to_json_schema`](CanonicalSchema::to_json_schema), inspect with
//!   [`view`](CanonicalSchema::view), or combine via the schema algebra.
//! - [`CanonicalView`] - a total, match-once view of one canonical node for structural inspection.
//!
//! # Limitations
//!
//! - Inputs the IR cannot model exactly - unknown `$schema` dialects, lossy `unevaluated*` interactions, numerics
//!   outside the exactly-representable range, unretrievable references - canonicalize *successfully* to an opaque
//!   [`CanonicalView::Raw`] leaf: the original document verbatim, equivalent but inert (no folding; decision queries
//!   stay conservative). Match on [`CanonicalKind::Raw`](CanonicalKind) to detect this.
//! - `$dynamicRef`/`$recursiveRef` resolve against the runtime dynamic scope the structural form does not model, so
//!   each becomes an opaque [`CanonicalView::DynamicRef`] leaf (like `Reference`/`Recursive`). Static `$ref` recursion
//!   is modeled and round-trips through [`definitions`](CanonicalSchema::definitions).
//! - [`negate`](CanonicalSchema::negate) / [`subtract`](CanonicalSchema::subtract) are total, but over an opaque node
//!   they wrap it in `not` rather than pushing inward: exact but not constructive, so get witnesses by validating
//!   against the original.
//! - [`is_subschema_of`](CanonicalSchema::is_subschema_of) returns `None` when containment cannot be decided
//!   structurally; `None` is "unknown", not "not a subschema".

use std::sync::Arc;

use ahash::AHashMap;
use referencing::{Draft, Resolver};
use serde_json::Value;

pub mod json;

pub(crate) mod algebra;
pub(crate) mod context;
pub(crate) mod definitions;
pub(crate) mod document;
pub(crate) mod emit;
pub(crate) mod error;
pub(crate) mod ir;
pub(crate) mod leaves;
pub(crate) mod options;
pub(crate) mod oracle;
pub(crate) mod parse;
pub(crate) mod recursion;
pub(crate) mod rewrite;
pub(crate) mod schema;
pub(crate) mod view;

#[cfg(test)]
pub(crate) mod tests_util;

pub use error::CanonicalizationError;
#[cfg(feature = "resolve-async")]
pub use options::{async_options, AsyncCanonicalizeOptions};
pub use options::{options, CanonicalizeOptions};
pub use schema::CanonicalSchema;
pub use view::{
    ArrayView, BooleanVariant, BooleanView, CanonicalKind, CanonicalView, ContainsView,
    ContentFacetView, IfThenElseView, NumericView, ObjectConstraintView, ObjectRequirementView,
    ObjectView, StringView, TypedGroupView,
};

pub(crate) use algebra::{intersect, negate};
pub(crate) use definitions::{
    collect_all_symbolic_refs, definition_entry, reachable_definitions, DefinitionMap,
};
pub(crate) use ir::{cardinality, intern};
pub(crate) use leaves::numeric;
pub(crate) use oracle::{coverage, membership, prover};
pub(crate) use rewrite::{canonicalize_ir, structural::const_enum, walk};

use crate::canonical::{context::CanonicalizationContext, document::raw_schema, ir::SharedSchema};

pub(crate) fn canonicalize_with_resolver(
    value: &Value,
    draft: Draft,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'_>,
    inline_budget: usize,
) -> Result<CanonicalSchema, CanonicalizationError> {
    // `parse` inlines resolvable acyclic refs within budget, leaves cyclic/over-budget ones symbolic, and returns
    // their definition bodies alongside the root.
    let parsed = match parse::parse_graph(
        value,
        draft,
        Some(resolver.clone()),
        inline_budget,
        ctx.pattern_options(),
    ) {
        Ok(parsed) => parsed,
        Err(
            error @ (CanonicalizationError::InvalidPattern { .. }
            | CanonicalizationError::UnguardedRecursion(_)
            | CanonicalizationError::InfiniteRecursion(_)
            | CanonicalizationError::ValidationError(_)),
        ) => return Err(error),
        Err(_) => {
            return Ok(raw_schema(
                value,
                draft,
                ctx.pattern_options(),
                ctx.validates_formats(),
            ));
        }
    };
    let canonical = canonicalize_ir(&parsed.root, ctx);
    // `$dynamicRef`/`$recursiveRef` resolve against the runtime dynamic scope the structural IR can't model or emit, so
    // a schema carrying one is preserved raw (surfaces as `RawView`); static `Recursive` round-trips via definitions.
    if !canonical.mask.is_disjoint(ir::CanonicalKind::DynamicRef) {
        return Ok(raw_schema(
            value,
            draft,
            ctx.pattern_options(),
            ctx.validates_formats(),
        ));
    }
    // Canonicalization can narrow a `Reference`/`Recursive` leaf out of the tree (e.g. an `allOf` branch collapses),
    // orphaning its definition. Prune unreachable entries to keep the definitions invariant and stay idempotent.
    let definitions = Arc::new(canonical_definitions(&parsed.definitions, ctx));
    let definitions = reachable_definitions(&canonical, &definitions);
    Ok(CanonicalSchema::with_definitions(
        canonical,
        draft,
        ctx.pattern_options(),
        ctx.validates_formats(),
        definitions,
    ))
}

/// Canonicalize each parsed definition body into the shared definitions map.
fn canonical_definitions(
    parsed: &AHashMap<Arc<str>, SharedSchema>,
    ctx: &CanonicalizationContext,
) -> DefinitionMap {
    parsed
        .iter()
        .map(|(uri, body)| (Arc::clone(uri), canonicalize_ir(body, ctx)))
        .collect()
}

#[cfg(test)]
mod tests;

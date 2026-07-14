//! Not-elimination via truth-table identities: `not not x -> x`, `not True -> False`, `not False -> True`.
//!
//! Full per-keyword push-down lives in [`crate::canonical::negate`] and is excluded from this pipeline on purpose:
//! downstream contradiction detectors recognize unsatisfiable conjunctions only while `Not(inner)` is intact.

use std::sync::Arc;

use crate::canonical::{
    context::{CanonicalizationContext, WalkStage},
    intern::shared,
    intersect::multi_type_or_false,
    ir::{CanonicalKind, Schema, SchemaKindSet, SharedSchema},
    negate::negate_type_guard,
    walk::map_children,
};

/// Eliminates `not` via truth-table identities and, when the inner schema is type-set-equivalent, the type-set
/// complement. Deeper per-keyword negation is deferred to `negate` so downstream contradiction detectors still see an intact `not`.
///
/// ```text
/// BEFORE: {"not": {"not": {"type": "string"}}}
/// AFTER:  {"type": "string"}
///
/// BEFORE: {"not": {"type": "boolean"}}
/// AFTER:  {"type": ["null", "number", "string", "array", "object"]}
/// ```
#[must_use]
pub(crate) fn canonicalize(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
    super::run::<NotElimStage>(schema, ctx)
}

struct NotElimStage;

impl super::StructuralStage for NotElimStage {
    const WALK: WalkStage = WalkStage::NotElim;
    const MASK: SchemaKindSet = SchemaKindSet::of(CanonicalKind::Not);

    fn rewrite(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
        let Schema::Not(inner) = schema.as_schema() else {
            return map_children(schema, |child| canonicalize(child, ctx));
        };
        let collapsed = canonicalize(inner, ctx);
        match collapsed.as_schema() {
            Schema::Not(inner_inner) => Arc::clone(inner_inner),
            Schema::True => shared(Schema::False),
            Schema::False => shared(Schema::True),
            other => {
                if let Schema::TypeGuard { ty, body } = other {
                    if let Some(folded) = negate_type_guard(*ty, body) {
                        return folded;
                    }
                }
                // `Not(X)` for type-set-equivalent `X` collapses to the complement set when representable.
                if let Some(folded) = collapse_via_type_set(other) {
                    folded
                } else if Arc::ptr_eq(&collapsed, inner) {
                    Arc::clone(schema)
                } else {
                    shared(Schema::Not(collapsed))
                }
            }
        }
    }
}

fn collapse_via_type_set(inner: &Schema) -> Option<SharedSchema> {
    let set = inner.as_type_set()?;
    let complement = Schema::type_set_complement(set)?;
    Some(multi_type_or_false(complement))
}

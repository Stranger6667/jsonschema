//! Normalization: tighten leaves and detect emptiness.
//!
//! These passes snap numeric bounds, cap array lengths, close object requirements, and map contradictions to
//! `false`. Unlike [`super::structural`], they work on leaf values rather than logical shape.

pub(crate) mod array;
pub(crate) mod collapse;
pub(crate) mod emptiness;
mod intervals;
pub(crate) mod numeric;
pub(crate) mod object;

pub(crate) use collapse::collapse;

use std::sync::Arc;

use crate::canonical::{
    context::{CanonicalizationContext, WalkStage},
    ir::{SchemaKindSet, SharedSchema},
    rewrite::walk::map_children,
};

/// One uniform normalization stage: recurse into children, then rewrite the current node.
/// `run` supplies the scaffold (mask fast-path + per-stage memo + recursion); implementors supply `rewrite`.
pub(crate) trait NormalizeStage {
    /// Per-stage memo keyspace.
    const WALK: WalkStage;
    /// Fast-path gate: skip subtrees with no relevant kind. `SchemaKindSet::empty()` = no gate (always run).
    const MASK: SchemaKindSet;
    /// Rewrite the node whose children are already normalized. `recursed` is passed by value so a
    /// no-op stage can return it directly.
    fn rewrite(recursed: SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema;
}

/// Run a normalization stage over `schema` to its node-local fixpoint contribution.
pub(crate) fn run<S: NormalizeStage>(
    schema: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    // Empty mask means "no gate": an empty set is disjoint from everything, so guard explicitly or the stage never runs.
    if S::MASK != SchemaKindSet::empty() && schema.mask.is_disjoint(S::MASK) {
        return Arc::clone(schema);
    }
    ctx.with_walk_memo(S::WALK, schema, || {
        let recursed = map_children(schema, |child| run::<S>(child, ctx));
        S::rewrite(recursed, ctx)
    })
}

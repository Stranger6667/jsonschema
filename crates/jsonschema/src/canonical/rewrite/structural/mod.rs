//! Structural rewrites: pick one representative per equivalence class.
//!
//! These rewrite a schema's logical shape - value sets, `oneOf`, `if`/`then`/`else`, `not`, `allOf`-by-type -
//! without tightening leaf values (that is [`super::normalize`]). Each module folds the keyword it is named for.

pub(crate) mod const_enum;
pub(crate) mod if_then_else;
pub(crate) mod not_elim;
pub(crate) mod one_of;
pub(crate) mod type_partition;

use std::sync::Arc;

use crate::canonical::{
    context::{CanonicalizationContext, WalkStage},
    ir::{SchemaKindSet, SharedSchema},
};

/// A mask-gated, memoised pre-order rewrite. `run` supplies the scaffold; implementors supply only
/// `rewrite`, which recurses into children itself (through `run`).
///
/// Pre-order unlike [`super::normalize::NormalizeStage`]: the handler controls recursion, so a pass
/// can short-circuit a node without descending (e.g. an empty `oneOf` collapses to `False`).
pub(crate) trait StructuralStage {
    /// Per-stage memo keyspace.
    const WALK: WalkStage;
    /// Fast-path gate; `SchemaKindSet::empty()` = no gate.
    const MASK: SchemaKindSet;
    fn rewrite(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema;
}

/// Run a structural stage under its memo, skipping gated-out subtrees.
pub(crate) fn run<S: StructuralStage>(
    schema: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    // An empty mask is disjoint from everything, so treat it as "no gate" explicitly.
    if S::MASK != SchemaKindSet::empty() && schema.mask.is_disjoint(S::MASK) {
        return Arc::clone(schema);
    }
    ctx.with_walk_memo(S::WALK, schema, || S::rewrite(schema, ctx))
}

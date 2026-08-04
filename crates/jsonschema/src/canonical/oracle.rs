//! Reasoning queries over canonical IR nodes.
use crate::canonical::{
    algebra,
    context::CanonicalizationContext,
    ir::{Schema, Verdict},
};

/// Whether `right` admits every value `left` admits.
///
/// The intersection accepts exactly `left ∩ right`, so structural equality with `left` proves the
/// containment. Inequality proves nothing, hence `Unknown` rather than `Rejects`.
///
/// A meet the canonical form has no exact spelling for stands in for the real one, and one wider
/// than it would carry that equality without the containment holding. Only a meet declined by this
/// query is read that way: the run may have declined one before it, and that decline says nothing
/// about the pair asked about here. The decline does stay recorded, since the memo hands the
/// approximation to whoever asks for the same pair next.
pub(crate) fn covers(left: &Schema, right: &Schema, ctx: &CanonicalizationContext) -> Verdict {
    let declined_before = ctx.saw_unspellable_meet();
    let meet = algebra::intersect(left.clone(), right.clone(), ctx);
    if !declined_before && ctx.saw_unspellable_meet() {
        return Verdict::Unknown;
    }
    if meet == *left {
        Verdict::Admits
    } else {
        Verdict::Unknown
    }
}

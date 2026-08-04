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
pub(crate) fn covers(left: &Schema, right: &Schema, ctx: &CanonicalizationContext) -> Verdict {
    if algebra::intersect(left.clone(), right.clone(), ctx) == *left {
        Verdict::Admits
    } else {
        Verdict::Unknown
    }
}

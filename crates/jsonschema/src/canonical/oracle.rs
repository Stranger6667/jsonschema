//! Reasoning queries over canonical IR nodes.
use crate::canonical::{
    algebra,
    context::CanonicalizationContext,
    ir::{Schema, Verdict},
};

/// Whether `outer` admits every value `inner` admits.
///
/// `outer ∩ inner == inner` proves it. Anything else is `Unknown`, since two schemas can
/// accept the same values in different forms.
pub(crate) fn covers(outer: &Schema, inner: &Schema, ctx: &CanonicalizationContext) -> Verdict {
    // A facet no checker covers reads here as met, so the equality below would prove a containment
    // a validator carrying that checker does not have - unless `inner` demands those facets too,
    // where its own values carry them.
    let unchecked = algebra::uncheckable_string_facets(outer, ctx);
    if !unchecked.is_empty()
        && !unchecked.is_subset(&algebra::uncheckable_string_facets(inner, ctx))
    {
        return Verdict::Unknown;
    }
    let (intersection, inexact) =
        ctx.probe(|| algebra::intersect(outer.clone(), inner.clone(), ctx));
    // An intersection this call could only approximate may be wider than the real one, which would
    // make the equality below prove nothing.
    if inexact {
        return Verdict::Unknown;
    }
    // Both sides are compared through what their pointers name, or a pointer would never be found
    // to cover what it names - itself included.
    let intersection = algebra::resolved(intersection, ctx);
    let inner = algebra::resolved(inner.clone(), ctx);
    if intersection == inner {
        Verdict::Admits
    } else {
        Verdict::Unknown
    }
}

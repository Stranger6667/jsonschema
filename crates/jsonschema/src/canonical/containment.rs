//! Containment queries over canonical IR nodes.
use std::sync::Arc;

use crate::canonical::{
    algebra,
    context::CanonicalizationContext,
    ir::{Schema, SchemaKind, Verdict},
};

/// Intersections one proof under assumption may spend, nested proofs included. A nested result
/// holds only under its assumption and is recomputed in every scope; past this the proof gives up.
const PROOF_BUDGET: u64 = 500;

/// Whether `outer` accepts every value `inner` accepts.
///
/// Intersecting the two and getting `inner` back proves it; two pointers to recursive targets are
/// compared through their bodies, with the containment assumed where the bodies reach the pair
/// again. Anything else is `Unknown`, since two schemas can accept the same values in different
/// forms.
pub(crate) fn covers(outer: &Schema, inner: &Schema, ctx: &CanonicalizationContext) -> Verdict {
    let verdict = covers_as_written(outer, inner, ctx);
    if verdict == Verdict::Accepts {
        return verdict;
    }
    // Pointers to two recursive targets never compare as equal: whether one holds the other rests
    // on the bodies, which reach the same pair again further down.
    let (SchemaKind::Reference(_), SchemaKind::Reference(_)) = (outer.kind(), inner.kind()) else {
        return verdict;
    };
    let outer = algebra::resolved(outer.clone(), ctx);
    let inner = algebra::resolved(inner.clone(), ctx);
    match (outer.kind(), inner.kind()) {
        // Skip a pair the intersection above already tried inside an enclosing proof.
        (SchemaKind::Reference(outer), SchemaKind::Reference(inner))
            if !ctx.attempted(inner, outer) && contained_by_assumption(outer, inner, ctx) =>
        {
            Verdict::Accepts
        }
        _ => verdict,
    }
}

fn covers_as_written(outer: &Schema, inner: &Schema, ctx: &CanonicalizationContext) -> Verdict {
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
        Verdict::Accepts
    } else {
        Verdict::Unknown
    }
}

/// Whether the target `inner` names lies inside the one `outer` names: assumed by a proof in
/// progress, or proven on the two bodies with it assumed.
fn contained_by_assumption(
    outer: &Arc<str>,
    inner: &Arc<str>,
    ctx: &CanonicalizationContext,
) -> bool {
    if ctx.assumes_contained(inner, outer) {
        return true;
    }
    if let Some(held) = ctx.settled_containment(inner, outer) {
        return held;
    }
    let (Some(outer_body), Some(inner_body)) =
        (ctx.guarded_target(outer), ctx.guarded_target(inner))
    else {
        return false;
    };
    let (proven, outgrew) = ctx.capped(PROOF_BUDGET, || {
        ctx.assuming_contained(inner, outer, || covers(outer_body, inner_body, ctx))
    });
    let held = !outgrew && proven == Some(Verdict::Accepts);
    ctx.settle_containment(inner, outer, held);
    held
}

/// The narrower of two pointers a containment proof in progress reaches, where it holds one inside
/// the other. Outside a proof it returns `None` after one check.
pub(crate) fn narrower<'a>(
    first: &'a Arc<str>,
    second: &'a Arc<str>,
    ctx: &CanonicalizationContext,
) -> Option<&'a Arc<str>> {
    if !ctx.assumes_any() {
        return None;
    }
    // A proof intersects its outer side with its inner one, so the pointer it wants back arrives
    // second, and wins where both directions are assumed.
    if ctx.assumes_contained(second, first) {
        return Some(second);
    }
    if ctx.assumes_contained(first, second) {
        return Some(first);
    }
    // Only the direction the proof wants is proven: trying both would explore every failing pair
    // below twice over, once per level.
    ctx.note_attempt(second, first);
    contained_by_assumption(first, second, ctx).then_some(second)
}

//! Negation-focused collapse passes.

use std::sync::Arc;

use crate::{
    canonical::{
        context::CanonicalizationContext,
        coverage::covers,
        intern::shared,
        intersect::intersect_canonical,
        ir::{Schema, SharedSchema},
    },
    JsonType,
};

/// De Morgan: fuse sibling `not` branches into one. In an `allOf`, `not A` and `not B` become a single
/// `not: {anyOf: [A, B]}` (pass `Schema::AnyOf`); in an `anyOf`, they become `not: {allOf: [A, B]}` (pass
/// `Schema::AllOf`).
///
/// ```text
/// BEFORE: {"allOf": [{"not": {"const": 1}}, {"not": {"const": 2}}]}
/// AFTER:  {"not": {"anyOf": [{"const": 1}, {"const": 2}]}}
/// ```
pub(super) fn combine_negations(
    branches: &mut Vec<SharedSchema>,
    wrap_inners: fn(Vec<SharedSchema>) -> Schema,
) -> bool {
    let not_count = branches
        .iter()
        .filter(|branch| matches!(branch.as_schema(), Schema::Not(_)))
        .count();
    if not_count < 2 {
        return false;
    }
    let mut inners: Vec<SharedSchema> = Vec::with_capacity(not_count);
    branches.retain(|branch| match branch.as_schema() {
        Schema::Not(inner) => {
            inners.push(Arc::clone(inner));
            false
        }
        _ => true,
    });
    branches.push(shared(Schema::Not(shared(wrap_inners(inners)))));
    true
}

/// A union next to a negation distributes the negation into the union: drop each union member that a sibling
/// `not: X` already excludes (`covers(X, member)`).
///
/// ```text
/// BEFORE: {"allOf": [{"anyOf": [{"const": 1}, {"const": 2}]}, {"not": {"const": 1}}]}
/// AFTER:  {"allOf": [{"const": 2}, {"not": {"const": 1}}]}   // the `const 1` member is dropped
/// ```
pub(super) fn distribute_not_through_any_of(
    branches: &mut [SharedSchema],
    ctx: &CanonicalizationContext,
) -> bool {
    let negations: Vec<SharedSchema> = branches
        .iter()
        .filter_map(|branch| match branch.as_schema() {
            Schema::Not(inner) => Some(Arc::clone(inner)),
            _ => None,
        })
        .collect();
    if negations.is_empty() {
        return false;
    }
    let mut changed = false;
    for branch in branches.iter_mut() {
        let Schema::AnyOf(inner) = branch.as_schema() else {
            continue;
        };
        let mut remaining_branches: Vec<SharedSchema> = inner
            .iter()
            .filter(|inner_branch| !negations.iter().any(|neg| covers(neg, inner_branch, ctx)))
            .cloned()
            .collect();
        if remaining_branches.len() == inner.len() {
            continue;
        }
        changed = true;
        *branch = match remaining_branches.len() {
            0 => shared(Schema::False),
            1 => remaining_branches.pop().expect("len == 1"),
            _ => shared(Schema::AnyOf(remaining_branches)),
        };
    }
    changed
}

/// Drop a `not` whose scalar/value leaf is disjoint from a positive scalar sibling - the interval/value disjointness
/// [`drop_disjoint_negations`] misses. Restricted to these leaves so `intersect_canonical` resolves without re-entering the fold.
///
/// ```text
/// BEFORE: {"allOf": [{"type": "integer", "minimum": 1}, {"not": {"const": 0}}]}
/// AFTER:  {"type": "integer", "minimum": 1}   // 0 is already outside the interval, so excluding it adds nothing
/// ```
pub(super) fn drop_value_disjoint_negations(
    branches: &mut Vec<SharedSchema>,
    ctx: &CanonicalizationContext,
) -> bool {
    fn is_concrete_leaf(schema: &Schema) -> bool {
        matches!(
            schema,
            Schema::Integer(_)
                | Schema::Number(_)
                | Schema::String(_)
                | Schema::Null
                | Schema::Boolean(_)
                | Schema::Const(_)
                | Schema::Enum(_)
        )
    }
    let positives: Vec<SharedSchema> = branches
        .iter()
        .filter(|branch| !matches!(branch.as_schema(), Schema::Not(_)))
        .filter(|branch| is_concrete_leaf(branch.as_schema()))
        .map(Arc::clone)
        .collect();
    if positives.is_empty() {
        return false;
    }
    let original_len = branches.len();
    branches.retain(|branch| {
        let Schema::Not(inner) = branch.as_schema() else {
            return true;
        };
        if !is_concrete_leaf(inner.as_schema()) {
            return true;
        }
        let disjoint = positives.iter().any(|positive| {
            matches!(
                intersect_canonical(inner, positive, ctx).as_schema(),
                Schema::False
            )
        });
        !disjoint
    });
    branches.len() != original_len
}

/// Drop a `not` branch whose type can never overlap any positive branch's type. `integer` and `number` overlap, so
/// `not: {type: "number"}` is kept next to an `integer` branch.
///
/// ```text
/// BEFORE: {"allOf": [{"type": "string"}, {"not": {"type": "integer"}}]}
/// AFTER:  {"type": "string"}   // a string is never an integer, so the negation is redundant
/// ```
pub(super) fn drop_disjoint_negations(branches: &mut Vec<SharedSchema>) -> bool {
    let positive_types: Vec<JsonType> = branches
        .iter()
        .filter_map(|branch| match branch.as_schema() {
            Schema::Not(_) => None,
            _ => pinned_ty(branch),
        })
        .collect();
    if positive_types.is_empty() {
        return false;
    }
    let original_len = branches.len();
    branches.retain(|branch| {
        let Schema::Not(inner) = branch.as_schema() else {
            return true;
        };
        let Some(neg_types) = negation_types(inner) else {
            return true;
        };
        // Drop only when every type is disjoint from every positive.
        !neg_types
            .iter()
            .all(|nt| positive_types.iter().all(|p| !p.overlaps(*nt)))
    });
    branches.len() != original_len
}

/// Returns `None` when any branch isn't type-pinned.
fn negation_types(inner: &SharedSchema) -> Option<Vec<JsonType>> {
    match inner.as_schema() {
        Schema::AnyOf(branches) => {
            let mut types = Vec::with_capacity(branches.len());
            for branch in branches {
                types.push(pinned_ty(branch)?);
            }
            Some(types)
        }
        _ => Some(vec![pinned_ty(inner)?]),
    }
}

/// `None` for shapes that do not saturate a single JSON type. Const/Enum here must saturate the type, not just match it.
fn pinned_ty(schema: &SharedSchema) -> Option<JsonType> {
    let schema = schema.as_schema();
    schema.pinned_kind().or_else(|| {
        schema
            .finite_values()
            .and_then(Schema::finite_values_saturate_type)
    })
}

/// `true` when a `not` branch rejects everything the positive branches accept - either a single positive is fully
/// covered by the negated schema, or the conjunction of all positives is (so no single positive triggers it alone).
///
/// ```text
/// BEFORE: {"allOf": [{"type": "integer", "minimum": 5}, {"not": {"type": "integer", "minimum": 0}}]}
/// AFTER:  false   // every integer >= 5 is also >= 0, which the negation forbids
/// ```
pub(super) fn not_branch_contradicts_positive(
    branches: &[SharedSchema],
    ctx: &CanonicalizationContext,
) -> bool {
    let positives: Vec<SharedSchema> = branches
        .iter()
        .filter(|branch| !matches!(branch.as_schema(), Schema::Not(_)))
        .map(Arc::clone)
        .collect();
    // The conjunction of all positives - `Not(inner) ∧ positives` is empty when that conjunction is covered by `inner`,
    // even if no single positive is (e.g. `A ∧ B ∧ ¬(A ∧ B)`).
    let conjunction = (positives.len() > 1).then(|| shared(Schema::AllOf(positives.clone())));
    for branch in branches {
        let Schema::Not(inner) = branch.as_schema() else {
            continue;
        };
        if positives
            .iter()
            .any(|positive| covers(inner, positive, ctx))
        {
            return true;
        }
        if let Some(conjunction) = &conjunction {
            if covers(inner, conjunction, ctx) {
                return true;
            }
        }
    }
    false
}

/// `true` when a positive branch `B` and a sibling `not: Y` together admit everything: if `B` covers `Y`, every
/// value either satisfies `B` or fails `Y`, so the union is `true`.
///
/// ```text
/// BEFORE: {"anyOf": [{"type": "integer", "minimum": 0}, {"not": {"type": "integer", "minimum": 5}}]}
/// AFTER:  true   // any value failing ">= 5" plus every integer >= 0 covers the whole universe
/// ```
pub(super) fn any_of_contains_complementary_pair(
    branches: &[SharedSchema],
    ctx: &CanonicalizationContext,
) -> bool {
    let mut negation_inners: Vec<&SharedSchema> = Vec::new();
    let mut positives: Vec<&SharedSchema> = Vec::new();
    for branch in branches {
        match branch.as_schema() {
            Schema::Not(inner) => negation_inners.push(inner),
            _ => positives.push(branch),
        }
    }
    if negation_inners.is_empty() || positives.is_empty() {
        return false;
    }
    negation_inners.iter().any(|inner| {
        positives
            .iter()
            .any(|positive| Arc::ptr_eq(positive, inner) || covers(positive, inner, ctx))
    })
}

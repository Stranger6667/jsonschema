//! Conservative schema-set containment checks.

#![cfg_attr(not(feature = "arbitrary-precision"), allow(clippy::clone_on_copy))]

use std::sync::Arc;

use ahash::AHashSet;
use num_traits::Zero;

use crate::{
    canonical::{
        context::CanonicalizationContext,
        intern::shared,
        intersect::intersect_canonical,
        ir::{
            ArrayLeaf, BoundCardinality, BoundFraction, ContainsClause, IntegerBounds, IntegerLeaf,
            LengthBounds, NumberBounds, NumberLeaf, ObjectLeaf, ObjectRequirement,
            PropertyNameMatcher, Schema, SharedSchema, StringLeaf,
        },
        leaves::{
            object::scope::{compiled_patterns, non_true_catch_all},
            Membership, TypedLeaf,
        },
        membership::admits,
        negate::negate,
        numeric::{number_not_multiple_of_to_integer, NumericBounds, NumericLeaf},
        prover::Prover,
    },
    JsonTypeSet,
};

/// `covers(big, small)`: every JSON value satisfying `small` also satisfies `big` (a conservative subset check).
///
/// ```text
/// covers({"type": ["integer", "string"]}, {"enum": [1, "x"]})          -> true
/// covers({"type": "integer", "minimum": 0}, {"type": "integer", "minimum": 5})  -> true
/// covers({"const": 1}, {"type": "integer"})                            -> false
/// ```
#[must_use]
pub(crate) fn covers(
    big: &SharedSchema,
    small: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> bool {
    covers_with(&Prover::without_definitions(ctx), big, small)
}

/// True when some branch outside `skip` covers `delta`.
///
/// `skip` excludes a branch from covering its own delta - self-exclusion is load-bearing for
/// soundness, so it lives here rather than at each `*_covered_by_sibling` call site.
#[must_use]
pub(crate) fn any_sibling_covers(
    branches: &[SharedSchema],
    skip: &[usize],
    delta: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> bool {
    branches
        .iter()
        .enumerate()
        .any(|(sibling, branch)| !skip.contains(&sibling) && covers(branch, delta, ctx))
}

/// [`covers`] under an explicit prover (definitions environment + assumption state).
#[must_use]
pub(crate) fn covers_with(prover: &Prover<'_>, big: &SharedSchema, small: &SharedSchema) -> bool {
    // Scope the partition budget to the outermost query; nested calls share it.
    prover.ctx().enter_covers_query();
    let result = covers_within_query(prover, big, small);
    prover.ctx().exit_covers_query();
    result
}

/// `Some(big ⊇ small)` when both are the same leaf kind; `None` for different variants (handled by
/// the caller's match).
fn same_kind_leaf_covers(big: &Schema, small: &Schema, prover: &Prover<'_>) -> Option<bool> {
    fn check<L: TypedLeaf>(big: &Schema, small: &Schema, prover: &Prover<'_>) -> Option<bool> {
        Some(
            L::project(big)?
                .covers(L::project(small)?, prover)
                .is_proven(),
        )
    }
    check::<IntegerLeaf>(big, small, prover)
        .or_else(|| check::<NumberLeaf>(big, small, prover))
        .or_else(|| check::<StringLeaf>(big, small, prover))
        .or_else(|| check::<ArrayLeaf>(big, small, prover))
        .or_else(|| check::<ObjectLeaf>(big, small, prover))
}

fn covers_within_query(prover: &Prover<'_>, big: &SharedSchema, small: &SharedSchema) -> bool {
    if prover.nodes_equal(big, small) {
        return true;
    }
    // Coinductive hypothesis: this pair is already being proven further up; conclude it. Sound for
    // finite JSON values - a separating value has finite depth and the proof tree unfolds past it.
    if prover.assumption_holds(big, small) {
        return true;
    }
    // An env-less prover pushes no coinductive assumption, so its verdicts are unconditional and
    // memoizable for the whole query - keeping sibling/dominance passes from re-walking the same pair.
    if !prover.has_definitions() {
        let key = (Arc::clone(big), Arc::clone(small));
        if let Some(cached) = prover.ctx().covers_memo().borrow().get(&key).copied() {
            return cached;
        }
        let result = covers_uncached(prover, big, small);
        // A fuel-exhausted `false` is non-authoritative; don't let it mask a later provable `true`.
        if result || prover.ctx().partition_fuel_remaining() > 0 {
            prover.ctx().covers_memo().borrow_mut().insert(key, result);
        }
        return result;
    }
    covers_uncached(prover, big, small)
}

/// The coverage decision itself, without the query-level memo (see [`covers_within_query`]).
///
/// The `nodes_equal` and coinductive-assumption fast paths are already checked by the caller.
fn covers_uncached(prover: &Prover<'_>, big: &SharedSchema, small: &SharedSchema) -> bool {
    // Same-kind leaf pairs dispatch through one helper rather than five identical arms. Safe to hoist:
    // no earlier arm matches a plain leaf both sides, and cross-kind `Number ⊇ Integer` returns `None` here.
    if let Some(result) = same_kind_leaf_covers(big.as_schema(), small.as_schema(), prover) {
        return result;
    }
    match (big.as_schema(), small.as_schema()) {
        (Schema::True, _) | (_, Schema::False) => true,
        // A union is covered iff every branch is covered.
        (_, Schema::AnyOf(branches)) => branches
            .iter()
            .all(|branch| covers_with(prover, big, branch)),
        // `⋂ branches ⊆ big` if `big` covers any single conjunct (the intersection is within it).
        (_, Schema::AllOf(branches)) => {
            if branches
                .iter()
                .any(|branch| covers_with(prover, big, branch))
            {
                return true;
            }
            // `big`'s facets may be spread across conjuncts. Since `big = ⋂ facets`, `⋂ branches ⊆ big`
            // iff every facet covers it; each facet is one constraint, so this hits the single-conjunct check.
            if let Schema::String(big_leaf) = big.as_schema() {
                let facets = string_leaf_facets(big_leaf);
                if facets.len() >= 2 {
                    return facets.iter().all(|facet| covers_with(prover, facet, small));
                }
            }
            false
        }
        // `MultiType(s)` covers any schema whose admitted types subset `s` under `Integer ⊆ Number`. A
        // constrained typed leaf (e.g. `Integer{>= 0}`) is covered when its kind is in `s`.
        (Schema::MultiType(big_set), _) => {
            if let Some(small_set) = small.as_schema().as_type_set() {
                return Schema::type_set_covers(*big_set, small_set);
            }
            // `Const`/`Enum` values are covered when each value's type is in the set.
            match small.as_schema() {
                Schema::Const(value) => {
                    return Schema::type_set_contains_kind(*big_set, value.json_type());
                }
                Schema::Enum(values) => {
                    return values
                        .iter()
                        .all(|value| Schema::type_set_contains_kind(*big_set, value.json_type()));
                }
                _ => {}
            }
            if let Some(kind) = small.as_schema().pinned_kind() {
                return Schema::type_set_contains_kind(*big_set, kind);
            }
            false
        }
        // Big-side structural arms come before the small-side `MultiType` arm below, which would otherwise shadow them
        // (an `AnyOf`/`AllOf`/`Not` has no `as_type_set`, so it would wrongly fall through to `false`).
        (Schema::AnyOf(branches), _) => {
            // Env-carrying prover results depend on its definitions/assumptions, so they stay in the
            // per-query memo; env-less results are memoized once by `covers_within_query`.
            if prover.has_definitions() {
                if let Some(cached) = prover.local_memo_get(big, small) {
                    return cached;
                }
                let result = any_of_union_covers(branches, small, prover);
                prover.local_memo_insert(big, small, result);
                return result;
            }
            any_of_union_covers(branches, small, prover)
        }
        // `small ⊆ ⋂ branches` iff every conjunct covers it.
        (Schema::AllOf(branches), _) => branches
            .iter()
            .all(|branch| covers_with(prover, branch, small)),
        // `small ⊆ ¬inner` iff `small` and `inner` are disjoint.
        (Schema::Not(inner), _) => provably_disjoint(inner.as_schema(), small.as_schema()),
        (Schema::TypeGuard { ty, body }, _)
            if small.as_schema().pinned_kind().is_some()
                || matches!(
                    small.as_schema(),
                    Schema::Const(_) | Schema::Enum(_) | Schema::MultiType(_)
                ) =>
        {
            // The guard passes every non-`ty` value; `ty`-kinded values go through the body.
            if let Some(kind) = small.as_schema().pinned_kind() {
                !ty.overlaps(kind) || (ty.covers(kind) && covers_with(prover, body, small))
            } else if let Schema::MultiType(set) = small.as_schema() {
                set.iter().all(|kind| !ty.overlaps(kind))
            } else {
                let values = match small.as_schema() {
                    Schema::Const(value) => std::slice::from_ref(value),
                    Schema::Enum(values) => values.as_slice(),
                    _ => unreachable!("guarded by match condition"),
                };
                values.iter().all(|value| !ty.overlaps(value.json_type()))
            }
        }
        (_, Schema::MultiType(small_set)) => {
            let Some(big_set) = big.as_schema().as_type_set() else {
                return false;
            };
            Schema::type_set_covers(big_set, *small_set)
        }
        (
            Schema::TypedGroup {
                ty: big_ty,
                body: big_body,
            },
            _,
        ) => {
            let Some(small_view) = small.as_typed_view() else {
                return false;
            };
            big_ty.covers(small_view.ty) && covers_with(prover, big_body, &small_view.schema)
        }
        (
            _,
            Schema::TypedGroup {
                body: small_body, ..
            },
        ) => covers_with(prover, big, small_body),
        (Schema::Enum(big_values), Schema::Enum(small_values)) => {
            let set: AHashSet<&_> = big_values.iter().collect();
            small_values.iter().all(|value| set.contains(value))
        }
        (Schema::Number(big_leaf), Schema::Integer(small_leaf)) => {
            number_covers_integer(big_leaf, small_leaf)
        }
        (Schema::Enum(big_values), Schema::Const(small_value)) => big_values.contains(small_value),
        // A schema covers a finite value set iff it definitely admits every member.
        (_, Schema::Const(value)) => {
            admits(value, big.as_schema(), prover.ctx()) == Membership::Yes
        }
        (_, Schema::Enum(values)) => values
            .iter()
            .all(|value| admits(value, big.as_schema(), prover.ctx()) == Membership::Yes),
        // Unfold symbolic references through the owning side's definitions under an assumption.
        (Schema::Recursive(name), _) => prover
            .resolve_big(name)
            .is_some_and(|body| prover.assume(big, small) && covers_with(prover, body, small)),
        (Schema::Reference(uri), _) => prover
            .resolve_big(uri.as_str())
            .is_some_and(|body| prover.assume(big, small) && covers_with(prover, body, small)),
        (_, Schema::Recursive(name)) => prover
            .resolve_small(name)
            .is_some_and(|body| prover.assume(big, small) && covers_with(prover, big, body)),
        (_, Schema::Reference(uri)) => prover
            .resolve_small(uri.as_str())
            .is_some_and(|body| prover.assume(big, small) && covers_with(prover, big, body)),
        _ => prover.nodes_equal(big, small),
    }
}

/// `leaf = ⋂ facets` exactly: one single-constraint string leaf per length bound, pattern,
/// not-pattern, format, and content facet, so containment holds when constraints are spread across an `AllOf`.
fn string_leaf_facets(leaf: &StringLeaf) -> Vec<SharedSchema> {
    let mut facets = Vec::new();
    if leaf.min_length.is_some() {
        facets.push(shared(Schema::String(StringLeaf {
            min_length: leaf.min_length.clone(),
            ..StringLeaf::default()
        })));
    }
    if leaf.max_length.is_some() {
        facets.push(shared(Schema::String(StringLeaf {
            max_length: leaf.max_length.clone(),
            ..StringLeaf::default()
        })));
    }
    for pattern in &leaf.patterns {
        facets.push(shared(Schema::String(StringLeaf {
            patterns: vec![Arc::clone(pattern)],
            ..StringLeaf::default()
        })));
    }
    for not_pattern in &leaf.not_patterns {
        facets.push(shared(Schema::String(StringLeaf {
            not_patterns: vec![Arc::clone(not_pattern)],
            ..StringLeaf::default()
        })));
    }
    if leaf.format.is_some() {
        facets.push(shared(Schema::String(StringLeaf {
            format: leaf.format.clone(),
            ..StringLeaf::default()
        })));
    }
    for content in &leaf.content {
        facets.push(shared(Schema::String(StringLeaf {
            content: vec![content.clone()],
            ..StringLeaf::default()
        })));
    }
    facets
}

/// Above this branch count the partition search is skipped - the fan-out isn't worth it.
const PARTITION_BRANCH_LIMIT: usize = 6;
/// Operands above this tree size are excluded from partitioning: each step interns intersect+canonicalize
/// intermediates for the context's lifetime, so the size gate is the only bound on total partition cost.
const PARTITION_SIZE_LIMIT: u32 = 12;

/// `small ⊆ ⋃ branches`. Direct: some branch covers `small`.
///
/// Otherwise partition - pick a branch `B` and prove the rest cover the canonicalized `small ∧ ¬B`
/// (so `small = (small ∧ B) ∪ (small ∧ ¬B) ⊆ B ∪ ⋃rest`); shared partition fuel bounds total work.
fn any_of_union_covers(
    branches: &[SharedSchema],
    small: &SharedSchema,
    prover: &Prover<'_>,
) -> bool {
    if branches
        .iter()
        .any(|branch| covers_with(prover, branch, small))
    {
        return true;
    }
    if branches.len() < 2
        || branches.len() > PARTITION_BRANCH_LIMIT
        || small.size > PARTITION_SIZE_LIMIT
        || branches
            .iter()
            .any(|branch| branch.size > PARTITION_SIZE_LIMIT)
    {
        return false;
    }
    for index in 0..branches.len() {
        if !prover.ctx().consume_partition_fuel() {
            return false;
        }
        let remainder =
            intersect_canonical(small, &negate(&branches[index], prover.ctx()), prover.ctx());
        if matches!(remainder.as_schema(), Schema::False) {
            continue;
        }
        let rest: Vec<SharedSchema> = branches
            .iter()
            .enumerate()
            .filter(|&(other, _)| other != index)
            .map(|(_, branch)| Arc::clone(branch))
            .collect();
        if any_of_union_covers(&rest, &remainder, prover) {
            return true;
        }
    }
    false
}

/// Conservative upper bound on the JSON types a schema can admit; `None` when its type domain can't be pinned down.
fn accepted_types(schema: &Schema) -> Option<JsonTypeSet> {
    schema.type_domain_upper_bound()
}

/// `true` when no value can satisfy both.
///
/// Finite value sets are disjoint iff they share no member; otherwise fall back to type-domain
/// disjointness. Conservative: returns `false` when neither test proves disjointness.
fn provably_disjoint(left: &Schema, right: &Schema) -> bool {
    if let (Some(left_values), Some(right_values)) = (left.finite_values(), right.finite_values()) {
        let set: AHashSet<&_> = left_values.iter().collect();
        return !right_values.iter().any(|value| set.contains(value));
    }
    let (Some(left_types), Some(right_types)) = (accepted_types(left), accepted_types(right))
    else {
        return false;
    };
    !left_types.iter().any(|left_ty| {
        right_types
            .iter()
            .any(|right_ty| left_ty.overlaps(right_ty))
    })
}

/// Whether object leaf `big` covers `small`.
///
/// Compares `required`, min/max properties, `propertyNames`, and per-property constraints (incl. catch-all
/// governance). Declines on dependent/pattern-property requirements - none has a direct coverage check.
pub(crate) fn object_leaf_covers(
    big: &ObjectLeaf,
    small: &ObjectLeaf,
    prover: &Prover<'_>,
) -> bool {
    // Equal leaves reach here only when `nodes_equal` declined them: `$ref`-carrying leaves under
    // incompatible definition environments, where the shared name resolves to different value sets. Decline.
    if big == small {
        return false;
    }
    // `big`'s `propertyNames` constrains every key, so `small ⊆ big` needs `small` to carry a
    // `propertyNames` that `big`'s covers.
    if let Some(big_names) = big.property_names.as_ref() {
        match small.property_names.as_ref() {
            Some(small_names) if covers_with(prover, big_names, small_names) => {}
            _ => return false,
        }
    }
    // A dependent/existential entry is instance-absolute (except the leaf-scoped additional-name
    // existential), so it is satisfied only when `small` carries the identical entry; otherwise bail.
    let mut big_required: Vec<&Arc<str>> = Vec::new();
    for requirement in &big.requirements {
        match requirement {
            ObjectRequirement::RequiredProperty(name) => big_required.push(name),
            ObjectRequirement::MinProperties(_) | ObjectRequirement::MaxProperties(_) => {}
            ObjectRequirement::PatternPropertyRequirement {
                matcher: PropertyNameMatcher::AdditionalProperties,
                ..
            } => return false,
            ObjectRequirement::DependentPropertiesRequirement { .. }
            | ObjectRequirement::DependentSchemaRequirement { .. }
            | ObjectRequirement::PatternPropertyRequirement { .. } => {
                if !small.requirements.contains(requirement) {
                    return false;
                }
            }
        }
    }
    // `small ⊆ big` needs every property `big` requires to be required by `small` too.
    let small_required: AHashSet<&Arc<str>> = small
        .requirements
        .iter()
        .filter_map(|requirement| match requirement {
            ObjectRequirement::RequiredProperty(name) => Some(name),
            _ => None,
        })
        .collect();
    if !big_required
        .iter()
        .all(|name| small_required.contains(name))
    {
        return false;
    }
    let (big_minimum, big_maximum) = object_property_bounds(&big.requirements);
    let (small_minimum, small_maximum) = object_property_bounds(&small.requirements);
    if big_minimum > small_minimum {
        return false;
    }
    if let Some(big_maximum) = big_maximum {
        match small_maximum {
            None => return false,
            Some(small_maximum) if big_maximum < small_maximum => return false,
            _ => {}
        }
    }
    // A `small` admitting no properties (`maxProperties: 0`) is the empty object - it satisfies every
    // per-property constraint vacuously, so `big`'s constraints impose nothing (bounds checked above).
    let small_admits_no_properties = small_maximum.as_ref().is_some_and(Zero::is_zero);
    // `S_big == True` is vacuous and skipped.
    for big_constraint in &big.constraints {
        if small_admits_no_properties || matches!(big_constraint.schema.as_schema(), Schema::True) {
            continue;
        }
        let Some(small_constraint) = small
            .constraints
            .iter()
            .find(|c| c.matcher == big_constraint.matcher)
        else {
            // Without a same-matcher entry, the names `big`'s matcher governs go through `small`'s
            // matchers (the catch-all at minimum), so covering every `small` schema covers them.
            if !small
                .constraints
                .iter()
                .any(|c| matches!(c.matcher, PropertyNameMatcher::AdditionalProperties))
            {
                return false;
            }
            if !small.constraints.iter().all(|small_constraint| {
                covers_with(prover, &big_constraint.schema, &small_constraint.schema)
            }) {
                return false;
            }
            continue;
        };
        if !covers_with(prover, &big_constraint.schema, &small_constraint.schema) {
            return false;
        }
    }
    // Names matched only by a small-side matcher escape `small`'s catch-all but `big`'s still governs
    // them, so `big`'s catch-all must cover whatever those names can hold.
    if !small_admits_no_properties {
        if let Some(big_catch_all) = non_true_catch_all(big) {
            let big_matchers: AHashSet<&PropertyNameMatcher> =
                big.constraints.iter().map(|c| &c.matcher).collect();
            let big_patterns = compiled_patterns(&big.constraints, prover.ctx());
            for constraint in &small.constraints {
                if big_matchers.contains(&constraint.matcher) {
                    continue;
                }
                match &constraint.matcher {
                    // An extra small catch-all only restricts names `big` leaves open.
                    PropertyNameMatcher::AdditionalProperties => {}
                    // A name matching a big pattern is governed by that already-verified pattern, not the catch-all.
                    PropertyNameMatcher::NamedProperty(name)
                        if big_patterns.iter().any(|(regex, _)| regex.is_match(name)) => {}
                    PropertyNameMatcher::NamedProperty(_)
                    | PropertyNameMatcher::PatternProperty(_) => {
                        if !covers_with(prover, big_catch_all, &constraint.schema) {
                            return false;
                        }
                    }
                }
            }
        }
    }
    true
}

/// Reduce `MinProperties`/`MaxProperties` entries into `(minimum, maximum)`.
///
/// Folds defensively (`max` for minimum, `min` for maximum) regardless of normalisation state; distinct
/// `required` names also floor the minimum, since normalisation drops a `minProperties` at or below their count.
fn object_property_bounds(
    requirements: &[ObjectRequirement],
) -> (BoundCardinality, Option<BoundCardinality>) {
    let mut minimum = BoundCardinality::from(0u64);
    let mut maximum: Option<BoundCardinality> = None;
    let mut required: AHashSet<&Arc<str>> = AHashSet::new();
    for requirement in requirements {
        match requirement {
            ObjectRequirement::RequiredProperty(name) => {
                required.insert(name);
            }
            ObjectRequirement::MinProperties(value) if value > &minimum => {
                minimum = (value).owned();
            }
            ObjectRequirement::MaxProperties(value) => {
                maximum = Some(match maximum.as_ref() {
                    Some(current) => (current.min(value)).owned(),
                    None => (value).owned(),
                });
            }
            _ => {}
        }
    }
    let required_floor = BoundCardinality::from(required.len() as u64);
    if required_floor > minimum {
        minimum = required_floor;
    }
    (minimum, maximum)
}

/// Whether array leaf `big` covers `small`.
///
/// Compares length bounds, `uniqueItems`, `repeatedItems`, and per-position item schemas (prefix slots,
/// then tail). Declines on any `contains` clause not vacuous against `small`'s length cap.
pub(crate) fn array_leaf_covers(big: &ArrayLeaf, small: &ArrayLeaf, prover: &Prover<'_>) -> bool {
    // See `object_leaf_covers`: equal leaves arrive only for `$ref`-carrying leaves under incompatible defs.
    if big == small {
        return false;
    }
    if !length_bounds_cover(&big.length, &small.length) {
        return false;
    }
    if big.unique_items && !small.unique_items {
        // Arrays holding at most one item are vacuously unique.
        let at_most_one = small
            .length
            .maximum
            .as_ref()
            .is_some_and(|max| *max <= 1u64);
        if !at_most_one {
            return false;
        }
    }
    // `repeated_items` is a restriction (requires a duplicate); `big` can only cover `small` if `small` carries it too
    if big.repeated_items && !small.repeated_items {
        return false;
    }
    // A clause demanding nothing (`minContains: 0`) whose upper bound is at or above `small`'s length
    // cap is satisfied by every array `small` admits.
    let clause_vacuous = |clause: &ContainsClause| {
        clause.min_contains.is_zero()
            && clause.max_contains.as_ref().is_none_or(|max| {
                small
                    .length
                    .maximum
                    .as_ref()
                    .is_some_and(|small_max| small_max <= max)
            })
    };
    if !big.contains.iter().all(clause_vacuous) {
        return false;
    }
    // An item at `index` exists only when `small`'s max length exceeds it; past that `big`'s per-item
    // constraint is vacuous (`small` has no item there to violate it).
    let item_vacuous = |index: usize| {
        small
            .length
            .maximum
            .as_ref()
            .is_some_and(|max| index as u64 >= *max)
    };
    let max_prefix_len = big.prefix.len().max(small.prefix.len());
    for index in 0..max_prefix_len {
        if item_vacuous(index) {
            continue;
        }
        let big_at = big.prefix.get(index).unwrap_or(&big.tail);
        let small_at = small.prefix.get(index).unwrap_or(&small.tail);
        if !covers_with(prover, big_at, small_at) {
            return false;
        }
    }
    item_vacuous(max_prefix_len) || covers_with(prover, &big.tail, &small.tail)
}

fn length_bounds_cover(big: &LengthBounds, small: &LengthBounds) -> bool {
    if big.minimum > small.minimum {
        return false;
    }
    match (&big.maximum, &small.maximum) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(big_max), Some(small_max)) => big_max >= small_max,
    }
}

/// Whether string leaf `big` covers `small`.
///
/// `big`'s length window must contain `small`'s, and its `pattern`/not-pattern/`contentEncoding`/`format`
/// must each subset `small`'s. The constraints conjoin, so the side carrying more is the smaller set.
pub(crate) fn string_leaf_covers(big: &StringLeaf, small: &StringLeaf) -> bool {
    // No `big == small` guard (unlike object/array helpers): string leaves carry no subschemas, so
    // equal ones are always caught upstream by `nodes_equal` - only `$ref`-carrying leaves need the guard.
    if big.extended_regex() != small.extended_regex() {
        return false;
    }
    if let Some(big_min_length) = &big.min_length {
        match &small.min_length {
            Some(small_min_length) if big_min_length <= small_min_length => {}
            _ => return false,
        }
    }
    if let Some(big_max_length) = &big.max_length {
        match &small.max_length {
            Some(small_max_length) if big_max_length >= small_max_length => {}
            _ => return false,
        }
    }
    let small_set: AHashSet<&Arc<str>> = small.patterns.iter().collect();
    if !big.patterns.iter().all(|p| small_set.contains(p)) {
        return false;
    }
    // Every pattern `big` excludes must also be excluded by `small`, else `big` is more restrictive.
    let small_not_patterns: AHashSet<&Arc<str>> = small.not_patterns.iter().collect();
    if !big
        .not_patterns
        .iter()
        .all(|p| small_not_patterns.contains(p))
    {
        return false;
    }
    let small_content: AHashSet<&_> = small.content.iter().collect();
    if !big
        .content
        .iter()
        .all(|facet| small_content.contains(facet))
    {
        return false;
    }
    match (&big.format, &small.format) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(b), Some(s)) => b == s,
    }
}

fn number_covers_integer(big: &NumberLeaf, small: &IntegerLeaf) -> bool {
    let lifted = integer_bounds_as_fraction(&small.bounds);
    if !big.bounds.covers(&lifted) {
        return false;
    }
    // An integer leaf without `multipleOf` still admits only multiples of one.
    let lifted_modulus = small.multiple_of.as_ref().map_or_else(
        || BoundFraction::from(1),
        |value| BoundFraction::from((value).owned()),
    );
    NumberLeaf::modulus_covers(big.multiple_of.as_ref(), Some(&lifted_modulus))
        && big.not_multiple_of.iter().all(|modulus| {
            number_not_multiple_of_to_integer(modulus).is_some_and(|excluded| {
                small
                    .not_multiple_of
                    .iter()
                    .any(|finer| IntegerLeaf::modulus_covers(Some(finer), Some(&excluded)))
            })
        })
}

fn integer_bounds_as_fraction(bounds: &IntegerBounds) -> NumberBounds {
    NumberBounds {
        minimum: bounds
            .minimum
            .as_ref()
            .map(|v| BoundFraction::from((v).owned())),
        maximum: bounds
            .maximum
            .as_ref()
            .map(|v| BoundFraction::from((v).owned())),
        exclusive_minimum: bounds.exclusive_minimum,
        exclusive_maximum: bounds.exclusive_maximum,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use test_case::test_case;

    use crate::{canonical::options, canonicalize};

    // Array-leaf coverage facets through `array_leaf_covers`: missing duplicate requirement,
    // a `maxItems` making a prefix slot vacuous, and an incompatible prefix slot.
    #[test_case(
        json!({"type": "array", "minItems": 2, "items": {"type": "integer"}}),
        json!({"type": "array", "not": {"uniqueItems": true}}),
        Some(false)
        ; "missing_duplicate_requirement_declines")]
    #[test_case(
        json!({"type": "array", "maxItems": 1, "prefixItems": [{"type": "integer"}]}),
        json!({"type": "array", "prefixItems": [{"type": "integer"}, {"type": "string"}]}),
        Some(true)
        ; "capped_length_makes_prefix_slot_vacuous")]
    #[test_case(
        json!({"type": "array", "prefixItems": [{"type": "string"}]}),
        json!({"type": "array", "prefixItems": [{"type": "integer"}]}),
        Some(false)
        ; "incompatible_prefix_slot_declines")]
    #[allow(clippy::needless_pass_by_value)]
    fn array_leaf_subschema(small: Value, big: Value, expected: Option<bool>) {
        let small = canonicalize(&small).expect("small");
        let big = canonicalize(&big).expect("big");
        assert_eq!(small.is_subschema_of(&big), expected);
    }

    // Leaves with the same `$ref` URI but incompatible definitions: `nodes_equal` declines the aliased
    // subtree, and the `big == small` guard keeps the verdict inconclusive instead of unsound `Some(true)`.
    #[test_case(
        json!({"type": "object", "properties": {"a": {"$ref": "#/$defs/A"}}, "$defs": {"A": {"type": "integer"}}}),
        json!({"type": "object", "properties": {"a": {"$ref": "#/$defs/A"}}, "$defs": {"A": {"type": "string"}}})
        ; "object")]
    #[test_case(
        json!({"type": "array", "items": {"$ref": "#/$defs/A"}, "$defs": {"A": {"type": "integer"}}}),
        json!({"type": "array", "items": {"$ref": "#/$defs/A"}, "$defs": {"A": {"type": "string"}}})
        ; "array")]
    #[allow(clippy::needless_pass_by_value)]
    fn aliased_refs_under_incompatible_definitions_are_inconclusive(left: Value, right: Value) {
        let left = options()
            .with_inline_budget(0)
            .canonicalize(&left)
            .expect("left");
        let right = options()
            .with_inline_budget(0)
            .canonicalize(&right)
            .expect("right");
        assert_eq!(left.is_subschema_of(&right), None);
        assert_eq!(right.is_subschema_of(&left), None);
    }

    // String-leaf coverage facets through `string_leaf_covers`: `small ⊆ big` holds when every facet
    // `big` carries is at least as permissive as `small`'s.
    #[test_case(
        json!({"type": "string", "minLength": 5}), json!({"type": "string", "minLength": 2}), Some(true)
        ; "tighter_min_length_is_covered")]
    #[test_case(
        json!({"type": "string", "maxLength": 2}), json!({"type": "string", "maxLength": 5}), Some(true)
        ; "tighter_max_length_is_covered")]
    #[test_case(
        json!({"type": "string", "minLength": 1}),
        json!({"type": "string", "minLength": 1, "contentEncoding": "base64"}), None
        ; "extra_content_facet_declines")]
    #[test_case(
        json!({"type": "string", "pattern": "(?=a)b"}), json!({"type": "string", "pattern": "abc"}), None
        ; "regex_engine_mismatch_declines")]
    #[allow(clippy::needless_pass_by_value)]
    fn string_leaf_subschema(small: Value, big: Value, expected: Option<bool>) {
        let small = canonicalize(&small).expect("small");
        let big = canonicalize(&big).expect("big");
        assert_eq!(small.is_subschema_of(&big), expected);
    }

    // A union repeated across two properties: the first query fills the definition-scoped coverage memo
    // and the second hits it (the `$ref` in `small` makes the prover env non-empty).
    #[test]
    fn repeated_union_property_hits_definition_scoped_memo() {
        let big = canonicalize(&json!({"type": "object", "properties": {
            "p1": {"anyOf": [{"type": "string", "minLength": 1}, {"type": "integer", "minimum": 0}]},
            "p2": {"anyOf": [{"type": "string", "minLength": 1}, {"type": "integer", "minimum": 0}]}
        }}))
        .expect("big");
        let small = options()
            .with_inline_budget(0)
            .canonicalize(&json!({"type": "object", "properties": {
                "p1": {"type": "string", "minLength": 1},
                "p2": {"type": "string", "minLength": 1},
                "p3": {"$ref": "#/$defs/J"}
            }, "$defs": {"J": {"type": "boolean"}}}))
            .expect("small");
        assert_eq!(small.is_subschema_of(&big), Some(true));
    }

    // Drives a `contentEncoding`-carrying string through facet-spread over an unmerged `allOf` (content facet of `string_leaf_facets`).
    #[test]
    fn string_content_facet_spreads_over_allof() {
        let big =
            canonicalize(&json!({"type": "string", "contentEncoding": "base64", "minLength": 1}))
                .expect("big");
        let small = options()
            .with_inline_budget(0)
            .canonicalize(&json!({
                "allOf": [{"type": "string", "minLength": 3}, {"$ref": "#/$defs/X"}],
                "$defs": {"X": {"type": "string"}}
            }))
            .expect("small");
        // `small` admits non-base64 strings, so it is not a subschema of the encoding-constrained `big`.
        assert_ne!(small.is_subschema_of(&big), Some(true));
    }

    // `covers` underlies `is_subschema_of`. Each restrictive schema is its open type minus a negation
    // residual; the open type covers it, yet the residual keeps the open type outside the restriction.
    #[test_case(json!({"type": "integer"}), json!({"type": "integer", "not": {"multipleOf": 2}}); "integer_excludes_multiple")]
    #[test_case(json!({"type": "number"}), json!({"type": "number", "not": {"multipleOf": 0.5}}); "number_excludes_multiple")]
    #[test_case(json!({"type": "string"}), json!({"type": "string", "not": {"pattern": "^a"}}); "string_excludes_pattern")]
    #[test_case(json!({"type": "array"}), json!({"type": "array", "not": {"uniqueItems": true}}); "array_excludes_unique")]
    #[allow(clippy::needless_pass_by_value)]
    fn open_type_covers_residual_restriction(open: Value, restrictive: Value) {
        let open = canonicalize(&open).expect("valid schema");
        let restrictive = canonicalize(&restrictive).expect("valid schema");
        // The open type covers the restriction: every restricted value still has that type.
        assert_eq!(restrictive.is_subschema_of(&open), Some(true));
        // The restriction does not cover the open type: the excluded residual stays outside it.
        assert_ne!(open.is_subschema_of(&restrictive), Some(true));
    }
}

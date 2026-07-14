//! Type-guard and multi-type fold/split machinery.

use std::sync::Arc;

use ahash::AHashSet;

use crate::{
    canonical::{
        context::CanonicalizationContext,
        intern::shared,
        intersect::{is_unmergeable_guard_value_pair, multi_type_or_false, open_typed_leaf},
        ir::{ArrayLeaf, IntegerLeaf, NumberLeaf, ObjectLeaf, Schema, SharedSchema, StringLeaf},
        negate::negate_type_guard,
    },
    JsonType, JsonTypeSet,
};

use super::combinators::{replace_pair_with, try_pairwise_intersect};

pub(super) fn intersect_type_guard_siblings(
    branches: &mut Vec<SharedSchema>,
    ctx: &CanonicalizationContext,
) -> bool {
    try_pairwise_intersect(
        branches,
        ctx,
        |branch| match branch.as_schema() {
            Schema::TypeGuard { ty, body } => Some((*ty, Arc::clone(body))),
            _ => None,
        },
        |(ty, body), other| type_guard_can_merge(*ty, body, other, ctx),
        false,
    )
}

fn type_guard_can_merge(
    guard_ty: JsonType,
    guard_body: &SharedSchema,
    other: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> bool {
    match other.as_schema() {
        Schema::TypeGuard { ty, .. } => *ty == guard_ty,
        // Value sets filter through only when every membership verdict is decidable; else it bails to `AllOf`
        // and the fold must not recurse.
        Schema::Const(_) | Schema::Enum(_) => {
            !is_unmergeable_guard_value_pair(guard_ty, guard_body, other, ctx)
        }
        // Type sets and unions distribute; `null`/boolean never share the guard's kind. None stall.
        Schema::MultiType(_) | Schema::AnyOf(_) | Schema::Null | Schema::Boolean(_) => true,
        _ => other
            .as_typed_view()
            .is_some_and(|view| !guard_ty.overlaps(view.ty) || guard_ty.covers(view.ty)),
    }
}

/// A multi-type branch next to a union distributes in: members outside the type set drop, the rest intersect.
/// Restricted to `AnyOf` siblings - a multi-type-vs-leaf pair re-wraps as `AllOf` and would re-trigger this pass.
///
/// ```text
/// BEFORE: {"allOf": [{"type": ["integer", "string"]},
///                    {"anyOf": [{"type": "integer"}, {"type": "object"}]}]}
/// AFTER:  {"type": "integer"}   // the object member falls outside {integer, string}
/// ```
pub(super) fn intersect_multi_type_with_any_of_sibling(
    branches: &mut Vec<SharedSchema>,
    ctx: &CanonicalizationContext,
) -> bool {
    try_pairwise_intersect(
        branches,
        ctx,
        |branch| matches!(branch.as_schema(), Schema::MultiType(_)).then_some(()),
        |(), other| matches!(other.as_schema(), Schema::AnyOf(_)),
        false,
    )
}

/// A `type` list beside a typed leaf restricts the leaf to the kinds the list admits: the leaf's
/// own kind survives, sibling kinds collapse to `false`.
///
/// ```text
/// BEFORE: {"allOf": [{"type": ["integer", "string"]}, {"type": "number", "minimum": 0, "maximum": 1}]}
/// AFTER:  {"enum": [0, 1]}   // only the integers in [0, 1] are both
/// ```
pub(super) fn intersect_multi_type_with_typed_sibling(
    branches: &mut Vec<SharedSchema>,
    ctx: &CanonicalizationContext,
) -> bool {
    try_pairwise_intersect(
        branches,
        ctx,
        |branch| matches!(branch.as_schema(), Schema::MultiType(_)).then_some(()),
        |(), other| {
            matches!(
                other.as_schema(),
                Schema::Integer(_)
                    | Schema::Number(_)
                    | Schema::String(_)
                    | Schema::Boolean(_)
                    | Schema::Null
                    | Schema::Array(_)
                    | Schema::Object(_)
            )
        },
        false,
    )
}

/// A body `L` (admitting only kind-`k` values) unioned with "every kind except `k`" is exactly the type guard
/// `TypeGuard{k, L}` - the canonical spelling that `negate(negate(guard))` must return to.
///
/// ```text
/// BEFORE: {"anyOf": [{"type": "integer", "minimum": 0},
///                    {"type": ["null", "boolean", "number", "string", "array", "object"]}]}
/// AFTER:  TypeGuard{integer, {minimum: 0}}   // non-integers pass, integers must be >= 0
/// ```
pub(super) fn fold_type_guard_any_of(branches: &mut Vec<SharedSchema>) -> bool {
    if branches.len() != 2 {
        return false;
    }
    for complement_index in 0..branches.len() {
        let Some(ty) = guarded_kind_from_type_complement(branches[complement_index].as_schema())
        else {
            continue;
        };
        let body_index = 1 - complement_index;
        if !guard_spelling_is_canonical(ty, &branches[body_index]) {
            continue;
        }
        let body = Arc::clone(&branches[body_index]);
        *branches = vec![shared(Schema::TypeGuard { ty, body })];
        return true;
    }
    false
}

/// A conjunction of disjoint guards in a union splits into its kind partition (each kind keeps its body, others
/// pass), letting surrounding type sets merge. The re-fold below is its inverse, like the single-guard split.
///
/// ```text
/// BEFORE: {"anyOf": [{"type": ["null", "boolean", "integer", "array", "object"]},
///                    {"allOf": [{"multipleOf": 1.5}, {"pattern": "^a"}]}]}
/// AFTER:  {"anyOf": [{"type": ["null", "boolean", "integer", "array", "object"]},
///                    {"type": "number", "multipleOf": 1.5}, {"type": "string", "pattern": "^a"},
///                    {"type": ["null", "boolean", "array", "object"]}]}
/// ```
pub(super) fn split_guard_conjunction_in_any_of(branches: &mut Vec<SharedSchema>) -> bool {
    if branches.len() < 2 {
        return false;
    }
    for index in 0..branches.len() {
        let Schema::AllOf(members) = branches[index].as_schema() else {
            continue;
        };
        let mut guards: Vec<(JsonType, SharedSchema)> = Vec::with_capacity(members.len());
        for member in members {
            let Schema::TypeGuard { ty, body } = member.as_schema() else {
                guards.clear();
                break;
            };
            if guards.iter().any(|(seen, _)| seen.overlaps(*ty)) {
                guards.clear();
                break;
            }
            guards.push((*ty, Arc::clone(body)));
        }
        if guards.len() < 2 {
            continue;
        }
        let union = guards
            .iter()
            .fold(JsonTypeSet::empty(), |acc, (ty, _)| acc.insert(*ty));
        let Some(complement) = Schema::type_set_complement(union) else {
            continue;
        };
        branches.remove(index);
        for (ty, body) in guards {
            branches.push(shared(Schema::TypedGroup { ty, body }));
        }
        branches.push(multi_type_or_false(complement));
        return true;
    }
    false
}

/// A union partitioning the JSON types among disjoint guard-canonical branches plus the exact complement `MultiType`
/// is a guard conjunction's negation image; re-fold so both build orders reach the `allOf` of `TypeGuard`s.
///
/// ```text
/// BEFORE: {"anyOf": [{"type": "string", "pattern": "^a"},
///                    {"type": "array", "uniqueItems": true},
///                    {"type": ["null", "boolean", "number", "object"]}]}
/// AFTER:  {"allOf": [{"pattern": "^a"}, {"uniqueItems": true}]}
/// ```
pub(super) fn fold_guard_conjunction_any_of(branches: &mut Vec<SharedSchema>) -> bool {
    if branches.len() < 2 {
        return false;
    }
    let mut complement: Option<(usize, JsonTypeSet)> = None;
    for (index, branch) in branches.iter().enumerate() {
        if let Schema::MultiType(set) = branch.as_schema() {
            if complement.is_some() {
                return false;
            }
            complement = Some((index, *set));
        }
    }
    let Some((complement_index, mut complement_set)) = complement else {
        return false;
    };
    let mut guards: Vec<(JsonType, SharedSchema)> = Vec::with_capacity(branches.len());
    // A bare `integer` member (no `number`) is the integer-valued-number guard in partition form:
    // it admits exactly the numbers `TypeGuard{number, open-integer}` admits.
    if complement_set
        .iter()
        .any(|member| member == JsonType::Integer)
    {
        complement_set = complement_set
            .iter()
            .filter(|member| *member != JsonType::Integer)
            .fold(JsonTypeSet::empty(), JsonTypeSet::insert);
        guards.push((
            JsonType::Number,
            shared(Schema::Integer(IntegerLeaf::default())),
        ));
    }
    if branches.len() + guards.len() < 3 {
        return false;
    }
    for (index, branch) in branches.iter().enumerate() {
        if index == complement_index {
            continue;
        }
        let Some(view) = branch.as_typed_view() else {
            return false;
        };
        // An integer-valued branch occupies the number slot of the partition (its guard spelling
        // is `TypeGuard{number, integer-leaf}`; `integer` itself has no type-set complement).
        let slot = view.ty.guard_slot();
        if guards.iter().any(|(seen, _)| seen.overlaps(slot)) {
            return false;
        }
        // Same restriction as the one-guard fold: only `negate_type_guard`-invertible bodies take
        // the guard spelling; general leaf complements must stay as `AnyOf`.
        if !guard_spelling_is_canonical(slot, &view.schema) {
            return false;
        }
        guards.push((slot, view.schema));
    }
    let union = guards
        .iter()
        .fold(JsonTypeSet::empty(), |acc, (ty, _)| acc.insert(*ty));
    if Schema::type_set_complement(union) != Some(complement_set) {
        return false;
    }
    *branches = vec![shared(Schema::AllOf(
        guards
            .into_iter()
            .map(|(ty, body)| shared(Schema::TypeGuard { ty, body }))
            .collect(),
    ))];
    true
}

/// A `TypeGuard` in a 2+-branch union splits into its body plus the complement type set, so parse-keeps-guard and
/// negation-splits reach one fixpoint. The two-branch re-fold above inverts it once the union is `body ∪ complement`.
///
/// ```text
/// BEFORE: {"anyOf": [{"required": ["a"]}, {"maxProperties": 0, "type": "object"}]}
/// AFTER:  {"anyOf": [{"required": ["a"], "type": "object"},
///                    {"maxProperties": 0, "type": "object"},
///                    {"type": ["null", "boolean", "number", "string", "array"]}]}
/// ```
pub(super) fn split_type_guard_in_any_of(branches: &mut Vec<SharedSchema>) -> bool {
    if branches.len() < 2 {
        return false;
    }
    let Some(guard_index) = branches
        .iter()
        .position(|branch| matches!(branch.as_schema(), Schema::TypeGuard { .. }))
    else {
        return false;
    };
    let Schema::TypeGuard { ty, body } = branches[guard_index].as_schema() else {
        unreachable!("position matched a TypeGuard");
    };
    let Some(complement) = Schema::type_set_complement(JsonTypeSet::from(*ty)) else {
        // `Integer` has no type-set complement (`integer` is a subtype of `number`).
        return false;
    };
    let replacement = shared(Schema::TypedGroup {
        ty: *ty,
        body: Arc::clone(body),
    });
    branches[guard_index] = replacement;
    branches.push(shared(Schema::MultiType(complement)));
    true
}

/// Only bodies with a closed guard negation (`negate_type_guard` has an IR dual), plus numeric-const, take the
/// guard spelling; general leaf complements stay `AnyOf`, the spelling `negate(leaf)` round-trips through.
fn guard_spelling_is_canonical(ty: JsonType, body: &SharedSchema) -> bool {
    if ty == JsonType::Number && matches!(body.as_schema(), Schema::Const(_)) {
        return true;
    }
    negate_type_guard(ty, body).is_some()
}

fn guarded_kind_from_type_complement(schema: &Schema) -> Option<JsonType> {
    let complement = Schema::type_set_complement(schema.as_type_set()?)?;
    if complement.len() == 1 {
        complement.iter().next()
    } else {
        None
    }
}

/// `True` when intersecting every determinable branch type domain is empty - no value inhabits all branches, so the
/// `AllOf` is `False`. Catches constrained-leaf-vs-`MultiType` disjointness [`intersect_multi_type_siblings`] misses.
pub(super) fn all_of_type_domain_empty(branches: &[SharedSchema]) -> bool {
    let mut domain: Option<JsonTypeSet> = None;
    for branch in branches {
        let Some(set) = branch.as_schema().type_domain() else {
            continue;
        };
        domain = Some(match domain {
            Some(prev) => Schema::type_set_intersect(prev, set),
            None => set,
        });
    }
    matches!(domain, Some(set) if set.is_empty())
}

/// `allOf` siblings that are multi-type leaves or `not` of a type set collapse by set arithmetic: positives
/// intersect, negatives subtract. An empty result is `false`; a single surviving type is a bare open leaf.
///
/// ```text
/// BEFORE: {"allOf": [{"type": ["integer", "string", "boolean"]}, {"not": {"type": "string"}}]}
/// AFTER:  {"type": ["boolean", "integer"]}
/// ```
pub(super) fn intersect_multi_type_siblings(branches: &mut Vec<SharedSchema>) -> bool {
    let mut positive_indices: Vec<usize> = Vec::new();
    let mut negative_indices: Vec<usize> = Vec::new();
    let mut combined: Option<JsonTypeSet> = None;
    let mut negatives: Vec<JsonTypeSet> = Vec::new();
    for (i, branch) in branches.iter().enumerate() {
        // Open typed leaves are positives (`as_type_set` gives their single-type set); constrained leaves return `None`.
        if let Some(set) = branch.as_schema().as_type_set() {
            combined = Some(match combined {
                Some(prev) => Schema::type_set_intersect(prev, set),
                None => set,
            });
            positive_indices.push(i);
            continue;
        }
        if let Schema::Not(inner) = branch.as_schema() {
            if let Some(set) = inner.as_schema().as_type_set() {
                negatives.push(set);
                negative_indices.push(i);
            }
        }
    }
    // Need a positive to anchor subtractions; "all - X" may not be representable.
    let Some(mut result) = combined else {
        return false;
    };
    for negative in &negatives {
        let Some(next) = Schema::type_set_subtract(result, *negative) else {
            return false;
        };
        result = next;
    }
    if positive_indices.len() + negative_indices.len() < 2 {
        return false;
    }
    let mut indices = positive_indices;
    indices.extend(negative_indices);
    indices.sort_unstable_by(|a, b| b.cmp(a));
    for idx in indices {
        branches.remove(idx);
    }
    if result.is_empty() {
        branches.push(shared(Schema::False));
    } else if result.len() == 1 {
        let only = result.iter().next().expect("len == 1");
        branches.push(open_typed_leaf(only));
    } else {
        branches.push(shared(Schema::MultiType(result)));
    }
    true
}

/// Pure type-marker `anyOf` branches (open leaf, `const null`, saturating boolean enum, multi-type) fold into one
/// branch holding their type union, leaving constrained siblings - making the form independent of grouping.
///
/// ```text
/// BEFORE: {"anyOf": [{"type": "integer"}, {"type": "string"}, {"type": "string", "minLength": 1}]}
/// AFTER:  {"anyOf": [{"type": "string", "minLength": 1}, {"type": ["integer", "string"]}]}
/// ```
pub(super) fn fold_to_multi_type(branches: &mut Vec<SharedSchema>) -> bool {
    let mut indices: Vec<usize> = Vec::new();
    let mut combined = JsonTypeSet::empty();
    for (i, branch) in branches.iter().enumerate() {
        if let Some(set) = branch.as_schema().as_type_set() {
            combined = combined.union(set);
            indices.push(i);
        }
    }
    if indices.len() < 2 {
        return false;
    }
    let combined = Schema::canonical_type_set(combined);
    for idx in indices.into_iter().rev() {
        branches.remove(idx);
    }
    // `multi_type_or_false` also folds a universe-covering set to `True`.
    branches.push(multi_type_or_false(combined));
    true
}

pub(super) fn merge_dual_facet_leaves(branches: &mut Vec<SharedSchema>) -> bool {
    fn open_leaf(kind: JsonType) -> Option<SharedSchema> {
        Some(shared(match kind {
            JsonType::String => Schema::String(StringLeaf::default()),
            JsonType::Number => Schema::Number(NumberLeaf::default()),
            JsonType::Array => Schema::Array(ArrayLeaf::default()),
            JsonType::Object => Schema::Object(ObjectLeaf::default()),
            _ => return None,
        }))
    }
    for left in 0..branches.len() {
        let Some(kind) = branches[left].as_schema().pinned_kind() else {
            continue;
        };
        // The guard duals are exactly the in-kind complements of single-facet leaves.
        let guard_kind = kind.guard_slot();
        let Some(negated) = negate_type_guard(guard_kind, &branches[left]) else {
            continue;
        };
        let Some(merged) = open_leaf(guard_kind) else {
            continue;
        };
        for right in 0..branches.len() {
            if right == left {
                continue;
            }
            if *branches[right] == *negated {
                return replace_pair_with(branches, left, right, merged);
            }
        }
    }
    false
}

/// Type guards of disjoint kinds union to everything: each passes every value outside its own kind, so together a
/// value of any kind is passed by at least one of them.
///
/// ```text
/// BEFORE: {"anyOf": [TypeGuard{integer, ...}, TypeGuard{string, ...}]}
/// AFTER:  true   // a string passes the integer guard, an integer passes the string guard, anything else passes both
/// ```
pub(super) fn disjoint_kind_guards_cover_everything(branches: &[SharedSchema]) -> bool {
    let mut guard_kinds: Vec<JsonType> = Vec::new();
    for branch in branches {
        if let Schema::TypeGuard { ty, .. } = branch.as_schema() {
            if guard_kinds.iter().any(|seen| !seen.overlaps(*ty)) {
                return true;
            }
            guard_kinds.push(*ty);
        }
    }
    false
}

/// True when every JSON type appears as an unconstrained branch.
pub(super) fn covers_every_json_type(branches: &[SharedSchema]) -> bool {
    let mut covered: AHashSet<JsonType> = AHashSet::new();
    for branch in branches {
        let Some(ty) = branch.as_schema().single_unconstrained_type() else {
            continue;
        };
        covered.insert(ty);
    }
    covered.len() == JsonTypeSet::all().len()
}

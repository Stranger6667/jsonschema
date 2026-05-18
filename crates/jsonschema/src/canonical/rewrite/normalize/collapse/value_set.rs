//! Const/enum value-set collapse passes.

use std::sync::Arc;

use ahash::AHashSet;

use crate::{
    canonical::{
        const_enum::intern_value_set,
        context::CanonicalizationContext,
        coverage::any_sibling_covers,
        intern::shared,
        intersect::intersect_typed_with_value_set,
        ir::{BooleanBounds, CanonicalJson, Schema, SharedSchema},
    },
    JsonType, JsonTypeSet,
};

use super::combinators::{replace_pair_with, try_pairwise_intersect};

/// A `const`/`enum` has no pinned kind, so the typed passes skip it. Fold contradictory `const`/`enum` pairs (and
/// `const`/`enum` vs union); limited to siblings `intersect_canonical` always resolves, else it recurses forever.
///
/// ```text
/// BEFORE: {"allOf": [{"const": 1}, {"const": 2}]}
/// AFTER:  false
/// ```
pub(super) fn intersect_value_set_siblings(
    branches: &mut Vec<SharedSchema>,
    ctx: &CanonicalizationContext,
) -> bool {
    try_pairwise_intersect(
        branches,
        ctx,
        |branch| matches!(branch.as_schema(), Schema::Const(_) | Schema::Enum(_)).then_some(()),
        |(), other| {
            matches!(
                other.as_schema(),
                Schema::Const(_) | Schema::Enum(_) | Schema::AnyOf(_)
            )
        },
        false,
    )
}

/// A `const`/`enum` next to a typed leaf keeps only the values the leaf admits. The membership check is non-recursive
/// (returns `None` for undecidable values), so unlike `intersect_canonical` it never bails to `AllOf` and recurses.
///
/// ```text
/// BEFORE: {"allOf": [{"enum": [1, 2, "x"]}, {"type": "integer"}]}
/// AFTER:  {"enum": [1, 2]}   // "x" is not an integer
/// ```
pub(super) fn intersect_value_set_with_typed_sibling(
    branches: &mut Vec<SharedSchema>,
    ctx: &CanonicalizationContext,
) -> bool {
    for outer in 0..branches.len() {
        if !matches!(
            branches[outer].as_schema(),
            Schema::Const(_) | Schema::Enum(_)
        ) {
            continue;
        }
        for inner in 0..branches.len() {
            if inner == outer {
                continue;
            }
            let Some(merged) =
                intersect_typed_with_value_set(&branches[outer], &branches[inner], ctx)
            else {
                continue;
            };
            return replace_pair_with(branches, outer, inner, merged);
        }
    }
    false
}

/// In a union, drop `const`/`enum` members a sibling branch already accepts - they add nothing. Keeps `anyOf`
/// canonical no matter how the value set was built, which is what makes intersection commutative after canonicalize.
///
/// ```text
/// BEFORE: {"anyOf": [{"type": "string"}, {"enum": ["", false]}]}
/// AFTER:  {"anyOf": [{"type": "string"}, {"const": false}]}   // "" already matches the string branch
/// ```
pub(super) fn drop_covered_value_set_members(
    branches: &mut Vec<SharedSchema>,
    ctx: &CanonicalizationContext,
) -> bool {
    let mut changed = false;
    let mut index = 0;
    while index < branches.len() {
        let values: Vec<CanonicalJson> = match branches[index].as_schema() {
            Schema::Const(value) => vec![value.clone()],
            Schema::Enum(values) => values.clone(),
            _ => {
                index += 1;
                continue;
            }
        };
        let kept: Vec<CanonicalJson> = values
            .iter()
            .filter(|value| {
                let probe = shared(Schema::Const((*value).clone()));
                !any_sibling_covers(branches, &[index], &probe, ctx)
            })
            .cloned()
            .collect();
        if kept.len() == values.len() {
            index += 1;
            continue;
        }
        changed = true;
        if kept.is_empty() {
            branches.remove(index);
        } else {
            branches[index] = intern_value_set(kept);
            index += 1;
        }
    }
    changed
}

/// Merge sibling `Enum`/`Const` branches (and the finite `{null, boolean}` `MultiType`) into one value-set union,
/// so both build orders converge with the emit/parse path.
///
/// ```text
/// BEFORE: {"anyOf": [{"enum": [1, 2]}, {"const": 3}]}
/// AFTER:  {"enum": [1, 2, 3]}
/// ```
pub(super) fn merge_enum_branches(branches: &mut Vec<SharedSchema>) -> bool {
    let value_set_count = branches
        .iter()
        .filter(|branch| {
            matches!(branch.as_schema(), Schema::Enum(_) | Schema::Const(_))
                || finite_multi_type_values(branch.as_schema()).is_some()
        })
        .count();
    if value_set_count <= 1 {
        return false;
    }
    let mut merged: Vec<CanonicalJson> = Vec::new();
    let mut seen: AHashSet<CanonicalJson> = AHashSet::new();
    let mut push = |value: &CanonicalJson, merged: &mut Vec<CanonicalJson>| {
        if seen.insert(value.clone()) {
            merged.push(value.clone());
        }
    };
    branches.retain(|branch| match branch.as_schema() {
        Schema::Enum(values) => {
            for value in values {
                push(value, &mut merged);
            }
            false
        }
        Schema::Const(value) => {
            push(value, &mut merged);
            false
        }
        other => match finite_multi_type_values(other) {
            Some(values) => {
                for value in values {
                    push(&value, &mut merged);
                }
                false
            }
            None => true,
        },
    });
    branches.push(intern_value_set(merged));
    true
}

/// The value set a `MultiType` denotes, when every member type has a finite domain. Null and
/// boolean are the only such types, so `{null, boolean}` is the only inhabitant.
fn finite_multi_type_values(schema: &Schema) -> Option<[CanonicalJson; 3]> {
    let Schema::MultiType(set) = schema else {
        return None;
    };
    let null_boolean = JsonTypeSet::from(JsonType::Null).insert(JsonType::Boolean);
    (*set == null_boolean).then(|| {
        ["false", "null", "true"]
            .map(|text| CanonicalJson::from_canonical_text(std::sync::Arc::from(text)))
    })
}

/// Union branches that are conjunctions of the same schemas plus a value set each factor into one conjunction over
/// the merged values - the inverse of negation distributing an undecidable conjunction-with-`enum` over its members.
///
/// ```text
/// BEFORE: {"anyOf": [{"allOf": [{"format": "email", "type": "string"}, {"const": ""}]},
///                    {"allOf": [{"format": "email", "type": "string"}, {"const": "0"}]}]}
/// AFTER:  {"allOf": [{"format": "email", "type": "string"}, {"enum": ["", "0"]}]}
/// ```
pub(super) fn factor_value_set_conjunctions(branches: &mut Vec<SharedSchema>) -> bool {
    fn split(branch: &SharedSchema) -> Option<(Vec<&SharedSchema>, Vec<&CanonicalJson>)> {
        let Schema::AllOf(members) = branch.as_schema() else {
            return None;
        };
        let mut rest: Vec<&SharedSchema> = Vec::new();
        let mut values: Option<Vec<&CanonicalJson>> = None;
        for member in members {
            match member.as_schema() {
                Schema::Const(value) if values.is_none() => {
                    values = Some(vec![value]);
                }
                Schema::Enum(entries) if values.is_none() => {
                    values = Some(entries.iter().collect());
                }
                Schema::Const(_) | Schema::Enum(_) => return None,
                _ => rest.push(member),
            }
        }
        values.map(|values| (rest, values))
    }

    for left_index in 0..branches.len() {
        let Some((left_rest, left_values)) = split(&branches[left_index]) else {
            continue;
        };
        for right_index in left_index + 1..branches.len() {
            let Some((right_rest, right_values)) = split(&branches[right_index]) else {
                continue;
            };
            if left_rest != right_rest {
                continue;
            }
            let mut merged: Vec<CanonicalJson> = left_values
                .iter()
                .chain(right_values.iter())
                .map(|value| (*value).clone())
                .collect();
            merged.sort_unstable();
            merged.dedup();
            let mut conjuncts: Vec<SharedSchema> =
                left_rest.iter().map(|member| Arc::clone(member)).collect();
            conjuncts.push(intern_value_set(merged));
            let replacement = shared(Schema::AllOf(conjuncts));
            branches[left_index] = replacement;
            branches.remove(right_index);
            return true;
        }
    }
    false
}

/// Enum values covering a complete finite type (`null`, or both booleans) split into a type-set branch when a
/// type-set sibling exists - which guarantees `fold_to_multi_type` consumes the extracted leaf (no oscillation).
///
/// ```text
/// BEFORE: {"anyOf": [{"type": "string"}, {"enum": [null, 0]}]}
/// AFTER:  {"anyOf": [{"const": 0}, {"type": ["null", "string"]}]}   // `null` folds into the type set
/// ```
pub(super) fn split_finite_type_values_from_enum(branches: &mut Vec<SharedSchema>) -> bool {
    let has_type_set_sibling = |branches: &[SharedSchema], skip: usize| {
        branches
            .iter()
            .enumerate()
            .any(|(index, branch)| index != skip && branch.as_schema().as_type_set().is_some())
    };
    for idx in 0..branches.len() {
        let Schema::Enum(values) = branches[idx].as_schema() else {
            continue;
        };
        if !has_type_set_sibling(branches, idx) {
            continue;
        }
        let has_null_domain = values.iter().any(|value| value.as_str() == "null");
        let has_false = values.iter().any(|value| value.as_str() == "false");
        let has_true = values.iter().any(|value| value.as_str() == "true");
        let has_complete_boolean_domain = has_false && has_true;
        if !has_null_domain && !has_complete_boolean_domain {
            continue;
        }
        let remaining_values: Vec<CanonicalJson> = values
            .iter()
            .filter(|value| {
                let text = value.as_str();
                !(has_null_domain && text == "null")
                    && !(has_complete_boolean_domain && (text == "false" || text == "true"))
            })
            .cloned()
            .collect();
        let mut extracted_type_branches: Vec<SharedSchema> = Vec::new();
        if has_null_domain {
            extracted_type_branches.push(shared(Schema::Null));
        }
        if has_complete_boolean_domain {
            extracted_type_branches.push(shared(Schema::Boolean(BooleanBounds::Any)));
        }
        if remaining_values.is_empty() {
            branches.remove(idx);
        } else {
            branches[idx] = intern_value_set(remaining_values);
        }
        branches.extend(extracted_type_branches);
        return true;
    }
    false
}

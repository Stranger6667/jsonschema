use std::{
    cmp::Ordering,
    sync::{Arc, LazyLock},
};

use ahash::AHashSet;

use crate::{
    canonical::{
        context::{keeps_draft4_integer_guard, CanonicalizationContext, WalkStage},
        intern::shared,
        ir::{BooleanBounds, CanonicalJson, Schema, SchemaKindSet, SharedSchema},
        walk::map_children,
    },
    JsonType, JsonTypeSet,
};

/// Standardizes value sets: an `enum` singleton becomes a `const`, a set that *saturates* two or more types (holds
/// every value of each) becomes a `type` list, and a sibling type pin filters out members of the wrong type.
///
/// ```text
/// BEFORE: {"enum": [5]}
/// AFTER:  {"const": 5}
///
/// BEFORE: {"enum": [null, true, false]}   (every null and boolean value)
/// AFTER:  {"type": ["null", "boolean"]}
///
/// BEFORE: {"type": "integer", "enum": [1, "x", 2]}
/// AFTER:  {"enum": [1, 2]}
/// ```
#[must_use]
pub(crate) fn canonicalize(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
    super::run::<ConstEnumStage>(schema, ctx)
}

pub(crate) struct ConstEnumStage;

impl super::StructuralStage for ConstEnumStage {
    const WALK: WalkStage = WalkStage::ConstEnum;
    // No gate: any node may carry a saturating leaf or a `Const`/`Enum` to fold.
    const MASK: SchemaKindSet = SchemaKindSet::empty();

    fn rewrite(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
        canonicalize_impl(schema, ctx)
    }
}

fn canonicalize_impl(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
    if let Some(promoted) = promote_single_value_leaf(schema) {
        return promoted;
    }
    if let Schema::Enum(values) = schema.as_schema() {
        // Standardize on `Const` for singletons; empty `Enum` is unsatisfiable.
        match values.len() {
            0 => return shared(Schema::False),
            1 => {
                return shared(Schema::Const(values.first().expect("len == 1").clone()));
            }
            _ => {
                // Sets saturating 2+ types collapse to `MultiType`. Single-type saturating sets (e.g.
                // `{false, true}` = boolean) stay `Enum`; `promote_single_value_leaf` is their canonical form.
                if let Some(set) = Schema::Enum(values.clone()).as_type_set() {
                    if set.len() >= 2 {
                        return shared(Schema::MultiType(set));
                    }
                }
                if let Some(sorted) = canonical_sort(values) {
                    return shared(Schema::Enum(sorted));
                }
            }
        }
    }
    let recursed = map_children(schema, |s| canonicalize(s, ctx));
    if let Schema::TypedGroup { ty, body } = recursed.as_schema() {
        if let Some(replacement) = collapse_typed_value_set(*ty, body, ctx) {
            return replacement;
        }
    }
    if let Schema::AllOf(branches) = recursed.as_schema() {
        if let Some(replacement) = filter_enums_by_type(branches) {
            return replacement;
        }
    }
    recursed
}

fn collapse_typed_value_set(
    kind: JsonType,
    body: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> Option<SharedSchema> {
    match body.as_schema() {
        Schema::Const(value) => {
            if !kind.covers(value.json_type()) {
                Some(shared(Schema::False))
            } else if keeps_draft4_integer_guard(kind, ctx.draft()) {
                None
            } else {
                Some(Arc::clone(body))
            }
        }
        Schema::Enum(values) => Some(finalize_typed_value_set(
            intern_value_set(filter_enum_values(values, kind)),
            kind,
            ctx,
        )),
        _ => None,
    }
}

/// Drop the surrounding type guard unless a Draft 4 integer set needs it (see [`keeps_draft4_integer_guard`]).
/// An unsatisfiable set is left bare since `Schema::False` matches nothing under any draft.
pub(crate) fn finalize_typed_value_set(
    value_set: SharedSchema,
    kind: JsonType,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    if keeps_draft4_integer_guard(kind, ctx.draft())
        && !matches!(value_set.as_schema(), Schema::False)
    {
        shared(Schema::TypedGroup {
            ty: kind,
            body: value_set,
        })
    } else {
        value_set
    }
}

/// Pack a list of canonical-JSON values into the smallest applicable shape.
#[must_use]
pub(crate) fn value_set_schema(values: Vec<CanonicalJson>) -> Schema {
    match values.len() {
        0 => Schema::False,
        1 => Schema::Const(values.into_iter().next().expect("len == 1")),
        _ => Schema::Enum(values),
    }
}

/// Pack and intern a list of canonical-JSON values into the smallest applicable shape.
#[must_use]
pub(crate) fn intern_value_set(values: Vec<CanonicalJson>) -> SharedSchema {
    shared(value_set_schema(values))
}

/// Promote a finite-valued typed leaf to `Const`/`Enum` so `{"type": "null"}` and `{"const": null}` produce identical IR.
fn promote_single_value_leaf(schema: &SharedSchema) -> Option<SharedSchema> {
    static NULL_CONST: LazyLock<SharedSchema> =
        LazyLock::new(|| shared(Schema::Const(canonical_json("null"))));
    static TRUE_CONST: LazyLock<SharedSchema> =
        LazyLock::new(|| shared(Schema::Const(canonical_json("true"))));
    static FALSE_CONST: LazyLock<SharedSchema> =
        LazyLock::new(|| shared(Schema::Const(canonical_json("false"))));
    static BOOLEAN_ENUM: LazyLock<SharedSchema> = LazyLock::new(|| {
        shared(Schema::Enum(vec![
            canonical_json("false"),
            canonical_json("true"),
        ]))
    });

    match schema.as_schema() {
        Schema::Null => Some(Arc::clone(&NULL_CONST)),
        Schema::Boolean(BooleanBounds::JustTrue) => Some(Arc::clone(&TRUE_CONST)),
        Schema::Boolean(BooleanBounds::JustFalse) => Some(Arc::clone(&FALSE_CONST)),
        Schema::Boolean(BooleanBounds::Any) => Some(Arc::clone(&BOOLEAN_ENUM)),
        _ => None,
    }
}

fn canonical_json(text: &'static str) -> CanonicalJson {
    CanonicalJson::from_canonical_text(Arc::from(text))
}

fn canonical_sort(values: &[CanonicalJson]) -> Option<Vec<CanonicalJson>> {
    match values {
        [] | [_] => None,
        [first, second] => match first.cmp(second) {
            Ordering::Equal => Some(vec![first.clone()]),
            Ordering::Greater => Some(vec![second.clone(), first.clone()]),
            Ordering::Less => None,
        },
        _ => {
            let already_minimal = values.is_sorted() && values.windows(2).all(|w| w[0] != w[1]);
            if already_minimal {
                return None;
            }
            let mut sorted = values.to_vec();
            sorted.sort();
            sorted.dedup();
            Some(sorted)
        }
    }
}

fn filter_enums_by_type(branches: &[SharedSchema]) -> Option<SharedSchema> {
    // Allowed types = intersection (Integer ⊆ Number aware) of every branch's restriction. A value-set with all
    // members outside it collapses to `False`; an empty intersection is itself unsatisfiable.
    let mut allowed: Option<JsonTypeSet> = None;
    for branch in branches {
        if let Some(restriction) = branch_type_restriction(branch) {
            allowed = Some(match allowed {
                None => restriction,
                Some(acc) => Schema::type_set_intersect(acc, restriction),
            });
        }
    }
    let allowed = allowed?;
    if allowed.is_empty() {
        return Some(shared(Schema::False));
    }
    let cover = Schema::semantic_cover(allowed);
    let mut changed = false;
    let mut new_branches: Vec<SharedSchema> = Vec::with_capacity(branches.len());
    for branch in branches {
        match branch.as_schema() {
            Schema::Enum(values) => {
                let matching_values = filter_enum_values_in_cover(values, cover);
                if matching_values.len() != values.len() {
                    changed = true;
                }
                if matching_values.is_empty() {
                    return Some(shared(Schema::False));
                }
                new_branches.push(intern_value_set(matching_values));
            }
            Schema::Const(value) => {
                if cover.contains(value.json_type()) {
                    new_branches.push(Arc::clone(branch));
                } else {
                    return Some(shared(Schema::False));
                }
            }
            _ => new_branches.push(Arc::clone(branch)),
        }
    }
    if !changed {
        return None;
    }
    Some(shared(Schema::AllOf(new_branches)))
}

// The type set a branch restricts an `AllOf` to: a `MultiType`/open-leaf set, a single pinned kind,
// or the union of an `AnyOf`'s branch restrictions. `None` when the branch admits any type.
fn branch_type_restriction(branch: &SharedSchema) -> Option<JsonTypeSet> {
    if let Schema::AnyOf(members) = branch.as_schema() {
        return members
            .iter()
            .try_fold(JsonTypeSet::empty(), |accumulated, member| {
                Some(accumulated.union(branch_type_restriction(member)?))
            });
    }
    branch.as_schema().as_type_set().or_else(|| {
        branch
            .as_schema()
            .single_type_domain()
            .map(JsonTypeSet::from)
    })
}

fn filter_enum_values_in_cover(values: &[CanonicalJson], cover: JsonTypeSet) -> Vec<CanonicalJson> {
    let mut seen: AHashSet<&str> = AHashSet::new();
    values
        .iter()
        .filter(|value| cover.contains(value.json_type()))
        .filter(|value| seen.insert(value.as_str()))
        .cloned()
        .collect()
}

fn filter_enum_values(values: &[CanonicalJson], allowed: JsonType) -> Vec<CanonicalJson> {
    let mut seen: AHashSet<&str> = AHashSet::new();
    values
        .iter()
        .filter(|value| allowed.covers(value.json_type()))
        .filter(|value| seen.insert(value.as_str()))
        .cloned()
        .collect()
}

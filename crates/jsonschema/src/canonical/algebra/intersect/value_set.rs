use ahash::AHashSet;
use serde_json::Value;

use crate::{
    canonical::{
        const_enum::{finalize_typed_value_set, intern_value_set, value_set_schema},
        context::{keeps_draft4_integer_guard, CanonicalizationContext},
        intern::shared,
        ir::{CanonicalJson, Schema, SharedSchema},
        leaves::{Leaf, Membership},
        numeric::NumericLeaf,
    },
    JsonType, JsonTypeSet,
};

use super::{integer, number, string};

/// Membership of an already-parsed numeric `value` in a numeric `leaf`: in-bounds, a multiple of
/// `multipleOf`, and excluded by no `not_multiple_of` entry. `multipleOf: 0` admits only zero.
fn value_matches<L: NumericLeaf>(value: &L::Scalar, leaf: &L) -> Membership {
    let bounds = leaf.bounds();
    if let Some(minimum) = bounds.minimum.as_ref() {
        let ordering = value.cmp(minimum);
        if ordering.is_lt() || (bounds.exclusive_minimum && ordering.is_eq()) {
            return Membership::No;
        }
    }
    if let Some(maximum) = bounds.maximum.as_ref() {
        let ordering = value.cmp(maximum);
        if ordering.is_gt() || (bounds.exclusive_maximum && ordering.is_eq()) {
            return Membership::No;
        }
    }
    if let Some(modulus) = leaf.multiple_of() {
        if L::modulus_is_zero(modulus) {
            return Membership::from_bool(L::scalar_is_zero(value));
        }
        match L::value_is_multiple(value, modulus) {
            Some(true) => {}
            Some(false) => return Membership::No,
            None => return Membership::Unknown,
        }
    }
    for excluded in leaf.not_multiple_of() {
        if L::modulus_is_zero(excluded) {
            // "not multipleOf 0" excludes only the value 0.
            if L::scalar_is_zero(value) {
                return Membership::No;
            }
            continue;
        }
        match L::value_is_multiple(value, excluded) {
            Some(true) => return Membership::No,
            Some(false) => {}
            None => return Membership::Unknown,
        }
    }
    Membership::Yes
}

/// Filter a `Const`/`Enum` set by the typed side. `None` when the pair isn't (typed | `TypedGroup`) x (Const | Enum) or
/// the typed side carries an unverifiable constraint - caller falls back to `AllOf` for soundness.
///
/// ```text
/// BEFORE: {"type": "integer", "minimum": 2}  and  {"enum": [1, 2, 3]}
/// AFTER:  {"enum": [2, 3]}
/// ```
pub(crate) fn intersect_typed_with_value_set(
    left: &SharedSchema,
    right: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> Option<SharedSchema> {
    let pair = typed_with_value_set(left, right).or_else(|| typed_with_value_set(right, left))?;
    let matching_values = filter_value_set(pair.values, |value| match pair.body {
        Some(body) => value_matches_typed(value, body, ctx),
        None => Membership::from_bool({
            let ty = pair.ty;
            ty.covers(value.json_type())
        }),
    })?;
    Some(finalize_typed_value_set(
        intern_value_set(matching_values),
        pair.ty,
        ctx,
    ))
}

/// Intersect a `MultiType` set with a `Const`/`Enum`: keep the values whose JSON type the set admits
/// (`Number` admits integers). An empty result collapses to `False`.
///
/// ```text
/// BEFORE: {"type": ["null", "string"]}  and  {"enum": [1, "a", 2]}
/// AFTER:  {"const": "a"}
/// ```
pub(crate) fn intersect_multi_type_with_value_set(
    set: JsonTypeSet,
    value_set: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    let values = match value_set.as_schema() {
        Schema::Const(value) => std::slice::from_ref(value),
        Schema::Enum(values) => values.as_slice(),
        _ => return SharedSchema::clone(value_set),
    };
    let kept: Vec<CanonicalJson> = values
        .iter()
        .filter(|value| set.iter().any(|member| member.covers(value.json_type())))
        .cloned()
        .collect();
    // Draft4 "integer" is token-based: an integer admitted only through the `Integer` member (no `Number`
    // in the set) must keep the pin, or the bare enum would match its `1.0` spelling numerically.
    let pin_integers = !set.contains(JsonType::Number);
    value_set_with_draft4_integer_pin(kept, pin_integers, ctx)
}

/// Pack kept values, keeping the Draft4 integer-token pin on integer members when `pin_integers`.
fn value_set_with_draft4_integer_pin(
    kept: Vec<CanonicalJson>,
    pin_integers: bool,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    if !pin_integers || !keeps_draft4_integer_guard(JsonType::Integer, ctx.draft()) {
        return intern_value_set(kept);
    }
    let (pinned, bare): (Vec<_>, Vec<_>) = kept
        .into_iter()
        .partition(|value| value.json_type() == JsonType::Integer);
    if pinned.is_empty() {
        return intern_value_set(bare);
    }
    let pinned = finalize_typed_value_set(intern_value_set(pinned), JsonType::Integer, ctx);
    if bare.is_empty() {
        return pinned;
    }
    shared(Schema::AnyOf(vec![pinned, intern_value_set(bare)]))
}

/// Pack values filtered through a guard: the integer-valued-number guard pins surviving integers
/// under Draft4 so the token restriction survives emission.
pub(super) fn guarded_value_set(
    guard_ty: JsonType,
    guard_body: &SharedSchema,
    kept: Vec<CanonicalJson>,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    let pin_integers = guard_ty == JsonType::Number
        && matches!(
            guard_body.as_schema(),
            Schema::Integer(_)
                | Schema::TypedGroup {
                    ty: JsonType::Integer,
                    ..
                }
        );
    value_set_with_draft4_integer_pin(kept, pin_integers, ctx)
}

/// Membership of one value against a type guard: values outside the guarded type pass, values of
/// it keep body membership.
fn membership_through_guard(
    guard_ty: JsonType,
    guard_body: &SharedSchema,
    value: &CanonicalJson,
    ctx: &CanonicalizationContext,
) -> Membership {
    if guard_ty.covers(value.json_type()) {
        value_matches_typed(value, guard_body.as_schema(), ctx)
    } else {
        Membership::Yes
    }
}

/// Filter a value set through a type guard. `None` on an `Unknown` verdict (asserted format,
/// extended regex, constrained body).
pub(super) fn filter_values_through_guard(
    guard_ty: JsonType,
    guard_body: &SharedSchema,
    values: &[CanonicalJson],
    ctx: &CanonicalizationContext,
) -> Option<Vec<CanonicalJson>> {
    filter_value_set(values, |value| {
        membership_through_guard(guard_ty, guard_body, value, ctx)
    })
}

/// True when intersecting `TypeGuard { guard_ty, guard_body }` with a `Const`/`Enum` bails back to
/// `AllOf` because some value's body membership is undecidable. Verdict-only: allocates nothing.
pub(crate) fn is_unmergeable_guard_value_pair(
    guard_ty: JsonType,
    guard_body: &SharedSchema,
    other: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> bool {
    let values = match other.as_schema() {
        Schema::Const(value) => std::slice::from_ref(value),
        Schema::Enum(values) => values.as_slice(),
        _ => return false,
    };
    values.iter().any(|value| {
        matches!(
            membership_through_guard(guard_ty, guard_body, value, ctx),
            Membership::Unknown
        )
    })
}

/// Verdict-only twin of `intersect_typed_with_value_set` for fold-safety predicates: `false` when
/// the pair cannot merge (undecidable membership or an unfilterable typed side).
pub(crate) fn value_set_merge_is_decidable(
    typed: &SharedSchema,
    value_set: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> bool {
    let Some((ty, body)) = ty_and_body(typed) else {
        return false;
    };
    let values = match value_set.as_schema() {
        Schema::Const(value) => std::slice::from_ref(value),
        Schema::Enum(values) => values.as_slice(),
        _ => return false,
    };
    !values.iter().any(|value| {
        let membership = match body {
            Some(body) => value_matches_typed(value, body, ctx),
            None => Membership::from_bool(ty.covers(value.json_type())),
        };
        matches!(membership, Membership::Unknown)
    })
}

/// Keep the values with a `Yes` verdict; `None` on any `Unknown` - the caller falls back to `AllOf`
/// so the validator checks the pair strictly.
fn filter_value_set(
    values: &[CanonicalJson],
    verdict: impl Fn(&CanonicalJson) -> Membership,
) -> Option<Vec<CanonicalJson>> {
    let mut kept: Vec<CanonicalJson> = Vec::with_capacity(values.len());
    for value in values {
        match verdict(value) {
            Membership::Yes => kept.push(value.clone()),
            Membership::No => {}
            Membership::Unknown => return None,
        }
    }
    Some(kept)
}

/// Intersect two finite `Const` / `Enum` value sets into the smallest schema shape.
///
/// ```text
/// BEFORE: {"enum": [1, 2, 3]}  and  {"enum": [2, 3, 4]}
/// AFTER:  {"enum": [2, 3]}
///
/// BEFORE: {"const": 2}  and  {"enum": [1, 2, 3]}
/// AFTER:  {"const": 2}
///
/// BEFORE: {"const": 5}  and  {"enum": [1, 2, 3]}
/// AFTER:  false
/// ```
#[must_use]
pub(crate) fn intersect_value_sets(left: &Schema, right: &Schema) -> Option<Schema> {
    match (left, right) {
        (Schema::Const(left), Schema::Const(right)) => Some(if left == right {
            value_set_schema(vec![left.clone()])
        } else {
            value_set_schema(Vec::new())
        }),
        (Schema::Enum(left), Schema::Enum(right)) => {
            let right_set: AHashSet<&CanonicalJson> = right.iter().collect();
            let shared_values: Vec<CanonicalJson> = left
                .iter()
                .filter(|value| right_set.contains(*value))
                .cloned()
                .collect();
            Some(value_set_schema(shared_values))
        }
        (Schema::Const(value), Schema::Enum(values))
        | (Schema::Enum(values), Schema::Const(value)) => Some(if values.contains(value) {
            value_set_schema(vec![value.clone()])
        } else {
            value_set_schema(Vec::new())
        }),
        _ => None,
    }
}

struct TypedSet<'a> {
    ty: JsonType,
    /// `None` when only the type check applies.
    body: Option<&'a Schema>,
    values: &'a [CanonicalJson],
}

fn typed_with_value_set<'a>(
    typed: &'a SharedSchema,
    value_set: &'a SharedSchema,
) -> Option<TypedSet<'a>> {
    let (ty, body) = ty_and_body(typed)?;
    let values: &[CanonicalJson] = match value_set.as_schema() {
        Schema::Const(value) => std::slice::from_ref(value),
        Schema::Enum(values) => values.as_slice(),
        _ => return None,
    };
    Some(TypedSet { ty, body, values })
}

/// Body is `Some` when its facets further constrain values; `None` (with the type) when only the type check
/// applies. A `TypedGroup` body outside the value-checkable shapes returns `None` overall so the caller falls
/// back to `AllOf` instead of filtering by type alone and dropping the body constraint.
pub(super) fn ty_and_body(typed: &SharedSchema) -> Option<(JsonType, Option<&Schema>)> {
    // Null and boolean never reach this typed side (see `intersect_typed`).
    match typed.as_schema() {
        Schema::Integer(_) => Some((JsonType::Integer, Some(typed.as_schema()))),
        Schema::Number(_) => Some((JsonType::Number, Some(typed.as_schema()))),
        Schema::String(_) => Some((JsonType::String, Some(typed.as_schema()))),
        Schema::Array(_) => Some((JsonType::Array, Some(typed.as_schema()))),
        Schema::Object(_) => Some((JsonType::Object, Some(typed.as_schema()))),
        Schema::TypedGroup { ty, body } => match body.as_schema() {
            Schema::Integer(_)
            | Schema::Number(_)
            | Schema::String(_)
            | Schema::Array(_)
            | Schema::Object(_)
            | Schema::Const(_)
            | Schema::Enum(_) => Some((*ty, Some(body.as_schema()))),
            Schema::True => Some((*ty, None)),
            _ => None,
        },
        _ => None,
    }
}

/// `Unknown` when the leaf carries a constraint we can't precisely check here (extended regex, format, non-trivial
/// array/object body).
#[expect(
    clippy::match_same_arms,
    reason = "arms are kept per-type to mirror the leaf taxonomy even when verdicts coincide"
)]
pub(crate) fn value_matches_typed(
    value: &CanonicalJson,
    body: &Schema,
    ctx: &CanonicalizationContext,
) -> Membership {
    let parsed = ctx.parse_canonical(value);
    // Null and boolean bodies don't occur here (see `intersect_typed`).
    match (body, parsed.as_ref()) {
        (Schema::Integer(leaf), Value::Number(number)) if ctx.is_integer(number) => {
            integer::scalar_from_json(number)
                .map_or(Membership::Unknown, |value| value_matches(&value, leaf))
        }
        (Schema::Integer(_), _) => Membership::No,
        (Schema::Number(leaf), Value::Number(number)) => number::scalar_from_json(number)
            .map_or(Membership::Unknown, |value| value_matches(&value, leaf)),
        (Schema::Number(_), _) => Membership::No,
        (Schema::String(leaf), Value::String(text)) => {
            string::value_matches_string(text, leaf, ctx)
        }
        (Schema::String(_), _) => Membership::No,
        // Only unconstrained array/object bodies give a definite verdict.
        (Schema::Array(leaf), Value::Array(_)) => {
            if leaf.is_open() {
                Membership::Yes
            } else {
                Membership::Unknown
            }
        }
        (Schema::Array(_), _) => Membership::No,
        (Schema::Object(leaf), Value::Object(_)) => {
            if leaf.is_open() {
                Membership::Yes
            } else {
                Membership::Unknown
            }
        }
        (Schema::Object(_), _) => Membership::No,
        // Value-set bodies under a `TypedGroup`: the outer type has filtered; membership is the final check.
        (Schema::Const(needed), _) => Membership::from_bool(value == needed),
        (Schema::Enum(members), _) => Membership::from_bool(members.contains(value)),
        _ => Membership::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        canonical::tests_util::{canonicalize, canonicalize_with},
        Draft,
    };

    // A TypedGroup with a composite body (here: the negation of required+propertyNames, which is
    // TypedGroup{Object, AnyOf[...]}) must keep its body constraints when intersected with a value set.
    #[test]
    fn typed_group_composite_body_keeps_constraints_against_value_set() {
        let negated =
            canonicalize(&json!({"required": ["a"], "propertyNames": {"minLength": 1}})).negate();
        let value_set = canonicalize(&json!({"enum": [{"a": 1}]}));
        let intersection = negated.intersect(&value_set).to_json_schema();
        let validator = crate::validator_for(&intersection).expect("intersection compiles");
        // {"a": 1} satisfies the enum but not the negation, so the intersection must reject it.
        assert!(!validator.is_valid(&json!({"a": 1})));
    }

    // A value past the fraction carrier (1e22 in the default build) has an undecidable membership,
    // so the typed sibling must survive as a strict conjunct.
    #[test]
    fn unrepresentable_member_keeps_typed_sibling() {
        let schema = json!({"allOf": [{"enum": [1e22, 3]}, {"type": "number", "maximum": 5}]});
        let canonical = canonicalize(&schema).to_json_schema();
        let validator = crate::validator_for(&canonical).expect("canonical compiles");
        assert!(validator.is_valid(&json!(3)));
        assert!(!validator.is_valid(&json!(1e22)));
    }

    // Draft4 "integer" is token-based: the merged value set must not admit the 1.0 spelling.
    #[test]
    fn draft4_multi_type_enum_intersection_keeps_integer_pin() {
        let left = canonicalize_with(&json!({"type": ["integer", "string"]}), Draft::Draft4);
        let right = canonicalize_with(&json!({"enum": [1, "a"]}), Draft::Draft4);
        let result = left.intersect(&right).to_json_schema();
        let validator = crate::options()
            .with_draft(Draft::Draft4)
            .build(&result)
            .expect("intersection compiles");
        assert!(validator.is_valid(&json!(1)));
        assert!(validator.is_valid(&json!("a")));
        assert!(!validator.is_valid(&json!(1.0)));
    }

    // Same pin through the integer-valued-number type guard (the canonical form of the full type list).
    #[test]
    fn draft4_type_guard_enum_intersection_keeps_integer_pin() {
        let left = canonicalize_with(
            &json!({"type": ["null", "boolean", "integer", "string", "array", "object"]}),
            Draft::Draft4,
        );
        let right = canonicalize_with(&json!({"enum": [1, "a"]}), Draft::Draft4);
        let result = left.intersect(&right).to_json_schema();
        let validator = crate::options()
            .with_draft(Draft::Draft4)
            .build(&result)
            .expect("intersection compiles");
        assert!(validator.is_valid(&json!(1)));
        assert!(validator.is_valid(&json!("a")));
        assert!(!validator.is_valid(&json!(1.0)));
    }

    // Draft4 negation of a typed value set produces TypedGroup{Integer, AnyOf[...]}; subtracting a
    // superset value set must not resurrect the subtracted member.
    #[test]
    fn draft4_subtract_covering_value_set_rejects_member() {
        let left = canonicalize_with(&json!({"enum": [1]}), Draft::Draft4);
        let right = canonicalize_with(&json!({"type": "integer", "enum": [1, 2]}), Draft::Draft4);
        let difference = left.subtract(&right).to_json_schema();
        let validator = crate::options()
            .with_draft(Draft::Draft4)
            .build(&difference)
            .expect("difference compiles");
        assert!(!validator.is_valid(&json!(1)));
    }
}

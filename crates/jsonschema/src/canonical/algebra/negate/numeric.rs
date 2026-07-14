//! Numeric-domain negation: integer and number leaves and their guard duals.

use crate::{
    canonical::{
        intern::shared,
        ir::{BoundFraction, IntegerBounds, NumberBounds, NumberLeaf, Schema, SharedSchema},
        numeric::{negate_in_kind, NumericBounds, NumericLeaf},
    },
    JsonType,
};

use super::{any_of_complement, not_wrap};

/// Generic dual for numeric guard bodies the compact paths leave: `Not(TypeGuard{Number, body}) = number AND
/// not body`. An `IntegerLeaf` body admits only integers, so its negation also adds every non-integer number
/// (`not multipleOf 1`), which the integer-kind complement alone misses.
pub(super) fn negate_numeric_bounds_type_guard(
    kind: JsonType,
    body: &SharedSchema,
) -> Option<SharedSchema> {
    if kind != JsonType::Number {
        return None;
    }
    let branches = match body.as_schema() {
        Schema::Number(leaf) => negate_in_kind(leaf),
        Schema::Integer(leaf) if !leaf.bounds.dual_is_unrepresentable() => {
            let mut branches = negate_in_kind(leaf);
            branches.push(shared(Schema::Number(NumberLeaf {
                not_multiple_of: vec![BoundFraction::one()],
                ..NumberLeaf::default()
            })));
            branches
        }
        _ => return None,
    };
    Some(match branches.len() {
        0 => shared(Schema::False),
        1 => branches.into_iter().next().expect("len == 1"),
        _ => shared(Schema::AnyOf(branches)),
    })
}

/// `multipleOf: q` and `not_multiple_of: [q]` are each other's in-kind duals; an open integer body
/// is the `q = 1` spelling (`multipleOf: 1` normalizes to it).
pub(super) fn negate_number_multiple_of_type_guard(
    kind: JsonType,
    body: &SharedSchema,
) -> Option<SharedSchema> {
    if kind != JsonType::Number {
        return None;
    }
    let fractional_only = |q| {
        shared(Schema::Number(NumberLeaf {
            not_multiple_of: vec![q],
            ..NumberLeaf::default()
        }))
    };
    match body.as_schema() {
        // Open integer body is the `q = 1` spelling, where an integral `Number{multipleOf q}` normalizes.
        Schema::Integer(leaf)
            if leaf.bounds == IntegerBounds::default() && leaf.not_multiple_of.is_empty() =>
        {
            let modulus = leaf
                .multiple_of
                .as_ref()
                .map_or_else(BoundFraction::one, |q| BoundFraction::from((q).owned()));
            Some(fractional_only(modulus))
        }
        Schema::Number(leaf) if leaf.bounds == NumberBounds::default() => {
            match (leaf.multiple_of.as_ref(), leaf.not_multiple_of.as_slice()) {
                (Some(modulus), []) => Some(fractional_only((modulus).owned())),
                (None, [modulus]) => Some(shared(Schema::Number(NumberLeaf {
                    multiple_of: Some((modulus).owned()),
                    ..NumberLeaf::default()
                }))),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The leaf is `A and B and C and notD` (`A`/`B` bounds, `C = multiple_of`, `notD = not_multiple_of`); negation
/// is `notA OR notB OR notC OR D` (bounds cover out-of-range; in-range `notC`/`D` are the multiple-of duals),
/// plus every other JSON type.
///
/// ```text
/// BEFORE: {"type": "number", "minimum": 0.5, "maximum": 2.5}
/// AFTER:  {"anyOf": [
///           {"type": "number", "exclusiveMaximum": 0.5},
///           {"type": "number", "exclusiveMinimum": 2.5},
///           {"type": ["null", "boolean", "string", "array", "object"]}
///         ]}
/// ```
pub(super) fn negate_numeric<L: NumericLeaf>(leaf: &L) -> SharedSchema
where
    crate::canonical::ir::Bounds<L::Scalar>: crate::canonical::numeric::NumericBounds,
{
    // A bound at the carrier edge has no sound constructive dual (JSON integers extend past it),
    // so the complement keeps the exact `Not` residual instead.
    if leaf.bounds().dual_is_unrepresentable() {
        let node = shared(L::into_schema(
            leaf.bounds().clone(),
            leaf.multiple_of().cloned(),
            leaf.not_multiple_of().to_vec(),
        ));
        return any_of_complement(L::JSON_TYPE, vec![not_wrap(node)]);
    }
    any_of_complement(L::JSON_TYPE, negate_in_kind(leaf))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        canonical::{
            intern::shared,
            ir::{BoundFraction, BoundInteger, IntegerBounds, IntegerLeaf, Schema},
            tests_util::canonicalize,
        },
        JsonType,
    };

    // Detects only the `Not(<numeric leaf with multipleOf>)` residual, not a generic "no Not anywhere" check:
    // `any_of_complement(Integer, ..)` legitimately emits `Not(open Integer)` for the fractional complement.
    fn has_multiple_of_not_residual(schema: &Schema) -> bool {
        if let Schema::Not(inner) = schema {
            if matches!(inner.as_schema(), Schema::Integer(l) if l.multiple_of.is_some())
                || matches!(inner.as_schema(), Schema::Number(l) if l.multiple_of.is_some())
            {
                return true;
            }
        }
        schema
            .children()
            .iter()
            .any(|c| has_multiple_of_not_residual(c.as_schema()))
    }

    #[test]
    fn multiple_of_negation_leaves_no_residual() {
        for schema in [
            json!({"type": "integer", "multipleOf": 4}),
            json!({"type": "number", "multipleOf": 0.5}),
            json!({"type": "integer", "minimum": 0, "maximum": 20, "multipleOf": 4}),
        ] {
            let negated = canonicalize(&schema).negate();
            assert!(!has_multiple_of_not_residual(negated.as_schema()));
        }
    }

    #[test]
    fn integer_with_bounds_and_multiple_of_negation_exposes_bound_branches() {
        // Bound complement exposes Integer{< 0} / Integer{> 10} as separate branches; the multipleOf branch
        // stays wrapped in Not (no algebraic complement).
        let schema = json!({
            "type": "integer",
            "minimum": 0,
            "maximum": 10,
            "multipleOf": 3
        });
        let negated = canonicalize(&schema).negate();
        let Schema::AnyOf(branches) = negated.as_schema() else {
            panic!("expected top-level AnyOf, got {negated:?}");
        };
        let has_unbounded_below = branches.iter().any(|branch| {
            matches!(
                branch.as_schema(),
                Schema::Integer(leaf)
                    if leaf.bounds.minimum.is_none()
                        && leaf.bounds.maximum.is_some()
                        && leaf.multiple_of.is_none()
            )
        });
        let has_unbounded_above = branches.iter().any(|branch| {
            matches!(
                branch.as_schema(),
                Schema::Integer(leaf)
                    if leaf.bounds.minimum.is_some()
                        && leaf.bounds.maximum.is_none()
                        && leaf.multiple_of.is_none()
            )
        });
        assert!(has_unbounded_below);
        assert!(has_unbounded_above);
    }

    // The integer-valued-number guard `TypeGuard{Number, IntegerLeaf}` (built internally, no surface JSON)
    // negates to add every non-integer number (`not multipleOf 1`), which the integer-kind complement misses.
    #[test]
    fn integer_body_number_guard_dual_adds_non_integers() {
        let body = shared(Schema::Integer(IntegerLeaf {
            bounds: IntegerBounds {
                minimum: Some(BoundInteger::from(0)),
                ..IntegerBounds::default()
            },
            ..IntegerLeaf::default()
        }));
        let dual =
            super::super::negate_type_guard(JsonType::Number, &body).expect("numeric guard dual");
        let Schema::AnyOf(branches) = dual.as_schema() else {
            panic!("expected AnyOf dual, got {dual:?}");
        };
        assert!(branches.iter().any(|branch| matches!(
            branch.as_schema(),
            Schema::Number(leaf) if leaf.not_multiple_of == [BoundFraction::from(1)]
        )));
        assert!(branches
            .iter()
            .any(|branch| matches!(branch.as_schema(), Schema::Integer(_))));
        assert!(!branches
            .iter()
            .any(|branch| matches!(branch.as_schema(), Schema::Not(_))));
    }
}

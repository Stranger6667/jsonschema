//! Sound 3-valued value-membership oracle, and a finite-domain emptiness proof built on it.

use serde_json::Value;

use crate::{
    canonical::{
        context::CanonicalizationContext,
        intersect::value_matches_typed,
        ir::{CanonicalJson, IntegerLeaf, Schema, SharedSchema},
        leaves::Membership,
    },
    JsonType,
};

/// Largest finite candidate domain enumerated; past it emptiness stays undecided (conservatively satisfiable).
pub(crate) const DOMAIN_CAP: usize = 256;

/// Whether `value` satisfies `schema`. Sound in both directions - a verdict is never wrong, only `Unknown`.
pub(crate) fn admits(
    value: &CanonicalJson,
    schema: &Schema,
    ctx: &CanonicalizationContext,
) -> Membership {
    match schema {
        Schema::True => Membership::Yes,
        Schema::False => Membership::No,
        Schema::Null => Membership::from_bool(value.json_type() == JsonType::Null),
        Schema::MultiType(set) => {
            let value_ty = value.json_type();
            Membership::from_bool(set.iter().any(|member| member.covers(value_ty)))
        }
        Schema::AnyOf(branches) => admits_any_of(value, branches, ctx),
        Schema::AllOf(branches) => admits_all_of(value, branches, ctx),
        Schema::Not(inner) => admits(value, inner.as_schema(), ctx).negate(),
        Schema::TypedGroup { ty, body } => match value.json_type() {
            // A conjunction: the value must be of the pinned kind and satisfy the body.
            value_ty if ty.covers(value_ty) => admits(value, body.as_schema(), ctx),
            _ => Membership::No,
        },
        Schema::TypeGuard { ty, body } => match value.json_type() {
            // An implication: a value outside the guarded kind is unconstrained; one inside must satisfy the body.
            value_ty if ty.covers(value_ty) => admits(value, body.as_schema(), ctx),
            _ => Membership::Yes,
        },
        Schema::Reference(_) | Schema::Recursive(_) | Schema::DynamicRef(_) | Schema::Raw(_) => {
            Membership::Unknown
        }
        // Typed leaves and value sets.
        _ => value_matches_typed(value, schema, ctx),
    }
}

fn admits_any_of(
    value: &CanonicalJson,
    branches: &[SharedSchema],
    ctx: &CanonicalizationContext,
) -> Membership {
    let mut unknown = false;
    for branch in branches {
        match admits(value, branch.as_schema(), ctx) {
            Membership::Yes => return Membership::Yes,
            Membership::No => {}
            Membership::Unknown => unknown = true,
        }
    }
    if unknown {
        Membership::Unknown
    } else {
        Membership::No
    }
}

fn admits_all_of(
    value: &CanonicalJson,
    branches: &[SharedSchema],
    ctx: &CanonicalizationContext,
) -> Membership {
    let mut unknown = false;
    for branch in branches {
        match admits(value, branch.as_schema(), ctx) {
            Membership::No => return Membership::No,
            Membership::Yes => {}
            Membership::Unknown => unknown = true,
        }
    }
    if unknown {
        Membership::Unknown
    } else {
        Membership::Yes
    }
}

/// `true` only when `schema` is provably empty: a finite domain that is a superset of its satisfying values exists, and
/// `admits` rejects every member. Conservative - returns `false` when no finite domain can be derived.
pub(crate) fn is_provably_empty(schema: &Schema, ctx: &CanonicalizationContext) -> bool {
    let Some(domain) = finite_domain(schema) else {
        return false;
    };
    domain
        .iter()
        .all(|value| admits(value, schema, ctx) == Membership::No)
}

/// A finite superset of the values `schema` can admit, when one is derivable. `AllOf` reuses any branch's domain (the
/// conjunction is a subset of each); `AnyOf` needs every branch bounded (their union).
fn finite_domain(schema: &Schema) -> Option<Vec<CanonicalJson>> {
    match schema {
        Schema::Const(value) => Some(vec![value.clone()]),
        Schema::Enum(values) => Some(values.clone()),
        Schema::Integer(leaf) => integer_domain(leaf),
        Schema::AllOf(branches) => branches
            .iter()
            .find_map(|branch| finite_domain(branch.as_schema())),
        Schema::AnyOf(branches) => {
            let mut values = Vec::new();
            for branch in branches {
                values.extend(finite_domain(branch.as_schema())?);
                if values.len() > DOMAIN_CAP {
                    return None;
                }
            }
            Some(values)
        }
        _ => None,
    }
}

/// Enumerate a bounded integer leaf; `None` when either bound is open or the interval exceeds [`DOMAIN_CAP`]. Inclusive
/// bounds over-cover an exclusive leaf, which `admits` filters back out.
fn integer_domain(leaf: &IntegerLeaf) -> Option<Vec<CanonicalJson>> {
    let (minimum, maximum) = (leaf.bounds.minimum.as_ref()?, leaf.bounds.maximum.as_ref()?);
    let mut values = Vec::new();
    let mut current = (minimum).owned();
    while current <= *maximum {
        if values.len() >= DOMAIN_CAP {
            return None;
        }
        values.push({
            let value: &Value = &Value::Number(current.to_number());
            CanonicalJson::from_value(value)
        });
        current = current.checked_increment()?;
    }
    Some(values)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use test_case::test_case;

    use super::*;
    use crate::canonical::{
        intern::shared,
        tests_util::{type_guard, typed_group},
    };

    // `TypedGroup` is a conjunction (kind ∧ body); `TypeGuard` is an implication (kind -> body).
    #[test_case(&typed_group(JsonType::Integer, Schema::True), &json!(1), Membership::Yes ; "group_in_type_defers_to_body")]
    #[test_case(&typed_group(JsonType::Integer, Schema::True), &json!("x"), Membership::No ; "group_rejects_off_type")]
    #[test_case(&type_guard(JsonType::Integer, Schema::False), &json!("x"), Membership::Yes ; "guard_passes_off_type")]
    #[test_case(&type_guard(JsonType::Integer, Schema::False), &json!(1), Membership::No ; "guard_in_type_defers_to_body")]
    // Under `Not`, a wrong `Yes` flips to a wrong `No` - the verdict `is_provably_empty` trusts to
    // declare a schema empty.
    #[test_case(&Schema::Not(shared(typed_group(JsonType::Integer, Schema::False))), &json!("x"), Membership::Yes ; "negated_group_admits_off_type")]
    fn admits_typed_variants(schema: &Schema, value: &Value, expected: Membership) {
        let ctx = CanonicalizationContext::default();
        assert_eq!(
            admits(&CanonicalJson::from_value(value), schema, &ctx),
            expected
        );
    }
}

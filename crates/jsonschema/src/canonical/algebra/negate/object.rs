//! Object-domain negation: requirement/constraint duals and existential complements.

use std::sync::Arc;

use crate::{
    canonical::{
        context::CanonicalizationContext,
        intern::shared,
        ir::{
            ObjectConstraint, ObjectLeaf, ObjectRequirement, PropertyNameMatcher, Schema,
            SharedSchema,
        },
    },
    JsonType,
};

use super::{any_of_complement, negate_with_options, not_wrap, wrap_in_kind, NegationOptions};

/// `required: [n]` and `properties: {n: false}` are each other's in-kind duals: "property `n`
/// exists" versus "property `n` is absent".
pub(super) fn negate_object_required_type_guard(
    kind: JsonType,
    body: &SharedSchema,
) -> Option<SharedSchema> {
    if kind != JsonType::Object {
        return None;
    }
    let Schema::Object(leaf) = body.as_schema() else {
        return None;
    };
    if leaf.property_names.is_some() {
        return None;
    }
    match (leaf.requirements.as_slice(), leaf.constraints.as_slice()) {
        ([ObjectRequirement::RequiredProperty(name)], []) => Some(single_constraint_object(
            PropertyNameMatcher::NamedProperty(Arc::clone(name)),
            shared(Schema::False),
        )),
        (
            [],
            [ObjectConstraint {
                matcher: PropertyNameMatcher::NamedProperty(name),
                schema,
            }],
        ) if matches!(schema.as_schema(), Schema::False) => Some(single_requirement_object(
            ObjectRequirement::RequiredProperty(Arc::clone(name)),
        )),
        _ => None,
    }
}

/// Disjoin per-facet negations; facets without an IR-direct complement fall back to `Not(leaf)`. A `required` name
/// negates to a branch banning that property, a `properties` constraint to a branch where the value violates it.
///
/// ```text
/// BEFORE: {"type": "object", "required": ["a"]}
/// AFTER:  {"anyOf": [
///           {"type": "object", "properties": {"a": false}},
///           {"type": ["null", "boolean", "number", "string", "array"]}
///         ]}
/// ```
pub(super) fn negate_object_leaf(
    schema: &SharedSchema,
    leaf: &ObjectLeaf,
    options: NegationOptions,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    let mut branches: Vec<SharedSchema> = Vec::new();
    let mut needs_fallback = false;

    // `additionalProperties` covers names no declared property/pattern matches, so its existential negation is
    // self-contained only when no such declarations exist.
    let has_named_or_pattern = leaf.constraints.iter().any(|constraint| {
        matches!(
            constraint.matcher,
            PropertyNameMatcher::NamedProperty(_) | PropertyNameMatcher::PatternProperty(_)
        )
    });

    for requirement in &leaf.requirements {
        match requirement {
            ObjectRequirement::RequiredProperty(name) => {
                branches.push(single_constraint_object(
                    PropertyNameMatcher::NamedProperty(Arc::clone(name)),
                    shared(Schema::False),
                ));
            }
            ObjectRequirement::PatternPropertyRequirement { matcher, schema } => {
                branches.push(single_constraint_object(
                    matcher.clone(),
                    negate_with_options(schema, options, ctx),
                ));
            }
            ObjectRequirement::MinProperties(n) if n.is_zero() => {}
            ObjectRequirement::MinProperties(n) => {
                if let Some(maximum) = n.checked_decrement() {
                    branches.push(single_requirement_object(ObjectRequirement::MaxProperties(
                        maximum,
                    )));
                }
            }
            ObjectRequirement::MaxProperties(n) => {
                // In the default build, `maxProperties: u64::MAX` has no representable successor; under
                // `arbitrary-precision`, the exact successor is stored.
                if let Some(min) = n.checked_increment() {
                    branches.push(single_requirement_object(ObjectRequirement::MinProperties(
                        min,
                    )));
                }
            }
            ObjectRequirement::DependentPropertiesRequirement { .. }
            | ObjectRequirement::DependentSchemaRequirement { .. } => {
                needs_fallback = true;
            }
        }
    }

    for constraint in &leaf.constraints {
        match &constraint.matcher {
            PropertyNameMatcher::NamedProperty(name) => {
                // Keep the exact name so `S` and the negated existential read as a contradiction (a `^name$`
                // pattern would need regex analysis).
                branches.push(single_requirement_object(
                    ObjectRequirement::PatternPropertyRequirement {
                        matcher: PropertyNameMatcher::NamedProperty(Arc::clone(name)),
                        schema: negate_with_options(&constraint.schema, options, ctx),
                    },
                ));
            }
            PropertyNameMatcher::PatternProperty(pattern) => {
                branches.push(single_requirement_object(
                    ObjectRequirement::PatternPropertyRequirement {
                        matcher: PropertyNameMatcher::PatternProperty(Arc::clone(pattern)),
                        schema: negate_with_options(&constraint.schema, options, ctx),
                    },
                ));
            }
            PropertyNameMatcher::AdditionalProperties if has_named_or_pattern => {
                // Declared properties scope the additional set, which a standalone existential can't express,
                // so keep this negation as a `Not` residual (sound, not constructive).
                needs_fallback = true;
            }
            PropertyNameMatcher::AdditionalProperties => {
                // not(all additional props satisfy S) = exists an additional prop satisfying not-S.
                branches.push(single_requirement_object(
                    ObjectRequirement::PatternPropertyRequirement {
                        matcher: PropertyNameMatcher::AdditionalProperties,
                        schema: negate_with_options(&constraint.schema, options, ctx),
                    },
                ));
            }
        }
    }

    if leaf.property_names.is_some() {
        needs_fallback = true;
    }

    if needs_fallback {
        branches.push(not_wrap(Arc::clone(schema)));
    }

    any_of_complement(JsonType::Object, wrap_in_kind(branches))
}

fn single_constraint_object(matcher: PropertyNameMatcher, schema: SharedSchema) -> SharedSchema {
    shared(Schema::Object(ObjectLeaf {
        requirements: Vec::new(),
        constraints: vec![ObjectConstraint { matcher, schema }],
        property_names: None,
    }))
}

fn single_requirement_object(requirement: ObjectRequirement) -> SharedSchema {
    shared(Schema::Object(ObjectLeaf {
        requirements: vec![requirement],
        constraints: Vec::new(),
        property_names: None,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::needless_pass_by_value)]

    use serde_json::{json, Value};
    use test_case::test_case;

    use crate::canonical::{
        ir::Schema,
        tests_util::{assert_json_complement, canonicalize},
    };

    fn has_object_not_residual(schema: &Schema) -> bool {
        if let Schema::Not(inner) = schema {
            if matches!(inner.as_schema(), Schema::Object(_)) {
                return true;
            }
        }
        schema
            .children()
            .iter()
            .any(|c| has_object_not_residual(c.as_schema()))
    }

    #[test]
    fn negate_additional_properties_has_no_object_not_residual() {
        let negated = canonicalize(&json!({
            "type": "object",
            "additionalProperties": {"type": "integer"}
        }))
        .negate();
        assert!(!has_object_not_residual(negated.as_schema()));
    }

    // additionalProperties:{integer} rejects an object with a non-integer additional prop.
    #[test_case(json!({"type": "object", "additionalProperties": {"type": "integer"}}),
        &[json!({}), json!({"a": 1})],
        &[json!({"a": "x"}), json!(0)]
        ; "additional_properties_existential")]
    // Declared `properties` scope the additional set: the negated existential must not count `a` as additional.
    #[test_case(json!({"type": "object", "properties": {"a": {"type": "integer"}}, "additionalProperties": false}),
        &[json!({}), json!({"a": 1})],
        &[json!({"a": "x"}), json!({"b": 2}), json!(5)]
        ; "additional_properties_false_scoped_by_properties")]
    fn negate_object_is_exact_complement(schema: Value, accepts: &[Value], rejects: &[Value]) {
        assert_json_complement(&schema, accepts, rejects);
    }
}

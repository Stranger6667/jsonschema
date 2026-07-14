//! Object-domain collapse passes.

use std::sync::Arc;

use crate::canonical::{
    context::CanonicalizationContext,
    coverage::any_sibling_covers,
    intern::shared,
    intersect::{intersect_canonical, object::any_object_requirement_contradiction},
    ir::{
        BoundCardinality, ObjectConstraint, ObjectLeaf, ObjectRequirement, PropertyNameMatcher,
        Schema, SharedSchema,
    },
    negate::negate,
};

/// Copy instance-absolute entries across sibling object conjuncts: requirements into every sibling,
/// named/pattern constraints only into siblings without catch-all scope (else the negation diverges).
///
/// ```text
/// BEFORE: {"allOf": [{"patternProperties": {"^x": {"type": "integer"}}, "type": "object"},
///                    {"additionalProperties": false, "properties": {"a": {"type": "integer"}},
///                     "type": "object"}]}
/// AFTER:  {"allOf": [{"maxProperties": 1, "patternProperties": {"^x": {"type": "integer"}},
///                     "properties": {"a": {"type": "integer"}}, "type": "object"},
///                    {"additionalProperties": false, "maxProperties": 1,
///                     "properties": {"a": {"type": "integer"}}, "type": "object"}]}
/// ```
pub(super) fn saturate_stalled_object_conjuncts(
    branches: &mut [SharedSchema],
    ctx: &CanonicalizationContext,
) -> bool {
    fn requirement_is_portable(requirement: &ObjectRequirement) -> bool {
        match requirement {
            ObjectRequirement::RequiredProperty(_)
            | ObjectRequirement::DependentPropertiesRequirement { .. }
            | ObjectRequirement::DependentSchemaRequirement { .. }
            | ObjectRequirement::MinProperties(_)
            | ObjectRequirement::MaxProperties(_) => true,
            ObjectRequirement::PatternPropertyRequirement { matcher, .. } => {
                !matches!(matcher, PropertyNameMatcher::AdditionalProperties)
            }
        }
    }

    // A constraint copied into a leaf with catch-all scope would silently join its exemption set.
    fn has_catch_all_scope(leaf: &ObjectLeaf) -> bool {
        leaf.constraints.iter().any(|constraint| {
            matches!(
                constraint.matcher,
                PropertyNameMatcher::AdditionalProperties
            )
        }) || leaf.requirements.iter().any(|requirement| {
            matches!(
                requirement,
                ObjectRequirement::PatternPropertyRequirement {
                    matcher: PropertyNameMatcher::AdditionalProperties,
                    ..
                }
            )
        })
    }

    let object_indices: Vec<usize> = branches
        .iter()
        .enumerate()
        .filter(|(_, branch)| matches!(branch.as_schema(), Schema::Object(_)))
        .map(|(index, _)| index)
        .collect();
    if object_indices.len() < 2 {
        return false;
    }
    let mut changed = false;
    for &target_index in &object_indices {
        let Schema::Object(target) = branches[target_index].as_schema() else {
            unreachable!("filtered to object leaves");
        };
        let mut updated = target.clone();
        let mut leaf_changed = false;
        let constraints_portable = !has_catch_all_scope(&updated);
        for &source_index in &object_indices {
            if source_index == target_index {
                continue;
            }
            let Schema::Object(source) = branches[source_index].as_schema() else {
                unreachable!("filtered to object leaves");
            };
            for requirement in &source.requirements {
                if requirement_is_portable(requirement)
                    && !updated.requirements.contains(requirement)
                {
                    updated.requirements.push(requirement.clone());
                    leaf_changed = true;
                }
            }
            if constraints_portable {
                for constraint in &source.constraints {
                    if !matches!(
                        constraint.matcher,
                        PropertyNameMatcher::NamedProperty(_)
                            | PropertyNameMatcher::PatternProperty(_)
                    ) || updated.constraints.contains(constraint)
                    {
                        continue;
                    }
                    // A matcher already constrained in the target intersects with the copied
                    // schema: duplicate matcher entries would collide in the emitted JSON map.
                    if let Some(existing) = updated
                        .constraints
                        .iter_mut()
                        .find(|candidate| candidate.matcher == constraint.matcher)
                    {
                        let merged = intersect_canonical(&existing.schema, &constraint.schema, ctx);
                        if merged != existing.schema {
                            existing.schema = merged;
                            leaf_changed = true;
                        }
                    } else {
                        updated.constraints.push(constraint.clone());
                        leaf_changed = true;
                    }
                }
            }
        }
        if leaf_changed {
            updated.requirements.sort();
            updated.requirements.dedup();
            updated.constraints.sort();
            updated.constraints.dedup();
            branches[target_index] = shared(Schema::Object(updated));
            changed = true;
        }
    }
    changed
}

/// After saturation, an object conjunct whose absolute entries subset a sibling's is implied and drops.
/// Catch-all-scope leaves (`additionalProperties`/existential) are skipped: their meaning shifts with the sibling.
pub(super) fn drop_object_conjunct_implied_by_sibling(branches: &mut Vec<SharedSchema>) -> bool {
    fn absolute_entries(
        leaf: &ObjectLeaf,
    ) -> Option<(Vec<&ObjectRequirement>, Vec<&ObjectConstraint>)> {
        let mut requirements: Vec<&ObjectRequirement> = Vec::new();
        for requirement in &leaf.requirements {
            if matches!(
                requirement,
                ObjectRequirement::PatternPropertyRequirement {
                    matcher: PropertyNameMatcher::AdditionalProperties,
                    ..
                }
            ) {
                return None;
            }
            requirements.push(requirement);
        }
        let mut constraints: Vec<&ObjectConstraint> = Vec::new();
        for constraint in &leaf.constraints {
            if matches!(
                constraint.matcher,
                PropertyNameMatcher::AdditionalProperties
            ) {
                return None;
            }
            constraints.push(constraint);
        }
        Some((requirements, constraints))
    }

    for index in 0..branches.len() {
        let Schema::Object(leaf) = branches[index].as_schema() else {
            continue;
        };
        let Some((requirements, constraints)) = absolute_entries(leaf) else {
            continue;
        };
        let implied = branches.iter().enumerate().any(|(sibling, branch)| {
            if sibling == index {
                return false;
            }
            let Schema::Object(other) = branch.as_schema() else {
                return false;
            };
            (leaf.property_names.is_none() || leaf.property_names == other.property_names)
                && requirements
                    .iter()
                    .all(|requirement| other.requirements.contains(requirement))
                && constraints
                    .iter()
                    .all(|constraint| other.constraints.contains(constraint))
        });
        if implied {
            branches.remove(index);
            return true;
        }
    }
    false
}

/// `true` when an `allOf` mixes object leaves whose existential requirement and universal constraint contradict.
/// Unsafe-to-flatten object pairs are never folded by `object::intersect`, so the contradiction is caught here.
///
/// ```text
/// BEFORE: {"allOf": [{"required": ["a"]}, {"properties": {"a": false}, "additionalProperties": false}]}
/// AFTER:  false   // one branch requires `a`, the other admits no properties at all
/// ```
pub(super) fn object_siblings_contradict(
    branches: &[SharedSchema],
    ctx: &CanonicalizationContext,
) -> bool {
    let leaves: Vec<&ObjectLeaf> = branches
        .iter()
        .filter_map(|branch| match branch.as_schema() {
            Schema::Object(leaf) => Some(leaf),
            _ => None,
        })
        .collect();
    leaves.len() > 1 && any_object_requirement_contradiction(&leaves, ctx)
}

/// An object branch with no requirements and constraints only on named `n` admits every object missing `n`; in a
/// union a sibling's `required: n` is then redundant (dropping it only adds objects this branch already covers).
///
/// ```text
/// BEFORE: {"anyOf": [{"required": ["n"], "properties": {"n": {"type": "integer"}}},
///                    {"properties": {"n": {"type": "string"}}}]}
/// AFTER:  {"anyOf": [{"properties": {"n": {"type": "integer"}}},
///                    {"properties": {"n": {"type": "string"}}}]}   // "n" no longer required in branch 1
/// ```
pub(super) fn drop_required_properties_covered_by_optional_property_branch(
    branches: &mut [SharedSchema],
) -> bool {
    let mut optional_property_names: Vec<Arc<str>> = Vec::new();
    for branch in branches.iter() {
        let Schema::Object(leaf) = branch.as_schema() else {
            continue;
        };
        if !leaf.requirements.is_empty() || leaf.property_names.is_some() {
            continue;
        }
        let mut names = leaf
            .constraints
            .iter()
            .map(|constraint| match &constraint.matcher {
                PropertyNameMatcher::NamedProperty(name) => Some(name),
                _ => None,
            });
        let Some(Some(first)) = names.next() else {
            continue;
        };
        let first = Arc::clone(first);
        if names.all(|name| name == Some(&first)) {
            optional_property_names.push(first);
        }
    }
    if optional_property_names.is_empty() {
        return false;
    }
    for branch in branches.iter_mut() {
        let Schema::Object(leaf) = branch.as_schema() else {
            continue;
        };
        let retained: Vec<ObjectRequirement> = leaf
            .requirements
            .iter()
            .filter(|requirement| {
                !matches!(
                    requirement,
                    ObjectRequirement::RequiredProperty(name) if optional_property_names.contains(name)
                )
            })
            .cloned()
            .collect();
        if retained.len() == leaf.requirements.len() {
            continue;
        }
        let replacement = shared(Schema::Object(ObjectLeaf {
            requirements: retained,
            constraints: leaf.constraints.clone(),
            property_names: leaf.property_names.clone(),
        }));
        *branch = replacement;
        return true;
    }
    false
}

/// Dropping `required: n` admits this branch's objects lacking `n`; if a sibling covers those, the weakened spelling
/// is canonical. Coverage-checked (unlike the structural optional-property fold), so shared `propertyNames`/facets work.
///
/// ```text
/// BEFORE: {"anyOf": [{"properties": {"a": {"type": "integer"}}, "propertyNames": {"pattern": "^[a-z]+$"}, "type": "object"},
///                    {"required": ["a", "b"], "propertyNames": {"pattern": "^[a-z]+$"}, "type": "object"}]}
/// AFTER:  {"anyOf": [{"properties": {"a": {"type": "integer"}}, "propertyNames": {"pattern": "^[a-z]+$"}, "type": "object"},
///                    {"required": ["b"], "propertyNames": {"pattern": "^[a-z]+$"}, "type": "object"}]}
/// ```
pub(super) fn drop_required_property_covered_by_sibling(
    branches: &mut [SharedSchema],
    ctx: &CanonicalizationContext,
) -> bool {
    super::drop_facet_covered_by_sibling(branches, ctx, |schema| {
        let Schema::Object(leaf) = schema else {
            return None;
        };
        let position = leaf.requirements.iter().position(|requirement| {
            matches!(requirement, ObjectRequirement::RequiredProperty(_))
        })?;
        let ObjectRequirement::RequiredProperty(name) = &leaf.requirements[position] else {
            unreachable!("position matched RequiredProperty");
        };
        let name = Arc::clone(name);
        let mut weakened_requirements = leaf.requirements.clone();
        weakened_requirements.remove(position);
        // The delta is this branch's objects that lack `name`: forbid the property with a `false`
        // constraint, replacing any existing constraint on the same name.
        let mut delta_constraints: Vec<ObjectConstraint> = leaf
            .constraints
            .iter()
            .filter(|constraint| {
                !matches!(&constraint.matcher, PropertyNameMatcher::NamedProperty(other) if *other == name)
            })
            .cloned()
            .collect();
        delta_constraints.push(ObjectConstraint {
            matcher: PropertyNameMatcher::NamedProperty(Arc::clone(&name)),
            schema: shared(Schema::False),
        });
        let delta = shared(Schema::Object(ObjectLeaf {
            requirements: weakened_requirements.clone(),
            constraints: delta_constraints,
            property_names: leaf.property_names.clone(),
        }));
        let weakened = shared(Schema::Object(ObjectLeaf {
            requirements: weakened_requirements,
            constraints: leaf.constraints.clone(),
            property_names: leaf.property_names.clone(),
        }));
        Some((weakened, delta))
    })
}

/// Dropping `minProperties` admits this branch's objects below the bound; if a sibling covers those, the weakened
/// spelling is canonical (negation recomposition produces it, so the direct build order must converge to it).
///
/// ```text
/// BEFORE: {"anyOf": [{"properties": {"a": {"type": "integer"}}, "type": "object"},
///                    {"minProperties": 1, "maxProperties": 1, "type": "object"}]}
/// AFTER:  {"anyOf": [{"properties": {"a": {"type": "integer"}}, "type": "object"},
///                    {"maxProperties": 1, "type": "object"}]}   // `{}` is covered by branch 1
/// ```
pub(super) fn drop_min_properties_covered_by_sibling(
    branches: &mut [SharedSchema],
    ctx: &CanonicalizationContext,
) -> bool {
    super::drop_facet_covered_by_sibling(branches, ctx, |schema| {
        let Schema::Object(leaf) = schema else {
            return None;
        };
        let position = leaf
            .requirements
            .iter()
            .position(|requirement| matches!(requirement, ObjectRequirement::MinProperties(_)))?;
        let ObjectRequirement::MinProperties(bound) = &leaf.requirements[position] else {
            unreachable!("position matched MinProperties");
        };
        if bound.is_zero() {
            return None;
        }
        let below = bound.clone() - BoundCardinality::from(1_u8);
        let mut weakened_requirements = leaf.requirements.clone();
        weakened_requirements.remove(position);
        // The below-bound slice keeps every other facet and caps the count at `bound - 1` (the
        // tighter of that and any existing `maxProperties`).
        let mut delta_requirements = weakened_requirements.clone();
        let mut has_max_properties = false;
        for requirement in &mut delta_requirements {
            if let ObjectRequirement::MaxProperties(max_properties) = requirement {
                has_max_properties = true;
                if *max_properties > below {
                    max_properties.clone_from(&below);
                }
            }
        }
        if !has_max_properties {
            delta_requirements.push(ObjectRequirement::MaxProperties(below));
        }
        let delta = shared(Schema::Object(ObjectLeaf {
            requirements: delta_requirements,
            constraints: leaf.constraints.clone(),
            property_names: leaf.property_names.clone(),
        }));
        let weakened = shared(Schema::Object(ObjectLeaf {
            requirements: weakened_requirements,
            constraints: leaf.constraints.clone(),
            property_names: leaf.property_names.clone(),
        }));
        Some((weakened, delta))
    })
}

/// A named-property constraint drops when the objects it excludes (the name held with a value outside it) are covered
/// by a sibling. Restricted to leaves with only named constraints (no catch-all/pattern/existential scope to shift).
///
/// ```text
/// BEFORE: {"anyOf": [{"type": "object", "maxProperties": 1, "required": ["a"]},
///                    {"type": "object", "maxProperties": 1, "properties": {"a": {"type": "integer"}}}]}
/// AFTER:  {"type": "object", "maxProperties": 1}   // non-integer `a` singletons are in the first branch
/// ```
pub(super) fn drop_property_constraint_covered_by_sibling(
    branches: &mut [SharedSchema],
    ctx: &CanonicalizationContext,
) -> bool {
    let mut changed = false;
    for index in 0..branches.len() {
        let Schema::Object(leaf) = branches[index].as_schema() else {
            continue;
        };
        if leaf
            .constraints
            .iter()
            .any(|constraint| !matches!(constraint.matcher, PropertyNameMatcher::NamedProperty(_)))
            || leaf.requirements.iter().any(|requirement| {
                matches!(
                    requirement,
                    ObjectRequirement::PatternPropertyRequirement {
                        matcher: PropertyNameMatcher::AdditionalProperties,
                        ..
                    }
                )
            })
        {
            continue;
        }
        for position in 0..leaf.constraints.len() {
            let PropertyNameMatcher::NamedProperty(name) = &leaf.constraints[position].matcher
            else {
                unreachable!("non-named constraints were screened out");
            };
            let mut weakened = leaf.clone();
            weakened.constraints.remove(position);
            // The dropped entry excluded objects holding the name with a value outside the constraint schema.
            let mut delta = weakened.clone();
            delta.constraints.push(ObjectConstraint {
                matcher: PropertyNameMatcher::NamedProperty(Arc::clone(name)),
                schema: negate(&leaf.constraints[position].schema, ctx),
            });
            delta.constraints.sort();
            delta
                .requirements
                .push(ObjectRequirement::RequiredProperty(Arc::clone(name)));
            delta.requirements.sort();
            delta.requirements.dedup();
            let delta = shared(Schema::Object(delta));
            let covered = any_sibling_covers(branches, &[index], &delta, ctx);
            if covered {
                branches[index] = shared(Schema::Object(weakened));
                changed = true;
                break;
            }
        }
    }
    changed
}

use std::sync::Arc;

use ahash::AHashSet;

use crate::canonical::{
    context::{CanonicalizationContext, WalkStage},
    intern::shared,
    ir::{
        BoundCardinality, CanonicalKind, ObjectConstraint, ObjectLeaf, ObjectRequirement,
        PropertyNameMatcher, Schema, SchemaKindSet, SharedSchema,
    },
};

/// Tightens object leaves: `dependentRequired` closes over `required`, redundant always-true property entries are
/// dropped, and tautological `additionalProperties`/`dependentSchemas` fold away.
///
/// ```text
/// BEFORE: {"type": "object", "required": ["a"],
///          "dependentRequired": {"a": ["b"]}}
/// AFTER:  {"type": "object", "required": ["a", "b"]}
///
/// BEFORE: {"type": "object", "properties": {"x": true}}
/// AFTER:  {"type": "object"}
///
/// BEFORE: {"type": "object", "dependentSchemas": {"a": true}}
/// AFTER:  {"type": "object"}
/// ```
#[must_use]
pub(crate) fn normalize(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
    super::run::<ObjectStage>(schema, ctx)
}

struct ObjectStage;

impl super::NormalizeStage for ObjectStage {
    const WALK: WalkStage = WalkStage::Object;
    const MASK: SchemaKindSet = SchemaKindSet::of(CanonicalKind::Object);

    fn rewrite(recursed: SharedSchema, _ctx: &CanonicalizationContext) -> SharedSchema {
        fn latest<'a>(
            rewritten: Option<&'a ObjectLeaf>,
            original: &'a ObjectLeaf,
        ) -> &'a ObjectLeaf {
            rewritten.unwrap_or(original)
        }
        fn finish(rewritten: Option<ObjectLeaf>, recursed: SharedSchema) -> SharedSchema {
            match rewritten {
                Some(leaf) => shared(Schema::Object(leaf)),
                None => recursed,
            }
        }
        let Schema::Object(original) = recursed.as_schema() else {
            return recursed;
        };
        // Each step reads the latest leaf; `required` closes under dependentRequired before the
        // collapse below sees the required-name set.
        let steps: [fn(&ObjectLeaf) -> Option<ObjectLeaf>; 4] = [
            drop_constraints_under_zero_max,
            normalize_requirements,
            fold_required_closure,
            drop_true_object_defaults,
        ];
        let mut rewritten: Option<ObjectLeaf> = None;
        for step in steps {
            if let Some(next) = step(latest(rewritten.as_ref(), original)) {
                rewritten = Some(next);
            }
        }
        let leaf = latest(rewritten.as_ref(), original);
        if !additional_properties_is_false(leaf) {
            // `Exact(name) -> True` only matters when a catch-all would treat the name differently for being listed; else drop.
            let trimmed = if has_additional_catch_all(leaf) {
                drop_catch_all_equivalent_named_constraints(leaf)
            } else {
                drop_redundant_true_constraints(leaf)
            };
            return finish(trimmed.or(rewritten), recursed);
        }
        if has_allowing_pattern(leaf) {
            return finish(rewritten, recursed);
        }
        let blocked = collect_blocked_exact_names(leaf);
        let constraints = if blocked.is_empty() {
            leaf.constraints.clone()
        } else {
            drop_blocked_constraints(&leaf.constraints, &blocked)
        };
        let allowing_named = count_allowing_exact_names(&constraints);
        if blocked.is_empty()
            && existing_max_properties(&leaf.requirements).is_some_and(|max| max <= allowing_named)
        {
            return finish(rewritten, recursed);
        }
        let drop_catch_all = allowing_named == 0;
        let final_constraints = if drop_catch_all {
            constraints
                .into_iter()
                .filter(|constraint| {
                    !matches!(
                        constraint.matcher,
                        PropertyNameMatcher::AdditionalProperties
                    )
                })
                .collect()
        } else {
            constraints
        };
        let requirements = normalize_requirement_entries(update_requirements(
            &leaf.requirements,
            &blocked,
            allowing_named,
        ));
        shared(Schema::Object(ObjectLeaf {
            requirements,
            constraints: final_constraints,
            property_names: leaf.property_names.clone(),
        }))
    }
}

/// A named constraint whose schema equals the catch-all's adds nothing: listing the name only hands it the
/// identical schema back. Restricted to leaves without pattern/existential scope, which shifts when a name drops.
fn drop_catch_all_equivalent_named_constraints(leaf: &ObjectLeaf) -> Option<ObjectLeaf> {
    if leaf
        .constraints
        .iter()
        .any(|constraint| matches!(constraint.matcher, PropertyNameMatcher::PatternProperty(_)))
    {
        return None;
    }
    if leaf.requirements.iter().any(|requirement| {
        matches!(
            requirement,
            ObjectRequirement::PatternPropertyRequirement {
                matcher: PropertyNameMatcher::AdditionalProperties,
                ..
            }
        )
    }) {
        return None;
    }
    let catch_all = leaf.constraints.iter().find_map(|constraint| {
        matches!(
            constraint.matcher,
            PropertyNameMatcher::AdditionalProperties
        )
        .then_some(&constraint.schema)
    })?;
    let redundant = |constraint: &ObjectConstraint| {
        matches!(constraint.matcher, PropertyNameMatcher::NamedProperty(_))
            && constraint.schema == *catch_all
    };
    if !leaf.constraints.iter().any(redundant) {
        return None;
    }
    let constraints: Vec<ObjectConstraint> = leaf
        .constraints
        .iter()
        .filter(|&constraint| !redundant(constraint))
        .cloned()
        .collect();
    Some(ObjectLeaf {
        requirements: leaf.requirements.clone(),
        constraints,
        property_names: leaf.property_names.clone(),
    })
}

/// `maxProperties: 0` admits only the empty object, so per-property constraints and `propertyNames` are vacuous.
/// Requirements stay: a `required` beside the zero cap is a contradiction the emptiness pass must still see.
fn drop_constraints_under_zero_max(leaf: &ObjectLeaf) -> Option<ObjectLeaf> {
    let zero_max_properties = leaf.requirements.iter().any(|requirement| {
        matches!(requirement, ObjectRequirement::MaxProperties(value) if value.is_zero())
    });
    if !zero_max_properties || (leaf.constraints.is_empty() && leaf.property_names.is_none()) {
        return None;
    }
    Some(ObjectLeaf {
        requirements: leaf.requirements.clone(),
        constraints: Vec::new(),
        property_names: None,
    })
}

fn normalize_requirements(leaf: &ObjectLeaf) -> Option<ObjectLeaf> {
    let requirements = normalize_requirement_entries(leaf.requirements.clone());
    if requirements == leaf.requirements {
        return None;
    }
    Some(ObjectLeaf {
        requirements,
        constraints: leaf.constraints.clone(),
        property_names: leaf.property_names.clone(),
    })
}

fn normalize_requirement_entries(requirements: Vec<ObjectRequirement>) -> Vec<ObjectRequirement> {
    let mut normalized: Vec<ObjectRequirement> = requirements
        .into_iter()
        .filter_map(normalize_requirement_entry)
        .collect();
    normalized.sort();
    normalized.dedup();
    fold_count_bound_entries(&mut normalized);
    drop_implied_min_properties(&mut normalized);
    normalized
}

/// Multiple count bounds conjoin: only the largest minimum and the smallest maximum survive.
fn fold_count_bound_entries(requirements: &mut Vec<ObjectRequirement>) {
    let mut minimum: Option<BoundCardinality> = None;
    let mut maximum: Option<BoundCardinality> = None;
    for requirement in requirements.iter() {
        match requirement {
            ObjectRequirement::MinProperties(value)
                if minimum.as_ref().is_none_or(|current| value > current) =>
            {
                minimum = Some((value).owned());
            }
            ObjectRequirement::MaxProperties(value)
                if maximum.as_ref().is_none_or(|current| value < current) =>
            {
                maximum = Some((value).owned());
            }
            _ => {}
        }
    }
    requirements.retain(|requirement| match requirement {
        ObjectRequirement::MinProperties(value) => minimum.as_ref() == Some(value),
        ObjectRequirement::MaxProperties(value) => maximum.as_ref() == Some(value),
        _ => true,
    });
}

/// The distinct `required` names alone force that many properties, so a `minProperties` at or below the count
/// drops. Every construction path must agree on the dropped spelling, else negation/conjunction diverge.
///
/// ```text
/// BEFORE: {"type": "object", "required": ["a", "b"], "minProperties": 2}
/// AFTER:  {"type": "object", "required": ["a", "b"]}   // two required names already force >= 2 properties
///
/// // a strictly larger bound survives:
/// {"type": "object", "required": ["a", "b"], "minProperties": 3}  // kept as-is
/// ```
fn drop_implied_min_properties(requirements: &mut Vec<ObjectRequirement>) {
    let required_count = BoundCardinality::from(
        requirements
            .iter()
            .filter(|requirement| matches!(requirement, ObjectRequirement::RequiredProperty(_)))
            .count(),
    );
    requirements.retain(|requirement| match requirement {
        ObjectRequirement::MinProperties(value) => value > &required_count,
        _ => true,
    });
}

fn normalize_requirement_entry(requirement: ObjectRequirement) -> Option<ObjectRequirement> {
    match requirement {
        ObjectRequirement::DependentPropertiesRequirement {
            property,
            mut required_properties,
        } => {
            required_properties.sort();
            required_properties.dedup();
            if required_properties.is_empty() {
                None
            } else {
                Some(ObjectRequirement::DependentPropertiesRequirement {
                    property,
                    required_properties,
                })
            }
        }
        ObjectRequirement::DependentSchemaRequirement { schema, .. }
            if matches!(schema.as_schema(), Schema::True) =>
        {
            None
        }
        // "Some property named `n` satisfies `true`" is exactly "property `n` exists".
        ObjectRequirement::PatternPropertyRequirement {
            matcher: PropertyNameMatcher::NamedProperty(name),
            schema,
        } if matches!(schema.as_schema(), Schema::True) => {
            Some(ObjectRequirement::RequiredProperty(name))
        }
        other => Some(other),
    }
}

fn has_additional_catch_all(leaf: &ObjectLeaf) -> bool {
    leaf.constraints.iter().any(|constraint| {
        matches!(
            constraint.matcher,
            PropertyNameMatcher::AdditionalProperties
        )
    })
}

fn drop_true_object_defaults(leaf: &ObjectLeaf) -> Option<ObjectLeaf> {
    let mut changed = false;
    let constraints: Vec<ObjectConstraint> = leaf
        .constraints
        .iter()
        .filter_map(|constraint| {
            if matches!(
                constraint.matcher,
                PropertyNameMatcher::AdditionalProperties
            ) && matches!(constraint.schema.as_schema(), Schema::True)
            {
                changed = true;
                None
            } else {
                Some(constraint.clone())
            }
        })
        .collect();
    changed.then(|| ObjectLeaf {
        requirements: leaf.requirements.clone(),
        constraints,
        property_names: leaf.property_names.clone(),
    })
}

/// Drop `Exact|Pattern -> True` constraints (the property is unconstrained regardless). `Some` only when at least one
/// was removed.
fn drop_redundant_true_constraints(leaf: &ObjectLeaf) -> Option<ObjectLeaf> {
    let new_constraints: Vec<ObjectConstraint> = leaf
        .constraints
        .iter()
        .filter(|constraint| {
            !(matches!(
                constraint.matcher,
                PropertyNameMatcher::NamedProperty(_) | PropertyNameMatcher::PatternProperty(_)
            ) && matches!(constraint.schema.as_schema(), Schema::True))
        })
        .cloned()
        .collect();
    if new_constraints.len() == leaf.constraints.len() {
        return None;
    }
    Some(ObjectLeaf {
        requirements: leaf.requirements.clone(),
        constraints: new_constraints,
        property_names: leaf.property_names.clone(),
    })
}

/// Close `required` under dependentRequired, following the dependency chain transitively. Returns `Some` only when the
/// set actually grew.
///
/// ```text
/// BEFORE: {"type": "object", "required": ["a"], "dependentRequired": {"a": ["b"], "b": ["c"]}}
/// AFTER:  {"type": "object", "required": ["a", "b", "c"]}   // a pulls in b, which pulls in c
/// ```
fn fold_required_closure(leaf: &ObjectLeaf) -> Option<ObjectLeaf> {
    let mut required: AHashSet<Arc<str>> = leaf
        .requirements
        .iter()
        .filter_map(|requirement| match requirement {
            ObjectRequirement::RequiredProperty(name) => Some(Arc::clone(name)),
            _ => None,
        })
        .collect();
    if required.is_empty() {
        return None;
    }
    let dependents: Vec<(Arc<str>, Vec<Arc<str>>)> = leaf
        .requirements
        .iter()
        .filter_map(|requirement| match requirement {
            ObjectRequirement::DependentPropertiesRequirement {
                property,
                required_properties,
            } => Some((Arc::clone(property), required_properties.clone())),
            _ => None,
        })
        .collect();
    if dependents.is_empty() {
        return None;
    }

    let original_required = required.clone();
    loop {
        let mut grew = false;
        for (trigger, names) in &dependents {
            if required.contains(trigger) {
                for name in names {
                    if required.insert(Arc::clone(name)) {
                        grew = true;
                    }
                }
            }
        }
        if !grew {
            break;
        }
    }
    if required == original_required {
        return None;
    }

    // Emit `Required` entries first so source order doesn't affect the canonical shape.
    let mut new_requirements: Vec<ObjectRequirement> = Vec::with_capacity(leaf.requirements.len());
    let mut emitted: AHashSet<Arc<str>> = AHashSet::new();
    for requirement in &leaf.requirements {
        if let ObjectRequirement::RequiredProperty(name) = requirement {
            if emitted.insert(Arc::clone(name)) {
                new_requirements.push(ObjectRequirement::RequiredProperty(Arc::clone(name)));
            }
        }
    }
    let mut additions: Vec<Arc<str>> = required
        .iter()
        .filter(|name| !emitted.contains(*name))
        .cloned()
        .collect();
    additions.sort();
    for name in additions {
        new_requirements.push(ObjectRequirement::RequiredProperty(name));
    }
    for requirement in &leaf.requirements {
        match requirement {
            ObjectRequirement::RequiredProperty(_) => {}
            // Trigger is unconditionally required, so the dependency is already subsumed by the `Required` entries above.
            ObjectRequirement::DependentPropertiesRequirement { property, .. }
                if required.contains(property) => {}
            other => new_requirements.push(other.clone()),
        }
    }

    Some(ObjectLeaf {
        requirements: normalize_requirement_entries(new_requirements),
        constraints: leaf.constraints.clone(),
        property_names: leaf.property_names.clone(),
    })
}

fn has_allowing_pattern(leaf: &ObjectLeaf) -> bool {
    leaf.constraints.iter().any(|constraint| {
        matches!(constraint.matcher, PropertyNameMatcher::PatternProperty(_))
            && !matches!(constraint.schema.as_schema(), Schema::False)
    })
}

fn additional_properties_is_false(leaf: &ObjectLeaf) -> bool {
    leaf.constraints.iter().any(|constraint| {
        matches!(
            constraint.matcher,
            PropertyNameMatcher::AdditionalProperties
        ) && matches!(constraint.schema.as_schema(), Schema::False)
    })
}

fn collect_blocked_exact_names(leaf: &ObjectLeaf) -> AHashSet<Arc<str>> {
    let mut blocked = AHashSet::new();
    for constraint in &leaf.constraints {
        if let PropertyNameMatcher::NamedProperty(name) = &constraint.matcher {
            if matches!(constraint.schema.as_schema(), Schema::False) {
                blocked.insert(Arc::clone(name));
            }
        }
    }
    blocked
}

fn drop_blocked_constraints(
    constraints: &[ObjectConstraint],
    blocked: &AHashSet<Arc<str>>,
) -> Vec<ObjectConstraint> {
    constraints
        .iter()
        .filter(|constraint| match &constraint.matcher {
            PropertyNameMatcher::NamedProperty(name) => !blocked.contains(name),
            _ => true,
        })
        .cloned()
        .collect()
}

fn count_allowing_exact_names(constraints: &[ObjectConstraint]) -> u64 {
    let count = constraints
        .iter()
        .filter(|constraint| matches!(constraint.matcher, PropertyNameMatcher::NamedProperty(_)))
        .count();
    u64::try_from(count).unwrap_or(u64::MAX)
}

fn existing_max_properties(requirements: &[ObjectRequirement]) -> Option<BoundCardinality> {
    requirements
        .iter()
        .find_map(|requirement| match requirement {
            ObjectRequirement::MaxProperties(value) => Some((value).owned()),
            _ => None,
        })
}

fn update_requirements(
    requirements: &[ObjectRequirement],
    blocked: &AHashSet<Arc<str>>,
    allowing_named: u64,
) -> Vec<ObjectRequirement> {
    let mut updated: Vec<ObjectRequirement> = Vec::with_capacity(requirements.len() + 1);
    let mut found_max_properties = false;
    let allowing_named = BoundCardinality::from(allowing_named);
    for requirement in requirements {
        match requirement {
            // Keep the unsatisfiable requirement so `simplify_leaves` collapses to `False`.
            ObjectRequirement::RequiredProperty(name) if blocked.contains(name) => {
                updated.push(requirement.clone());
            }
            ObjectRequirement::MaxProperties(value) => {
                found_max_properties = true;
                updated.push(ObjectRequirement::MaxProperties(
                    value.min(&allowing_named).owned(),
                ));
            }
            other => updated.push(other.clone()),
        }
    }
    if !found_max_properties {
        updated.push(ObjectRequirement::MaxProperties(allowing_named));
    }
    updated
}

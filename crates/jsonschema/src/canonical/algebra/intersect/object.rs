use std::{collections::hash_map::Entry, sync::Arc};

use ahash::{AHashMap, AHashSet};

use crate::canonical::{
    context::{CanonicalizationContext, CompiledMatcher},
    coverage,
    intern::shared,
    ir::{
        BoundCardinality, ObjectConstraint, ObjectLeaf, ObjectRequirement, PropertyNameMatcher,
        Schema, SharedSchema,
    },
    leaves::{
        object::scope::{compiled_patterns, find_catch_all, matcher_governs, non_true_catch_all},
        Intersection, Leaf, TypedLeaf, Verdict,
    },
    prover::Prover,
};

use super::{any_unordered_pair, intersect_canonical, intersect_internal, intersect_optional};

impl Leaf for ObjectLeaf {
    /// Union requirement lists, tighten `minProperties`/`maxProperties`, intersect per-property constraints.
    /// `Residual` when a catch-all plus opposite-side pattern cannot flatten into one leaf.
    ///
    /// ```text
    /// BEFORE: {"type": "object", "required": ["a"]}  and  {"type": "object", "required": ["b"]}
    /// AFTER:  {"type": "object", "required": ["a", "b"]}
    ///
    /// BEFORE: {"type": "object", "properties": {"x": {"minimum": 0}}}  and  {"type": "object", "properties": {"x": {"maximum": 10}}}
    /// AFTER:  {"type": "object", "properties": {"x": {"minimum": 0, "maximum": 10}}}
    /// ```
    fn intersect(&self, other: &Self, ctx: &CanonicalizationContext) -> Intersection<Self> {
        // A required key in one leaf can clash with the other's per-key constraint even when the
        // two can't be flattened together, so check this before the flatten bail below.
        if any_object_requirement_contradiction(&[self, other], ctx) {
            return Intersection::Empty;
        }
        // Catch-all plus pattern can't flatten into one object; keep both as AllOf. A pattern the
        // engine cannot compile has an unknown match scope, so its cross-side narrowing is undecidable.
        if unsafe_to_flatten(self, other)
            || has_uncompilable_pattern(self, ctx)
            || has_uncompilable_pattern(other, ctx)
        {
            return Intersection::Residual;
        }
        let leaf = ObjectLeaf {
            requirements: union_requirements(&self.requirements, &other.requirements, ctx),
            constraints: intersect_constraints(&self.constraints, &other.constraints, ctx),
            property_names: intersect_optional(
                self.property_names.as_ref(),
                other.property_names.as_ref(),
                ctx,
            ),
        };
        if requirement_contradicts_constraint(&leaf, ctx) {
            return Intersection::Empty;
        }
        Intersection::Merged(leaf)
    }

    fn covers(&self, other: &Self, prover: &Prover<'_>) -> Verdict {
        Verdict::proven_if(coverage::object_leaf_covers(self, other, prover))
    }

    /// Anything forcing a property to exist interacts with `propertyNames` / `dependentSchemas`
    /// in ways the pipeline does not decide.
    fn inhabited(&self, _formats_asserted: bool) -> Verdict {
        let needs_property = self
            .requirements
            .iter()
            .any(|requirement| match requirement {
                ObjectRequirement::RequiredProperty(_)
                | ObjectRequirement::PatternPropertyRequirement { .. } => true,
                ObjectRequirement::MinProperties(min) => !min.is_zero(),
                ObjectRequirement::MaxProperties(_)
                | ObjectRequirement::DependentPropertiesRequirement { .. }
                | ObjectRequirement::DependentSchemaRequirement { .. } => false,
            });
        Verdict::proven_if(
            !needs_property
                || (self.property_names.is_none()
                    && !self.requirements.iter().any(|requirement| {
                        matches!(
                            requirement,
                            ObjectRequirement::DependentSchemaRequirement { .. }
                        )
                    })),
        )
    }

    fn is_open(&self) -> bool {
        self.requirements.is_empty() && self.constraints.is_empty() && self.property_names.is_none()
    }

    fn is_empty(&self, ctx: &CanonicalizationContext) -> bool {
        let required_names = object_collect_required(self);
        if required_property_has_false_schema(self, &required_names) {
            return true;
        }
        if property_names_block_required(self, &required_names) {
            return true;
        }
        if object_existential_impossible(self, ctx) {
            return true;
        }
        object_size_bounds_empty(self, &required_names)
    }
}

impl TypedLeaf for ObjectLeaf {
    fn wrap(self) -> Schema {
        Schema::Object(self)
    }
    fn project(schema: &Schema) -> Option<&Self> {
        match schema {
            Schema::Object(leaf) => Some(leaf),
            _ => None,
        }
    }
}

fn object_collect_required(leaf: &ObjectLeaf) -> Vec<Arc<str>> {
    let mut seen: AHashSet<Arc<str>> = AHashSet::new();
    let mut required = Vec::new();
    for requirement in &leaf.requirements {
        if let ObjectRequirement::RequiredProperty(name) = requirement {
            if seen.insert(Arc::clone(name)) {
                required.push(Arc::clone(name));
            }
        }
    }
    required
}

fn required_property_has_false_schema(leaf: &ObjectLeaf, required: &[Arc<str>]) -> bool {
    for name in required {
        for constraint in &leaf.constraints {
            if let PropertyNameMatcher::NamedProperty(other) = &constraint.matcher {
                if other == name && matches!(constraint.schema.as_schema(), Schema::False) {
                    return true;
                }
            }
        }
    }
    false
}

fn property_names_block_required(leaf: &ObjectLeaf, required: &[Arc<str>]) -> bool {
    let Some(names_schema) = leaf.property_names.as_ref() else {
        return false;
    };
    required
        .iter()
        .any(|name| object_name_rejected(names_schema, name))
}

fn object_name_rejected(schema: &SharedSchema, name: &str) -> bool {
    match schema.as_schema() {
        Schema::False => true,
        Schema::String(string_leaf) => {
            let length = bytecount::num_chars(name.as_bytes()) as u64;
            if let Some(min) = &string_leaf.min_length {
                if length < *min {
                    return true;
                }
            }
            if let Some(max) = &string_leaf.max_length {
                if length > *max {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

fn object_existential_impossible(leaf: &ObjectLeaf, ctx: &CanonicalizationContext) -> bool {
    leaf.requirements.iter().any(|requirement| {
        let ObjectRequirement::PatternPropertyRequirement { matcher, schema: t } = requirement
        else {
            return false;
        };
        match matcher {
            PropertyNameMatcher::AdditionalProperties => {
                find_catch_all(&leaf.constraints).is_some()
                    && leaf
                        .constraints
                        .iter()
                        .all(|c| object_intersect_is_empty(&c.schema, t, ctx))
            }
            PropertyNameMatcher::NamedProperty(name) => leaf.constraints.iter().any(|c| {
                matches!(&c.matcher, PropertyNameMatcher::NamedProperty(m) if m == name)
                    && object_intersect_is_empty(&c.schema, t, ctx)
            }),
            PropertyNameMatcher::PatternProperty(_) => false,
        }
    })
}

fn object_intersect_is_empty(
    left: &SharedSchema,
    right: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> bool {
    matches!(
        intersect_canonical(left, right, ctx).as_schema(),
        Schema::False
    )
}

fn object_size_bounds_empty(leaf: &ObjectLeaf, required: &[Arc<str>]) -> bool {
    let min_required = BoundCardinality::from(required.len());
    let mut min_properties = min_required;
    let mut max_properties: Option<BoundCardinality> = None;
    for requirement in &leaf.requirements {
        match requirement {
            ObjectRequirement::MinProperties(value) if value > &min_properties => {
                min_properties = (value).owned();
            }
            ObjectRequirement::MaxProperties(value) => {
                max_properties = Some(match max_properties.as_ref() {
                    Some(existing) => (existing.min(value)).owned(),
                    None => (value).owned(),
                });
            }
            _ => {}
        }
    }
    matches!(max_properties.as_ref(), Some(max) if &min_properties > max)
}

/// Detects when several object leaves combined under `allOf` describe an object no value can satisfy.
///
/// An unsafe-to-flatten catch-all/pattern pair (see [`unsafe_to_flatten`]) stays split, so a contradiction can sit
/// *across* leaves where single-leaf [`requirement_contradicts_constraint`] never sees it. Three kinds are caught:
///
/// `minProperties`/`maxProperties` whose tightest bounds cross:
/// ```text
/// BEFORE: {"allOf": [{"minProperties": 2}, {"maxProperties": 1}]}
/// AFTER:  false                          // at least 2 keys, yet at most 1
/// ```
///
/// An existential requirement (some property must exist) against `maxProperties: 0`:
/// ```text
/// BEFORE: {"not": {"additionalProperties": false}, "maxProperties": 0}
/// AFTER:  false                          // the `not` forces one property; `maxProperties: 0` forbids all
/// ```
///
/// A required key matching `M` (value in `T`) against a same-`M` constraint requiring `S`, when `S ∧ T` is empty. The
/// constraint must provably govern the key (see [`matcher_governs`]); bare matcher equality would invent false contradictions.
/// ```text
/// BEFORE: {"allOf": [{"not": {"patternProperties": {"^x": {"type": "string"}}}},
///                    {"patternProperties": {"^x": {"type": "string"}}}]}
/// AFTER:  false                          // the `not` needs a non-string "^x" key; the sibling needs every "^x" key
///                                        // to be a string
/// ```
pub(crate) fn any_object_requirement_contradiction(
    leaves: &[&ObjectLeaf],
    ctx: &CanonicalizationContext,
) -> bool {
    // Tightest `minProperties`/`maxProperties` across all leaves; `min > max` is unsatisfiable. Catches contradictions
    // the unsafe-to-flatten bail keeps split across leaves (a pattern leaf's minimum vs a catch-all's maximum).
    let min_required = leaves
        .iter()
        .flat_map(|leaf| leaf.requirements.iter())
        .filter_map(|requirement| match requirement {
            ObjectRequirement::MinProperties(min) => Some(min),
            _ => None,
        })
        .max();
    let max_allowed = leaves
        .iter()
        .flat_map(|leaf| leaf.requirements.iter())
        .filter_map(|requirement| match requirement {
            ObjectRequirement::MaxProperties(max) => Some(max),
            _ => None,
        })
        .min();
    if let (Some(min), Some(max)) = (min_required, max_allowed) {
        if min > max {
            return true;
        }
    }
    // An existential needs at least one property; a `maxProperties: 0` sibling forbids all of them.
    let has_existential = leaves.iter().any(|leaf| {
        leaf.requirements.iter().any(|requirement| {
            matches!(
                requirement,
                ObjectRequirement::PatternPropertyRequirement { .. }
            )
        })
    });
    if has_existential
        && leaves.iter().any(|leaf| {
            leaf.requirements.iter().any(|requirement| {
                matches!(requirement, ObjectRequirement::MaxProperties(max) if max.is_zero())
            })
        })
    {
        return true;
    }
    leaves.iter().any(|requirement_leaf| {
        requirement_leaf.requirements.iter().any(|requirement| {
            let ObjectRequirement::PatternPropertyRequirement { matcher, schema } = requirement
            else {
                return false;
            };
            leaves.iter().any(|constraint_leaf| {
                constraint_leaf.constraints.iter().any(|constraint| {
                    matcher_governs(matcher, &constraint.matcher, constraint_leaf)
                        && matches!(
                            intersect_canonical(&constraint.schema, schema, ctx).as_schema(),
                            Schema::False
                        )
                })
            })
        })
    })
}

/// Single-leaf counterpart of the third case in [`any_object_requirement_contradiction`], after a flatten merge gathered
/// both into one leaf: a required `M`-key (value `T`) plus same-`M` constraint `S` needs a key in `S ∧ T`, else empty.
/// ```text
/// BEFORE: {"type": "object", "additionalProperties": {"type": "integer"},
///          "not": {"additionalProperties": {"type": ["integer", "string"]}}}
/// AFTER:  false                          // every property must be an integer, yet the `not` needs one that is
///                                        // neither integer nor string
/// ```
fn requirement_contradicts_constraint(leaf: &ObjectLeaf, ctx: &CanonicalizationContext) -> bool {
    leaf.requirements.iter().any(|requirement| {
        let ObjectRequirement::PatternPropertyRequirement { matcher, schema } = requirement else {
            return false;
        };
        leaf.constraints.iter().any(|constraint| {
            // `intersect_canonical` (not `intersect_internal`) so the collapse pass that detects an `S ∧ ¬S`
            // contradiction runs - the raw intersect would leave it an unresolved `AllOf`.
            matcher_governs(matcher, &constraint.matcher, leaf)
                && matches!(
                    intersect_canonical(&constraint.schema, schema, ctx).as_schema(),
                    Schema::False
                )
        })
    })
}

// One side's non-True catch-all combined with the other side's pattern is not representable as a single flat object
// (matched names would be wrongly exempted from the catch-all), so the caller bails to AllOf.
fn unsafe_to_flatten(left: &ObjectLeaf, right: &ObjectLeaf) -> bool {
    (non_true_catch_all(left).is_some() && has_pattern(right))
        || (non_true_catch_all(right).is_some() && has_pattern(left))
}

fn has_pattern(leaf: &ObjectLeaf) -> bool {
    leaf.constraints
        .iter()
        .any(|constraint| matches!(constraint.matcher, PropertyNameMatcher::PatternProperty(_)))
}

/// Whether the leaf carries a `patternProperties` regex the configured engine rejects; its match
/// scope is then unknown and every cross-side narrowing must defer.
fn has_uncompilable_pattern(leaf: &ObjectLeaf, ctx: &CanonicalizationContext) -> bool {
    leaf.constraints
        .iter()
        .any(|constraint| match &constraint.matcher {
            PropertyNameMatcher::PatternProperty(pattern) => ctx.compile_regex(pattern).is_none(),
            _ => false,
        })
}

// True when `left` and `right` are both objects whose intersection bails to `AllOf` (see
// `unsafe_to_flatten` and `has_uncompilable_pattern`).
pub(crate) fn is_unsafe_object_pair(
    left: &SharedSchema,
    right: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> bool {
    matches!(
        (left.as_schema(), right.as_schema()),
        (Schema::Object(left), Schema::Object(right))
            if unsafe_to_flatten(left, right)
                || has_uncompilable_pattern(left, ctx)
                || has_uncompilable_pattern(right, ctx)
    )
}

// True when any pair of branches is an unsafe object pair (its intersection bails back to `AllOf`).
pub(crate) fn has_unsafe_object_pair(
    branches: &[SharedSchema],
    ctx: &CanonicalizationContext,
) -> bool {
    any_unordered_pair(branches, |left, right| {
        is_unsafe_object_pair(left, right, ctx)
    })
}

fn union_requirements(
    left: &[ObjectRequirement],
    right: &[ObjectRequirement],
    ctx: &CanonicalizationContext,
) -> Vec<ObjectRequirement> {
    let mut dependent_required: AHashMap<Arc<str>, Vec<Arc<str>>> = AHashMap::new();
    let mut dependent_schema: AHashMap<Arc<str>, SharedSchema> = AHashMap::new();
    let mut min_properties: Option<BoundCardinality> = None;
    let mut max_properties: Option<BoundCardinality> = None;
    let mut seen_simple: AHashSet<ObjectRequirement> = AHashSet::new();
    let mut out: Vec<ObjectRequirement> = Vec::with_capacity(left.len() + right.len());

    for requirement in left.iter().chain(right.iter()) {
        match requirement {
            ObjectRequirement::MinProperties(value) => {
                min_properties = Some(match min_properties.as_ref() {
                    Some(current) => (current.max(value)).owned(),
                    None => (value).owned(),
                });
            }
            ObjectRequirement::MaxProperties(value) => {
                max_properties = Some(match max_properties.as_ref() {
                    Some(current) => (current.min(value)).owned(),
                    None => (value).owned(),
                });
            }
            ObjectRequirement::DependentPropertiesRequirement {
                property,
                required_properties,
            } => match dependent_required.entry(Arc::clone(property)) {
                Entry::Vacant(slot) => {
                    slot.insert(required_properties.clone());
                }
                Entry::Occupied(mut slot) => {
                    for name in required_properties {
                        if !slot.get().contains(name) {
                            slot.get_mut().push(Arc::clone(name));
                        }
                    }
                }
            },
            ObjectRequirement::DependentSchemaRequirement { property, schema } => {
                match dependent_schema.entry(Arc::clone(property)) {
                    Entry::Vacant(slot) => {
                        slot.insert(Arc::clone(schema));
                    }
                    Entry::Occupied(mut slot) => {
                        let combined = intersect_internal(slot.get(), schema, ctx);
                        slot.insert(combined);
                    }
                }
            }
            other @ (ObjectRequirement::RequiredProperty(_)
            | ObjectRequirement::PatternPropertyRequirement { .. }) => {
                // `PatternPropertyRequirement` is existential: each branch can be satisfied by a different matching
                // key. Dedup only on structural equality.
                if seen_simple.insert(other.clone()) {
                    out.push(other.clone());
                }
            }
        }
    }

    if let Some(value) = min_properties {
        out.push(ObjectRequirement::MinProperties(value));
    }
    if let Some(value) = max_properties {
        out.push(ObjectRequirement::MaxProperties(value));
    }
    // Fold dependentRequired into dependentSchemas for shared properties.
    for property in dependent_required
        .keys()
        .filter(|key| dependent_schema.contains_key(*key))
        .cloned()
        .collect::<Vec<_>>()
    {
        let required = dependent_required
            .remove(&property)
            .expect("collected only present keys");
        let existing = dependent_schema
            .remove(&property)
            .expect("collected only present keys");
        let required_schema = required_schema_for(&required);
        let combined = intersect_internal(&existing, &required_schema, ctx);
        dependent_schema.insert(property, combined);
    }
    let mut dependent_required: Vec<(Arc<str>, Vec<Arc<str>>)> =
        dependent_required.into_iter().collect();
    dependent_required.sort_by(|left, right| left.0.cmp(&right.0));
    for (property, mut required_properties) in dependent_required {
        required_properties.sort();
        out.push(ObjectRequirement::DependentPropertiesRequirement {
            property,
            required_properties,
        });
    }
    let mut dependent_schema: Vec<(Arc<str>, SharedSchema)> =
        dependent_schema.into_iter().collect();
    dependent_schema.sort_by(|left, right| left.0.cmp(&right.0));
    for (property, schema) in dependent_schema {
        out.push(ObjectRequirement::DependentSchemaRequirement { property, schema });
    }
    out
}

/// Schema equivalent of `{"required": [names...]}`.
fn required_schema_for(names: &[Arc<str>]) -> SharedSchema {
    shared(Schema::Object(ObjectLeaf {
        requirements: names
            .iter()
            .map(|name| ObjectRequirement::RequiredProperty(Arc::clone(name)))
            .collect(),
        constraints: Vec::new(),
        property_names: None,
    }))
}

fn intersect_constraints(
    left: &[ObjectConstraint],
    right: &[ObjectConstraint],
    ctx: &CanonicalizationContext,
) -> Vec<ObjectConstraint> {
    // A `True` catch-all is the intersection identity - it constrains nothing - so treat it as absent and skip
    // building the exact-name set it would otherwise need.
    let right_catch_all = find_catch_all(right);
    let left_catch_all = find_catch_all(left);
    let right_exact_names = right_catch_all.map(|_| exact_names(right));
    // Pre-compile patterns once; cross-matching runs them in a nested loop.
    let right_patterns = compiled_patterns(right, ctx);
    let left_patterns = compiled_patterns(left, ctx);
    // Index each side by matcher so same-matcher lookups are O(1). Canonical objects have unique matchers (JSON keys are
    // unique and `intersect_constraints` folds duplicates away), so a plain insert never overwrites.
    let mut right_by_matcher: AHashMap<&PropertyNameMatcher, SharedSchema> =
        AHashMap::with_capacity(right.len());
    for right_constraint in right {
        right_by_matcher.insert(
            &right_constraint.matcher,
            Arc::clone(&right_constraint.schema),
        );
    }
    let left_matchers: AHashSet<&PropertyNameMatcher> =
        left.iter().map(|constraint| &constraint.matcher).collect();
    let mut out: Vec<ObjectConstraint> = Vec::with_capacity(left.len() + right.len());
    for constraint in left {
        let mut schema = Arc::clone(&constraint.schema);
        if let Some(other) = right_by_matcher.get(&constraint.matcher) {
            schema = intersect_internal(&schema, other, ctx);
        }
        if let PropertyNameMatcher::NamedProperty(name) = &constraint.matcher {
            schema = narrow_named_by_opposite_side(
                schema,
                name,
                right_catch_all,
                &right_patterns,
                right_exact_names.as_ref(),
                ctx,
            );
        }
        out.push(ObjectConstraint {
            matcher: constraint.matcher.clone(),
            schema,
        });
    }
    for constraint in right {
        if left_matchers.contains(&constraint.matcher) {
            continue;
        }
        let mut schema = Arc::clone(&constraint.schema);
        if let PropertyNameMatcher::NamedProperty(name) = &constraint.matcher {
            // This loop only sees right names with no left matcher, so `name` is never a left exact name.
            schema = narrow_named_by_opposite_side(
                schema,
                name,
                left_catch_all,
                &left_patterns,
                None,
                ctx,
            );
        }
        out.push(ObjectConstraint {
            matcher: constraint.matcher.clone(),
            schema,
        });
    }
    out.sort_by(|left, right| left.matcher.cmp(&right.matcher));
    out
}

/// Fold the opposite side's catch-all and matching patterns into a named property's schema. The
/// catch-all governs only names its own side matches with nothing else: an exact-name twin
/// (`opposite_exact_names`) or a matching pattern exempts the name.
fn narrow_named_by_opposite_side(
    mut schema: SharedSchema,
    name: &str,
    opposite_catch_all: Option<&SharedSchema>,
    opposite_patterns: &[(Arc<CompiledMatcher>, SharedSchema)],
    opposite_exact_names: Option<&AHashSet<Arc<str>>>,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    if let Some(catch_all) = opposite_catch_all {
        let is_exact = opposite_exact_names.is_some_and(|names| names.contains(name));
        let matches_pattern = opposite_patterns
            .iter()
            .any(|(regex, _)| regex.is_match(name));
        if !is_exact && !matches_pattern {
            schema = intersect_internal(&schema, catch_all, ctx);
        }
    }
    for (regex, pattern_schema) in opposite_patterns {
        if regex.is_match(name) {
            schema = intersect_internal(&schema, pattern_schema, ctx);
        }
    }
    schema
}

fn exact_names(constraints: &[ObjectConstraint]) -> AHashSet<Arc<str>> {
    constraints
        .iter()
        .filter_map(|constraint| match &constraint.matcher {
            PropertyNameMatcher::NamedProperty(name) => Some(Arc::clone(name)),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::canonical::options;

    // A pattern the configured engine cannot compile makes its match scope unknown; the merge must
    // keep both operands instead of folding the catch-all into a named property.
    #[test]
    fn uncompilable_pattern_defers_instead_of_narrowing() {
        let left = json!({"properties": {"x1": {"type": "string"}}});
        let right = json!({
            "patternProperties": {"^x(?=1)": true},
            "additionalProperties": {"type": "integer"}
        });
        let canonical_left = options()
            .with_pattern_options(&crate::PatternOptions::regex())
            .canonicalize(&left)
            .expect("left canonicalizes");
        let canonical_right = options()
            .with_pattern_options(&crate::PatternOptions::regex())
            .canonicalize(&right)
            .expect("right canonicalizes");
        let intersection = canonical_left.intersect(&canonical_right).to_json_schema();
        let validator = crate::validator_for(&intersection).expect("intersection compiles");
        // {"x1": "foo"} satisfies both operands (the lookahead pattern governs "x1" and exempts it
        // from the integer catch-all), so the intersection must accept it.
        assert!(validator.is_valid(&json!({"x1": "foo"})));
    }
}

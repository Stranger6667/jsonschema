//! Property-name matcher scope, shared by intersect, coverage, and normalization.
//!
//! Constraint and requirement matchers are scoped differently, so relate them only via these helpers:
//! - A *constraint* matcher is leaf-scoped: `AdditionalProperties` matches names no sibling claims.
//! - A *requirement* (existential) matcher is absolute: `AdditionalProperties` means *any* key.

use std::sync::Arc;

use crate::canonical::{
    context::{CanonicalizationContext, CompiledMatcher},
    ir::{ObjectConstraint, ObjectLeaf, PropertyNameMatcher, Schema, SharedSchema},
};

/// Whether `constraint_matcher` (in `constraint_leaf`) binds every key `requirement_matcher` could
/// demand. Named/pattern govern only the identical key; `additionalProperties` only with no
/// named/pattern sibling (then the catch-all spans every key).
pub(crate) fn matcher_governs(
    requirement_matcher: &PropertyNameMatcher,
    constraint_matcher: &PropertyNameMatcher,
    constraint_leaf: &ObjectLeaf,
) -> bool {
    match (requirement_matcher, constraint_matcher) {
        // Exact same named/pattern key: the constraint binds it directly.
        (
            PropertyNameMatcher::PatternProperty(required),
            PropertyNameMatcher::PatternProperty(constrained),
        )
        | (
            PropertyNameMatcher::NamedProperty(required),
            PropertyNameMatcher::NamedProperty(constrained),
        ) => required == constrained,
        (PropertyNameMatcher::AdditionalProperties, PropertyNameMatcher::AdditionalProperties) => {
            !constraint_leaf.constraints.iter().any(|constraint| {
                matches!(
                    constraint.matcher,
                    PropertyNameMatcher::NamedProperty(_) | PropertyNameMatcher::PatternProperty(_)
                )
            })
        }
        _ => false,
    }
}

/// The `additionalProperties` constraint schema, if declared.
pub(crate) fn find_catch_all(constraints: &[ObjectConstraint]) -> Option<&SharedSchema> {
    constraints.iter().find_map(|constraint| {
        matches!(
            constraint.matcher,
            PropertyNameMatcher::AdditionalProperties
        )
        .then_some(&constraint.schema)
    })
}

/// The catch-all when it actually constrains (`True` is the intersection identity).
pub(crate) fn non_true_catch_all(leaf: &ObjectLeaf) -> Option<&SharedSchema> {
    find_catch_all(&leaf.constraints).filter(|schema| !matches!(schema.as_schema(), Schema::True))
}

/// Compiled `patternProperties` matchers paired with their schemas. A pattern that fails to compile
/// is dropped, so callers must read its absence as "unknown" - conservative at every call site.
pub(crate) fn compiled_patterns(
    constraints: &[ObjectConstraint],
    ctx: &CanonicalizationContext,
) -> Vec<(Arc<CompiledMatcher>, SharedSchema)> {
    constraints
        .iter()
        .filter_map(|constraint| match &constraint.matcher {
            PropertyNameMatcher::PatternProperty(pattern) => ctx
                .compile_regex(pattern)
                .map(|regex| (regex, Arc::clone(&constraint.schema))),
            _ => None,
        })
        .collect()
}

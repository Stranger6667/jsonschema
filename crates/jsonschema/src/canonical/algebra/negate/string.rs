//! String-domain negation: length windows, pattern duals, and format residuals.

use std::sync::Arc;

use crate::{
    canonical::{
        intern::shared,
        ir::{Schema, SharedSchema, StringLeaf},
    },
    JsonType,
};

use super::{any_of_complement, not_wrap};

pub(super) fn negate_string_pattern_type_guard(
    kind: JsonType,
    body: &SharedSchema,
) -> Option<SharedSchema> {
    if kind != JsonType::String {
        return None;
    }
    let Schema::String(leaf) = body.as_schema() else {
        return None;
    };
    let [pattern] = leaf.patterns.as_slice() else {
        return None;
    };
    if leaf.min_length.is_none()
        && leaf.max_length.is_none()
        && leaf.not_patterns.is_empty()
        && leaf.format.is_none()
        && leaf.content.is_empty()
    {
        Some(shared(Schema::String(StringLeaf {
            not_patterns: vec![Arc::clone(pattern)],
            ..StringLeaf::default()
        })))
    } else {
        None
    }
}

/// Complement a string leaf: the length window flips out, `pattern` ↔ `not_patterns` per branch, `format`/
/// `content` keep a `Not` residual scoped to those facets, and every non-string kind joins the union.
///
/// ```text
/// BEFORE: {"type": "string", "maxLength": 8}
/// AFTER:  {"anyOf": [
///           {"type": "string", "minLength": 9},
///           {"type": ["null", "boolean", "number", "array", "object"]}
///         ]}
/// ```
pub(super) fn negate_string(leaf: &StringLeaf) -> SharedSchema {
    let mut in_kind: Vec<SharedSchema> = Vec::new();
    if let Some(min_length) = &leaf.min_length {
        if let Some(max_length) = min_length.checked_decrement() {
            in_kind.push(shared(Schema::String(StringLeaf {
                min_length: None,
                max_length: Some(max_length),
                ..StringLeaf::default()
            })));
        }
    }
    if let Some(max_length) = &leaf.max_length {
        // A string violates `maxLength` only by being strictly longer. In the default build, `u64::MAX + 1` is not
        // representable, so that disjunct is empty; under `arbitrary-precision` the exact successor is stored.
        if let Some(min_length) = max_length.checked_increment() {
            in_kind.push(shared(Schema::String(StringLeaf {
                min_length: Some(min_length),
                max_length: None,
                ..StringLeaf::default()
            })));
        }
    }
    // not(p1 and .. and pn) = or_i not(pi): a string failing pattern `pi`.
    for pattern in &leaf.patterns {
        in_kind.push(shared(Schema::String(StringLeaf {
            not_patterns: vec![Arc::clone(pattern)],
            ..StringLeaf::default()
        })));
    }
    // The dual direction: a string that *does* match an excluded pattern.
    for pattern in &leaf.not_patterns {
        in_kind.push(shared(Schema::String(StringLeaf {
            patterns: vec![Arc::clone(pattern)],
            ..StringLeaf::default()
        })));
    }
    // `format`/`content` have no IR dual; keep a `Not` residual scoped to just those facets.
    if leaf.format.is_some() || !leaf.content.is_empty() {
        in_kind.push(not_wrap(shared(Schema::String(StringLeaf {
            format: leaf.format.clone(),
            content: leaf.content.clone(),
            ..StringLeaf::default()
        }))));
    }
    any_of_complement(JsonType::String, in_kind)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::canonical::{
        ir::{Schema, StringLeaf},
        tests_util::canonicalize,
    };

    // Detects only the `Not(String{patterns set})` residual the dual replaces.
    fn has_pattern_not_residual(schema: &Schema) -> bool {
        if let Schema::Not(inner) = schema {
            if matches!(inner.as_schema(), Schema::String(l) if !l.patterns.is_empty()) {
                return true;
            }
        }
        schema
            .children()
            .iter()
            .any(|c| has_pattern_not_residual(c.as_schema()))
    }

    #[test]
    fn negate_string_pattern_has_no_not_residual() {
        let negated = canonicalize(&json!({"type": "string", "pattern": "^a"})).negate();
        assert!(!has_pattern_not_residual(negated.as_schema()));
    }

    #[test]
    fn negate_untyped_string_pattern_returns_string_not_pattern() {
        let negated = canonicalize(&json!({"pattern": "^a"})).negate();
        assert_eq!(
            negated.as_schema(),
            &Schema::String(StringLeaf {
                not_patterns: vec![std::sync::Arc::from("^a")],
                ..StringLeaf::default()
            })
        );
    }
}

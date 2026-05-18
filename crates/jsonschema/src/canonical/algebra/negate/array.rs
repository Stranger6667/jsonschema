//! Array-domain negation: length windows, prefix/tail complements, and uniqueness duals.

use std::sync::Arc;

use crate::{
    canonical::{
        context::CanonicalizationContext,
        intern::shared,
        ir::{ArrayLeaf, BoundCardinality, ContainsClause, LengthBounds, Schema, SharedSchema},
    },
    JsonType,
};

use super::{any_of_complement, negate_with_options, not_wrap, wrap_in_kind, NegationOptions};

pub(super) fn negate_array_unique_items_type_guard(
    kind: JsonType,
    body: &SharedSchema,
) -> Option<SharedSchema> {
    if kind != JsonType::Array {
        return None;
    }
    let Schema::Array(leaf) = body.as_schema() else {
        return None;
    };
    if leaf.unique_items
        && !leaf.repeated_items
        && leaf.prefix.is_empty()
        && matches!(leaf.tail.as_schema(), Schema::True)
        && leaf.length == LengthBounds::default()
        && leaf.contains.is_empty()
    {
        return Some(shared(Schema::Array(
            ArrayLeaf {
                unique_items: false,
                repeated_items: true,
                ..leaf.clone()
            }
            .normalize_repeated_items()?,
        )));
    }
    None
}

/// Emit one branch per facet violation, falling back to `Not(leaf)` for facets without an IR-direct complement.
///
/// ```text
/// BEFORE: {"type": "array", "minItems": 2}
/// AFTER:  {"anyOf": [
///           {"type": "array", "maxItems": 1},
///           {"type": ["null", "boolean", "number", "string", "object"]}
///         ]}
/// ```
pub(super) fn negate_array_leaf(
    schema: &SharedSchema,
    leaf: &ArrayLeaf,
    options: NegationOptions,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    let mut branches: Vec<SharedSchema> = Vec::new();
    let mut needs_fallback = false;

    if let Some(maximum) = leaf.length.minimum.checked_decrement() {
        branches.push(shared(Schema::Array(empty_array_leaf_with_length(
            LengthBounds {
                minimum: BoundCardinality::from(0u64),
                maximum: Some(maximum),
            },
        ))));
    }
    if let Some(max) = &leaf.length.maximum {
        if let Some(minimum) = max.checked_increment() {
            branches.push(shared(Schema::Array(empty_array_leaf_with_length(
                LengthBounds {
                    minimum,
                    maximum: None,
                },
            ))));
        }
    }

    let true_padding = shared(Schema::True);
    for (index, prefix_schema) in leaf.prefix.iter().enumerate() {
        if matches!(prefix_schema.as_schema(), Schema::True) {
            continue;
        }
        let mut new_prefix: Vec<SharedSchema> = Vec::with_capacity(index + 1);
        new_prefix.extend(std::iter::repeat_n(Arc::clone(&true_padding), index));
        new_prefix.push(negate_with_options(prefix_schema, options, ctx));
        branches.push(shared(Schema::Array(ArrayLeaf {
            prefix: new_prefix,
            tail: Arc::clone(&true_padding),
            length: LengthBounds {
                minimum: BoundCardinality::from(index + 1),
                maximum: None,
            },
            ..ArrayLeaf::default()
        })));
    }

    // Tail negation via `contains` reads every position; only safe with no prefix.
    if leaf.prefix.is_empty()
        && !matches!(leaf.tail.as_schema(), Schema::True)
        && leaf.contains.is_empty()
        && options.use_contains_for_array_tail
    {
        branches.push(shared(Schema::Array(ArrayLeaf {
            contains: vec![ContainsClause {
                schema: negate_with_options(&leaf.tail, options, ctx),
                min_contains: BoundCardinality::from(1u64),
                max_contains: None,
            }],
            ..ArrayLeaf::default()
        })));
    } else if !matches!(leaf.tail.as_schema(), Schema::True) {
        needs_fallback = true;
    }

    // `uniqueItems` ↔ `repeated_items` dual (`not(unique)` is an array with a duplicate). Only clean when no
    // `contains`/tail-prefix fallback is in play.
    let clean = leaf.contains.is_empty() && !needs_fallback;
    if leaf.unique_items {
        if clean {
            branches.push(shared(Schema::Array(ArrayLeaf {
                unique_items: false,
                repeated_items: true,
                ..leaf.clone()
            })));
        } else {
            needs_fallback = true;
        }
    }
    if leaf.repeated_items {
        if clean {
            branches.push(shared(Schema::Array(ArrayLeaf {
                unique_items: true,
                repeated_items: false,
                ..leaf.clone()
            })));
        } else {
            needs_fallback = true;
        }
    }
    if !leaf.contains.is_empty() {
        needs_fallback = true;
    }

    if needs_fallback {
        branches.push(not_wrap(Arc::clone(schema)));
    }

    any_of_complement(JsonType::Array, wrap_in_kind(branches))
}

fn empty_array_leaf_with_length(length: LengthBounds) -> ArrayLeaf {
    ArrayLeaf {
        length,
        ..ArrayLeaf::default()
    }
}

#[cfg(test)]
mod tests {
    use referencing::Draft;
    use serde_json::json;

    use crate::canonical::{
        ir::Schema,
        tests_util::{canonicalize, canonicalize_with},
    };

    // Detects only the `Not(Array{uniqueItems})` residual the dual replaces.
    fn has_unique_not_residual(schema: &Schema) -> bool {
        if let Schema::Not(inner) = schema {
            if matches!(inner.as_schema(), Schema::Array(l) if l.unique_items) {
                return true;
            }
        }
        schema
            .children()
            .iter()
            .any(|c| has_unique_not_residual(c.as_schema()))
    }

    #[test]
    fn negate_array_unique_has_no_not_residual() {
        let negated = canonicalize(&json!({"type": "array", "uniqueItems": true})).negate();
        assert!(!has_unique_not_residual(negated.as_schema()));
    }

    // Draft 4 doesn't know `contains`, so `.negate()` runs in the `NegateWithoutContains` walk stage.
    #[test]
    fn negate_under_draft4_without_contains_stage() {
        let schema = json!({"type": "array", "items": {"type": "integer"}});
        let original = canonicalize_with(&schema, Draft::Draft4);
        let negated = original.negate();
        // Exact complement: a schema and its negation share no instance, and the negation is non-trivial.
        assert!(!original.intersect(&negated).is_satisfiable());
        assert!(negated.is_satisfiable());
        // ...and the negation is canonical (stable under re-canonicalization).
        assert_eq!(
            negated.as_schema(),
            canonicalize(&negated.to_json_schema()).as_schema()
        );
    }
}

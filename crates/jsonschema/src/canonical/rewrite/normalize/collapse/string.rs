//! String-domain collapse passes.

use std::sync::Arc;

use crate::canonical::{
    context::CanonicalizationContext,
    coverage::any_sibling_covers,
    intern::shared,
    intersect::{intersect_canonical, normalize_string_not_patterns},
    ir::{Schema, SharedSchema, StringLeaf},
};

fn string_leaf_is_format_only(leaf: &StringLeaf, ctx: &CanonicalizationContext) -> bool {
    leaf.min_length.is_none()
        && leaf.max_length.is_none()
        && leaf.patterns.is_empty()
        && leaf.not_patterns.is_empty()
        && leaf.content.is_empty()
        && !leaf.extended_regex(ctx)
}

/// Two distinct non-disjoint `format`s can't merge into one leaf, so they stay separate `allOf`
/// conjuncts. Park every non-format facet on the smallest-format leaf so both fold orders converge.
///
/// ```text
/// BEFORE: {"allOf": [{"type": "string", "format": "email", "pattern": "^a"}, {"type": "string", "format": "date"}]}
/// AFTER:  {"allOf": [{"type": "string", "format": "date", "pattern": "^a"}, {"type": "string", "format": "email"}]}
/// ```
pub(super) fn canonicalize_string_format_conjunction(
    branches: &mut Vec<SharedSchema>,
    ctx: &CanonicalizationContext,
) -> bool {
    let string_indices: Vec<usize> = branches
        .iter()
        .enumerate()
        .filter(|(_, branch)| matches!(branch.as_schema(), Schema::String(_)))
        .map(|(index, _)| index)
        .collect();
    if string_indices.len() < 2 {
        return false;
    }
    let mut formats: Vec<Arc<str>> = string_indices
        .iter()
        .filter_map(|&index| match branches[index].as_schema() {
            Schema::String(leaf) => leaf.format.clone(),
            _ => None,
        })
        .collect();
    formats.sort_unstable();
    formats.dedup();
    if formats.len() < 2 {
        return false;
    }
    let min_format = Arc::clone(&formats[0]);
    // Already canonical when only the smallest-format leaf carries facets.
    let misplaced = string_indices
        .iter()
        .any(|&index| match branches[index].as_schema() {
            Schema::String(leaf) => {
                leaf.format.as_ref() != Some(&min_format) && !string_leaf_is_format_only(leaf, ctx)
            }
            _ => false,
        });
    if !misplaced {
        return false;
    }
    let mut merged = shared(Schema::String(StringLeaf::default()));
    for &index in &string_indices {
        let Schema::String(leaf) = branches[index].as_schema() else {
            continue;
        };
        let mut stripped = leaf.clone();
        stripped.format = None;
        merged = intersect_canonical(&merged, &shared(Schema::String(stripped)), ctx);
    }
    let replacement = match merged.as_schema() {
        Schema::String(leaf) => {
            let mut leaf = leaf.clone();
            leaf.format = Some(Arc::clone(&min_format));
            let mut replacement = vec![shared(Schema::String(leaf))];
            for format in formats.iter().skip(1) {
                replacement.push(shared(Schema::String(StringLeaf {
                    format: Some(Arc::clone(format)),
                    ..StringLeaf::default()
                })));
            }
            replacement
        }
        Schema::False => vec![shared(Schema::False)],
        // The format-less merge of string leaves stays a string leaf or `False`.
        _ => return false,
    };
    for &index in string_indices.iter().rev() {
        branches.remove(index);
    }
    branches.extend(replacement);
    true
}

/// A string branch sitting next to `not: {pattern: r}` folds each excluded pattern `r` into the positive branch's
/// `not_patterns`. A contradiction (the same pattern required and forbidden) collapses to `false`.
///
/// ```text
/// BEFORE: {"allOf": [{"type": "string", "minLength": 1}, {"not": {"type": "string", "pattern": "^a"}}]}
/// AFTER:  {"type": "string", "minLength": 1, "not": {"type": "string", "pattern": "^a"}}
/// ```
pub(super) fn absorb_not_pattern_siblings(branches: &mut Vec<SharedSchema>) -> bool {
    // Some(patterns) if `schema` is a negation of string leaves with only one pattern set each.
    fn not_only_string_patterns(schema: &Schema) -> Option<Vec<Arc<str>>> {
        let Schema::Not(inner) = schema else {
            return None;
        };
        match inner.as_schema() {
            Schema::String(leaf) => Some(vec![single_string_pattern(leaf)?]),
            Schema::AnyOf(branches) => {
                let mut patterns = Vec::with_capacity(branches.len());
                for branch in branches {
                    let Schema::String(leaf) = branch.as_schema() else {
                        return None;
                    };
                    patterns.push(single_string_pattern(leaf)?);
                }
                (!patterns.is_empty()).then_some(patterns)
            }
            _ => None,
        }
    }

    fn single_string_pattern(leaf: &StringLeaf) -> Option<Arc<str>> {
        let [pattern] = leaf.patterns.as_slice() else {
            return None;
        };
        if leaf.min_length.is_none()
            && leaf.max_length.is_none()
            && leaf.not_patterns.is_empty()
            && leaf.format.is_none()
            && leaf.content.is_empty()
        {
            Some(Arc::clone(pattern))
        } else {
            None
        }
    }

    if !branches
        .iter()
        .any(|b| matches!(b.as_schema(), Schema::String(_)))
    {
        return false;
    }
    let mut excluded: Vec<Arc<str>> = Vec::new();
    branches.retain(|b| match not_only_string_patterns(b.as_schema()) {
        Some(patterns) => {
            excluded.extend(patterns);
            false
        }
        None => true,
    });
    if excluded.is_empty() {
        return false;
    }
    let (index, leaf) = branches
        .iter()
        .enumerate()
        .find_map(|(index, branch)| match branch.as_schema() {
            Schema::String(leaf) => Some((index, leaf)),
            _ => None,
        })
        .expect("positive string leaf still present");
    let mut merged = leaf.clone();
    merged.not_patterns.extend(excluded);
    if let Some(leaf) = normalize_string_not_patterns(merged) {
        branches[index] = shared(Schema::String(leaf));
    } else {
        *branches = vec![shared(Schema::False)];
    }
    true
}

/// Drop a pattern facet when a sibling already admits the strings the drop newly lets in (union
/// unchanged). `relax_required` picks which facet to relax; the probe flips it onto the opposite facet.
///
/// ```text
/// BEFORE: {"anyOf": [{"not": {"pattern": "^a"}}, {"type": "string", "pattern": "^a", "allOf": [{"pattern": "b$"}]}]}
/// AFTER:  {"anyOf": [{"not": {"pattern": "^a"}}, {"type": "string", "pattern": "b$"}]}   // `^a`-violators are covered
/// ```
pub(super) fn drop_string_pattern_facet_covered_by_sibling(
    branches: &mut [SharedSchema],
    ctx: &CanonicalizationContext,
) -> bool {
    let mut changed = false;
    for relax_required in [true, false] {
        changed |= drop_pattern_facet(branches, ctx, relax_required);
    }
    changed
}

fn drop_pattern_facet(
    branches: &mut [SharedSchema],
    ctx: &CanonicalizationContext,
    relax_required: bool,
) -> bool {
    for index in 0..branches.len() {
        let Schema::String(leaf) = branches[index].as_schema() else {
            continue;
        };
        let facet = if relax_required {
            &leaf.patterns
        } else {
            &leaf.not_patterns
        };
        if facet.is_empty() {
            continue;
        }
        let leaf = leaf.clone();
        let facet_len = facet.len();
        for facet_index in 0..facet_len {
            let mut widened = leaf.clone();
            let dropped = if relax_required {
                widened.patterns.remove(facet_index)
            } else {
                widened.not_patterns.remove(facet_index)
            };
            // Strings the drop newly admits: match the retained facets, then flip the dropped entry to the other facet.
            let mut newly_admitted = widened.clone();
            if relax_required {
                newly_admitted.not_patterns.push(dropped);
            } else {
                newly_admitted.patterns.push(dropped);
            }
            let Some(newly_admitted) = normalize_string_not_patterns(newly_admitted) else {
                continue;
            };
            let newly_admitted = shared(Schema::String(newly_admitted));
            if any_sibling_covers(branches, &[index], &newly_admitted, ctx) {
                if let Some(widened) = normalize_string_not_patterns(widened) {
                    branches[index] = shared(Schema::String(widened));
                    return true;
                }
            }
        }
    }
    false
}

//! Candidates for a string leaf.
use serde_json::Value;

use super::{demanded_length, CANDIDATE_LENGTH};
use crate::canonical::ir::{BoundCardinality, LengthBounds, StringFormat, StringLeaf};

/// Strings to try against a string leaf: the per-pattern strings joined, which alone can match a
/// start-anchored and an end-anchored pattern, then one per pattern and one per format.
pub(super) fn string(leaf: &StringLeaf) -> Vec<Value> {
    let mut proposals: Vec<String> = Vec::new();
    let per_pattern: Vec<String> = leaf
        .patterns
        .iter()
        .filter_map(|pattern| jsonschema_regex::pattern_witness(pattern))
        .collect();
    // Skip the concatenation when a pattern produced no string of its own: it would not be
    // checked against that pattern.
    if per_pattern.len() > 1 && per_pattern.len() == leaf.patterns.len() {
        proposals.push(per_pattern.concat());
    }
    proposals.extend(per_pattern);
    proposals.extend(
        leaf.formats
            .iter()
            .filter_map(StringFormat::example)
            .map(ToOwned::to_owned),
    );
    // The shortest string the window allows, for a leaf constrained only by length.
    if let Some(length) = demanded_length(leaf.lengths.minimum.as_ref()) {
        proposals.push("a".repeat(length));
    }
    proposals
        .into_iter()
        .filter_map(|text| fit_to_window(text, &leaf.lengths))
        .map(Value::String)
        .collect()
}

/// A string moved into the length window: padded when too short, dropped when too long or when the
/// window asks for more characters than a candidate is built with.
fn fit_to_window(mut text: String, window: &LengthBounds) -> Option<String> {
    let length = text.chars().count();
    if window
        .maximum
        .as_ref()
        .and_then(BoundCardinality::to_usize)
        .is_some_and(|maximum| length > maximum)
    {
        return None;
    }
    let minimum = window
        .minimum
        .as_ref()
        .and_then(BoundCardinality::to_usize)
        .unwrap_or(0);
    if minimum > length {
        if minimum as u64 > CANDIDATE_LENGTH {
            return None;
        }
        text.push_str(&"a".repeat(minimum - length));
    }
    Some(text)
}

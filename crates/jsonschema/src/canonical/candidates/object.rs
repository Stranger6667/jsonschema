//! Candidates for an object leaf.
use serde_json::Value;

use super::{demanded_length, Walk};
use crate::canonical::{
    algebra,
    context::CanonicalizationContext,
    ir::{ObjectLeaf, UncheckableFacet, Verdict},
};

/// The key a filler entry is given, where an object needs more of them than it requires.
const FILLER_KEY: &str = "a";

/// Names tried per seed before the next one. A leaf wanting more is answered by later seeds, or
/// left undecided.
const CANDIDATE_KEYS_PER_SEED: usize = 8;

/// Instances worth trying against an object leaf: one object holding the keys it demands.
pub(super) fn object<'a>(leaf: &'a ObjectLeaf, walk: &Walk<'a, '_>) -> Vec<Value> {
    let Some(size) = demanded_length(leaf.sizes.minimum.as_ref()) else {
        return Vec::new();
    };
    let ctx = walk.ctx();
    let mut object = serde_json::Map::new();
    for key in candidate_keys(leaf, size, ctx) {
        // A key answers to the entry declaring it, then a pattern entry it matches, then
        // `additionalProperties`. Where several apply to one key, the check below turns
        // down a value that satisfies only the one taken here.
        let governing = leaf
            .properties
            .get(key.as_str())
            .or_else(|| {
                leaf.pattern_properties
                    .iter()
                    .find(|(pattern, _)| algebra::matches_key(pattern, &key, ctx))
                    .map(|(_, schema)| schema)
            })
            .or(leaf.additional.as_ref());
        let Some(value) = governing.map_or(Some(Value::Null), |schema| walk.first(schema)) else {
            return Vec::new();
        };
        object.insert(key, value);
    }
    vec![Value::Object(object)]
}

/// Keys to build a candidate object out of: the ones the leaf demands, then names its constraints
/// admit, up to the size floor.
///
/// A key constraint can turn down every name of the plain filler sequence, so each pattern seeds
/// names of its own. A name already demanded is never offered twice.
fn candidate_keys(leaf: &ObjectLeaf, size: usize, ctx: &CanonicalizationContext) -> Vec<String> {
    let mut keys: Vec<String> = leaf.required.iter().map(ToString::to_string).collect();
    let wanted = size.max(keys.len());
    if keys.len() >= wanted {
        return keys;
    }
    let admitted = |key: &str| {
        leaf.property_names.as_ref().is_none_or(|names| {
            algebra::admits_value(
                names,
                &Value::String(key.to_string()),
                UncheckableFacet::Undecided,
                ctx,
            ) == Verdict::Admits
        })
    };
    let seeds = leaf
        .properties
        .keys()
        .map(ToString::to_string)
        .chain(
            leaf.pattern_properties
                .keys()
                .map(|pattern| literal_prefix(pattern)),
        )
        .chain(std::iter::once(FILLER_KEY.to_string()));
    for seed in seeds {
        for index in 0..CANDIDATE_KEYS_PER_SEED {
            if keys.len() >= wanted {
                return keys;
            }
            let key = format!("{seed}{index}");
            if !keys.contains(&key) && admitted(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

/// The literal head of a key pattern, which every key an anchored one matches carries. Empty where
/// the pattern holds nothing a name can be built from.
fn literal_prefix(pattern: &str) -> String {
    pattern
        .strip_prefix('^')
        .unwrap_or(pattern)
        .chars()
        .take_while(|character| character.is_alphanumeric() || *character == '_')
        .collect()
}

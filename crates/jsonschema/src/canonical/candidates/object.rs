//! Candidates for an object leaf.
use std::ops::ControlFlow;

use serde_json::{Map, Value};

use super::{demanded_length, holds, keep_accepted, shortest_instance, Visit, Walk};
use crate::{
    canonical::{
        algebra,
        context::CanonicalizationContext,
        ir::{BoundCardinality, ObjectLeaf, ObjectViolation, Schema, UncheckableFacet, Verdict},
        negate, DefinitionMap,
    },
    JsonTypeSet,
};

/// The seed filler keys are built from, where an object needs more keys than it requires.
const FILLER_KEY: &str = "a";

/// Names tried per seed before the next one. A leaf wanting more is answered by later seeds, or
/// left undecided.
const CANDIDATE_KEYS_PER_SEED: usize = 8;

/// Names tried per source for the entry a violation asks for. A leaf wanting more is left
/// undecided.
const VIOLATION_KEYS: usize = 8;

/// Candidates of a key's schema tried for a value its rule turns down: every one the walk builds,
/// which the walk's node count already bounds.
const FAILING_VALUE_ATTEMPTS: usize = usize::MAX;

/// Instances worth trying against an object leaf: one object holding the keys it demands, then,
/// where the leaf demands rules be broken, one that also holds an entry breaking each.
pub(super) fn object<'a>(
    leaf: &'a ObjectLeaf,
    walk: &Walk<'a, '_>,
    visit: &mut Visit<'_>,
) -> ControlFlow<()> {
    let Some(size) = demanded_length(leaf.sizes.minimum.as_ref()) else {
        return ControlFlow::Continue(());
    };
    let ctx = walk.ctx();
    let keys = candidate_keys(leaf, size, ctx);
    // Each violating entry is a key of its own, so it counts toward `minProperties`: the violating
    // object holds fewer demanded keys, which head the same sequence.
    let head: Vec<String> = if leaf.violations.is_empty() {
        Vec::new()
    } else {
        let count = size
            .saturating_sub(leaf.violations.len())
            .max(leaf.required.len())
            .min(keys.len());
        keys[..count].to_vec()
    };
    let mut demanded = Map::new();
    for key in keys {
        // Where several entries apply to one key, the caller's check turns down a value that
        // satisfies only the one taken here.
        let Some(value) =
            governing(leaf, &key, ctx).map_or(Some(Value::Null), |schema| walk.first(schema))
        else {
            return ControlFlow::Continue(());
        };
        demanded.insert(key, value);
    }
    if leaf.violations.is_empty() {
        return visit(Value::Object(demanded));
    }
    let mut object: Map<String, Value> = head
        .into_iter()
        .map(|key| {
            let value = demanded[key.as_str()].clone();
            (key, value)
        })
        .collect();
    visit(Value::Object(demanded))?;
    // The demanded object may break every rule through a value it already holds; the violating
    // object adds a key per rule, so it is built only where `maxProperties` leaves room for them.
    let count = BoundCardinality::from((object.len() + leaf.violations.len()) as u64);
    if leaf
        .sizes
        .maximum
        .as_ref()
        .is_some_and(|maximum| *maximum < count)
    {
        return ControlFlow::Continue(());
    }
    for violation in &leaf.violations {
        let Some((key, value)) = violating_entry(leaf, violation, &object, walk) else {
            return ControlFlow::Continue(());
        };
        debug_assert!(
            !object.contains_key(&key),
            "a violating entry takes a key of its own"
        );
        object.insert(key, value);
    }
    visit(Value::Object(object))
}

/// The schema a key's value answers to: the entry declaring it, then a pattern entry it matches,
/// then `additionalProperties`.
fn governing<'a>(
    leaf: &'a ObjectLeaf,
    key: &str,
    ctx: &CanonicalizationContext,
) -> Option<&'a Schema> {
    leaf.properties
        .get(key)
        .or_else(|| {
            leaf.pattern_properties
                .iter()
                .find(|(pattern, _)| holds(ctx, || algebra::matches_key(pattern, key, ctx)))
                .map(|(_, schema)| schema)
        })
        .or(leaf.additional.as_ref())
}

/// Whether the leaf's `propertyNames` accepts `key`.
fn accepts_name(leaf: &ObjectLeaf, key: &str, ctx: &CanonicalizationContext) -> bool {
    leaf.property_names.as_ref().is_none_or(|names| {
        holds(ctx, || {
            algebra::accepts_value(
                names,
                &Value::String(key.to_string()),
                UncheckableFacet::Undecided,
                ctx,
            ) == Verdict::Accepts
        })
    })
}

/// Keys to build a candidate object out of: the ones the leaf demands, then names its constraints
/// accept, up to `minProperties`.
///
/// `propertyNames` can turn down every name of the plain filler sequence, so each pattern seeds
/// names of its own. A name already demanded is never offered twice.
fn candidate_keys(leaf: &ObjectLeaf, size: usize, ctx: &CanonicalizationContext) -> Vec<String> {
    let mut keys: Vec<String> = leaf.required.iter().map(ToString::to_string).collect();
    let wanted = size.max(keys.len());
    if keys.len() >= wanted {
        return keys;
    }
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
            if !keys.contains(&key) && accepts_name(leaf, &key, ctx) {
                keys.push(key);
            }
        }
    }
    keys
}

/// An entry outside `taken` that breaks the rule `violation` names, and that the rest of the leaf
/// accepts.
fn violating_entry<'a>(
    leaf: &'a ObjectLeaf,
    violation: &'a ObjectViolation,
    taken: &Map<String, Value>,
    walk: &Walk<'a, '_>,
) -> Option<(String, Value)> {
    let ctx = walk.ctx();
    // Whether an entry under `key` is one the rule applies to.
    let reaches = |key: &str| {
        holds(ctx, || match violation {
            ObjectViolation::NameFails(names) => {
                algebra::rejects_value(names, &Value::String(key.to_string()), ctx)
            }
            ObjectViolation::UndeclaredValueFails {
                names, patterns, ..
            } => {
                !names.iter().any(|name| name.as_ref() == key)
                    && !patterns
                        .iter()
                        .any(|pattern| algebra::matches_key(pattern, key, ctx))
            }
            ObjectViolation::PatternValueFails { pattern, .. } => {
                algebra::matches_key(pattern, key, ctx)
            }
        })
    };
    // A string the violated pattern matches and names built on its literal head, then the filler
    // sequence, then names `propertyNames` accepts - built only once every name before them is
    // turned down. A finite `propertyNames` accepts its members alone, so no name is built for it.
    let builds_names = leaf
        .property_names
        .as_ref()
        .is_none_or(|names| names.kind().finite_values().is_none());
    let pattern = match violation {
        ObjectViolation::PatternValueFails { pattern, .. } => Some(pattern),
        ObjectViolation::NameFails(_) | ObjectViolation::UndeclaredValueFails { .. } => None,
    }
    .filter(|_| builds_names);
    let witness = pattern.and_then(|pattern| jsonschema_regex::pattern_witness(pattern));
    let seeded = pattern.into_iter().flat_map(|pattern| {
        let prefix = literal_prefix(pattern);
        (0..VIOLATION_KEYS).map(move |index| format!("{prefix}{index}"))
    });
    let fillers =
        (1..=if builds_names { VIOLATION_KEYS } else { 0 }).map(|length| FILLER_KEY.repeat(length));
    let named = leaf.property_names.iter().flat_map(|names| {
        let mut found = Vec::new();
        let _ = walk.each(names, &mut |name| {
            if let Value::String(name) = name {
                found.push(name);
            }
            if found.len() == VIOLATION_KEYS {
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        });
        found
    });
    let key = witness
        .into_iter()
        .chain(seeded)
        .chain(fillers)
        .chain(named)
        .find(|key| !taken.contains_key(key) && reaches(key) && accepts_name(leaf, key, ctx))?;
    let governing = governing(leaf, &key, ctx);
    let value = match violation {
        // The name breaks the rule, so the value only has to be one the leaf takes.
        ObjectViolation::NameFails(_) => {
            governing.map_or(Some(Value::Null), |schema| walk.first(schema))?
        }
        ObjectViolation::UndeclaredValueFails { additional, .. } => {
            failing_value(governing, additional, walk)?
        }
        ObjectViolation::PatternValueFails { schema, .. } => {
            failing_value(governing, schema, walk)?
        }
    };
    Some((key, value))
}

/// A value `governing` takes and `rule` turns down: a candidate of the one that the other rejects,
/// else a candidate of their difference where that difference is exact. An absent `governing`
/// takes every value.
fn failing_value<'a>(
    governing: Option<&'a Schema>,
    rule: &Schema,
    walk: &Walk<'a, '_>,
) -> Option<Value> {
    let ctx = walk.ctx();
    let fails = |value: &Value| holds(ctx, || algebra::rejects_value(rule, value, ctx));
    let found = match governing {
        Some(schema) => {
            let mut found = None;
            let _ = walk.each(
                schema,
                &mut keep_accepted(&mut found, FAILING_VALUE_ATTEMPTS, fails),
            );
            found
        }
        None => JsonTypeSet::all()
            .iter()
            .map(shortest_instance)
            .find(|value| fails(value)),
    };
    if found.is_some() {
        return found;
    }
    // Negated against no targets: a rule reading one keeps it under `not`, which builds no
    // candidate of its own.
    let (difference, inexact) = ctx.probe(|| {
        let negation = negate::negate_in_place(rule, &DefinitionMap::new(), ctx)?;
        Some(match governing {
            Some(schema) => algebra::intersect(schema.clone(), negation, ctx),
            None => negation,
        })
    });
    if inexact {
        return None;
    }
    walk.first_owned(&difference?)
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

#[cfg(test)]
mod tests {
    use std::{cell::Cell, sync::Arc};

    use referencing::Draft;
    use serde_json::{json, Value};
    use test_case::test_case;

    use super::*;
    use crate::{
        canonical::ir::{CanonicalJson, LengthBounds, SchemaKind},
        options::PatternEngineOptions,
    };

    #[test_case(1, &[json!({"a": null})] ; "no room at the ceiling")]
    #[test_case(2, &[json!({"a": null}), json!({"a": null, "aa": null})] ; "room below it")]
    fn violating_entry_under_size_ceiling(maximum: u64, expected: &[Value]) {
        let ctx = CanonicalizationContext::new(
            Draft::Draft202012,
            PatternEngineOptions::default(),
            false,
        );
        let leaf = ObjectLeaf {
            sizes: LengthBounds {
                minimum: None,
                maximum: Some(BoundCardinality::from(maximum)),
            },
            required: vec![Arc::from("a")],
            violations: vec![ObjectViolation::NameFails(Schema::new(SchemaKind::Const(
                CanonicalJson::from_value(&json!("a")),
            )))],
            ..ObjectLeaf::default()
        };
        let budget = Cell::new(super::super::NODES);
        let walk = Walk {
            target: &|_| None,
            depth: super::super::DEPTH,
            budget: &budget,
            ctx: &ctx,
        };
        let mut found = Vec::new();
        let _ = object(&leaf, &walk, &mut |value| {
            found.push(value);
            ControlFlow::Continue(())
        });
        assert_eq!(found, expected);
    }
}

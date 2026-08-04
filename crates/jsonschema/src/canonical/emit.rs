//! IR -> JSON Schema emit.

use referencing::Draft;
use serde_json::{json, Map, Value};

use crate::{
    canonical::{
        ir::{
            ArrayLeaf, BoundCardinality, BoundRational, CanonicalJson, ContainsFacet, Divisors,
            ExcludedDivisors, IntegerLeaf, NumberLeaf, ObjectLeaf, Schema, SchemaKind, StringLeaf,
        },
        DefinitionMap, CANONICAL_REFERENCE_PREFIX,
    },
    JsonTypeSet,
};

const CANONICAL_REFERENCE_SEGMENT: &percent_encoding::AsciiSet =
    &percent_encoding::CONTROLS.add(b'%');

/// The one-key object `{key: value}`. `json!` would re-copy an already-built `Value` through
/// `to_value` at every level of the recursion.
fn keyed(key: &str, value: Value) -> Value {
    let mut map = Map::new();
    map.insert(key.to_owned(), value);
    Value::Object(map)
}

pub(crate) fn to_json_schema(root: &Schema, draft: Draft, definitions: &DefinitionMap) -> Value {
    let value = emit(root.kind(), draft);
    if matches!(root.kind(), SchemaKind::Raw(_)) {
        return value;
    }
    let value = attach_definitions(value, definitions, draft);
    match schema_uri(draft) {
        Some(uri) => with_schema_uri(value, uri),
        None => value,
    }
}

fn emit(kind: &SchemaKind, draft: Draft) -> Value {
    match kind {
        SchemaKind::True if matches!(draft, Draft::Draft4) => Value::Object(Map::new()),
        SchemaKind::True => Value::Bool(true),
        SchemaKind::False if matches!(draft, Draft::Draft4) => json!({"not": {}}),
        SchemaKind::False => Value::Bool(false),
        // `{"const": null}` is identical to `{"type": "null"}` - prefer the type form.
        SchemaKind::Const(value) if value.as_value().is_null() => json!({"type": "null"}),
        SchemaKind::Const(value) if matches!(draft, Draft::Draft4) => {
            keyed("enum", Value::Array(vec![value.to_value()]))
        }
        SchemaKind::Const(value) => keyed("const", value.to_value()),
        SchemaKind::Enum(values) => emit_enum(values.as_slice()),
        SchemaKind::String(leaf) => emit_string(leaf.get()),
        SchemaKind::Integer(leaf) => emit_integer(leaf.get()),
        SchemaKind::Number(leaf) => emit_number(leaf.get(), draft),
        SchemaKind::Array(leaf) => emit_array(leaf.get(), draft),
        SchemaKind::Object(leaf) => emit_object(leaf.get(), draft),
        SchemaKind::MultiType(set) => emit_multi_type(*set),
        // The body emits a `const`/`enum` object without a `type` key, so adding `type` beside it
        // expresses "both must hold" and re-parses to the same IR.
        SchemaKind::TypedGroup { ty, body } => {
            let mut map = match emit(body.kind(), draft) {
                Value::Object(map) => map,
                other @ (Value::Null
                | Value::Bool(_)
                | Value::Number(_)
                | Value::String(_)
                | Value::Array(_)) => unreachable!("value-set body emits an object: {other:?}"),
            };
            map.insert("type".into(), Value::String(ty.to_string()));
            Value::Object(map)
        }
        SchemaKind::Not(schema) => keyed("not", emit(schema.kind(), draft)),
        SchemaKind::AllOf(branches) => keyed("allOf", emit_branches(branches.as_slice(), draft)),
        SchemaKind::AnyOf(branches) => keyed("anyOf", emit_branches(branches.as_slice(), draft)),
        SchemaKind::OneOf(branches) => keyed("oneOf", emit_branches(branches, draft)),
        SchemaKind::Reference(uri) => emit_reference(uri, draft),
        SchemaKind::Raw(value) => value.get().clone(),
    }
}

fn emit_branches(branches: &[Schema], draft: Draft) -> Value {
    Value::Array(
        branches
            .iter()
            .map(|branch| emit(branch.kind(), draft))
            .collect(),
    )
}

fn emit_reference(uri: &str, draft: Draft) -> Value {
    if !uri.starts_with(CANONICAL_REFERENCE_PREFIX) {
        return keyed("$ref", Value::String(uri.to_owned()));
    }
    let mut reference = format!("#/{}/", definition_keyword(draft));
    let mut segment = String::with_capacity(uri.len());
    referencing::write_escaped_str(&mut segment, uri);
    reference.extend(percent_encoding::utf8_percent_encode(
        &segment,
        CANONICAL_REFERENCE_SEGMENT,
    ));
    keyed("$ref", Value::String(reference))
}

fn attach_definitions(mut value: Value, definitions: &DefinitionMap, draft: Draft) -> Value {
    if definitions.is_empty() {
        return value;
    }
    let Value::Object(root) = &mut value else {
        return value;
    };
    let generated_keyword = definition_keyword(draft);
    for (keyword, prefix) in [("$defs", "#/$defs/"), ("definitions", "#/definitions/")] {
        let mut entries: Map<_, _> = definitions
            .iter()
            .filter_map(|(uri, schema)| {
                definition_name(uri, prefix).map(|name| (name, emit(schema.kind(), draft)))
            })
            .collect();
        if keyword == generated_keyword {
            for (uri, schema) in definitions {
                if uri.starts_with(CANONICAL_REFERENCE_PREFIX) {
                    entries.insert(uri.to_string(), emit(schema.kind(), draft));
                }
            }
        }
        if !entries.is_empty() {
            root.insert(keyword.into(), Value::Object(entries));
        }
    }
    value
}

fn definition_name(uri: &str, prefix: &str) -> Option<String> {
    let encoded = uri.strip_prefix(prefix)?;
    let decoded = percent_encoding::percent_decode_str(encoded)
        .decode_utf8()
        .ok()?;
    Some(referencing::unescape_segment(&decoded).into_owned())
}

const fn definition_keyword(draft: Draft) -> &'static str {
    if matches!(draft, Draft::Draft4 | Draft::Draft6 | Draft::Draft7) {
        "definitions"
    } else {
        "$defs"
    }
}

/// Emit a string leaf as `{"type":"string"}` plus its length bounds and facets. A single pattern or
/// format is inline; the rest become `allOf` conjuncts, since one leaf holds only one `pattern`, one
/// `format`, and one `not`.
fn emit_string(leaf: &StringLeaf) -> Value {
    debug_assert!(
        leaf.excluded_formats
            .iter()
            .all(|format| !leaf.formats.contains(format)),
        "a format both demanded and barred leaves no string, which is `False`"
    );
    debug_assert!(
        leaf.excluded_patterns
            .iter()
            .all(|pattern| !leaf.patterns.contains(pattern)),
        "a pattern both demanded and barred leaves no string, which is `False`"
    );
    let mut map = Map::new();
    map.insert("type".into(), Value::String("string".into()));
    if let Some(min) = &leaf.lengths.minimum {
        map.insert("minLength".into(), Value::Number(min.to_number()));
    }
    if let Some(max) = &leaf.lengths.maximum {
        map.insert("maxLength".into(), Value::Number(max.to_number()));
    }
    let mut conjuncts: Vec<Value> = Vec::new();
    match leaf.patterns.as_slice() {
        [] => {}
        [pattern] => {
            map.insert("pattern".into(), Value::String(pattern.to_string()));
        }
        patterns => conjuncts.extend(
            patterns
                .iter()
                .map(|pattern| keyed("pattern", Value::String(pattern.to_string()))),
        ),
    }
    match leaf.formats.as_slice() {
        [] => {}
        [format] => {
            map.insert("format".into(), Value::String(format.to_string()));
        }
        formats => conjuncts.extend(
            formats
                .iter()
                .map(|format| keyed("format", Value::String(format.to_string()))),
        ),
    }
    // Every barred facet goes into its own `allOf` branch: the main object already spells what a
    // string must satisfy, and one `not` slot cannot hold several of them.
    conjuncts.extend(leaf.excluded_formats.iter().map(|format| {
        let mut inner = Map::new();
        inner.insert("format".into(), Value::String(format.to_string()));
        keyed("not", Value::Object(inner))
    }));
    conjuncts.extend(leaf.excluded_patterns.iter().map(|pattern| {
        let mut inner = Map::new();
        inner.insert("pattern".into(), Value::String(pattern.to_string()));
        keyed("not", Value::Object(inner))
    }));
    // A media type and an encoding sharing one schema object decode-then-check under the runtime
    // dispatch (`compile_media_type` reads its sibling `contentEncoding`), which would silently
    // recompose two facets this leaf keeps independent - so whenever both are present, every value
    // of either goes into its own `allOf` branch instead of beside `type` on the main object.
    let both_content_facets_present =
        !leaf.content_media_types.is_empty() && !leaf.content_encodings.is_empty();
    match leaf.content_media_types.as_slice() {
        [] => {}
        [media_type] if !both_content_facets_present => {
            map.insert(
                "contentMediaType".into(),
                Value::String(media_type.to_string()),
            );
        }
        media_types => conjuncts.extend(
            media_types
                .iter()
                .map(|media_type| keyed("contentMediaType", Value::String(media_type.to_string()))),
        ),
    }
    match leaf.content_encodings.as_slice() {
        [] => {}
        [encoding] if !both_content_facets_present => {
            map.insert(
                "contentEncoding".into(),
                Value::String(encoding.to_string()),
            );
        }
        encodings => conjuncts.extend(
            encodings
                .iter()
                .map(|encoding| keyed("contentEncoding", Value::String(encoding.to_string()))),
        ),
    }
    // `enum` in every draft, so one spelling covers Draft 4 as well.
    if !leaf.excluded.is_empty() {
        let members = Value::Array(
            leaf.excluded
                .iter()
                .map(|value| Value::String(value.to_string()))
                .collect(),
        );
        let mut inner = Map::new();
        inner.insert("enum".into(), members);
        map.insert("not".into(), Value::Object(inner));
    }
    if !conjuncts.is_empty() {
        map.insert("allOf".into(), Value::Array(conjuncts));
    }
    Value::Object(map)
}

/// Emit a number leaf as `{"type":"number"}` plus its interval bounds, using the exclusive spelling
/// for an endpoint the interval does not admit.
fn emit_number(leaf: &NumberLeaf, draft: Draft) -> Value {
    debug_assert!(
        !leaf.excludes_integers || matches!(draft, Draft::Draft4),
        "the integer exclusion survives normalization only under Draft 4"
    );
    let mut map = Map::new();
    map.insert("type".into(), Value::String("number".into()));
    // Draft 4 spells exclusivity as a boolean flag beside the bound; later drafts give it its own
    // numeric keyword.
    let draft4 = matches!(draft, Draft::Draft4);
    for (bound, inclusive_key, exclusive_key) in [
        (leaf.minimum.as_ref(), "minimum", "exclusiveMinimum"),
        (leaf.maximum.as_ref(), "maximum", "exclusiveMaximum"),
    ] {
        let Some(bound) = bound else {
            continue;
        };
        let limit = Value::Number(bound.to_number());
        if bound.is_inclusive() {
            map.insert(inclusive_key.into(), limit);
        } else if draft4 {
            map.insert(inclusive_key.into(), limit);
            map.insert(exclusive_key.into(), Value::Bool(true));
        } else {
            map.insert(exclusive_key.into(), limit);
        }
    }
    emit_divisors(
        &mut map,
        &leaf.multiple_of,
        &leaf.not_multiple_of,
        leaf.excludes_integers,
    );
    Value::Object(map)
}

/// Emit an array leaf as `{"type":"array"}` plus its length bounds, uniqueness and element schemas.
/// A tuple prefix is spelled `prefixItems` with an `items` tail in 2020-12, and array-form `items`
/// with an `additionalItems` tail in 2019-09 and earlier.
fn emit_array(leaf: &ArrayLeaf, draft: Draft) -> Value {
    let mut map = Map::new();
    map.insert("type".into(), Value::String("array".into()));
    let tuple_draft = matches!(draft, Draft::Draft202012 | Draft::Unknown);
    if !leaf.prefix.is_empty() {
        let prefix: Vec<Value> = leaf
            .prefix
            .iter()
            .map(|schema| emit(schema.kind(), draft))
            .collect();
        let key = if tuple_draft { "prefixItems" } else { "items" };
        map.insert(key.into(), Value::Array(prefix));
    }
    if let Some(items) = &leaf.items {
        let key = if leaf.prefix.is_empty() || tuple_draft {
            "items"
        } else {
            "additionalItems"
        };
        map.insert(key.into(), emit(items.kind(), draft));
    }
    // One `contains` demand sits inline; the keyword is single-valued, so the rest conjoin as
    // single-demand `allOf` branches.
    debug_assert!(
        leaf.contains
            .windows(2)
            .all(|pair| pair[0].schema < pair[1].schema),
        "contains demands are sorted and schema-deduplicated"
    );
    debug_assert!(
        !leaf
            .contains
            .iter()
            .any(|facet| facet.minimum.as_ref() == Some(&BoundCardinality::from(1))),
        "a default contains minimum is spelled as absent"
    );
    let mut facets = leaf.contains.iter();
    if let Some(facet) = facets.next() {
        insert_contains(&mut map, facet, draft);
        let rest: Vec<Value> = facets
            .map(|facet| {
                let mut entry = Map::new();
                insert_contains(&mut entry, facet, draft);
                Value::Object(entry)
            })
            .collect();
        if !rest.is_empty() {
            map.insert("allOf".into(), Value::Array(rest));
        }
    }
    if leaf.unique {
        map.insert("uniqueItems".into(), Value::Bool(true));
    }
    if let Some(min) = &leaf.lengths.minimum {
        map.insert("minItems".into(), Value::Number(min.to_number()));
    }
    if let Some(max) = &leaf.lengths.maximum {
        map.insert("maxItems".into(), Value::Number(max.to_number()));
    }
    Value::Object(map)
}

/// Emit one `contains` demand into `map`; the count window keys appear only where a draft put them.
fn insert_contains(map: &mut Map<String, Value>, facet: &ContainsFacet, draft: Draft) {
    map.insert("contains".into(), emit(facet.schema.kind(), draft));
    if let Some(minimum) = &facet.minimum {
        map.insert("minContains".into(), Value::Number(minimum.to_number()));
    }
    if let Some(maximum) = &facet.maximum {
        map.insert("maxContains".into(), Value::Number(maximum.to_number()));
    }
}

/// Emit an object leaf as `{"type":"object"}` plus its key constraint, required keys and bounds.
fn emit_object(leaf: &ObjectLeaf, draft: Draft) -> Value {
    let mut map = Map::new();
    map.insert("type".into(), Value::String("object".into()));
    // Draft 4 has no `propertyNames`, so a key constraint is spelled as the closed maps it takes to
    // name exactly the keys it admits.
    let spelling = if matches!(draft, Draft::Draft4) {
        draft4_key_spelling(leaf)
    } else {
        None
    };
    if let Some(names) = &leaf.property_names {
        if spelling.is_none() {
            // Draft 4 ignores `propertyNames`, so reaching it there would silently widen the
            // emitted schema; every key constraint Draft 4 can hold has a closed-map spelling.
            debug_assert!(
                !matches!(draft, Draft::Draft4),
                "a Draft 4 key constraint reached emit without a closed-map spelling"
            );
            map.insert("propertyNames".into(), emit(names.kind(), draft));
        }
    }
    match &spelling {
        // One closed map names every admitted key, so the entries sit inside it.
        Some(Draft4Keys::Fused(clause)) => insert_fused_map(&mut map, clause, leaf, draft),
        // No single closed map names them, so the constraint gets maps of its own and the entries
        // stay beside them, saying what values the keys carry without closing anything.
        Some(Draft4Keys::Split(clauses)) => {
            insert_entries(&mut map, leaf, draft);
            map.insert(
                "allOf".into(),
                Value::Array(clauses.iter().map(emit_closed_clause).collect()),
            );
        }
        None => insert_entries(&mut map, leaf, draft),
    }
    if !leaf.required.is_empty() {
        map.insert(
            "required".into(),
            Value::Array(
                leaf.required
                    .iter()
                    .map(|key| Value::String(key.to_string()))
                    .collect(),
            ),
        );
    }
    if let Some(min) = &leaf.sizes.minimum {
        map.insert("minProperties".into(), Value::Number(min.to_number()));
    }
    if let Some(max) = &leaf.sizes.maximum {
        map.insert("maxProperties".into(), Value::Number(max.to_number()));
    }
    Value::Object(map)
}

/// Emit the entries closed over `clause`, with one restored for each key or pattern the clause
/// names that carries no entry of its own.
fn insert_fused_map(
    map: &mut Map<String, Value>,
    clause: &KeyClause,
    leaf: &ObjectLeaf,
    draft: Draft,
) {
    let mut entries: Map<String, Value> = clause
        .keys
        .iter()
        .map(|key| (key.clone(), Value::Object(Map::new())))
        .collect();
    entries.extend(
        leaf.properties
            .iter()
            .map(|(key, schema)| (key.to_string(), emit(schema.kind(), draft))),
    );
    if !entries.is_empty() {
        map.insert("properties".into(), Value::Object(entries));
    }
    let mut patterns: Map<String, Value> = clause
        .patterns
        .iter()
        .map(|pattern| (pattern.clone(), Value::Object(Map::new())))
        .collect();
    patterns.extend(
        leaf.pattern_properties
            .iter()
            .map(|(pattern, schema)| (pattern.to_string(), emit(schema.kind(), draft))),
    );
    if !patterns.is_empty() {
        map.insert("patternProperties".into(), Value::Object(patterns));
    }
    map.insert("additionalProperties".into(), Value::Bool(false));
}

/// Emit the entries that say what a key carries without saying which keys may be present.
fn insert_entries(map: &mut Map<String, Value>, leaf: &ObjectLeaf, draft: Draft) {
    if !leaf.properties.is_empty() {
        let entries: Map<String, Value> = leaf
            .properties
            .iter()
            .map(|(key, schema)| (key.to_string(), emit(schema.kind(), draft)))
            .collect();
        map.insert("properties".into(), Value::Object(entries));
    }
    if !leaf.pattern_properties.is_empty() {
        let entries: Map<String, Value> = leaf
            .pattern_properties
            .iter()
            .map(|(pattern, schema)| (pattern.to_string(), emit(schema.kind(), draft)))
            .collect();
        map.insert("patternProperties".into(), Value::Object(entries));
    }
    if let Some(additional) = &leaf.additional {
        map.insert(
            "additionalProperties".into(),
            emit(additional.kind(), draft),
        );
    }
}

/// A closed map over the clause's keys and patterns, saying nothing about the values they carry.
fn emit_closed_clause(clause: &KeyClause) -> Value {
    let mut map = Map::new();
    if !clause.keys.is_empty() {
        let entries: Map<String, Value> = clause
            .keys
            .iter()
            .map(|key| (key.clone(), Value::Object(Map::new())))
            .collect();
        map.insert("properties".into(), Value::Object(entries));
    }
    if !clause.patterns.is_empty() {
        let entries: Map<String, Value> = clause
            .patterns
            .iter()
            .map(|pattern| (pattern.clone(), Value::Object(Map::new())))
            .collect();
        map.insert("patternProperties".into(), Value::Object(entries));
    }
    map.insert("additionalProperties".into(), Value::Bool(false));
    Value::Object(map)
}

/// The keys and patterns one closed map names: a key is admitted when it is named or matched.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
struct KeyClause {
    keys: Vec<String>,
    patterns: Vec<String>,
}

impl KeyClause {
    /// The clause admitting every key either one admits.
    fn join(&self, other: &Self) -> Self {
        let mut keys = self.keys.clone();
        keys.extend_from_slice(&other.keys);
        keys.sort_unstable();
        keys.dedup();
        let mut patterns = self.patterns.clone();
        patterns.extend_from_slice(&other.patterns);
        patterns.sort_unstable();
        patterns.dedup();
        Self { keys, patterns }
    }
}

/// How Draft 4 spells an object leaf's key constraint.
enum Draft4Keys {
    /// One closed map names every admitted key, so the leaf's entries fit inside it.
    Fused(KeyClause),
    /// Several closed maps are needed, so the leaf's entries stay outside them.
    Split(Vec<KeyClause>),
}

/// The closed maps spelling `leaf`'s key constraint, or `None` when none of them name it exactly.
fn draft4_key_spelling(leaf: &ObjectLeaf) -> Option<Draft4Keys> {
    let names = leaf.property_names.as_ref()?;
    let clauses = draft4_key_clauses(names)?;
    debug_assert!(
        !clauses.is_empty(),
        "a key constraint spells at least one closed map"
    );
    // A lone clause takes the entries in only when it already names every pattern among them: one
    // it leaves out would admit the keys that pattern matches, and a shield beside it would take
    // the place of the `additionalProperties: false` closing the map.
    if let [clause] = clauses.as_slice() {
        if leaf.additional.is_none()
            && leaf.pattern_properties.keys().all(|pattern| {
                clause
                    .patterns
                    .iter()
                    .any(|named| named.as_str() == &**pattern)
            })
        {
            return Some(Draft4Keys::Fused(clause.clone()));
        }
    }
    Some(Draft4Keys::Split(clauses))
}

/// The closed maps every admitted key is named by and no other key is, or `None` for a constraint
/// closed maps cannot name.
fn draft4_key_clauses(names: &Schema) -> Option<Vec<KeyClause>> {
    match names.kind() {
        SchemaKind::Const(value) => Some(vec![KeyClause {
            keys: vec![value.as_value().as_str()?.to_string()],
            patterns: Vec::new(),
        }]),
        SchemaKind::Enum(values) => {
            let mut keys = Vec::with_capacity(values.as_slice().len());
            for value in values.as_slice() {
                keys.push(value.as_value().as_str()?.to_string());
            }
            keys.sort_unstable();
            keys.dedup();
            Some(vec![KeyClause {
                keys,
                patterns: Vec::new(),
            }])
        }
        // A key matches every pattern the leaf carries, so each pattern closes the map on its own.
        SchemaKind::String(leaf) => {
            let leaf = leaf.get();
            // Any facet beyond the patterns narrows the constraint below what they spell.
            // The barred facets are defensive: no synthesis path reaches here carrying one.
            if leaf.lengths.minimum.is_some()
                || leaf.lengths.maximum.is_some()
                || !leaf.formats.is_empty()
                || !leaf.excluded_formats.is_empty()
                || !leaf.content_media_types.is_empty()
                || !leaf.content_encodings.is_empty()
                || !leaf.excluded_patterns.is_empty()
                || !leaf.excluded.is_empty()
            {
                return None;
            }
            debug_assert!(
                !leaf.patterns.is_empty(),
                "a string leaf carrying no other facet carries a pattern"
            );
            Some(
                leaf.patterns
                    .iter()
                    .map(|pattern| KeyClause {
                        keys: Vec::new(),
                        patterns: vec![pattern.to_string()],
                    })
                    .collect(),
            )
        }
        // A key holds the union by holding one branch, so taking one clause from each branch and
        // joining them gives a map every admitted key is named by - one such map per combination.
        // e.g.  anyOf [{"type": "string", "pattern": "^a"}, {"enum": ["x"]}]
        //       met with
        //       anyOf [{"type": "string", "pattern": "^b"}, {"enum": ["x"]}]
        //       =>  allOf [
        //             {"properties": {"x": {}}, "patternProperties": {"^a": {}}, "additionalProperties": false},
        //             {"properties": {"x": {}}, "patternProperties": {"^b": {}}, "additionalProperties": false}
        //           ]
        SchemaKind::AnyOf(branches) => {
            let mut clauses = vec![KeyClause::default()];
            for branch in branches.as_slice() {
                let alternatives = draft4_key_clauses(branch)?;
                clauses = clauses
                    .iter()
                    .flat_map(|clause| {
                        alternatives
                            .iter()
                            .map(|alternative| clause.join(alternative))
                    })
                    .collect();
            }
            clauses.sort_unstable();
            clauses.dedup();
            Some(clauses)
        }
        SchemaKind::True
        | SchemaKind::False
        | SchemaKind::MultiType(_)
        | SchemaKind::TypedGroup { .. }
        | SchemaKind::Integer(_)
        | SchemaKind::Number(_)
        | SchemaKind::Array(_)
        | SchemaKind::Object(_)
        | SchemaKind::Not(_)
        | SchemaKind::AllOf(_)
        | SchemaKind::OneOf(_)
        | SchemaKind::Reference(_)
        | SchemaKind::Raw(_) => None,
    }
}

/// Emit an integer leaf as `{"type":"integer"}` plus its interval bounds.
fn emit_integer(leaf: &IntegerLeaf) -> Value {
    let mut map = Map::new();
    map.insert("type".into(), Value::String("integer".into()));
    if let Some(min) = &leaf.bounds.minimum {
        map.insert("minimum".into(), Value::Number(min.to_number()));
    }
    if let Some(max) = &leaf.bounds.maximum {
        map.insert("maximum".into(), Value::Number(max.to_number()));
    }
    emit_divisors(&mut map, &leaf.multiple_of, &leaf.not_multiple_of, false);
    Value::Object(map)
}

/// A lone divisor sits beside the other facets, and a lone barred constraint under `not`; several
/// of either are spelled as an `allOf`, since one keyword slot cannot carry them.
fn emit_divisors(
    map: &mut Map<String, Value>,
    divisors: &Divisors,
    barred: &ExcludedDivisors,
    excludes_integers: bool,
) {
    let step_object = |step: &BoundRational| {
        let mut object = Map::new();
        object.insert("multipleOf".into(), Value::Number(step.to_number()));
        Value::Object(object)
    };
    let mut conjuncts: Vec<Value> = Vec::new();
    match divisors.as_slice() {
        [] => {}
        [step] => {
            map.insert("multipleOf".into(), Value::Number(step.to_number()));
        }
        steps => conjuncts.extend(steps.iter().map(step_object)),
    }
    let mut negated: Vec<Value> = Vec::new();
    if excludes_integers {
        negated.push(keyed("type", Value::String("integer".into())));
    }
    negated.extend(barred.as_slice().iter().map(step_object));
    match <[Value; 1]>::try_from(negated) {
        Ok([sole]) => {
            map.insert("not".into(), sole);
        }
        Err(negated) => conjuncts.extend(negated.into_iter().map(|inner| keyed("not", inner))),
    }
    if !conjuncts.is_empty() {
        map.insert("allOf".into(), Value::Array(conjuncts));
    }
}

/// Emit a standalone `Enum`; collapse to `type:[...]` when the value set saturates one or more JSON types.
fn emit_enum(values: &[CanonicalJson]) -> Value {
    if let Some(set) = SchemaKind::finite_values_saturated_domain(values) {
        return emit_multi_type(set);
    }
    keyed(
        "enum",
        Value::Array(values.iter().map(CanonicalJson::to_value).collect()),
    )
}

/// Emit a type set as `{"type": "x"}` for a singleton or `{"type": [...]}` otherwise.
fn emit_multi_type(set: JsonTypeSet) -> Value {
    // `set.iter()` yields in canonical order (null, boolean, integer, ...).
    let mut names = set.iter().map(|ty| ty.to_string());
    match (names.next(), names.next()) {
        (Some(only), None) => keyed("type", Value::String(only)),
        (first, second) => {
            let names: Vec<Value> = first
                .into_iter()
                .chain(second)
                .chain(names)
                .map(Value::String)
                .collect();
            keyed("type", Value::Array(names))
        }
    }
}

pub(crate) fn schema_uri(draft: Draft) -> Option<&'static str> {
    match draft {
        Draft::Draft4 => Some("http://json-schema.org/draft-04/schema#"),
        Draft::Draft6 => Some("http://json-schema.org/draft-06/schema#"),
        Draft::Draft7 => Some("http://json-schema.org/draft-07/schema#"),
        Draft::Draft201909 => Some("https://json-schema.org/draft/2019-09/schema"),
        Draft::Draft202012 => Some("https://json-schema.org/draft/2020-12/schema"),
        // `Draft::Unknown` (unrecognised `$schema`) has no canonical meta-schema; omit `$schema`.
        _ => None,
    }
}

/// Insert `$schema` into the document, first rewriting a boolean form into its object shape.
fn with_schema_uri(value: Value, uri: &'static str) -> Value {
    let mut map = match value {
        Value::Object(map) => map,
        Value::Bool(true) => Map::new(),
        Value::Bool(false) => {
            let mut map = Map::new();
            map.insert("not".into(), Value::Object(Map::new()));
            map
        }
        other @ (Value::Null | Value::Number(_) | Value::String(_) | Value::Array(_)) => {
            unreachable!("emit yields only objects or booleans: {other:?}")
        }
    };
    map.insert("$schema".into(), Value::String(uri.into()));
    Value::Object(map)
}

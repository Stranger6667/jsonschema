//! Parsing schema documents into structural IR; anything not modeled stays `Raw`.
use referencing::Draft;
use serde_json::Value;

use crate::{
    canonical::ir::{CanonicalJson, Schema, SchemaKind},
    JsonType, JsonTypeSet,
};

/// Parse a document into structural IR when every construct is modeled; `None` keeps it `Raw`.
/// Keywords the draft does not define are annotations the validator ignores, so they never block
/// modeling - except an unknown `$schema`, whose dialect semantics are unknowable.
pub(crate) fn parse(value: &Value, draft: Draft) -> Option<Schema> {
    let map = match value {
        Value::Bool(true) => return Some(Schema::new(SchemaKind::True)),
        Value::Bool(false) => return Some(Schema::new(SchemaKind::False)),
        Value::Object(map) => map,
        _ => return None,
    };

    let mut type_set = None;
    let mut enum_values = None;
    let mut const_value = None;
    for (key, entry) in map {
        match key.as_str() {
            "$schema" => {
                let uri = entry.as_str().expect("meta-valid $schema is a string");
                if matches!(Draft::from_schema_uri(uri), Draft::Unknown) {
                    return None;
                }
            }
            "type" => type_set = Some(parse_type_set(entry)),
            // TODO(canonical): not modeled yet - `const`/`enum` numbers without a plain spelling
            // have no exact runtime comparison; such documents stay raw.
            "enum" if draft.is_known_keyword("enum") => {
                if !finite_value_spelling_is_exact(entry) {
                    return None;
                }
                enum_values = Some(entry.as_array().expect("meta-valid enum is an array"));
            }
            "const" if draft.is_known_keyword("const") => {
                if !finite_value_spelling_is_exact(entry) {
                    return None;
                }
                const_value = Some(entry);
            }
            // TODO(canonical): not modeled yet - every other known keyword keeps the document raw.
            other if draft.is_known_keyword(other) => return None,
            _ => {}
        }
    }

    // TODO(canonical): not modeled yet - Draft 4 `integer` mixed with other types in a type list
    // alongside `const`/`enum`.
    if matches!(draft, Draft::Draft4)
        && (enum_values.is_some() || const_value.is_some())
        && type_set.is_some_and(|set| {
            set.contains(JsonType::Integer) && set != JsonTypeSet::from(JsonType::Integer)
        })
    {
        return None;
    }

    Some(
        match (type_set, admitted_values(enum_values, const_value)) {
            (None, None) => Schema::new(SchemaKind::True),
            (Some(set), None) => type_set_schema(set),
            (None, Some(values)) => canonicalize_value_set(values),
            (Some(set), Some(values)) => restrict_values_to_types(values, set, draft),
        },
    )
}

/// The finite value set admitted by `const` and `enum` together: their conjunction.
fn admitted_values(
    enum_values: Option<&Vec<Value>>,
    const_value: Option<&Value>,
) -> Option<Vec<CanonicalJson>> {
    let mut values: Option<Vec<CanonicalJson>> =
        enum_values.map(|entries| entries.iter().map(CanonicalJson::from_value).collect());
    if let Some(constant) = const_value {
        let constant = CanonicalJson::from_value(constant);
        values = Some(match values {
            Some(members) => members
                .into_iter()
                .filter(|value| *value == constant)
                .collect(),
            None => vec![constant],
        });
    }
    values
}

/// Intersect admitted values with a `type` set: drop values outside it, then pack the rest.
fn restrict_values_to_types(values: Vec<CanonicalJson>, set: JsonTypeSet, draft: Draft) -> Schema {
    let cover = SchemaKind::semantic_cover(set);
    let filtered = values
        .into_iter()
        .filter(|value| cover.contains(value.json_type()))
        .collect();
    let value_set = canonicalize_value_set(filtered);
    // Draft 4 says `1.0` is not an integer, and value equality cannot tell `1` from `1.0` -
    // keep the type check.
    if keeps_draft4_integer_guard(set, draft) && !matches!(value_set.kind(), SchemaKind::False) {
        Schema::new(SchemaKind::TypedGroup {
            ty: JsonType::Integer,
            body: value_set,
        })
    } else {
        value_set
    }
}

/// Whether every number nested in an instance-data value keeps a plain canonical spelling.
#[cfg(feature = "arbitrary-precision")]
fn finite_value_spelling_is_exact(value: &Value) -> bool {
    match value {
        Value::Number(number) => {
            let canonical = crate::canonical::json::canonical_number(number.as_str());
            let text = canonical.as_deref().unwrap_or(number.as_str());
            !text.bytes().any(|byte| matches!(byte, b'e' | b'E'))
        }
        Value::Array(items) => items.iter().all(finite_value_spelling_is_exact),
        Value::Object(map) => map.values().all(finite_value_spelling_is_exact),
        _ => true,
    }
}

#[cfg(not(feature = "arbitrary-precision"))]
fn finite_value_spelling_is_exact(_value: &Value) -> bool {
    // Default-build numbers are `i64`/`u64`/`f64`; their canonical spellings never go scientific.
    true
}

/// Read a `type` keyword value - a single name or a list of names - into a [`JsonTypeSet`].
fn parse_type_set(value: &Value) -> JsonTypeSet {
    let name_to_type = |name: &str| name.parse().expect("meta-valid type name");
    match value {
        Value::String(name) => JsonTypeSet::from(name_to_type(name)),
        Value::Array(names) => names.iter().fold(JsonTypeSet::empty(), |set, name| {
            set.insert(name_to_type(
                name.as_str().expect("meta-valid type entry is a string"),
            ))
        }),
        other => unreachable!("meta-valid type is a string or array: {other:?}"),
    }
}

/// Canonical node for a bare type set: `null`/`boolean` become their finite value sets, the full
/// set is `True`, anything else stays a `MultiType`.
fn type_set_schema(set: JsonTypeSet) -> Schema {
    let set = SchemaKind::canonical_type_set(set);
    if SchemaKind::semantic_cover(set) == JsonTypeSet::all() {
        return Schema::new(SchemaKind::True);
    }
    if set == JsonTypeSet::from(JsonType::Null) {
        return Schema::new(SchemaKind::Const(CanonicalJson::from_value(&Value::Null)));
    }
    if set == JsonTypeSet::from(JsonType::Boolean) {
        return Schema::new(SchemaKind::Enum(vec![
            CanonicalJson::from_value(&Value::Bool(false)),
            CanonicalJson::from_value(&Value::Bool(true)),
        ]));
    }
    Schema::new(SchemaKind::MultiType(set))
}

/// Pack members into the canonical value-set shape: empty is unsatisfiable, singletons are `const`,
/// larger sets are sorted, deduplicated `enum`s - unless they saturate 2+ types, which is a type list.
pub(crate) fn canonicalize_value_set(mut members: Vec<CanonicalJson>) -> Schema {
    members.sort();
    members.dedup();
    match members.len() {
        0 => Schema::new(SchemaKind::False),
        1 => Schema::new(SchemaKind::Const(
            members.into_iter().next().expect("len == 1"),
        )),
        _ => {
            if let Some(type_set) = SchemaKind::finite_values_saturated_domain(&members) {
                if type_set.len() >= 2 {
                    return Schema::new(SchemaKind::MultiType(type_set));
                }
            }
            Schema::new(SchemaKind::Enum(members))
        }
    }
}

/// Draft 4 says `1.0` is not an integer, so its `integer` check cannot fold into value equality.
fn keeps_draft4_integer_guard(set: JsonTypeSet, draft: Draft) -> bool {
    matches!(draft, Draft::Draft4)
        && set.contains(JsonType::Integer)
        && !set.contains(JsonType::Number)
}

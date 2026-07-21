//! Parsing schema documents into structural IR; anything not modeled stays `Raw`.
use std::sync::Arc;

use referencing::Draft;
use serde_json::Value;

use crate::{
    canonical::{
        algebra,
        context::CanonicalizationContext,
        ir::{
            BoundCardinality, BoundInteger, CanonicalJson, IntegerBounds, IntegerLeaf, Schema,
            SchemaKind, StringLeaf,
        },
        CanonicalizationError,
    },
    JsonType, JsonTypeSet,
};

/// Parse a document into structural IR when every construct is modeled; `Ok(None)` keeps it `Raw`.
/// Keywords the draft does not define are annotations the validator ignores, so they never block
/// modeling - except an unknown `$schema`, whose dialect semantics are unknowable.
pub(crate) fn parse(
    value: &Value,
    ctx: &CanonicalizationContext,
) -> Result<Option<Schema>, CanonicalizationError> {
    parse_schema(value, ctx, true)
}

fn parse_schema(
    value: &Value,
    ctx: &CanonicalizationContext,
    is_root: bool,
) -> Result<Option<Schema>, CanonicalizationError> {
    let map = match value {
        Value::Bool(true) => return Ok(Some(Schema::new(SchemaKind::True))),
        Value::Bool(false) => return Ok(Some(Schema::new(SchemaKind::False))),
        Value::Object(map) => map,
        _ => unreachable!("meta-validation rejects a non-object, non-boolean schema document"),
    };

    let mut type_set = None;
    let mut enum_values = None;
    let mut const_value = None;
    let mut min_length: Option<BoundCardinality> = None;
    let mut max_length: Option<BoundCardinality> = None;
    let mut patterns: Vec<Arc<str>> = Vec::new();
    let mut minimum: Option<BoundInteger> = None;
    let mut maximum: Option<BoundInteger> = None;
    let mut conjuncts: Vec<Schema> = Vec::new();
    for (key, entry) in map {
        match key.as_str() {
            // TODO(canonical): not modeled yet - a nested `$schema` starts an embedded resource
            // with its own dialect.
            "$schema" if !is_root => return Ok(None),
            "$schema" => {
                let uri = entry.as_str().expect("meta-valid $schema is a string");
                if matches!(Draft::from_schema_uri(uri), Draft::Unknown) {
                    return Ok(None);
                }
            }
            "allOf" => {
                for branch in entry.as_array().expect("meta-valid allOf is an array") {
                    match parse_schema(branch, ctx, false)? {
                        Some(schema) => conjuncts.push(schema),
                        None => return Ok(None),
                    }
                }
            }
            "anyOf" => {
                let mut branches = Vec::new();
                for branch in entry.as_array().expect("meta-valid anyOf is an array") {
                    match parse_schema(branch, ctx, false)? {
                        Some(schema) => branches.push(schema),
                        None => return Ok(None),
                    }
                }
                conjuncts.push(algebra::union(branches, ctx));
            }
            "type" => type_set = Some(parse_type_set(entry)),
            // TODO(canonical): not modeled yet - `const`/`enum` numbers without a plain spelling
            // have no exact runtime comparison; such documents stay raw.
            "enum" if ctx.draft().is_known_keyword("enum") => {
                if !finite_value_spelling_is_exact(entry) {
                    return Ok(None);
                }
                enum_values = Some(entry.as_array().expect("meta-valid enum is an array"));
            }
            "const" if ctx.draft().is_known_keyword("const") => {
                if !finite_value_spelling_is_exact(entry) {
                    return Ok(None);
                }
                const_value = Some(entry);
            }
            // In the default build a length bound past `u64` has no modeled form; keep the document raw.
            "minLength" if ctx.draft().is_known_keyword("minLength") => match length_bound(entry) {
                Some(bound) => min_length = Some(bound),
                None => return Ok(None),
            },
            "maxLength" if ctx.draft().is_known_keyword("maxLength") => match length_bound(entry) {
                Some(bound) => max_length = Some(bound),
                None => return Ok(None),
            },
            "pattern" if ctx.draft().is_known_keyword("pattern") => {
                let pattern: Arc<str> =
                    Arc::from(entry.as_str().expect("meta-valid pattern is a string"));
                if ctx.compile_regex(&pattern).is_none() {
                    return Err(CanonicalizationError::InvalidPattern {
                        pattern: pattern.to_string(),
                    });
                }
                patterns.push(pattern);
            }
            // A fractional or (default build) out-of-`i64` bound has no modeled integer form; keep it raw.
            "minimum" if ctx.draft().is_known_keyword("minimum") => match numeric_bound(entry) {
                Some(bound) => minimum = Some(bound),
                None => return Ok(None),
            },
            "maximum" if ctx.draft().is_known_keyword("maximum") => match numeric_bound(entry) {
                Some(bound) => maximum = Some(bound),
                None => return Ok(None),
            },
            // TODO(canonical): not modeled yet - every other known keyword keeps the document raw.
            other if ctx.draft().is_known_keyword(other) => return Ok(None),
            _ => {}
        }
    }

    // TODO(canonical): not modeled yet - Draft 4 `integer` mixed with other types in a type list
    // alongside `const`/`enum`.
    if matches!(ctx.draft(), Draft::Draft4)
        && (enum_values.is_some() || const_value.is_some())
        && type_set.is_some_and(|set| {
            set.contains(JsonType::Integer) && set != JsonTypeSet::from(JsonType::Integer)
        })
    {
        return Ok(None);
    }

    // `minLength: 0` is the type-default, so drop it: the leaf then compares equal to one without it.
    if min_length.as_ref().is_some_and(BoundCardinality::is_zero) {
        min_length = None;
    }
    if min_length.is_some() || max_length.is_some() || !patterns.is_empty() {
        patterns.sort();
        patterns.dedup();
        let leaf = StringLeaf {
            min_length,
            max_length,
            patterns,
        };
        conjuncts.push(string_facet_schema(leaf, ctx));
    }

    if minimum.is_some() || maximum.is_some() {
        // Only `type: integer` bounds are modeled yet; `number`/untyped numeric facets stay raw.
        if type_set == Some(JsonTypeSet::from(JsonType::Integer)) {
            conjuncts.push(algebra::integer_leaf(IntegerLeaf {
                bounds: IntegerBounds { minimum, maximum },
            }));
        } else {
            return Ok(None);
        }
    }

    let base = match (type_set, admitted_values(enum_values, const_value)) {
        (None, None) => Schema::new(SchemaKind::True),
        (Some(set), None) => type_set_schema(set),
        (None, Some(values)) => canonicalize_value_set(values),
        (Some(set), Some(values)) => restrict_values_to_types(values, set, ctx),
    };
    // A schema object's keywords all apply to the same value at once, so combine them by intersection.
    Ok(Some(
        conjuncts.into_iter().fold(base, |result, conjunct| {
            algebra::intersect(result, conjunct, ctx)
        }),
    ))
}

/// A modeled length bound, or `None` (keep the document raw) when the count is not representable.
fn length_bound(value: &Value) -> Option<BoundCardinality> {
    let Value::Number(number) = value else {
        unreachable!("meta-valid length bound is a number")
    };
    BoundCardinality::from_number(number)
}

/// A modeled integer bound, or `None` (keep the document raw) when the value is not an integer.
fn numeric_bound(value: &Value) -> Option<BoundInteger> {
    let Value::Number(number) = value else {
        unreachable!("meta-valid numeric bound is a number")
    };
    BoundInteger::from_number(number)
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
pub(crate) fn restrict_values_to_types(
    values: Vec<CanonicalJson>,
    set: JsonTypeSet,
    ctx: &CanonicalizationContext,
) -> Schema {
    let cover = SchemaKind::semantic_cover(set);
    let filtered: Vec<CanonicalJson> = values
        .into_iter()
        .filter(|value| cover.contains(value.json_type()))
        .collect();
    if !keeps_draft4_integer_guard(set, ctx.draft()) {
        return canonicalize_value_set(filtered);
    }
    // Draft 4 cannot tell `1` from `1.0` by value equality, so integer members keep the integer type
    // guard; members of other types (which the set also admits) do not.
    let (integers, others): (Vec<_>, Vec<_>) = filtered
        .into_iter()
        .partition(|value| value.json_type() == JsonType::Integer);
    let mut branches = Vec::new();
    let integer_set = canonicalize_value_set(integers);
    if !matches!(integer_set.kind(), SchemaKind::False) {
        branches.push(Schema::new(SchemaKind::TypedGroup {
            ty: JsonType::Integer,
            body: integer_set,
        }));
    }
    let other_set = canonicalize_value_set(others);
    if !matches!(other_set.kind(), SchemaKind::False) {
        branches.push(other_set);
    }
    algebra::union(branches, ctx)
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
pub(crate) fn type_set_schema(set: JsonTypeSet) -> Schema {
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

/// A string facet constrains only strings, so `{"minLength": 3}` becomes
/// `anyOf: [<non-string types>, {"type": "string", "minLength": 3}]`.
fn string_facet_schema(leaf: StringLeaf, ctx: &CanonicalizationContext) -> Schema {
    let non_string = Schema::new(SchemaKind::MultiType(
        JsonTypeSet::all().remove(JsonType::String),
    ));
    algebra::union(vec![non_string, algebra::string_leaf(leaf)], ctx)
}

/// Draft 4 says `1.0` is not an integer, so its `integer` check cannot fold into value equality.
fn keeps_draft4_integer_guard(set: JsonTypeSet, draft: Draft) -> bool {
    matches!(draft, Draft::Draft4)
        && set.contains(JsonType::Integer)
        && !set.contains(JsonType::Number)
}

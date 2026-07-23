//! Parsing schema documents into structural IR; anything not modeled stays `Raw`.
use std::sync::Arc;

use referencing::Draft;
use serde_json::Value;

use crate::{
    canonical::{
        algebra,
        context::CanonicalizationContext,
        ir::{
            AtLeastTwo, BoundCardinality, BoundInteger, BoundNumber, CanonicalJson, IntegerLeaf,
            LengthBounds, NumberLeaf, Schema, SchemaKind, Side, StringLeaf,
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
        // Not a schema document; the root is rejected earlier, a nested one keeps the document raw.
        Value::Null | Value::Number(_) | Value::String(_) | Value::Array(_) => return Ok(None),
    };

    let mut type_set = None;
    let mut enum_values = None;
    let mut const_value = None;
    let mut min_length: Option<BoundCardinality> = None;
    let mut max_length: Option<BoundCardinality> = None;
    let mut min_items: Option<BoundCardinality> = None;
    let mut max_items: Option<BoundCardinality> = None;
    let mut min_properties: Option<BoundCardinality> = None;
    let mut max_properties: Option<BoundCardinality> = None;
    let mut patterns: Vec<Arc<str>> = Vec::new();
    let mut formats: Vec<Arc<str>> = Vec::new();
    let mut multiple_of: Option<BoundInteger> = None;
    // The number domain keeps each end as written: on the reals an excluded bound has no successor
    // to fold it into, unlike the integer path below.
    let mut real_minimum: Option<BoundNumber> = None;
    let mut real_maximum: Option<BoundNumber> = None;
    // Draft 4 spells exclusivity as a boolean modifier on `minimum`/`maximum`, which may be read
    // before the bound it modifies, so it is applied once the whole object has been read.
    let mut draft4_exclusive_minimum = false;
    let mut draft4_exclusive_maximum = false;
    let mut conjuncts: Vec<Schema> = Vec::new();
    for (key, entry) in map {
        match (key.as_str(), entry) {
            // TODO(canonical): not modeled yet - a nested `$schema` starts an embedded resource
            // with its own dialect.
            ("$schema", _) if !is_root => return Ok(None),
            ("$schema", Value::String(uri)) => {
                if matches!(Draft::from_schema_uri(uri), Draft::Unknown) {
                    return Ok(None);
                }
            }
            ("allOf", Value::Array(branches)) => {
                for branch in branches {
                    match parse_schema(branch, ctx, false)? {
                        Some(schema) => conjuncts.push(schema),
                        None => return Ok(None),
                    }
                }
            }
            ("anyOf", Value::Array(items)) => {
                let mut branches = Vec::new();
                for branch in items {
                    match parse_schema(branch, ctx, false)? {
                        Some(schema) => branches.push(schema),
                        None => return Ok(None),
                    }
                }
                conjuncts.push(algebra::union(branches, ctx));
            }
            ("type", value) => match parse_type_set(value) {
                Some(set) => type_set = Some(set),
                None => return Ok(None),
            },
            // TODO(canonical): not modeled yet - `const`/`enum` numbers without a plain spelling
            // have no exact runtime comparison; such documents stay raw.
            ("enum", Value::Array(values)) if ctx.draft().is_known_keyword("enum") => {
                if !values.iter().all(finite_value_spelling_is_exact) {
                    return Ok(None);
                }
                enum_values = Some(values);
            }
            ("const", value) if ctx.draft().is_known_keyword("const") => {
                if !finite_value_spelling_is_exact(value) {
                    return Ok(None);
                }
                const_value = Some(value);
            }
            // In the default build a length bound past `u64` has no modeled form; keep the document raw.
            ("minLength", Value::Number(number)) if ctx.draft().is_known_keyword("minLength") => {
                match BoundCardinality::from_number(number) {
                    Some(bound) => min_length = Some(bound),
                    None => return Ok(None),
                }
            }
            ("maxLength", Value::Number(number)) if ctx.draft().is_known_keyword("maxLength") => {
                match BoundCardinality::from_number(number) {
                    Some(bound) => max_length = Some(bound),
                    None => return Ok(None),
                }
            }
            ("minItems", Value::Number(number)) if ctx.draft().is_known_keyword("minItems") => {
                match BoundCardinality::from_number(number) {
                    Some(bound) => min_items = Some(bound),
                    None => return Ok(None),
                }
            }
            ("maxItems", Value::Number(number)) if ctx.draft().is_known_keyword("maxItems") => {
                match BoundCardinality::from_number(number) {
                    Some(bound) => max_items = Some(bound),
                    None => return Ok(None),
                }
            }
            ("minProperties", Value::Number(number))
                if ctx.draft().is_known_keyword("minProperties") =>
            {
                match BoundCardinality::from_number(number) {
                    Some(bound) => min_properties = Some(bound),
                    None => return Ok(None),
                }
            }
            ("maxProperties", Value::Number(number))
                if ctx.draft().is_known_keyword("maxProperties") =>
            {
                match BoundCardinality::from_number(number) {
                    Some(bound) => max_properties = Some(bound),
                    None => return Ok(None),
                }
            }
            ("pattern", Value::String(text)) if ctx.draft().is_known_keyword("pattern") => {
                let pattern: Arc<str> = Arc::from(text.as_str());
                if ctx.compile_regex(&pattern).is_none() {
                    return Err(CanonicalizationError::InvalidPattern {
                        pattern: pattern.to_string(),
                    });
                }
                // `pattern` matches anywhere in the string, so an empty one matches every string.
                if !pattern.is_empty() {
                    patterns.push(pattern);
                }
            }
            // An annotation-only `format` constrains nothing, so it leaves no trace in the IR.
            ("format", Value::String(name)) if ctx.draft().is_known_keyword("format") => {
                if ctx.validate_formats() {
                    formats.push(Arc::from(name.as_str()));
                }
            }
            // Only a positive whole divisor `f64` holds exactly is modeled. A fractional one
            // constrains integers in a way the interval algebra cannot express, and past `f64`
            // precision the runtime's float division disagrees with exact arithmetic - both keep
            // the document raw.
            ("multipleOf", Value::Number(number)) if ctx.draft().is_known_keyword("multipleOf") => {
                match BoundInteger::from_number(number)
                    .filter(BoundInteger::is_positive)
                    .filter(BoundInteger::is_exact_in_f64)
                {
                    Some(step) => multiple_of = Some(step),
                    None => return Ok(None),
                }
            }
            ("minimum", Value::Number(number)) if ctx.draft().is_known_keyword("minimum") => {
                real_minimum = tighter_real(real_minimum, number, true, Side::Lower);
            }
            ("maximum", Value::Number(number)) if ctx.draft().is_known_keyword("maximum") => {
                real_maximum = tighter_real(real_maximum, number, true, Side::Upper);
            }
            // Draft 6+ spells an exclusive bound as its own numeric keyword.
            ("exclusiveMinimum", Value::Number(number))
                if !matches!(ctx.draft(), Draft::Draft4)
                    && ctx.draft().is_known_keyword("exclusiveMinimum") =>
            {
                real_minimum = tighter_real(real_minimum, number, false, Side::Lower);
            }
            ("exclusiveMaximum", Value::Number(number))
                if !matches!(ctx.draft(), Draft::Draft4)
                    && ctx.draft().is_known_keyword("exclusiveMaximum") =>
            {
                real_maximum = tighter_real(real_maximum, number, false, Side::Upper);
            }
            ("exclusiveMinimum", Value::Bool(flag)) if matches!(ctx.draft(), Draft::Draft4) => {
                draft4_exclusive_minimum = *flag;
            }
            ("exclusiveMaximum", Value::Bool(flag)) if matches!(ctx.draft(), Draft::Draft4) => {
                draft4_exclusive_maximum = *flag;
            }
            // TODO(canonical): not modeled yet - every other known keyword keeps the document raw.
            (other, _) if ctx.draft().is_known_keyword(other) => return Ok(None),
            _ => {}
        }
    }

    if draft4_exclusive_minimum {
        real_minimum = real_minimum.map(BoundNumber::excluded);
    }
    if draft4_exclusive_maximum {
        real_maximum = real_maximum.map(BoundNumber::excluded);
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
    if min_length.is_some() || max_length.is_some() || !patterns.is_empty() || !formats.is_empty() {
        patterns.sort();
        patterns.dedup();
        formats.sort();
        formats.dedup();
        let leaf = StringLeaf {
            lengths: LengthBounds {
                minimum: min_length,
                maximum: max_length,
            },
            patterns,
            formats,
        };
        conjuncts.push(string_facet_schema(leaf, ctx));
    }

    // `minItems: 0` is the type-default, so drop it: the window then compares equal to one without it.
    if min_items.as_ref().is_some_and(BoundCardinality::is_zero) {
        min_items = None;
    }
    if min_items.is_some() || max_items.is_some() {
        conjuncts.push(array_facet_schema(
            LengthBounds {
                minimum: min_items,
                maximum: max_items,
            },
            ctx,
        ));
    }

    // `minProperties: 0` is the type-default, so drop it: the window then compares equal to one without it.
    if min_properties
        .as_ref()
        .is_some_and(BoundCardinality::is_zero)
    {
        min_properties = None;
    }
    if min_properties.is_some() || max_properties.is_some() {
        conjuncts.push(object_facet_schema(
            LengthBounds {
                minimum: min_properties,
                maximum: max_properties,
            },
            ctx,
        ));
    }

    if real_minimum.is_some() || real_maximum.is_some() || multiple_of.is_some() {
        // A divisor is modeled only on `integer`; on the reals it needs its own arithmetic.
        if multiple_of.is_some() && type_set != Some(JsonTypeSet::from(JsonType::Integer)) {
            return Ok(None);
        }
        let leaf = NumberLeaf {
            minimum: real_minimum,
            maximum: real_maximum,
        };
        // The integers the interval admits must be representable: the interval may still meet
        // `integer` through an `allOf`, and there it is the only form left to express.
        let Some(bounds) = algebra::integer_bounds_within(&leaf) else {
            return Ok(None);
        };
        if type_set == Some(JsonTypeSet::from(JsonType::Integer)) {
            // Every integer is a multiple of one, so the divisor leaves no trace here. It still had
            // to reach this point: on `number` it means "whole", which the branch above keeps raw.
            let multiple_of = multiple_of.filter(|step| !step.is_one());
            conjuncts.push(algebra::integer_leaf(
                IntegerLeaf {
                    bounds,
                    multiple_of,
                },
                ctx,
            ));
        } else {
            conjuncts.push(number_facet_schema(leaf, ctx));
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
        Value::Null | Value::Bool(_) | Value::String(_) => true,
    }
}

#[cfg(not(feature = "arbitrary-precision"))]
fn finite_value_spelling_is_exact(_value: &Value) -> bool {
    // Default-build numbers are `i64`/`u64`/`f64`; their canonical spellings never go scientific.
    true
}

/// Read a `type` keyword value - a single name or a list of names - into a [`JsonTypeSet`];
/// `None` when it is not a type declaration this build understands.
fn parse_type_set(value: &Value) -> Option<JsonTypeSet> {
    match value {
        Value::String(name) => Some(JsonTypeSet::from(name.parse::<JsonType>().ok()?)),
        Value::Array(names) => names.iter().try_fold(JsonTypeSet::empty(), |set, name| {
            Some(set.insert(name.as_str()?.parse::<JsonType>().ok()?))
        }),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => None,
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
        return canonicalize_value_set(vec![
            CanonicalJson::from_value(&Value::Bool(false)),
            CanonicalJson::from_value(&Value::Bool(true)),
        ]);
    }
    Schema::new(SchemaKind::MultiType(set))
}

/// Pack members into the canonical value-set shape: empty is unsatisfiable, singletons are `const`,
/// larger sets are sorted, deduplicated `enum`s - unless they saturate 2+ types, which is a type list.
pub(crate) fn canonicalize_value_set(members: Vec<CanonicalJson>) -> Schema {
    match AtLeastTwo::new(members) {
        Ok(values) => {
            if let Some(type_set) = SchemaKind::finite_values_saturated_domain(values.as_slice()) {
                if type_set.len() >= 2 {
                    return Schema::new(SchemaKind::MultiType(type_set));
                }
            }
            Schema::new(SchemaKind::Enum(values))
        }
        Err(mut lone) => match lone.pop() {
            Some(only) => Schema::new(SchemaKind::Const(only)),
            None => Schema::new(SchemaKind::False),
        },
    }
}

/// Keep whichever end admits fewer values.
fn tighter_real(
    current: Option<BoundNumber>,
    limit: &serde_json::Number,
    inclusive: bool,
    side: Side,
) -> Option<BoundNumber> {
    let bound = BoundNumber::new(limit, inclusive);
    match current {
        Some(current) if current.is_tighter_than(&bound, side) => Some(current),
        _ => Some(bound),
    }
}

/// A numeric facet constrains only numbers, so `{"minimum": 3}` becomes
/// `anyOf: [<non-number types>, {"type": "number", "minimum": 3}]`.
fn number_facet_schema(leaf: NumberLeaf, ctx: &CanonicalizationContext) -> Schema {
    let non_number = Schema::new(SchemaKind::MultiType(
        JsonTypeSet::all()
            .remove(JsonType::Number)
            .remove(JsonType::Integer),
    ));
    algebra::union(vec![non_number, algebra::number_leaf(leaf)], ctx)
}

/// A string facet constrains only strings, so `{"minLength": 3}` becomes
/// `anyOf: [<non-string types>, {"type": "string", "minLength": 3}]`.
fn string_facet_schema(leaf: StringLeaf, ctx: &CanonicalizationContext) -> Schema {
    let non_string = Schema::new(SchemaKind::MultiType(
        JsonTypeSet::all().remove(JsonType::String),
    ));
    algebra::union(vec![non_string, algebra::string_leaf(leaf, ctx)], ctx)
}

/// A length facet constrains only arrays, so `{"minItems": 1}` becomes
/// `anyOf: [<non-array types>, {"type": "array", "minItems": 1}]`.
fn array_facet_schema(lengths: LengthBounds, ctx: &CanonicalizationContext) -> Schema {
    let non_array = Schema::new(SchemaKind::MultiType(
        JsonTypeSet::all().remove(JsonType::Array),
    ));
    algebra::union(vec![non_array, algebra::array_leaf(lengths)], ctx)
}

/// A property-count facet constrains only objects, so `{"minProperties": 1}` becomes
/// `anyOf: [<non-object types>, {"type": "object", "minProperties": 1}]`.
fn object_facet_schema(sizes: LengthBounds, ctx: &CanonicalizationContext) -> Schema {
    let non_object = Schema::new(SchemaKind::MultiType(
        JsonTypeSet::all().remove(JsonType::Object),
    ));
    algebra::union(vec![non_object, algebra::object_leaf(sizes)], ctx)
}

/// Draft 4 says `1.0` is not an integer, so its `integer` check cannot fold into value equality.
fn keeps_draft4_integer_guard(set: JsonTypeSet, draft: Draft) -> bool {
    matches!(draft, Draft::Draft4)
        && set.contains(JsonType::Integer)
        && !set.contains(JsonType::Number)
}

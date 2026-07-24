//! Parsing schema documents into structural IR; anything not modeled stays `Raw`.
use std::{collections::BTreeMap, sync::Arc};

use ahash::AHashSet;

use referencing::Draft;
use serde_json::Value;

use crate::{
    canonical::{
        algebra,
        context::CanonicalizationContext,
        ir::{
            canonicalize_value_set, type_set_schema, typed_group, ArrayLeaf, BoundCardinality,
            BoundNumber, BoundRational, CanonicalJson, ContainsFacet, Divisors, IntegerLeaf,
            LengthBounds, NumberLeaf, ObjectLeaf, Schema, SchemaKind, Side, StringLeaf,
        },
        negate, CanonicalizationError,
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
    let mut unique_items = false;
    let mut min_items: Option<BoundCardinality> = None;
    let mut max_items: Option<BoundCardinality> = None;
    let mut items: Option<Schema> = None;
    let mut contains_schema: Option<Schema> = None;
    let mut min_contains: Option<BoundCardinality> = None;
    let mut max_contains: Option<BoundCardinality> = None;
    let mut item_prefix: Option<Vec<Schema>> = None;
    let mut additional_items: Option<&Value> = None;
    let mut required: Vec<Arc<str>> = Vec::new();
    let mut property_names: Option<Schema> = None;
    let mut properties: BTreeMap<Arc<str>, Schema> = BTreeMap::new();
    let mut pattern_properties: BTreeMap<Arc<str>, Schema> = BTreeMap::new();
    let mut forbid_unmatched_keys = false;
    let mut min_properties: Option<BoundCardinality> = None;
    let mut max_properties: Option<BoundCardinality> = None;
    let mut patterns: Vec<Arc<str>> = Vec::new();
    let mut formats: Vec<Arc<str>> = Vec::new();
    let mut multiple_of = Divisors::default();
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
            // When no two branches share a value, "exactly one matches" is "at least one
            // matches", so a pairwise-disjoint `oneOf` is its `anyOf`. Overlapping branches
            // take the exact encoding - each branch beside the complements of the others -
            // when every complement is expressible and the expansion stays small.
            ("oneOf", Value::Array(items)) => {
                let mut branches = Vec::new();
                for branch in items {
                    match parse_schema(branch, ctx, false)? {
                        Some(schema) => branches.push(schema),
                        None => return Ok(None),
                    }
                }
                let overlaps = pairwise_overlaps(&branches, ctx);
                if overlaps.is_empty() {
                    conjuncts.push(algebra::union(branches, ctx));
                } else {
                    match exactly_one_of(branches, overlaps, ctx) {
                        Some(schema) => conjuncts.push(schema),
                        None => return Ok(None),
                    }
                }
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
            ("uniqueItems", Value::Bool(flag)) if ctx.draft().is_known_keyword("uniqueItems") => {
                unique_items = *flag;
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
            // The uniform schema form constrains every element. Draft 4 stays unmodeled: a value set
            // intersected with an item schema can pin a nested number to its integer spelling, which
            // Draft 4's `integer` type would reject.
            ("items", value @ (Value::Object(_) | Value::Bool(_)))
                if ctx.draft().is_known_keyword("items")
                    && !matches!(ctx.draft(), Draft::Draft4) =>
            {
                match parse_schema(value, ctx, false)? {
                    Some(schema) => items = Some(schema),
                    None => return Ok(None),
                }
            }
            // The 2020-12 tuple: each element carries the schema at its index.
            ("prefixItems", Value::Array(schemas))
                if ctx.draft().is_known_keyword("prefixItems") =>
            {
                match parse_prefix(schemas, ctx)? {
                    Some(prefix) => item_prefix = Some(prefix),
                    None => return Ok(None),
                }
            }
            // Array-form `items` is the tuple in 2019-09 and earlier; 2020-12 spells it `prefixItems`.
            // Draft 4 stays unmodeled for the same per-index integer aliasing reason as uniform items.
            ("items", Value::Array(schemas))
                if matches!(
                    ctx.draft(),
                    Draft::Draft6 | Draft::Draft7 | Draft::Draft201909
                ) =>
            {
                match parse_prefix(schemas, ctx)? {
                    Some(prefix) => item_prefix = Some(prefix),
                    None => return Ok(None),
                }
            }
            // `additionalItems` constrains the elements beyond an array-form `items` tuple. It is
            // inert when `items` is a schema or absent, and unknown in 2020-12, so its value is
            // held raw and parsed only once a tuple makes it live.
            ("additionalItems", value @ (Value::Object(_) | Value::Bool(_)))
                if matches!(
                    ctx.draft(),
                    Draft::Draft6 | Draft::Draft7 | Draft::Draft201909
                ) =>
            {
                additional_items = Some(value);
            }
            ("contains", value @ (Value::Object(_) | Value::Bool(_)))
                if ctx.draft().is_known_keyword("contains") =>
            {
                match parse_schema(value, ctx, false)? {
                    Some(schema) => contains_schema = Some(schema),
                    None => return Ok(None),
                }
            }
            ("minContains", Value::Number(number))
                if ctx.draft().is_known_keyword("minContains") =>
            {
                match BoundCardinality::from_number(number) {
                    Some(bound) => min_contains = Some(bound),
                    None => return Ok(None),
                }
            }
            ("maxContains", Value::Number(number))
                if ctx.draft().is_known_keyword("maxContains") =>
            {
                match BoundCardinality::from_number(number) {
                    Some(bound) => max_contains = Some(bound),
                    None => return Ok(None),
                }
            }
            ("required", Value::Array(names))
                if ctx.draft().is_known_keyword("required")
                    && names.iter().all(Value::is_string) =>
            {
                required.extend(names.iter().filter_map(Value::as_str).map(Arc::from));
            }
            ("properties", Value::Object(entries))
                if ctx.draft().is_known_keyword("properties") =>
            {
                for (key, value) in entries {
                    match parse_schema(value, ctx, false)? {
                        Some(schema) => {
                            properties.insert(Arc::from(key.as_str()), schema);
                        }
                        None => return Ok(None),
                    }
                }
            }
            ("patternProperties", Value::Object(entries))
                if ctx.draft().is_known_keyword("patternProperties") =>
            {
                for (pattern, value) in entries {
                    let pattern: Arc<str> = Arc::from(pattern.as_str());
                    if ctx.compile_regex(&pattern).is_none() {
                        return Err(CanonicalizationError::InvalidPattern {
                            pattern: pattern.to_string(),
                        });
                    }
                    match parse_schema(value, ctx, false)? {
                        Some(schema) => {
                            pattern_properties.insert(pattern, schema);
                        }
                        None => return Ok(None),
                    }
                }
            }
            ("propertyNames", value) if ctx.draft().is_known_keyword("propertyNames") => {
                match parse_schema(value, ctx, false)? {
                    Some(schema) => property_names = Some(schema),
                    None => return Ok(None),
                }
            }
            // A schema admitting everything says nothing about a key, so `true`/`{}` leaves no
            // trace. One admitting nothing forbids the unmatched keys, which a key constraint can
            // carry on drafts that have one. Anything in between is a demand conditional on the
            // leaf's own coverage, which the algebra cannot intersect exactly: the document stays
            // raw.
            ("additionalProperties", value @ (Value::Object(_) | Value::Bool(_)))
                if ctx.draft().is_known_keyword("additionalProperties") =>
            {
                match parse_schema(value, ctx, false)? {
                    Some(schema) if matches!(schema.kind(), SchemaKind::True) => {}
                    Some(schema)
                        if matches!(schema.kind(), SchemaKind::False)
                            && ctx.draft().is_known_keyword("propertyNames") =>
                    {
                        forbid_unmatched_keys = true;
                    }
                    Some(_) | None => return Ok(None),
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
            // Only a positive divisor whose spelling denotes an exact rational is modeled; without
            // one the validator's own division is what decides membership.
            ("multipleOf", Value::Number(number)) if ctx.draft().is_known_keyword("multipleOf") => {
                match BoundRational::new(number) {
                    Some(step) => multiple_of = Divisors::one(step),
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
            // The complement of the negated schema, when the IR can spell it; an unmodeled child or
            // an inexpressible complement keeps the whole document raw.
            ("not", value) if ctx.draft().is_known_keyword("not") => {
                match parse_schema(value, ctx, false)? {
                    Some(child) => match negate::negate(&child, ctx) {
                        Some(complement) => conjuncts.push(complement),
                        None => return Ok(None),
                    },
                    None => return Ok(None),
                }
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
    // A tuple's tail is spelled `additionalItems` before 2020-12 and schema-form `items` in it. A
    // schema-form `items` with no tuple constrains every element, so it is the tail of an empty
    // prefix, and `additionalItems` is then inert.
    let (prefix, tail) = match item_prefix {
        Some(prefix)
            if matches!(
                ctx.draft(),
                Draft::Draft6 | Draft::Draft7 | Draft::Draft201909
            ) =>
        {
            let tail = match additional_items {
                Some(value) => match parse_schema(value, ctx, false)? {
                    Some(schema) => Some(schema),
                    None => return Ok(None),
                },
                None => None,
            };
            (prefix, tail)
        }
        Some(prefix) => (prefix, items),
        None => (Vec::new(), items),
    };
    // `minContains`/`maxContains` constrain the `contains` count and say nothing without it.
    let contains: Vec<ContainsFacet> = contains_schema
        .map(|schema| ContainsFacet {
            schema,
            minimum: min_contains,
            maximum: max_contains,
        })
        .into_iter()
        .collect();
    if min_items.is_some()
        || max_items.is_some()
        || unique_items
        || !prefix.is_empty()
        || tail.is_some()
        || !contains.is_empty()
    {
        conjuncts.push(array_facet_schema(
            ArrayLeaf {
                lengths: LengthBounds {
                    minimum: min_items,
                    maximum: max_items,
                },
                unique: unique_items,
                prefix,
                items: tail,
                contains,
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
    // `additionalProperties: false` forbids every key the property map does not name and no
    // pattern matches, which a key constraint spells: the named keys and the patterns' keys,
    // met into any stored constraint.
    // e.g.  {"type": "object", "properties": {"a": {"type": "string"}}, "additionalProperties": false}
    //       =>  {"type": "object", "propertyNames": {"const": "a"}, "properties": {"a": {"type": "string"}}}
    if forbid_unmatched_keys {
        let mut allowed: Vec<Schema> =
            Vec::with_capacity(properties.len() + pattern_properties.len());
        for key in properties.keys() {
            allowed.push(Schema::new(SchemaKind::Const(CanonicalJson::from_value(
                &Value::String(key.to_string()),
            ))));
        }
        for pattern in pattern_properties.keys() {
            allowed.push(algebra::string_leaf(
                StringLeaf {
                    lengths: LengthBounds::default(),
                    patterns: vec![Arc::clone(pattern)],
                    formats: Vec::new(),
                },
                ctx,
            ));
        }
        let allowed = algebra::union(allowed, ctx);
        property_names = Some(match property_names.take() {
            Some(names) => algebra::intersect(names, allowed, ctx),
            None => allowed,
        });
    }
    if min_properties.is_some()
        || max_properties.is_some()
        || !required.is_empty()
        || property_names.is_some()
        || !properties.is_empty()
        || !pattern_properties.is_empty()
    {
        // Every draft marks `required` as unique, so the meta-validated list only needs ordering.
        required.sort();
        conjuncts.push(object_facet_schema(
            ObjectLeaf {
                sizes: LengthBounds {
                    minimum: min_properties,
                    maximum: max_properties,
                },
                required,
                property_names,
                properties,
                pattern_properties,
            },
            ctx,
        ));
    }

    if real_minimum.is_some() || real_maximum.is_some() || !multiple_of.is_empty() {
        let leaf = NumberLeaf {
            minimum: real_minimum,
            maximum: real_maximum,
            multiple_of,
        };
        // The integers the interval admits must be representable: the interval may still meet
        // `integer` through an `allOf`, and there it is the only form left to express.
        let Some(bounds) = algebra::integer_bounds_within(&leaf) else {
            return Ok(None);
        };
        if type_set == Some(JsonTypeSet::from(JsonType::Integer)) {
            conjuncts.push(algebra::integer_leaf(
                IntegerLeaf {
                    bounds,
                    multiple_of: leaf.multiple_of,
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

/// "Exactly one branch matches": some branch matches and no two-branch overlap does, so only the
/// overlaps need complements — a branch overlapping nothing is never negated. `None` when an
/// overlap's complement is inexpressible.
fn exactly_one_of(
    branches: Vec<Schema>,
    overlaps: Vec<Schema>,
    ctx: &CanonicalizationContext,
) -> Option<Schema> {
    let mut result = algebra::union(branches, ctx);
    for overlap in overlaps {
        result = algebra::intersect(result, negate::negate(&overlap, ctx)?, ctx);
    }
    Some(result)
}

/// Every region two branches share: the values repeating across finite-value branches packed as
/// one value set, and the non-`False` pairwise intersections involving structural branches. Empty
/// exactly when the branches are pairwise disjoint, so `oneOf` degrades to `anyOf`.
///
/// Finite-value branches share a value exactly when a member repeats across them, so one hash set
/// replaces their share of the quadratic sweep; only the remaining branches pay a pairwise
/// `intersect`, plus one `intersect` against each finite-value branch.
fn pairwise_overlaps(branches: &[Schema], ctx: &CanonicalizationContext) -> Vec<Schema> {
    let mut seen: AHashSet<&CanonicalJson> = AHashSet::new();
    let mut shared: Vec<CanonicalJson> = Vec::new();
    let mut finite: Vec<&Schema> = Vec::new();
    let mut structural: Vec<&Schema> = Vec::new();
    for branch in branches {
        match branch.kind() {
            SchemaKind::Const(value) => {
                if !seen.insert(value) {
                    shared.push(value.clone());
                }
                finite.push(branch);
            }
            SchemaKind::Enum(values) => {
                for value in values.as_slice() {
                    if !seen.insert(value) {
                        shared.push(value.clone());
                    }
                }
                finite.push(branch);
            }
            SchemaKind::MultiType(_)
            | SchemaKind::TypedGroup { .. }
            | SchemaKind::String(_)
            | SchemaKind::Integer(_)
            | SchemaKind::Number(_)
            | SchemaKind::Array(_)
            | SchemaKind::Object(_)
            | SchemaKind::AnyOf(_)
            | SchemaKind::True
            | SchemaKind::False
            | SchemaKind::Raw(_) => structural.push(branch),
        }
    }
    let mut overlaps = Vec::new();
    if !shared.is_empty() {
        overlaps.push(canonicalize_value_set(shared));
    }
    for (index, left) in structural.iter().enumerate() {
        for right in structural[index + 1..].iter().chain(&finite) {
            let intersection = algebra::intersect((*left).clone(), (*right).clone(), ctx);
            if !matches!(intersection.kind(), SchemaKind::False) {
                overlaps.push(intersection);
            }
        }
    }
    overlaps
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
        branches.push(typed_group(JsonType::Integer, integer_set));
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
    algebra::union(vec![non_number, algebra::number_leaf(leaf, ctx)], ctx)
}

/// A string facet constrains only strings, so `{"minLength": 3}` becomes
/// `anyOf: [<non-string types>, {"type": "string", "minLength": 3}]`.
fn string_facet_schema(leaf: StringLeaf, ctx: &CanonicalizationContext) -> Schema {
    let non_string = Schema::new(SchemaKind::MultiType(
        JsonTypeSet::all().remove(JsonType::String),
    ));
    algebra::union(vec![non_string, algebra::string_leaf(leaf, ctx)], ctx)
}

/// Parse a tuple's per-index schemas; `Ok(None)` when any element is unmodeled, keeping the document raw.
fn parse_prefix(
    schemas: &[Value],
    ctx: &CanonicalizationContext,
) -> Result<Option<Vec<Schema>>, CanonicalizationError> {
    let mut prefix = Vec::with_capacity(schemas.len());
    for schema in schemas {
        match parse_schema(schema, ctx, false)? {
            Some(schema) => prefix.push(schema),
            None => return Ok(None),
        }
    }
    Ok(Some(prefix))
}

/// An array facet constrains only arrays, so `{"minItems": 1}` becomes
/// `anyOf: [<non-array types>, {"type": "array", "minItems": 1}]`.
fn array_facet_schema(leaf: ArrayLeaf, ctx: &CanonicalizationContext) -> Schema {
    let non_array = Schema::new(SchemaKind::MultiType(
        JsonTypeSet::all().remove(JsonType::Array),
    ));
    algebra::union(vec![non_array, algebra::array_leaf(leaf)], ctx)
}

/// An object facet constrains only objects, so `{"minProperties": 1}` becomes
/// `anyOf: [<non-object types>, {"type": "object", "minProperties": 1}]`.
fn object_facet_schema(leaf: ObjectLeaf, ctx: &CanonicalizationContext) -> Schema {
    let non_object = Schema::new(SchemaKind::MultiType(
        JsonTypeSet::all().remove(JsonType::Object),
    ));
    algebra::union(vec![non_object, algebra::object_leaf(leaf, ctx)], ctx)
}

/// Draft 4 says `1.0` is not an integer, so its `integer` check cannot fold into value equality.
fn keeps_draft4_integer_guard(set: JsonTypeSet, draft: Draft) -> bool {
    matches!(draft, Draft::Draft4)
        && set.contains(JsonType::Integer)
        && !set.contains(JsonType::Number)
}

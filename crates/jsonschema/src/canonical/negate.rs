//! Structural complement of a canonical node.
use std::{collections::BTreeMap, sync::Arc};

use serde_json::{Number, Value};

use crate::{
    canonical::{
        algebra,
        context::CanonicalizationContext,
        ir::{
            type_set_schema, ArrayLeaf, AtLeastTwo, BoundCardinality, BoundNumber, CanonicalJson,
            ContainsFacet, Discrete, Divisors, LengthBounds, NumberLeaf, ObjectLeaf, Schema,
            SchemaKind, StringLeaf,
        },
    },
    JsonType, JsonTypeSet,
};

/// The complement schema, or `None` when the IR cannot spell it and the caller keeps the document
/// `Raw`. Negation has no safe default direction, so every arm is exact or declines.
pub(crate) fn negate(schema: &Schema, ctx: &CanonicalizationContext) -> Option<Schema> {
    match schema.kind() {
        SchemaKind::True => Some(Schema::new(SchemaKind::False)),
        SchemaKind::False => Some(Schema::new(SchemaKind::True)),
        SchemaKind::MultiType(set) => negate_type_set(*set),
        SchemaKind::Const(value) => negate_finite_values(std::slice::from_ref(value), ctx),
        SchemaKind::Enum(values) => negate_finite_values(values.as_slice(), ctx),
        SchemaKind::Number(leaf) => negate_number_leaf(leaf.get(), ctx),
        SchemaKind::String(leaf) => negate_string_leaf(leaf.get(), ctx),
        SchemaKind::Array(leaf) => negate_array_leaf(leaf.get(), ctx),
        SchemaKind::Object(leaf) => negate_object_leaf(leaf.get(), ctx),
        SchemaKind::Not(inner) => Some(inner.clone()),
        // De Morgan: the complement of a union is the intersection of the branch complements, so
        // one inexpressible branch declines the whole node.
        SchemaKind::AnyOf(branches) => {
            let mut result = Schema::new(SchemaKind::True);
            for branch in branches.as_slice() {
                result = algebra::intersect(result, negate(branch, ctx)?, ctx);
            }
            Some(result)
        }
        // De Morgan in the other direction restores a union when every branch has an exact
        // structural complement. Otherwise `Not` preserves the opaque conjunction exactly.
        SchemaKind::AllOf(branches) => {
            let mut complements = Vec::with_capacity(branches.as_slice().len());
            for branch in branches.as_slice() {
                let Some(complement) = negate(branch, ctx) else {
                    return Some(Schema::new(SchemaKind::Not(schema.clone())));
                };
                complements.push(complement);
            }
            debug_assert!(
                complements.len() >= 2,
                "an AllOf has at least two branches before applying De Morgan"
            );
            Some(algebra::union(complements, ctx))
        }
        SchemaKind::OneOf(_) | SchemaKind::Reference(_) => {
            Some(Schema::new(SchemaKind::Not(schema.clone())))
        }
        SchemaKind::TypedGroup { .. } | SchemaKind::Integer(_) | SchemaKind::Raw(_) => None,
    }
}

/// Complement of a finite value set: the untouched types stay whole, an unpaired boolean leaves the
/// other one, the numeric members carve rays and gaps out of the number line, the string members
/// become exclusions on the strings, and an empty container leaves the sizes above it.
/// ```text
/// e.g.  {"not": {"const": null}}  =>  {"type": ["boolean", "number", "string", "array", "object"]}
/// e.g.  {"not": {"const": []}}
///       =>  anyOf: [<non-array types>, {"type": "array", "minItems": 1}]
/// e.g.  {"not": {"const": [1]}}  =>  unchanged: array inequality is inexpressible
/// ```
fn negate_finite_values(values: &[CanonicalJson], ctx: &CanonicalizationContext) -> Option<Schema> {
    let mut remaining = JsonTypeSet::all();
    let mut booleans = Vec::new();
    let mut numbers: Vec<Number> = Vec::new();
    let mut strings: Vec<Arc<str>> = Vec::new();
    let mut empty_array = false;
    let mut empty_object = false;
    for value in values {
        match value.as_value() {
            Value::Null => remaining = remaining.remove(JsonType::Null),
            Value::Bool(member) => {
                remaining = remaining.remove(JsonType::Boolean);
                booleans.push(*member);
            }
            Value::Number(number) => {
                remaining = remaining.remove(JsonType::Number).remove(JsonType::Integer);
                numbers.push(number.clone());
            }
            Value::String(text) => {
                remaining = remaining.remove(JsonType::String);
                strings.push(Arc::from(text.as_str()));
            }
            // An empty container is the only one of its size, so the sizes above it are the rest of
            // its type. Any other one needs a value to differ somewhere, which no facet spells.
            Value::Array(items) if items.is_empty() => {
                remaining = remaining.remove(JsonType::Array);
                empty_array = true;
            }
            Value::Object(entries) if entries.is_empty() => {
                remaining = remaining.remove(JsonType::Object);
                empty_object = true;
            }
            Value::Array(_) | Value::Object(_) => return None,
        }
    }
    let mut branches = vec![type_set_schema(remaining)];
    if empty_array {
        branches.push(algebra::array_leaf(
            ArrayLeaf {
                lengths: above_empty(),
                unique: false,
                prefix: Vec::new(),
                items: None,
                contains: Vec::new(),
            },
            ctx,
        ));
    }
    if empty_object {
        branches.push(object_branch(
            above_empty(),
            Vec::new(),
            BTreeMap::new(),
            ctx,
        ));
    }
    if let [member] = booleans.as_slice() {
        branches.push(Schema::new(SchemaKind::Const(CanonicalJson::from_value(
            &Value::Bool(!member),
        ))));
    }
    branches.extend(number_gaps(&numbers, ctx));
    if !strings.is_empty() {
        strings.sort();
        strings.dedup();
        branches.push(algebra::string_leaf(
            StringLeaf {
                lengths: LengthBounds::default(),
                patterns: Vec::new(),
                formats: Vec::new(),
                excluded_formats: Vec::new(),
                content_media_types: Vec::new(),
                content_encodings: Vec::new(),
                excluded: strings,
            },
            ctx,
        ));
    }
    Some(algebra::union(branches, ctx))
}

/// The number-line complement of a finite set of numbers: the outer rays and the open gaps
/// between neighbours. Empty input adds nothing - the whole `number` type then stays remaining.
fn number_gaps(numbers: &[Number], ctx: &CanonicalizationContext) -> Vec<Schema> {
    if numbers.is_empty() {
        return Vec::new();
    }
    let mut ends: Vec<BoundNumber> = numbers
        .iter()
        .map(|number| BoundNumber::new(number, false))
        .collect();
    ends.sort();
    let mut branches = Vec::with_capacity(ends.len() + 1);
    let mut lower: Option<BoundNumber> = None;
    for end in ends {
        branches.push(number_window(lower.take(), Some(end.clone()), ctx));
        lower = Some(end);
    }
    branches.push(number_window(lower, None, ctx));
    branches
}

fn number_window(
    minimum: Option<BoundNumber>,
    maximum: Option<BoundNumber>,
    ctx: &CanonicalizationContext,
) -> Schema {
    algebra::number_leaf(
        NumberLeaf {
            minimum,
            maximum,
            multiple_of: Divisors::default(),
        },
        ctx,
    )
}

/// Complement of a number window: the values of every other type plus the outer rays, each
/// endpoint's inclusivity flipped.
/// ```text
/// e.g.  {"not": {"type": "number", "minimum": 5}}
///       =>  anyOf: [<non-number types>, {"type": "number", "exclusiveMaximum": 5}]
/// ```
fn negate_number_leaf(leaf: &NumberLeaf, ctx: &CanonicalizationContext) -> Option<Schema> {
    if !leaf.multiple_of.is_empty() {
        return None;
    }
    let mut branches = vec![type_set_schema(
        JsonTypeSet::all()
            .remove(JsonType::Number)
            .remove(JsonType::Integer),
    )];
    if let Some(minimum) = &leaf.minimum {
        branches.push(number_window(None, Some(flipped(minimum)), ctx));
    }
    if let Some(maximum) = &leaf.maximum {
        branches.push(number_window(Some(flipped(maximum)), None, ctx));
    }
    Some(algebra::union(branches, ctx))
}

/// The sizes a container holding something can take.
fn above_empty() -> LengthBounds {
    LengthBounds {
        minimum: Some(BoundCardinality::from(1)),
        maximum: None,
    }
}

/// A value set holding exactly these strings.
fn finite_strings(values: &[Arc<str>]) -> Schema {
    let members: Vec<CanonicalJson> = values
        .iter()
        .map(|value| CanonicalJson::from_value(&Value::String(value.to_string())))
        .collect();
    match AtLeastTwo::new(members) {
        Ok(set) => Schema::new(SchemaKind::Enum(set)),
        Err(mut single) => Schema::new(SchemaKind::Const(
            single.pop().expect("a non-empty exclusion list"),
        )),
    }
}

/// The same limit admitting exactly the values the original end rejects.
fn flipped(bound: &BoundNumber) -> BoundNumber {
    BoundNumber::new(&bound.to_number(), !bound.is_inclusive())
}

/// Complements of a count window: the ray below the floor and the ray above the ceiling. A floor
/// of zero excludes nothing below it; a ceiling with no successor in this build declines.
pub(crate) fn length_windows(lengths: &LengthBounds) -> Option<Vec<LengthBounds>> {
    let mut windows = Vec::new();
    if let Some(below) = lengths
        .minimum
        .as_ref()
        .and_then(|minimum| minimum.clone().checked_decrement())
    {
        windows.push(LengthBounds {
            minimum: None,
            maximum: Some(below),
        });
    }
    if let Some(maximum) = &lengths.maximum {
        let above = maximum.clone().checked_increment()?;
        windows.push(LengthBounds {
            minimum: Some(above),
            maximum: None,
        });
    }
    Some(windows)
}

/// ```text
/// e.g.  {"not": {"type": "string", "minLength": 3}}
///       =>  anyOf: [<non-string types>, {"type": "string", "maxLength": 2}]
/// ```
fn negate_string_leaf(leaf: &StringLeaf, ctx: &CanonicalizationContext) -> Option<Schema> {
    if !leaf.excluded.is_empty() {
        // The dual of the arm above: a leaf that only excludes values complements to those values.
        let mut branches = vec![type_set_schema(JsonTypeSet::all().remove(JsonType::String))];
        branches.push(finite_strings(&leaf.excluded));
        let positive = StringLeaf {
            excluded: Vec::new(),
            ..leaf.clone()
        };
        branches.push(negate_string_leaf(&positive, ctx)?);
        return Some(algebra::union(branches, ctx));
    }
    if !leaf.patterns.is_empty()
        || !leaf.content_media_types.is_empty()
        || !leaf.content_encodings.is_empty()
    {
        return None;
    }
    let windows = length_windows(&leaf.lengths)?;
    let mut branches = vec![type_set_schema(JsonTypeSet::all().remove(JsonType::String))];
    branches.extend(windows.into_iter().map(|lengths| {
        algebra::string_leaf(
            StringLeaf {
                lengths,
                ..StringLeaf::default()
            },
            ctx,
        )
    }));
    // A string fails a run of formats as soon as it fails one of them, so each gets its own branch
    // - and a branch barring one format says nothing about the length or the others.
    branches.extend(leaf.formats.iter().map(|format| {
        algebra::string_leaf(
            StringLeaf {
                excluded_formats: vec![Arc::clone(format)],
                ..StringLeaf::default()
            },
            ctx,
        )
    }));
    branches.extend(leaf.excluded_formats.iter().map(|format| {
        algebra::string_leaf(
            StringLeaf {
                formats: vec![Arc::clone(format)],
                ..StringLeaf::default()
            },
            ctx,
        )
    }));
    Some(algebra::union(branches, ctx))
}

/// An element schema fails on an array exactly when one element violates it, which is a `contains`
/// demand for its complement. A demand for one match fails exactly when every element violates it,
/// which is the same trade the other way round.
/// ```text
/// e.g.  {"not": {"type": "array", "maxItems": 2}}
///       =>  anyOf: [<non-array types>, {"type": "array", "minItems": 3}]
/// e.g.  {"not": {"type": "array", "items": {"type": "string"}}}
///       =>  anyOf: [<non-array types>,
///                   {"type": "array", "contains": {"type": <every type but string>}}]
/// e.g.  {"not": {"type": "array", "contains": {"type": "string"}}}
///       =>  anyOf: [<non-array types>,
///                   {"type": "array", "items": {"type": <every type but string>}}]
/// ```
fn negate_array_leaf(leaf: &ArrayLeaf, ctx: &CanonicalizationContext) -> Option<Schema> {
    if leaf.unique || !leaf.prefix.is_empty() {
        return None;
    }
    let windows = length_windows(&leaf.lengths)?;
    let mut branches = vec![type_set_schema(JsonTypeSet::all().remove(JsonType::Array))];
    if let Some(items) = &leaf.items {
        // Draft 4 has no `contains`, so a validator there ignores the branch and admits every array.
        if !ctx.draft().is_known_keyword("contains") {
            return None;
        }
        branches.push(algebra::array_leaf(
            ArrayLeaf {
                lengths: LengthBounds::default(),
                unique: false,
                prefix: Vec::new(),
                items: None,
                contains: vec![ContainsFacet {
                    schema: negate(items, ctx)?,
                    minimum: None,
                    maximum: None,
                }],
            },
            ctx,
        ));
    }
    for facet in &leaf.contains {
        // Missing a window on the count means landing anywhere else in it, and an element schema
        // holding for every element can only say "nowhere".
        if facet.maximum.is_some() || facet.effective_minimum() != BoundCardinality::from(1) {
            return None;
        }
        branches.push(algebra::array_leaf(
            ArrayLeaf {
                lengths: LengthBounds::default(),
                unique: false,
                prefix: Vec::new(),
                items: Some(negate(&facet.schema, ctx)?),
                contains: Vec::new(),
            },
            ctx,
        ));
    }
    branches.extend(windows.into_iter().map(|lengths| {
        algebra::array_leaf(
            ArrayLeaf {
                lengths,
                unique: false,
                prefix: Vec::new(),
                items: None,
                contains: Vec::new(),
            },
            ctx,
        )
    }));
    Some(algebra::union(branches, ctx))
}

/// The leaf is a conjunction of facets over objects, so its complement is the union of the
/// per-facet complements beside the non-object types: a size window flips into its outer rays, a
/// required key into its absence, and a property schema into the key held with a violating value.
/// ```text
/// e.g.  {"not": {"type": "object", "required": ["a"], "minProperties": 2}}
///       =>  anyOf: [<non-object types>,
///                   {"type": "object", "properties": {"a": false}},
///                   {"type": "object", "maxProperties": 1}]
/// ```
fn negate_object_leaf(leaf: &ObjectLeaf, ctx: &CanonicalizationContext) -> Option<Schema> {
    if leaf.property_names.is_some()
        || !leaf.pattern_properties.is_empty()
        || leaf.additional.is_some()
    {
        return None;
    }
    let mut branches = vec![type_set_schema(JsonTypeSet::all().remove(JsonType::Object))];
    for sizes in length_windows(&leaf.sizes)? {
        branches.push(object_branch(sizes, Vec::new(), BTreeMap::new(), ctx));
    }
    for key in &leaf.required {
        let absent = BTreeMap::from([(key.clone(), Schema::new(SchemaKind::False))]);
        branches.push(object_branch(
            LengthBounds::default(),
            Vec::new(),
            absent,
            ctx,
        ));
    }
    for (key, schema) in &leaf.properties {
        let violating = negate(schema, ctx)?;
        let held = BTreeMap::from([(key.clone(), violating)]);
        branches.push(object_branch(
            LengthBounds::default(),
            vec![key.clone()],
            held,
            ctx,
        ));
    }
    Some(algebra::union(branches, ctx))
}

fn object_branch(
    sizes: LengthBounds,
    required: Vec<std::sync::Arc<str>>,
    properties: BTreeMap<std::sync::Arc<str>, Schema>,
    ctx: &CanonicalizationContext,
) -> Schema {
    algebra::object_leaf(
        ObjectLeaf {
            sizes,
            required,
            property_names: None,
            properties,
            pattern_properties: BTreeMap::new(),
            additional: None,
        },
        ctx,
    )
}

/// Complement of a type set over the value space. `None` when the set admits `integer` but not
/// `number`: the complement then admits non-integer numbers, which no type set can name.
/// ```text
/// e.g.  {"not": {"type": "string"}}  =>  {"type": ["null", "boolean", "number", "array", "object"]}
/// ```
fn negate_type_set(set: JsonTypeSet) -> Option<Schema> {
    if set.contains(JsonType::Integer) && !set.contains(JsonType::Number) {
        return None;
    }
    let mut complement = JsonTypeSet::empty();
    for ty in [
        JsonType::Null,
        JsonType::Boolean,
        JsonType::String,
        JsonType::Array,
        JsonType::Object,
    ] {
        if !set.contains(ty) {
            complement = complement.insert(ty);
        }
    }
    // A set carrying `number` admits every number, so its complement admits none; a set carrying
    // neither numeric type admits no number, so its complement admits all of them.
    if !set.contains(JsonType::Number) {
        complement = complement.insert(JsonType::Number);
    }
    if complement.is_empty() {
        return Some(Schema::new(SchemaKind::False));
    }
    // The shared constructor, so a complement spelling a lone `null` or `boolean` lands on the same
    // canonical node as the direct spelling.
    Some(type_set_schema(complement))
}

#[cfg(test)]
mod tests {
    use referencing::Draft;
    use serde_json::{json, Value};

    use super::*;
    use crate::options::PatternEngineOptions;

    fn context() -> CanonicalizationContext {
        CanonicalizationContext::new(Draft::Draft202012, PatternEngineOptions::default(), false)
    }

    const TYPES: [JsonType; 7] = [
        JsonType::Null,
        JsonType::Boolean,
        JsonType::Integer,
        JsonType::Number,
        JsonType::String,
        JsonType::Array,
        JsonType::Object,
    ];

    // One value per equivalence class of the type vocabulary; `1` and `1.5` are distinct classes
    // because an integer satisfies both `integer` and `number` while a fraction satisfies only
    // `number`.
    fn representatives() -> [Value; 7] {
        [
            json!(null),
            json!(true),
            json!(1),
            json!(1.5),
            json!("x"),
            json!([]),
            json!({}),
        ]
    }

    fn admits(set: JsonTypeSet, value: &Value) -> bool {
        match value {
            Value::Null => set.contains(JsonType::Null),
            Value::Bool(_) => set.contains(JsonType::Boolean),
            Value::Number(number) if number.is_i64() => {
                set.contains(JsonType::Integer) || set.contains(JsonType::Number)
            }
            Value::Number(_) => set.contains(JsonType::Number),
            Value::String(_) => set.contains(JsonType::String),
            Value::Array(_) => set.contains(JsonType::Array),
            Value::Object(_) => set.contains(JsonType::Object),
        }
    }

    // Membership for the canonical shapes a complement can take: a type set, its boolean-schema
    // collapses, and the value-set spellings of a lone `null` or `boolean` type.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn complement_admits(schema: &Schema, value: &Value) -> bool {
        match schema.kind() {
            SchemaKind::True => true,
            SchemaKind::False => false,
            SchemaKind::MultiType(set) => admits(*set, value),
            SchemaKind::Const(constant) => {
                assert_eq!(constant.as_value(), &Value::Null);
                value.is_null()
            }
            SchemaKind::Enum(values) => {
                let members: Vec<&Value> = values
                    .as_slice()
                    .iter()
                    .map(CanonicalJson::as_value)
                    .collect();
                assert_eq!(members, [&Value::Bool(false), &Value::Bool(true)]);
                value.is_boolean()
            }
            other => {
                panic!("scaffold complement of a type set is a type-set shape, got {other:?}")
            }
        }
    }

    // The scaffold's domain is finite, so the complement-membership law is proven exhaustively: for
    // every one of the 128 type sets, either negate declines (integer without number) or its result
    // admits a value exactly when the original does not.
    #[test]
    fn type_set_complement_partitions_the_value_space() {
        let ctx = context();
        for mask in 0u8..128 {
            let mut set = JsonTypeSet::empty();
            for ty in TYPES {
                if mask & ty as u8 != 0 {
                    set = set.insert(ty);
                }
            }
            let schema = Schema::new(SchemaKind::MultiType(set));
            let complement = negate(&schema, &ctx);
            if set.contains(JsonType::Integer) && !set.contains(JsonType::Number) {
                assert!(
                    complement.is_none(),
                    "integer-only set {set:?} must decline"
                );
                continue;
            }
            let complement = complement.expect("expressible complement");
            for value in &representatives() {
                assert_ne!(
                    admits(set, value),
                    complement_admits(&complement, value),
                    "membership not partitioned for set {set:?} on {value}"
                );
            }
        }
    }

    #[test]
    fn boolean_schemas_negate_to_each_other() {
        let ctx = context();
        assert!(matches!(
            negate(&Schema::new(SchemaKind::True), &ctx).map(|s| s.kind().clone()),
            Some(SchemaKind::False)
        ));
        assert!(matches!(
            negate(&Schema::new(SchemaKind::False), &ctx).map(|s| s.kind().clone()),
            Some(SchemaKind::True)
        ));
    }
}

//! Structural complement of a canonical node.
use std::collections::BTreeMap;

use serde_json::{Number, Value};

use crate::{
    canonical::{
        algebra,
        context::CanonicalizationContext,
        ir::{
            type_set_schema, ArrayLeaf, BoundNumber, CanonicalJson, Discrete, Divisors,
            LengthBounds, NumberLeaf, ObjectLeaf, Schema, SchemaKind, StringLeaf,
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
        SchemaKind::TypedGroup { .. }
        | SchemaKind::Integer(_)
        | SchemaKind::AnyOf(_)
        | SchemaKind::Raw(_) => None,
    }
}

/// Complement of a finite value set, expressible when every member is a null, boolean, or number:
/// the untouched types stay whole, an unpaired boolean leaves the other one, and the numeric
/// members carve rays and gaps out of the number line.
/// ```text
/// e.g.  {"not": {"const": null}}  =>  {"type": ["boolean", "number", "string", "array", "object"]}
/// e.g.  {"not": {"const": "a"}}  =>  unchanged: string inequality is inexpressible
/// ```
fn negate_finite_values(values: &[CanonicalJson], ctx: &CanonicalizationContext) -> Option<Schema> {
    let mut remaining = JsonTypeSet::all();
    let mut booleans = Vec::new();
    let mut numbers: Vec<Number> = Vec::new();
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
            Value::String(_) | Value::Array(_) | Value::Object(_) => return None,
        }
    }
    let mut branches = vec![type_set_schema(remaining)];
    if let [member] = booleans.as_slice() {
        branches.push(Schema::new(SchemaKind::Const(CanonicalJson::from_value(
            &Value::Bool(!member),
        ))));
    }
    branches.extend(number_gaps(&numbers, ctx));
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

/// The same limit admitting exactly the values the original end rejects.
fn flipped(bound: &BoundNumber) -> BoundNumber {
    BoundNumber::new(&bound.to_number(), !bound.is_inclusive())
}

/// Complements of a count window: the ray below the floor and the ray above the ceiling. A floor
/// of zero excludes nothing below it; a ceiling with no successor in this build declines.
fn length_windows(lengths: &LengthBounds) -> Option<Vec<LengthBounds>> {
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
    if !leaf.patterns.is_empty() || !leaf.formats.is_empty() {
        return None;
    }
    let windows = length_windows(&leaf.lengths)?;
    let mut branches = vec![type_set_schema(JsonTypeSet::all().remove(JsonType::String))];
    branches.extend(windows.into_iter().map(|lengths| {
        algebra::string_leaf(
            StringLeaf {
                lengths,
                patterns: Vec::new(),
                formats: Vec::new(),
            },
            ctx,
        )
    }));
    Some(algebra::union(branches, ctx))
}

/// ```text
/// e.g.  {"not": {"type": "array", "maxItems": 2}}
///       =>  anyOf: [<non-array types>, {"type": "array", "minItems": 3}]
/// ```
fn negate_array_leaf(leaf: &ArrayLeaf, ctx: &CanonicalizationContext) -> Option<Schema> {
    if leaf.unique || !leaf.prefix.is_empty() || leaf.items.is_some() || !leaf.contains.is_empty() {
        return None;
    }
    let windows = length_windows(&leaf.lengths)?;
    let mut branches = vec![type_set_schema(JsonTypeSet::all().remove(JsonType::Array))];
    branches.extend(windows.into_iter().map(|lengths| {
        algebra::array_leaf(ArrayLeaf {
            lengths,
            unique: false,
            prefix: Vec::new(),
            items: None,
            contains: Vec::new(),
        })
    }));
    Some(algebra::union(branches, ctx))
}

/// ```text
/// e.g.  {"not": {"type": "object", "minProperties": 2}}
///       =>  anyOf: [<non-object types>, {"type": "object", "maxProperties": 1}]
/// ```
fn negate_object_leaf(leaf: &ObjectLeaf, ctx: &CanonicalizationContext) -> Option<Schema> {
    if !leaf.required.is_empty()
        || leaf.property_names.is_some()
        || !leaf.properties.is_empty()
        || !leaf.pattern_properties.is_empty()
    {
        return None;
    }
    let windows = length_windows(&leaf.sizes)?;
    let mut branches = vec![type_set_schema(JsonTypeSet::all().remove(JsonType::Object))];
    branches.extend(windows.into_iter().map(|sizes| {
        algebra::object_leaf(
            ObjectLeaf {
                sizes,
                required: Vec::new(),
                property_names: None,
                properties: BTreeMap::new(),
                pattern_properties: BTreeMap::new(),
            },
            ctx,
        )
    }));
    Some(algebra::union(branches, ctx))
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

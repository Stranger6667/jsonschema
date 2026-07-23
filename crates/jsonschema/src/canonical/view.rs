use std::collections::BTreeMap;

use serde_json::{Number, Value};

use crate::{
    canonical::{
        ir::{
            ArrayLeaf, BoundCardinality, BoundInteger, BoundNumber, BoundRational, CanonicalJson,
            Divisors, IntegerLeaf, NumberLeaf, ObjectLeaf, SchemaKind, StringLeaf,
        },
        CanonicalSchema,
    },
    JsonType, JsonTypeSet,
};

pub use crate::canonical::ir::CanonicalKind;

impl CanonicalKind {
    /// Stable `snake_case` label of this kind (e.g. `"multi_type"`, `"raw"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// A canonical node: one arm per IR variant. Constructs beyond value sets surface as `Raw`.
///
/// Exhaustive on purpose: a new variant must break every consumer that maps views, the bindings
/// included, rather than reaching a runtime fallback.
#[derive(Debug, Clone, PartialEq)]
pub enum CanonicalView {
    /// A value matches iff its JSON type is in the set.
    MultiType(JsonTypeSet),
    /// A value matches iff its JSON type is `ty` *and* it satisfies `body`; other types do not match.
    TypedGroup(TypedGroupView),
    /// A string value within a length window.
    String(StringView),
    /// An integer value within a range.
    Integer(IntegerView),
    /// A number value within a real interval.
    Number(NumberView),
    /// An array value whose length is within a window.
    Array(ArrayView),
    /// An object value whose property count is within a window.
    Object(ObjectView),
    Const(Value),
    Enum(Vec<Value>),
    /// A value matches iff at least one branch matches.
    AnyOf(Vec<CanonicalSchema>),
    True,
    False,
    Raw(Value),
}

/// Payload of [`CanonicalView::TypedGroup`]: JSON type `ty` and a `body` schema constraining its values.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedGroupView {
    pub ty: JsonType,
    pub body: CanonicalSchema,
}

/// Payload of [`CanonicalView::String`]: the `minLength`/`maxLength` bounds, patterns and formats on
/// a string value.
#[derive(Debug, Clone, PartialEq)]
pub struct StringView {
    pub min_length: Option<Number>,
    pub max_length: Option<Number>,
    pub patterns: Vec<String>,
    pub formats: Vec<String>,
}

/// Payload of [`CanonicalView::Number`]: the interval bounds on a number value, each with whether
/// the bound itself is admitted, and the divisor every admitted value is a multiple of.
#[derive(Debug, Clone, PartialEq)]
pub struct NumberView {
    pub minimum: Option<Number>,
    pub exclusive_minimum: bool,
    pub maximum: Option<Number>,
    pub exclusive_maximum: bool,
    pub multiple_of: Vec<Number>,
}

/// Payload of [`CanonicalView::Array`]: the `minItems`/`maxItems` bounds and uniqueness of an array value.
// The fields carry the keywords they came from, whose names share the suffix.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayView {
    pub min_items: Option<Number>,
    pub max_items: Option<Number>,
    pub unique_items: bool,
}

/// Payload of [`CanonicalView::Object`]: the constraints on an object value.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectView {
    pub min_properties: Option<Number>,
    pub max_properties: Option<Number>,
    pub required: Vec<String>,
    /// The schema every key satisfies, narrowed to the string domain.
    pub property_names: Option<CanonicalSchema>,
    /// The schema each named key satisfies when the object carries it.
    pub properties: BTreeMap<String, CanonicalSchema>,
}

/// Payload of [`CanonicalView::Integer`]: the interval bounds and divisor on an integer value.
#[derive(Debug, Clone, PartialEq)]
pub struct IntegerView {
    pub minimum: Option<Number>,
    pub maximum: Option<Number>,
    pub multiple_of: Vec<Number>,
}

impl CanonicalSchema {
    /// This node's structural view.
    #[must_use]
    pub fn view(&self) -> CanonicalView {
        match self.schema_kind() {
            SchemaKind::MultiType(set) => CanonicalView::MultiType(*set),
            SchemaKind::TypedGroup { ty, body } => CanonicalView::TypedGroup(TypedGroupView {
                ty: *ty,
                body: self.wrap_child(body),
            }),
            SchemaKind::String(leaf) => CanonicalView::String(string_view(leaf.get())),
            SchemaKind::Integer(bounds) => CanonicalView::Integer(integer_view(bounds.get())),
            SchemaKind::Number(leaf) => CanonicalView::Number(number_view(leaf.get())),
            SchemaKind::Array(leaf) => CanonicalView::Array(array_view(leaf.get())),
            SchemaKind::Object(leaf) => CanonicalView::Object(object_view(
                leaf.get(),
                leaf.get()
                    .property_names
                    .as_ref()
                    .map(|names| self.wrap_child(names)),
                leaf.get()
                    .properties
                    .iter()
                    .map(|(key, schema)| (key.to_string(), self.wrap_child(schema)))
                    .collect(),
            )),
            SchemaKind::Const(value) => CanonicalView::Const(value.to_value()),
            SchemaKind::Enum(values) => CanonicalView::Enum(
                values
                    .as_slice()
                    .iter()
                    .map(CanonicalJson::to_value)
                    .collect(),
            ),
            SchemaKind::AnyOf(branches) => CanonicalView::AnyOf(
                branches
                    .as_slice()
                    .iter()
                    .map(|branch| self.wrap_child(branch))
                    .collect(),
            ),
            SchemaKind::True => CanonicalView::True,
            SchemaKind::False => CanonicalView::False,
            SchemaKind::Raw(_) => CanonicalView::Raw(self.to_json_schema()),
        }
    }

    /// This node's structural kind.
    #[must_use]
    pub fn kind(&self) -> CanonicalKind {
        self.schema_kind().into()
    }
}

fn number_view(leaf: &NumberLeaf) -> NumberView {
    NumberView {
        minimum: leaf.minimum.as_ref().map(BoundNumber::to_number),
        exclusive_minimum: leaf.minimum.as_ref().is_some_and(|b| !b.is_inclusive()),
        maximum: leaf.maximum.as_ref().map(BoundNumber::to_number),
        exclusive_maximum: leaf.maximum.as_ref().is_some_and(|b| !b.is_inclusive()),
        multiple_of: divisor_numbers(&leaf.multiple_of),
    }
}

fn integer_view(leaf: &IntegerLeaf) -> IntegerView {
    IntegerView {
        minimum: leaf.bounds.minimum.as_ref().map(BoundInteger::to_number),
        maximum: leaf.bounds.maximum.as_ref().map(BoundInteger::to_number),
        multiple_of: divisor_numbers(&leaf.multiple_of),
    }
}

fn array_view(leaf: &ArrayLeaf) -> ArrayView {
    ArrayView {
        min_items: leaf
            .lengths
            .minimum
            .as_ref()
            .map(BoundCardinality::to_number),
        max_items: leaf
            .lengths
            .maximum
            .as_ref()
            .map(BoundCardinality::to_number),
        unique_items: leaf.unique,
    }
}

// The nested children need the schema-level wrapping only the caller can do, so they arrive already
// wrapped instead of being read off the leaf.
fn object_view(
    leaf: &ObjectLeaf,
    property_names: Option<CanonicalSchema>,
    properties: BTreeMap<String, CanonicalSchema>,
) -> ObjectView {
    ObjectView {
        min_properties: leaf.sizes.minimum.as_ref().map(BoundCardinality::to_number),
        max_properties: leaf.sizes.maximum.as_ref().map(BoundCardinality::to_number),
        required: leaf.required.iter().map(ToString::to_string).collect(),
        property_names,
        properties,
    }
}

fn string_view(leaf: &StringLeaf) -> StringView {
    StringView {
        min_length: leaf
            .lengths
            .minimum
            .as_ref()
            .map(BoundCardinality::to_number),
        max_length: leaf
            .lengths
            .maximum
            .as_ref()
            .map(BoundCardinality::to_number),
        patterns: leaf.patterns.iter().map(ToString::to_string).collect(),
        formats: leaf.formats.iter().map(ToString::to_string).collect(),
    }
}

fn divisor_numbers(divisors: &Divisors) -> Vec<Number> {
    divisors
        .as_slice()
        .iter()
        .map(BoundRational::to_number)
        .collect()
}

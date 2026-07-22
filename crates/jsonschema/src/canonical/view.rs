use serde_json::{Number, Value};

use crate::{
    canonical::{
        ir::{
            BoundCardinality, BoundInteger, CanonicalJson, IntegerBounds, SchemaKind, StringLeaf,
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

/// A canonical node: one arm per IR variant.
// TODO(canonical): not modeled yet - constructs beyond value sets surface as `Raw`; new variants
// arrive here as they become modeled.
#[non_exhaustive]
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

/// Payload of [`CanonicalView::String`]: the `minLength`/`maxLength` bounds and patterns on a string value.
#[derive(Debug, Clone, PartialEq)]
pub struct StringView {
    pub min_length: Option<Number>,
    pub max_length: Option<Number>,
    pub patterns: Vec<String>,
}

/// Payload of [`CanonicalView::Integer`]: the interval bounds on an integer value.
#[derive(Debug, Clone, PartialEq)]
pub struct IntegerView {
    pub minimum: Option<Number>,
    pub maximum: Option<Number>,
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
            SchemaKind::String(leaf) => CanonicalView::String(string_view(leaf)),
            SchemaKind::Integer(bounds) => CanonicalView::Integer(integer_view(bounds)),
            SchemaKind::Const(value) => CanonicalView::Const(value.to_value()),
            SchemaKind::Enum(values) => {
                CanonicalView::Enum(values.iter().map(CanonicalJson::to_value).collect())
            }
            SchemaKind::AnyOf(branches) => CanonicalView::AnyOf(
                branches
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

fn integer_view(bounds: &IntegerBounds) -> IntegerView {
    IntegerView {
        minimum: bounds.minimum.as_ref().map(BoundInteger::to_number),
        maximum: bounds.maximum.as_ref().map(BoundInteger::to_number),
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
    }
}

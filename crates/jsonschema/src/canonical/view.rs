use serde_json::Value;

use crate::{
    canonical::{
        ir::{CanonicalJson, SchemaKind},
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
#[derive(Debug, Clone)]
pub enum CanonicalView {
    /// A value matches iff its JSON type is in the set.
    MultiType(JsonTypeSet),
    /// A value matches iff its JSON type is `ty` *and* it satisfies `body`; other types do not match.
    TypedGroup(TypedGroupView),
    Const(Value),
    Enum(Vec<Value>),
    True,
    False,
    Raw(Value),
}

/// Payload of [`CanonicalView::TypedGroup`]: JSON type `ty` and a `body` schema constraining its values.
#[derive(Debug, Clone)]
pub struct TypedGroupView {
    pub ty: JsonType,
    pub body: CanonicalSchema,
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
            SchemaKind::Const(value) => CanonicalView::Const(value.to_value()),
            SchemaKind::Enum(values) => {
                CanonicalView::Enum(values.iter().map(CanonicalJson::to_value).collect())
            }
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

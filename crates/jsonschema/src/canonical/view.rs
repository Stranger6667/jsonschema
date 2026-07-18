use serde_json::Value;

use crate::canonical::{ir::SchemaKind, CanonicalSchema};

pub use crate::canonical::ir::CanonicalKind;

/// A canonical node: one arm per IR variant.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum CanonicalView {
    Raw(Value),
}

impl CanonicalSchema {
    /// This node's structural view.
    #[must_use]
    pub fn view(&self) -> CanonicalView {
        match self.schema_kind() {
            SchemaKind::Raw(_) => CanonicalView::Raw(self.to_json_schema()),
        }
    }

    /// This node's structural kind.
    #[must_use]
    pub fn kind(&self) -> CanonicalKind {
        self.schema_kind().into()
    }
}

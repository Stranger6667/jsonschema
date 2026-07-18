//! IR -> JSON Schema emit.

use serde_json::Value;

use crate::canonical::ir::{Schema, SchemaKind};

pub(crate) fn to_json_schema(root: &Schema) -> Value {
    match root.kind() {
        SchemaKind::Raw(value) => value.get().clone(),
    }
}

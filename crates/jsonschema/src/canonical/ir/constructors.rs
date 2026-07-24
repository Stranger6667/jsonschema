//! Constructors for canonical shapes built from more than one module.
//!
//! Convergence is an IR-equality contract: two spellings of one value set must land on the same
//! node. A `SchemaKind` shape constructed in two or more modules gets its constructor here, and
//! parse, algebra, and negate all call it.
use serde_json::Value;

use super::{AtLeastTwo, CanonicalJson, Schema, SchemaKind};
use crate::{JsonType, JsonTypeSet};

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

/// Type-guard a body, collapsing an empty one: a group with a `False` body admits nothing.
pub(crate) fn typed_group(ty: JsonType, body: Schema) -> Schema {
    if matches!(body.kind(), SchemaKind::False) {
        Schema::new(SchemaKind::False)
    } else {
        Schema::new(SchemaKind::TypedGroup { ty, body })
    }
}

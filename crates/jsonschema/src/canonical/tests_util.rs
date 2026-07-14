#![allow(clippy::needless_pass_by_value)]

use serde_json::Value;

use crate::{
    canonical::{
        intern::shared,
        ir::{BoundCardinality, CanonicalJson, Schema},
        options, CanonicalSchema,
    },
    Draft, JsonType,
};

// Canonicalize under the default draft, panicking with the schema for context.
pub(crate) fn canonicalize(schema: &Value) -> CanonicalSchema {
    crate::canonicalize(schema)
        .unwrap_or_else(|error| panic!("canonicalize({schema}) failed: {error}"))
}

// Canonicalize under an explicit draft via the public options builder.
pub(crate) fn canonicalize_with(schema: &Value, draft: Draft) -> CanonicalSchema {
    options()
        .with_draft(draft)
        .canonicalize(schema)
        .unwrap_or_else(|error| panic!("canonicalize({schema}) failed: {error}"))
}

// Zero inline budget keeps shared refs symbolic so definition relocation is observable.
pub(crate) fn canonicalize_symbolic(schema: &Value) -> CanonicalSchema {
    options()
        .with_inline_budget(0)
        .canonicalize(schema)
        .unwrap_or_else(|error| panic!("canonicalize({schema}) failed: {error}"))
}

pub(crate) fn cardinality(value: u64) -> BoundCardinality {
    BoundCardinality::from(value)
}

pub(crate) fn const_json(value: Value) -> CanonicalJson {
    CanonicalJson::from_value(&value)
}

// Negate a JSON schema through the pipeline and assert exact set complement: rejects the original's
// accepts, accepts its rejects.
pub(crate) fn assert_json_complement(schema: &Value, accepts: &[Value], rejects: &[Value]) {
    let pos = crate::validator_for(schema).expect("positive compiles");
    let negated = canonicalize(schema).negate().to_json_schema();
    let neg = crate::validator_for(&negated).expect("negated compiles");
    for value in accepts {
        assert!(pos.is_valid(value));
        assert!(!neg.is_valid(value));
    }
    for value in rejects {
        assert!(!pos.is_valid(value));
        assert!(neg.is_valid(value));
    }
}

pub(crate) fn typed_group(ty: JsonType, body: Schema) -> Schema {
    Schema::TypedGroup {
        ty,
        body: shared(body),
    }
}

pub(crate) fn type_guard(ty: JsonType, body: Schema) -> Schema {
    Schema::TypeGuard {
        ty,
        body: shared(body),
    }
}

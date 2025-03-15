use std::borrow::Cow;

use serde_json::Value;

use crate::paths::Location;

#[derive(Debug)]
pub struct ValidationErrorV2<'a> {
    repr: Box<ErrorRepr<'a>>,
}

#[derive(Debug)]
struct ErrorRepr<'a> {
    instance: Cow<'a, Value>,
    kind: ValidationErrorKind,
    /// Path to the JSON Schema keyword that failed validation.
    schema_path: Location,
}

impl<'a> ValidationErrorV2<'a> {
    pub fn ty(instance: &'a Value, schema_path: Location) -> Self {
        Self {
            repr: Box::new(ErrorRepr {
                instance: Cow::Borrowed(instance),
                kind: ValidationErrorKind::Type,
                schema_path,
            }),
        }
    }
    pub fn bool(instance: &'a Value, schema_path: Location) -> Self {
        Self {
            repr: Box::new(ErrorRepr {
                instance: Cow::Borrowed(instance),
                kind: ValidationErrorKind::Bool,
                schema_path,
            }),
        }
    }
    pub fn minimum(instance: &'a Value, schema_path: Location) -> Self {
        Self {
            repr: Box::new(ErrorRepr {
                instance: Cow::Borrowed(instance),
                kind: ValidationErrorKind::Minimum,
                schema_path,
            }),
        }
    }
    pub fn maximum(instance: &'a Value, schema_path: Location) -> Self {
        Self {
            repr: Box::new(ErrorRepr {
                instance: Cow::Borrowed(instance),
                kind: ValidationErrorKind::Maximum,
                schema_path,
            }),
        }
    }
    pub fn multiple_of(instance: &'a Value, schema_path: Location) -> Self {
        Self {
            repr: Box::new(ErrorRepr {
                instance: Cow::Borrowed(instance),
                kind: ValidationErrorKind::MultipleOf,
                schema_path,
            }),
        }
    }
}

#[derive(Debug)]
pub enum ValidationErrorKind {
    Type,
    Bool,
    Minimum,
    Maximum,
    ExclusiveMinimum,
    ExclusiveMaximum,
    MultipleOf,
}

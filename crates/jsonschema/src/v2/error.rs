use std::borrow::Cow;

use serde_json::Value;

use crate::paths::Location;

#[derive(Debug)]
pub struct ValidationError<'a> {
    pub instance: Cow<'a, Value>,
    pub kind: ValidationErrorKind,
    /// Path to the JSON Schema keyword that failed validation.
    pub schema_path: Location,
}

#[derive(Debug)]
pub enum ValidationErrorKind {
    Type,
    Minimum,
}

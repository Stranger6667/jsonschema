//! Shared types used by the evaluation outputs (annotations, errors, and output units).

use std::{fmt, sync::Arc};

use crate::ValidationError;
use ahash::AHashMap;

/// Annotations associated with an output unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotations(Arc<serde_json::Value>);

impl Annotations {
    /// The `serde_json::Value` of the annotation.
    #[must_use]
    pub fn value(&self) -> &serde_json::Value {
        &self.0
    }
}

impl From<&AHashMap<String, serde_json::Value>> for Annotations {
    fn from(anns: &AHashMap<String, serde_json::Value>) -> Self {
        let mut object = serde_json::Map::with_capacity(anns.len());
        for (key, value) in anns {
            object.insert(key.clone(), value.clone());
        }
        Annotations(Arc::new(serde_json::Value::Object(object)))
    }
}

impl From<&serde_json::Value> for Annotations {
    fn from(v: &serde_json::Value) -> Self {
        Annotations(Arc::new(v.clone()))
    }
}

impl From<Arc<serde_json::Value>> for Annotations {
    fn from(v: Arc<serde_json::Value>) -> Self {
        Annotations(v)
    }
}

impl From<&Arc<serde_json::Value>> for Annotations {
    fn from(v: &Arc<serde_json::Value>) -> Self {
        Annotations(Arc::clone(v))
    }
}

impl From<serde_json::Value> for Annotations {
    fn from(v: serde_json::Value) -> Self {
        Annotations(Arc::new(v))
    }
}

impl serde::Serialize for Annotations {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

/// Description of a validation error used within evaluation outputs.
#[derive(serde::Serialize, Debug, Clone, PartialEq, Eq)]
pub struct ErrorDescription(String);

impl ErrorDescription {
    /// Returns the inner [`String`] of the error description.
    #[inline]
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for ErrorDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<ValidationError<'_>> for ErrorDescription {
    fn from(e: ValidationError<'_>) -> Self {
        ErrorDescription(e.to_string())
    }
}

impl<'a> From<&'a str> for ErrorDescription {
    fn from(s: &'a str) -> Self {
        ErrorDescription(s.to_string())
    }
}

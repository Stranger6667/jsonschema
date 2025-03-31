use crate::{
    paths::{LazyLocation, Location},
    validator::Validate,
    TracingCallback, TracingContext, ValidationError,
};
use serde_json::{Map, Value};

pub(crate) struct CustomKeyword {
    inner: Box<dyn Keyword>,
    location: Location,
}

impl CustomKeyword {
    pub(crate) fn new(inner: Box<dyn Keyword>, location: Location) -> Self {
        Self { inner, location }
    }
}

impl Validate for CustomKeyword {
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        self.inner.validate(instance, location)
    }

    fn is_valid(&self, instance: &Value) -> bool {
        self.inner.is_valid(instance)
    }
    fn schema_path(&self) -> &Location {
        &self.location
    }
    fn matches_type(&self, _: &Value) -> bool {
        true
    }
    fn trace(
        &self,
        instance: &Value,
        location: &LazyLocation,
        callback: TracingCallback<'_>,
    ) -> bool {
        let result = self.is_valid(instance);
        let rv = if self.matches_type(instance) {
            Some(result)
        } else {
            None
        };
        TracingContext::new(location, self.schema_path(), rv).call(callback);
        if self.inner.is_informational() {
            // Keyword does not affect validation results
            true
        } else {
            result
        }
    }
}

/// Trait that allows implementing custom validation for keywords.
pub trait Keyword: Send + Sync {
    /// Validate instance according to a custom specification.
    ///
    /// A custom keyword validator may be used when a validation that cannot be
    /// easily or efficiently expressed in JSON schema.
    ///
    /// The custom validation is applied in addition to the JSON schema validation.
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>>;
    /// Validate instance and return a boolean result.
    ///
    /// Could be potentilly faster than [`Keyword::validate`] method.
    fn is_valid(&self, instance: &Value) -> bool;
    fn is_informational(&self) -> bool {
        false
    }
}

pub(crate) trait KeywordFactory: Send + Sync {
    fn init<'a>(
        &self,
        parent: &'a Map<String, Value>,
        schema: &'a Value,
        path: Location,
    ) -> Result<Box<dyn Keyword>, ValidationError<'a>>;
}

impl<F> KeywordFactory for F
where
    F: for<'a> Fn(
            &'a Map<String, Value>,
            &'a Value,
            Location,
        ) -> Result<Box<dyn Keyword>, ValidationError<'a>>
        + Send
        + Sync,
{
    fn init<'a>(
        &self,
        parent: &'a Map<String, Value>,
        schema: &'a Value,
        path: Location,
    ) -> Result<Box<dyn Keyword>, ValidationError<'a>> {
        self(parent, schema, path)
    }
}

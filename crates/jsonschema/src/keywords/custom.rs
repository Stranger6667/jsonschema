use crate::{
    paths::{LazyLocation, Location, RefTracker},
    tracing::{TracingCallback, TracingContext},
    validator::{Validate, ValidationContext},
    ValidationError,
};
use serde_json::{Map, Value};

pub(crate) struct CustomKeyword {
    inner: Box<dyn Keyword>,
    location: Location,
    keyword: String,
}

impl CustomKeyword {
    pub(crate) fn new(inner: Box<dyn Keyword>, location: Location, keyword: String) -> Self {
        Self {
            inner,
            location,
            keyword,
        }
    }
}

impl Validate for CustomKeyword {
    fn validate<'i>(
        &self,
        instance: &'i Value,
        instance_path: &LazyLocation,
        _tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        self.inner
            .validate(instance)
            .map_err(|err| err.with_context(instance, instance_path, &self.location, &self.keyword))
    }

    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
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
        _ctx: &mut ValidationContext,
    ) -> bool {
        let result = self.inner.is_valid(instance);
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

/// Trait for implementing custom keyword validators.
///
/// Custom keywords extend JSON Schema validation with domain-specific rules.
///
/// # Example
///
/// ```rust
/// use jsonschema::{Keyword, ValidationError};
/// use serde_json::Value;
///
/// struct EvenNumberValidator;
///
/// impl Keyword for EvenNumberValidator {
///     fn validate<'i>(&self, instance: &'i Value) -> Result<(), ValidationError<'i>> {
///         if let Some(n) = instance.as_u64() {
///             if n % 2 != 0 {
///                 return Err(ValidationError::custom("number must be even"));
///             }
///         }
///         Ok(())
///     }
///
///     fn is_valid(&self, instance: &Value) -> bool {
///         instance.as_u64().map_or(true, |n| n % 2 == 0)
///     }
/// }
/// ```
pub trait Keyword: Send + Sync {
    /// Validate an instance against this custom keyword.
    ///
    /// Use [`ValidationError::custom`] for error messages. Path information
    /// (`instance_path` and `schema_path`) is filled in automatically.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] if the instance is invalid.
    fn validate<'i>(&self, instance: &'i Value) -> Result<(), ValidationError<'i>>;

    /// Check validity without collecting error details.
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
        schema_path: Location,
        keyword: &str,
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
        schema_path: Location,
        keyword: &str,
    ) -> Result<Box<dyn Keyword>, ValidationError<'a>> {
        self(parent, schema, schema_path.clone())
            .map_err(|err| err.with_schema_context(schema, schema_path, keyword))
    }
}

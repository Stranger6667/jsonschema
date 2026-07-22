use crate::{
    error::ErrorIterator,
    paths::{LazyLocation, Location, RefTracker},
    validator::{Validate, ValidationContext},
    Json, Node, ValidationError,
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

// Custom keywords consume `&Value`; foreign representations are materialized per call.
impl<F: Json> Validate<F> for CustomKeyword {
    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        instance_path: &LazyLocation,
        _tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        let value = instance.to_value();
        self.inner.validate(&value).map_err(|err| {
            err.with_context(&value, instance_path, &self.location, &self.keyword)
                .to_owned()
        })
    }

    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        self.inner.is_valid(&instance.to_value())
    }

    fn iter_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        instance_path: &LazyLocation,
        _tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        let value = instance.to_value();
        let errors: Vec<_> = self
            .inner
            .iter_errors(&value)
            .map(|err| {
                err.with_context(&value, instance_path, &self.location, &self.keyword)
                    .to_owned()
            })
            .collect();
        ErrorIterator::from_iterator(errors.into_iter())
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

    /// Validate an instance, yielding every error at once.
    ///
    /// Override this to report multiple problems from a single keyword. The
    /// default yields at most one error, from [`validate`](Keyword::validate).
    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
    ) -> Box<dyn Iterator<Item = ValidationError<'i>> + 'i> {
        Box::new(self.validate(instance).err().into_iter())
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

use crate::{
    error::ErrorIterator,
    paths::{LazyLocation, Location, RefTracker},
    validator::{Validate, ValidationContext},
    Json, Node, SerdeJson, ValidationError,
};
use serde_json::{Map, Value};

pub(crate) struct CustomKeyword<F: Json> {
    inner: Box<dyn for<'i> Keyword<'i, F>>,
    location: Location,
    keyword: String,
}

impl<F: Json> CustomKeyword<F> {
    pub(crate) fn new(
        inner: Box<dyn for<'i> Keyword<'i, F>>,
        location: Location,
        keyword: String,
    ) -> Self {
        Self {
            inner,
            location,
            keyword,
        }
    }
}

impl<F: Json> Validate<F> for CustomKeyword<F> {
    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        instance_path: &LazyLocation,
        _tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        self.inner.validate(instance.clone()).map_err(|err| {
            let value = instance.to_value();
            err.with_context(&value, instance_path, &self.location, &self.keyword)
                .to_owned()
        })
    }

    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        self.inner.is_valid(instance.clone())
    }

    fn iter_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        instance_path: &LazyLocation,
        _tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        let inner_errors: Vec<_> = self.inner.iter_errors(instance.clone()).collect();
        if inner_errors.is_empty() {
            return ErrorIterator::from_iterator(Vec::new().into_iter());
        }
        let value = instance.to_value();
        let errors: Vec<_> = inner_errors
            .into_iter()
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
/// Custom keywords extend JSON Schema validation with domain-specific rules. `F` selects the
/// instance representation the keyword operates on; the default is `serde_json`. The `'i`
/// lifetime is the validated instance's; implementations are generic over it
/// (`impl<'i> Keyword<'i> for ...`).
///
/// # Example
///
/// ```rust
/// use jsonschema::{Keyword, ValidationError};
/// use serde_json::Value;
///
/// struct EvenNumberValidator;
///
/// impl<'i> Keyword<'i> for EvenNumberValidator {
///     fn validate(&self, instance: &'i Value) -> Result<(), ValidationError<'i>> {
///         if self.is_valid(instance) {
///             Ok(())
///         } else {
///             Err(ValidationError::custom("number must be even"))
///         }
///     }
///
///     fn is_valid(&self, instance: &'i Value) -> bool {
///         instance.as_u64().is_none_or(|number| number % 2 == 0)
///     }
/// }
/// ```
///
/// A keyword written against the [`Node`] accessors works for every representation:
///
/// ```rust
/// use jsonschema::{
///     json::{Json, JsonNumber, Node},
///     Keyword, ValidationError,
/// };
///
/// struct EvenNumberValidator;
///
/// impl<'i, F: Json> Keyword<'i, F> for EvenNumberValidator {
///     fn validate(&self, instance: F::Node<'i>) -> Result<(), ValidationError<'i>> {
///         if Keyword::<F>::is_valid(self, instance) {
///             Ok(())
///         } else {
///             Err(ValidationError::custom("number must be even"))
///         }
///     }
///
///     fn is_valid(&self, instance: F::Node<'i>) -> bool {
///         instance
///             .as_number()
///             .and_then(|number| number.as_u64())
///             .is_none_or(|number| number % 2 == 0)
///     }
/// }
/// ```
pub trait Keyword<'i, F: Json = SerdeJson>: Send + Sync {
    /// Validate an instance against this custom keyword.
    ///
    /// Use [`ValidationError::custom`] for error messages. Path information
    /// (`instance_path` and `schema_path`) is filled in automatically.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] if the instance is invalid.
    fn validate(&self, instance: F::Node<'i>) -> Result<(), ValidationError<'i>>;

    /// Check validity without collecting error details.
    fn is_valid(&self, instance: F::Node<'i>) -> bool;

    /// Validate an instance, yielding every error at once.
    ///
    /// Override this to report multiple problems from a single keyword. The
    /// default yields at most one error, from [`validate`](Keyword::validate).
    fn iter_errors(
        &self,
        instance: F::Node<'i>,
    ) -> Box<dyn Iterator<Item = ValidationError<'i>> + 'i> {
        Box::new(self.validate(instance).err().into_iter())
    }
}

pub(crate) trait KeywordFactory<F: Json>: Send + Sync {
    fn init<'a>(
        &self,
        parent: &'a Map<String, Value>,
        schema: &'a Value,
        schema_path: Location,
        keyword: &str,
    ) -> Result<Box<dyn for<'i> Keyword<'i, F>>, ValidationError<'a>>;
}

impl<F: Json, Func> KeywordFactory<F> for Func
where
    Func: for<'a> Fn(
            &'a Map<String, Value>,
            &'a Value,
            Location,
        ) -> Result<Box<dyn for<'i> Keyword<'i, F>>, ValidationError<'a>>
        + Send
        + Sync,
{
    fn init<'a>(
        &self,
        parent: &'a Map<String, Value>,
        schema: &'a Value,
        schema_path: Location,
        keyword: &str,
    ) -> Result<Box<dyn for<'i> Keyword<'i, F>>, ValidationError<'a>> {
        self(parent, schema, schema_path.clone())
            .map_err(|err| err.with_schema_context(schema, schema_path, keyword))
    }
}

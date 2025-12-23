use crate::{
    paths::{EvaluationPathTracker, LazyLocation, Location},
    thread::ThreadBound,
    validator::{Validate, ValidationContext},
    ValidationError,
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
        instance_path: &LazyLocation,
        _evaluation_path: &EvaluationPathTracker,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        self.inner
            .validate(instance, instance_path, ctx, &self.location)
    }

    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        self.inner.is_valid(instance)
    }
}

/// Trait for implementing custom keyword validators.
///
/// Custom keywords extend JSON Schema validation with domain-specific rules.
///
/// # Example
///
/// ```rust
/// use jsonschema::{
///     paths::{LazyLocation, Location},
///     Keyword, ValidationContext, ValidationError,
/// };
/// use serde_json::Value;
///
/// struct EvenNumberValidator;
///
/// impl Keyword for EvenNumberValidator {
///     fn validate<'i>(
///         &self,
///         instance: &'i Value,
///         instance_path: &LazyLocation,
///         ctx: &mut ValidationContext,
///         schema_path: &Location,
///     ) -> Result<(), ValidationError<'i>> {
///         if let Some(n) = instance.as_u64() {
///             if n % 2 != 0 {
///                 return Err(ctx.custom_error(
///                     schema_path,
///                     instance_path,
///                     instance,
///                     "number must be even",
///                 ));
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
pub trait Keyword: ThreadBound {
    /// Validate an instance against this custom keyword.
    ///
    /// Use `ctx.custom_error()` to create errors with correct `evaluation_path`.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] if the instance fails validation.
    fn validate<'i>(
        &self,
        instance: &'i Value,
        instance_path: &LazyLocation,
        ctx: &mut ValidationContext,
        schema_path: &Location,
    ) -> Result<(), ValidationError<'i>>;

    /// Check validity without collecting error details.
    fn is_valid(&self, instance: &Value) -> bool;
}

pub(crate) trait KeywordFactory: ThreadBound {
    fn init<'a>(
        &self,
        parent: &'a Map<String, Value>,
        schema: &'a Value,
        schema_path: Location,
    ) -> Result<Box<dyn Keyword>, ValidationError<'a>>;
}

impl<F> KeywordFactory for F
where
    F: for<'a> Fn(
            &'a Map<String, Value>,
            &'a Value,
            Location,
        ) -> Result<Box<dyn Keyword>, ValidationError<'a>>
        + ThreadBound,
{
    fn init<'a>(
        &self,
        parent: &'a Map<String, Value>,
        schema: &'a Value,
        schema_path: Location,
    ) -> Result<Box<dyn Keyword>, ValidationError<'a>> {
        self(parent, schema, schema_path)
    }
}

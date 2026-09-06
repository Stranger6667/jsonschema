use crate::{
    paths::{LazyLocation, Location, RefTracker},
    tracing::{TracingCallback, TracingContext},
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
        if self.inner.is_informational() {
            return Ok(());
        }
        self.inner.validate(instance.clone()).map_err(|err| {
            let value = instance.to_value();
            err.with_context(&value, instance_path, &self.location, &self.keyword)
                .to_owned()
        })
    }

    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        self.inner.is_informational() || self.inner.is_valid(instance.clone())
    }

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        instance_path: &LazyLocation,
        _tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        if self.inner.is_informational() {
            return;
        }
        let mut inner_errors = self.inner.iter_errors(instance.clone()).peekable();
        if inner_errors.peek().is_none() {
            return;
        }
        let value = instance.to_value();
        errors.extend(inner_errors.map(|err| {
            err.with_context(&value, instance_path, &self.location, &self.keyword)
                .to_owned()
        }));
    }

    fn schema_path(&self) -> &Location {
        &self.location
    }
    fn matches_type(&self, _: &F::Node<'_>) -> bool {
        true
    }
    fn trace(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        callback: TracingCallback<'_>,
        _ctx: &mut ValidationContext,
    ) -> bool {
        let result = self.inner.is_valid(instance.clone());
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
    ///
    /// [`Validator::is_valid`](crate::Validator::is_valid) calls this instead of
    /// [`validate`](Keyword::validate), so the two must agree on every instance.
    /// A keyword that carries state across calls must update it in both.
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

    /// Whether the keyword only reports: its outcome reaches
    /// [`Validator::trace`](crate::Validator::trace) callbacks but never fails validation.
    fn is_informational(&self) -> bool {
        false
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

#[cfg(test)]
mod tests {
    use crate::{paths::Location, tracing::NodeEvaluationResult, Keyword, ValidationError};
    use serde_json::{json, Map, Value};

    struct Rejecting {
        informational: bool,
    }

    impl Keyword<'_> for Rejecting {
        fn validate(&self, _: &Value) -> Result<(), ValidationError<'static>> {
            Err(ValidationError::custom("rejected"))
        }

        fn is_valid(&self, _: &Value) -> bool {
            false
        }

        fn is_informational(&self) -> bool {
            self.informational
        }
    }

    #[allow(clippy::unnecessary_wraps, clippy::result_large_err)]
    fn informational<'a>(
        _: &'a Map<String, Value>,
        _: &'a Value,
        _: Location,
    ) -> Result<Box<dyn for<'i> Keyword<'i>>, ValidationError<'a>> {
        Ok(Box::new(Rejecting {
            informational: true,
        }))
    }

    #[allow(clippy::unnecessary_wraps, clippy::result_large_err)]
    fn asserting<'a>(
        _: &'a Map<String, Value>,
        _: &'a Value,
        _: Location,
    ) -> Result<Box<dyn for<'i> Keyword<'i>>, ValidationError<'a>> {
        Ok(Box::new(Rejecting {
            informational: false,
        }))
    }

    // An informational keyword reports its outcome through `trace` and fails nothing.
    #[test]
    fn informational_keyword_reports_without_failing() {
        let validator = crate::options()
            .with_keyword("note", informational)
            .build(&json!({"note": 1}))
            .expect("builds");
        assert!(validator.is_valid(&json!(1)));
        assert!(validator.validate(&json!(1)).is_ok());
        assert_eq!(validator.iter_errors(&json!(1)).count(), 0);
        let mut reported = Vec::new();
        assert!(validator.trace(&json!(1), &mut |ctx| {
            reported.push((ctx.schema_location.to_string(), ctx.result));
        }));
        assert_eq!(
            reported,
            vec![
                ("/note".to_owned(), NodeEvaluationResult::Invalid),
                (String::new(), NodeEvaluationResult::Valid),
            ]
        );
    }

    #[test]
    fn asserting_keyword_fails() {
        let validator = crate::options()
            .with_keyword("note", asserting)
            .build(&json!({"note": 1}))
            .expect("builds");
        assert!(!validator.is_valid(&json!(1)));
        assert!(validator.validate(&json!(1)).is_err());
        assert_eq!(validator.iter_errors(&json!(1)).count(), 1);
        assert!(!validator.trace(&json!(1), &mut |_| {}));
    }
}

use crate::{
    compiler,
    error::ValidationError,
    keywords::{
        helpers::{fail_on_non_positive_integer, size_limit},
        CompilationResult,
    },
    paths::{LazyLocation, Location, RefTracker},
    validator::{evaluate_assertion, EvaluationResult, Validate, ValidationContext},
    Json, Node,
};
use serde_json::{Map, Value};

pub(crate) struct MaxLengthValidator {
    limit: u64,
    location: Location,
}

impl MaxLengthValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
        location: Location,
    ) -> CompilationResult<'a, F> {
        let Some(limit) = size_limit(ctx, schema) else {
            return Err(fail_on_non_positive_integer(schema, location));
        };
        Ok(Box::new(MaxLengthValidator { limit, location }))
    }
}

impl<F: Json> Validate<F> for MaxLengthValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        instance.string_length_within(0, self.limit) != Some(false)
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if instance.string_length_within(0, self.limit) == Some(false) {
            return Err(ValidationError::max_length(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.lazy_value(),
                self.limit,
            ));
        }
        Ok(())
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        evaluate_assertion::<F, _>(self, instance, ctx)
    }
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    // a sibling `minLength` checks both bounds off one measurement
    if size_limit(ctx, schema).is_some()
        && !ctx.is_keyword_overridden("minLength")
        && parent
            .get("minLength")
            .and_then(|min| size_limit(ctx, min))
            .is_some()
    {
        return None;
    }
    let location = ctx.location().join("maxLength");
    Some(MaxLengthValidator::compile(ctx, schema, location))
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test]
    fn location() {
        tests_util::assert_schema_location(&json!({"maxLength": 1}), &json!("ab"), "/maxLength");
    }

    // Lengths count code points: multibyte strings sit at the bounds by code points, not bytes.
    #[test_case(&json!({"maxLength": 2}), &json!("🦀🦀"), true; "max astral at limit")]
    #[test_case(&json!({"maxLength": 2}), &json!("🦀🦀🦀"), false; "max astral past limit")]
    #[test_case(&json!({"maxLength": 3}), &json!("ééé"), true; "max two-byte at limit")]
    #[test_case(&json!({"maxLength": 2}), &json!("abc"), false; "max ascii past limit")]
    #[test_case(&json!({"minLength": 3}), &json!("🦀🦀"), false; "min astral below limit")]
    #[test_case(&json!({"minLength": 2}), &json!("🦀🦀"), true; "min astral at limit")]
    #[test_case(&json!({"minLength": 6}), &json!("héllo"), false; "min bytes reach limit")]
    #[test_case(&json!({"minLength": 2, "maxLength": 2}), &json!("🦀🦀"), true; "range astral exact")]
    #[test_case(&json!({"minLength": 3, "maxLength": 4}), &json!("ééé"), true; "range two-byte inside")]
    #[test_case(&json!({"minLength": 1, "maxLength": 2}), &json!("ééé"), false; "range two-byte past maximum")]
    #[test_case(&json!({"minLength": 4, "maxLength": 9}), &json!("ééé"), false; "range two-byte below minimum")]
    #[test_case(&json!({"maxLength": 0}), &json!(5), true; "non-string")]
    fn length_counts_code_points(schema: &Value, instance: &Value, expected: bool) {
        let validator = crate::validator_for(schema).expect("schema builds");
        assert_eq!(validator.is_valid(instance), expected);
        assert_eq!(validator.validate(instance).is_ok(), expected);
    }
}

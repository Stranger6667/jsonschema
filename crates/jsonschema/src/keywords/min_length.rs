use crate::{
    compiler,
    error::ValidationError,
    keywords::{
        helpers::{fail_on_non_positive_integer, size_limit},
        CompilationResult,
    },
    paths::{LazyLocation, Location, RefTracker},
    validator::{Validate, ValidationContext},
    Json, Node,
};
use serde_json::{Map, Value};

pub(crate) struct MinLengthValidator {
    limit: u64,
    location: Location,
}

impl<F: Json> Validate<F> for MinLengthValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(length) = instance.string_length() {
            if length < self.limit {
                return false;
            }
        }
        true
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(length) = instance.string_length() {
            if length < self.limit {
                return Err(ValidationError::min_length(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance.lazy_value(),
                    self.limit,
                ));
            }
        }
        Ok(())
    }
}

/// `minLength` and `maxLength` together. A length is a count of code points, so measuring it
/// walks the whole string; two validators walk it twice.
pub(crate) struct LengthRangeValidator {
    minimum: u64,
    maximum: u64,
    min_location: Location,
    max_location: Location,
}

impl LengthRangeValidator {
    fn min_error<'i, F: Json>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
    ) -> ValidationError<'i> {
        ValidationError::min_length(
            self.min_location.clone(),
            crate::paths::capture_evaluation_path(tracker, &self.min_location),
            location.into(),
            instance.lazy_value(),
            self.minimum,
        )
    }

    fn max_error<'i, F: Json>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
    ) -> ValidationError<'i> {
        ValidationError::max_length(
            self.max_location.clone(),
            crate::paths::capture_evaluation_path(tracker, &self.max_location),
            location.into(),
            instance.lazy_value(),
            self.maximum,
        )
    }
}

impl<F: Json> Validate<F> for LengthRangeValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(length) = instance.string_length() {
            if length < self.minimum || length > self.maximum {
                return false;
            }
        }
        true
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(length) = instance.string_length() {
            // `minLength` sorts first
            if length < self.minimum {
                return Err(self.min_error::<F>(instance, location, tracker));
            }
            if length > self.maximum {
                return Err(self.max_error::<F>(instance, location, tracker));
            }
        }
        Ok(())
    }

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        if let Some(length) = instance.string_length() {
            // `minLength` above `maxLength` fails both at once, as two validators would
            if length < self.minimum {
                errors.push(self.min_error::<F>(instance, location, tracker));
            }
            if length > self.maximum {
                errors.push(self.max_error::<F>(instance, location, tracker));
            }
        }
    }
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    let location = ctx.location().join("minLength");
    let Some(minimum) = size_limit(ctx, schema) else {
        return Some(Err(fail_on_non_positive_integer(schema, location)));
    };
    // `max_length::compile` steps aside when it sees this pair
    if let Some(maximum) = parent.get("maxLength").and_then(|max| size_limit(ctx, max)) {
        return Some(Ok(Box::new(LengthRangeValidator {
            minimum,
            maximum,
            min_location: location,
            max_location: ctx.location().join("maxLength"),
        })));
    }
    Some(Ok(Box::new(MinLengthValidator {
        limit: minimum,
        location,
    })))
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::json;

    #[test]
    fn location() {
        tests_util::assert_schema_location(&json!({"minLength": 1}), &json!(""), "/minLength");
    }

    #[test]
    fn fused_locations() {
        let schema = json!({"minLength": 2, "maxLength": 4});
        tests_util::assert_schema_location(&schema, &json!("a"), "/minLength");
        tests_util::assert_schema_location(&schema, &json!("abcde"), "/maxLength");
    }

    #[test]
    fn fused_bounds_are_inclusive() {
        let schema = json!({"minLength": 2, "maxLength": 4});
        tests_util::is_valid(&schema, &json!("ab"));
        tests_util::is_valid(&schema, &json!("abcd"));
        tests_util::is_not_valid(&schema, &json!("a"));
        tests_util::is_not_valid(&schema, &json!("abcde"));
    }

    #[test]
    fn fused_ignores_non_strings() {
        let schema = json!({"minLength": 2, "maxLength": 4});
        tests_util::is_valid(&schema, &json!(1));
        tests_util::is_valid(&schema, &json!(null));
        tests_util::is_valid(&schema, &json!(["a"]));
    }

    #[test]
    fn fused_counts_code_points() {
        // 5 code points in 6 bytes; counting bytes would put it over
        tests_util::is_valid(&json!({"minLength": 5, "maxLength": 5}), &json!("héllo"));
        // 2 code points in 8 bytes
        tests_util::is_valid(&json!({"minLength": 2, "maxLength": 2}), &json!("💩💩"));
        tests_util::is_not_valid(&json!({"minLength": 3, "maxLength": 4}), &json!("💩💩"));
    }

    #[test]
    fn unsatisfiable_range_reports_both() {
        // Not `assert_locations`: its `zip` would pass on a single error.
        let validator = crate::validator_for(&json!({"minLength": 5, "maxLength": 3})).unwrap();
        let mut locations: Vec<String> = validator
            .iter_errors(&json!("abcd"))
            .map(|error| error.schema_path().as_str().to_string())
            .collect();
        locations.sort();
        assert_eq!(locations, ["/maxLength", "/minLength"]);
    }

    #[test]
    fn invalid_sibling_still_reported() {
        // An unusable bound keeps its own error instead of being folded away.
        assert!(crate::validator_for(&json!({"minLength": 1, "maxLength": -1})).is_err());
        assert!(crate::validator_for(&json!({"minLength": -1, "maxLength": 1})).is_err());
    }
}

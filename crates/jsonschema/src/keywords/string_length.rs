#![allow(clippy::float_cmp, clippy::cast_sign_loss)]
use crate::{
    compiler,
    error::{no_error, ErrorIterator, ValidationError},
    keywords::CompilationResult,
    paths::{LazyLocation, Location, RefTracker},
    validator::{Validate, ValidationContext},
};
use serde_json::Value;

pub(crate) struct MinMaxLengthValidator {
    min: u64,
    max: u64,
    min_location: Location,
    max_location: Location,
}

impl MinMaxLengthValidator {
    #[inline]
    pub(crate) fn compile(
        min: u64,
        max: u64,
        min_location: Location,
        max_location: Location,
    ) -> CompilationResult<'static> {
        Ok(Box::new(MinMaxLengthValidator {
            min,
            max,
            min_location,
            max_location,
        }))
    }
}

impl Validate for MinMaxLengthValidator {
    #[inline]
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::String(item) = instance {
            let len = bytecount::num_chars(item.as_bytes()) as u64;
            len >= self.min && len <= self.max
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::String(item) = instance {
            let len = bytecount::num_chars(item.as_bytes()) as u64;
            if len < self.min {
                return Err(ValidationError::min_length(
                    self.min_location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.min_location),
                    location.into(),
                    instance,
                    self.min,
                ));
            }
            if len > self.max {
                return Err(ValidationError::max_length(
                    self.max_location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.max_location),
                    location.into(),
                    instance,
                    self.max,
                ));
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::String(item) = instance {
            let len = bytecount::num_chars(item.as_bytes()) as u64;
            let mut errors = Vec::new();
            if len < self.min {
                errors.push(ValidationError::min_length(
                    self.min_location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.min_location),
                    location.into(),
                    instance,
                    self.min,
                ));
            }
            if len > self.max {
                errors.push(ValidationError::max_length(
                    self.max_location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.max_location),
                    location.into(),
                    instance,
                    self.max,
                ));
            }
            if !errors.is_empty() {
                return ErrorIterator::from_iterator(errors.into_iter());
            }
        }
        no_error()
    }
}

/// Try to parse a value as u64 limit, handling integer-valued floats for newer drafts.
#[inline]
pub(crate) fn parse_limit(ctx: &compiler::Context, schema: &Value) -> Option<u64> {
    if let Some(limit) = schema.as_u64() {
        return Some(limit);
    }
    if ctx.supports_integer_valued_numbers() {
        if let Some(limit) = schema.as_f64() {
            if limit.trunc() == limit {
                #[allow(clippy::cast_possible_truncation)]
                return Some(limit as u64);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use test_case::test_case;

    #[test_case(&json!({"minLength": 2, "maxLength": 5}), &json!("ab"), true)]
    #[test_case(&json!({"minLength": 2, "maxLength": 5}), &json!("abcde"), true)]
    #[test_case(&json!({"minLength": 2, "maxLength": 5}), &json!("abc"), true)]
    #[test_case(&json!({"minLength": 2, "maxLength": 5}), &json!("a"), false)]
    #[test_case(&json!({"minLength": 2, "maxLength": 5}), &json!("abcdef"), false)]
    #[test_case(&json!({"minLength": 2, "maxLength": 5}), &json!(123), true)] // non-string passes
    fn minmax_length(schema: &serde_json::Value, instance: &serde_json::Value, expected: bool) {
        let validator = crate::validator_for(schema).unwrap();
        assert_eq!(validator.is_valid(instance), expected);
    }

    #[test]
    fn iter_errors_both_violations() {
        // String that's both too short for one schema and too long for another
        // This tests that we properly detect each error type
        let schema = json!({"minLength": 5, "maxLength": 3});
        let validator = crate::validator_for(&schema).unwrap();

        // Too short
        let instance = json!("ab");
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].to_string().contains("shorter"));

        // Too long
        let instance = json!("abcdef");
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].to_string().contains("longer"));
    }

    #[test]
    fn error_locations() {
        let schema = json!({"minLength": 2, "maxLength": 5});
        let validator = crate::validator_for(&schema).unwrap();

        let instance = json!("a");
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].schema_path().to_string(), "/minLength");

        let instance = json!("abcdefg");
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].schema_path().to_string(), "/maxLength");
    }
}

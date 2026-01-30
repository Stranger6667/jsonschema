use std::sync::Arc;

use crate::{
    compiler,
    error::ValidationError,
    keywords::CompilationResult,
    options::PatternEngineOptions,
    paths::{LazyEvaluationPath, LazyLocation, Location, RefTracker},
    regex::{pattern_as_prefix, RegexEngine, RegexError},
    types::JsonType,
    validator::{Validate, ValidationContext},
};
use serde_json::{Map, Value};

/// Validator for patterns that are simple prefixes (optimized path).
pub(crate) struct PrefixPatternValidator {
    prefix: Arc<str>,
    pattern: Arc<str>,
    location: Location,
}

impl Validate for PrefixPatternValidator {
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::String(item) = instance {
            item.starts_with(self.prefix.as_ref())
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
            if !item.starts_with(self.prefix.as_ref()) {
                return Err(ValidationError::pattern(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance,
                    self.pattern.to_string(),
                ));
            }
        }
        Ok(())
    }
}

pub(crate) struct PatternValidator<R> {
    regex: Arc<R>,
    location: Location,
}

impl<R: RegexEngine> Validate for PatternValidator<R> {
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::String(item) = instance {
            match self.regex.is_match(item) {
                Ok(is_match) => {
                    if !is_match {
                        return Err(ValidationError::pattern(
                            self.location.clone(),
                            crate::paths::capture_evaluation_path(tracker, &self.location),
                            location.into(),
                            instance,
                            self.regex.pattern().to_string(),
                        ));
                    }
                }
                Err(e) => {
                    return Err(ValidationError::backtrack_limit(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        location.into(),
                        instance,
                        e.into_backtrack_error()
                            .expect("Can only fail with the fancy-regex crate"),
                    ));
                }
            }
        }
        Ok(())
    }

    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::String(item) = instance {
            return self.regex.is_match(item).unwrap_or(false);
        }
        true
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    if let Value::String(item) = schema {
        // Try prefix optimization first
        if let Some(prefix) = pattern_as_prefix(item) {
            return Some(Ok(Box::new(PrefixPatternValidator {
                prefix: Arc::from(prefix),
                pattern: Arc::from(item.as_str()),
                location: ctx.location().join("pattern"),
            })));
        }
        // Fall back to regex compilation
        match ctx.config().pattern_options() {
            PatternEngineOptions::FancyRegex { .. } => {
                let Ok(regex) = ctx.get_or_compile_regex(item) else {
                    return Some(Err(invalid_regex(ctx, schema)));
                };
                Some(Ok(Box::new(PatternValidator {
                    regex,
                    location: ctx.location().join("pattern"),
                })))
            }
            PatternEngineOptions::Regex { .. } => {
                let Ok(regex) = ctx.get_or_compile_standard_regex(item) else {
                    return Some(Err(invalid_regex(ctx, schema)));
                };
                Some(Ok(Box::new(PatternValidator {
                    regex,
                    location: ctx.location().join("pattern"),
                })))
            }
        }
    } else {
        let location = ctx.location().join("pattern");
        Some(Err(ValidationError::single_type_error(
            location.clone(),
            location,
            Location::new(),
            schema,
            JsonType::String,
        )))
    }
}

fn invalid_regex<'a>(ctx: &compiler::Context, schema: &'a Value) -> ValidationError<'a> {
    ValidationError::format(
        ctx.location().join("pattern"),
        LazyEvaluationPath::SameAsSchemaPath,
        Location::new(),
        schema,
        "regex",
    )
}

#[cfg(test)]
mod tests {
    use crate::{tests_util, PatternOptions};
    use serde_json::json;
    use test_case::test_case;

    #[test_case("^(?!eo:)", "eo:bands", false)]
    #[test_case("^(?!eo:)", "proj:epsg", true)]
    fn negative_lookbehind_match(pattern: &str, text: &str, is_matching: bool) {
        let text = json!(text);
        let schema = json!({"pattern": pattern});
        let validator = crate::validator_for(&schema).unwrap();
        assert_eq!(validator.is_valid(&text), is_matching);
    }

    #[test]
    fn location() {
        tests_util::assert_schema_location(&json!({"pattern": "^f"}), &json!("b"), "/pattern");
    }

    #[test_case("^/", "/api/users", true)]
    #[test_case("^/", "api/users", false)]
    #[test_case("^x-", "x-custom-header", true)]
    #[test_case("^x-", "custom-header", false)]
    #[test_case("^foo", "foobar", true)]
    #[test_case("^foo", "barfoo", false)]
    fn prefix_pattern_optimization(pattern: &str, text: &str, is_matching: bool) {
        let text = json!(text);
        let schema = json!({"pattern": pattern});
        let validator = crate::validator_for(&schema).unwrap();
        assert_eq!(validator.is_valid(&text), is_matching);
    }

    #[test]
    #[ignore = "fancy-regex 0.16 no longer fails for this test case"]
    fn test_fancy_regex_backtrack_limit_exceeded() {
        let schema = json!({"pattern": "(?i)(a|b|ab)*(?=c)"});
        let validator = crate::options()
            .with_pattern_options(PatternOptions::fancy_regex().backtrack_limit(1))
            .build(&schema)
            .expect("Schema should be valid");

        let instance = json!("abababababababababababababababababababababababababababab");

        let error = validator.validate(&instance).expect_err("Should fail");
        assert_eq!(
            error.to_string(),
            "Error executing regex: Max limit for backtracking count exceeded"
        );
    }

    #[test]
    fn test_regex_engine_validation() {
        let schema = json!({"pattern": "^[a-z]+$"});
        let validator = crate::options()
            .with_pattern_options(PatternOptions::regex())
            .build(&schema)
            .expect("Schema should be valid");

        let valid = json!("hello");
        assert!(validator.is_valid(&valid));
        let invalid = json!("Hello123");
        assert!(!validator.is_valid(&invalid));
    }
}

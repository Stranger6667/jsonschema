use std::sync::Arc;

use crate::{
    compiler,
    error::ValidationError,
    keywords::CompilationResult,
    options::PatternEngineOptions,
    paths::{LazyEvaluationPath, LazyLocation, Location, RefTracker},
    regex::{
        analyze_pattern, is_ecma_whitespace, PatternOptimization, RegexEngine, RegexError,
        RegexFailureReason,
    },
    types::JsonType,
    validator::{Validate, ValidationContext},
};
use serde_json::{Map, Value};

/// Validator for patterns that are simple prefixes (optimized path).
pub(crate) struct PrefixPatternValidator {
    prefix: String,
    pattern: String,
    location: Location,
}

impl Validate for PrefixPatternValidator {
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::String(item) = instance {
            item.starts_with(&self.prefix)
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
            if !item.starts_with(&self.prefix) {
                return Err(ValidationError::pattern(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance,
                    self.pattern.clone(),
                ));
            }
        }
        Ok(())
    }
}

/// Validator for patterns that are exact-match anchored patterns.
pub(crate) struct ExactPatternValidator {
    exact: String,
    pattern: String,
    location: Location,
}

impl Validate for ExactPatternValidator {
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::String(item) = instance {
            item.as_str() == self.exact
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
            if item.as_str() != self.exact {
                return Err(ValidationError::pattern(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance,
                    self.pattern.clone(),
                ));
            }
        }
        Ok(())
    }
}

/// Validator for `^(a|b|c)$` alternation patterns (linear scan).
pub(crate) struct AlternationPatternValidator {
    alternatives: Vec<String>,
    pattern: String,
    location: Location,
}

impl Validate for AlternationPatternValidator {
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::String(item) = instance {
            self.alternatives
                .iter()
                .any(|a| a.as_str() == item.as_str())
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
            if !self
                .alternatives
                .iter()
                .any(|a| a.as_str() == item.as_str())
            {
                return Err(ValidationError::pattern(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance,
                    self.pattern.clone(),
                ));
            }
        }
        Ok(())
    }
}

/// Validator for `^\S*$` — rejects any string containing ECMA-262 whitespace.
pub(crate) struct NoWhitespacePatternValidator {
    pattern: String,
    location: Location,
}

impl Validate for NoWhitespacePatternValidator {
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::String(item) = instance {
            !item.chars().any(is_ecma_whitespace)
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
            if item.chars().any(is_ecma_whitespace) {
                return Err(ValidationError::pattern(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance,
                    self.pattern.clone(),
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
                    let pattern = self.regex.pattern().to_string();
                    let tracker = crate::paths::capture_evaluation_path(tracker, &self.location);
                    return Err(match e.into_failure_reason() {
                        RegexFailureReason::FancyRegex(error) => ValidationError::backtrack_limit(
                            self.location.clone(),
                            tracker,
                            location.into(),
                            instance,
                            error,
                        ),
                        RegexFailureReason::Panicked => ValidationError::regex_engine_failure(
                            self.location.clone(),
                            tracker,
                            location.into(),
                            instance,
                            format!("Regex engine failed to evaluate pattern '{pattern}'"),
                        ),
                    });
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
        // Try literal optimizations before compiling a full regex.
        match analyze_pattern(item) {
            Some(PatternOptimization::Exact(exact)) => {
                return Some(Ok(Box::new(ExactPatternValidator {
                    exact,
                    pattern: item.clone(),
                    location: ctx.location().join("pattern"),
                })));
            }
            Some(PatternOptimization::Prefix(prefix)) => {
                return Some(Ok(Box::new(PrefixPatternValidator {
                    prefix,
                    pattern: item.clone(),
                    location: ctx.location().join("pattern"),
                })));
            }
            Some(PatternOptimization::Alternation(alternatives)) => {
                return Some(Ok(Box::new(AlternationPatternValidator {
                    alternatives,
                    pattern: item.clone(),
                    location: ctx.location().join("pattern"),
                })));
            }
            Some(PatternOptimization::NoWhitespace) => {
                return Some(Ok(Box::new(NoWhitespacePatternValidator {
                    pattern: item.clone(),
                    location: ctx.location().join("pattern"),
                })));
            }
            None => {}
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
    #[test_case("^\\/", "/api/users", true; "escaped slash match")]
    #[test_case("^\\/", "api/users", false; "escaped slash no match")]
    fn prefix_pattern_optimization(pattern: &str, text: &str, is_matching: bool) {
        let text = json!(text);
        let schema = json!({"pattern": pattern});
        let validator = crate::validator_for(&schema).unwrap();
        assert_eq!(validator.is_valid(&text), is_matching);
    }

    #[test_case("^\\$ref$", "$ref", true; "dollar ref exact match")]
    #[test_case("^\\$ref$", "$refs", false; "dollar ref suffix no match")]
    #[test_case("^\\$ref$", "ref", false; "dollar ref no dollar no match")]
    #[test_case("^\\$ref$", "$ref_", false; "dollar ref trailing no match")]
    fn exact_pattern_optimization(pattern: &str, text: &str, is_matching: bool) {
        let text = json!(text);
        let schema = json!({"pattern": pattern});
        let validator = crate::validator_for(&schema).unwrap();
        assert_eq!(validator.is_valid(&text), is_matching);
        assert_eq!(validator.validate(&text).is_ok(), is_matching);
    }

    #[test_case(r"^(get|put|post)$", "get", true ; "alternation match get")]
    #[test_case(r"^(get|put|post)$", "put", true ; "alternation match put")]
    #[test_case(r"^(get|put|post)$", "post", true ; "alternation match post")]
    #[test_case(r"^(get|put|post)$", "patch", false ; "alternation no match")]
    #[test_case(r"^(get|put|post)$", "GET", false ; "alternation case sensitive")]
    fn alternation_pattern_optimization(pattern: &str, text: &str, is_matching: bool) {
        let text = json!(text);
        let schema = json!({"pattern": pattern});
        let validator = crate::validator_for(&schema).unwrap();
        assert_eq!(validator.is_valid(&text), is_matching);
        assert_eq!(validator.validate(&text).is_ok(), is_matching);
    }

    #[test_case(r"^\S*$", "hello", true ; "no whitespace match")]
    #[test_case(r"^\S*$", "hello world", false ; "no whitespace space fail")]
    #[test_case(r"^\S*$", "hello\tworld", false ; "no whitespace tab fail")]
    #[test_case(r"^\S*$", "", true ; "no whitespace empty string")]
    fn no_whitespace_pattern_optimization(pattern: &str, text: &str, is_matching: bool) {
        let text = json!(text);
        let schema = json!({"pattern": pattern});
        let validator = crate::validator_for(&schema).unwrap();
        assert_eq!(validator.is_valid(&text), is_matching);
        assert_eq!(validator.validate(&text).is_ok(), is_matching);
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

    // `catch_unwind` is a no-op under `panic = "abort"` (e.g. the wasm32-wasip1 default), so the
    // recovery path can't be exercised there.
    #[cfg(panic = "unwind")]
    #[test]
    fn empty_string_with_large_bounded_quantifier_fancy_regex() {
        // Recovery for https://github.com/rust-lang/regex/issues/1344.
        let schema = json!({"type": "string", "pattern": r"^.{0,404600}$"});
        let validator = crate::options()
            .with_pattern_options(PatternOptions::fancy_regex().size_limit(1_000_000_000))
            .build(&schema)
            .expect("Schema should be valid");

        assert!(validator.is_valid(&json!("x")));
        assert!(!validator.is_valid(&json!("")));
        let empty = json!("");
        let error = validator
            .validate(&empty)
            .expect_err("expected a validation error");
        assert_eq!(
            error.to_string(),
            "Regex engine failed to evaluate pattern '^.{0,404600}$'"
        );
    }

    #[test]
    fn fancy_regex_backtrack_limit_exceeded() {
        let schema = json!({"type": "string", "pattern": r"(?<=ab)c"});
        let validator = crate::options()
            .with_pattern_options(PatternOptions::fancy_regex().backtrack_limit(1))
            .build(&schema)
            .expect("Schema should be valid");

        let instance = json!("abc");
        let error = validator
            .validate(&instance)
            .expect_err("expected a validation error");
        assert!(
            matches!(
                error.kind(),
                crate::error::ValidationErrorKind::BacktrackLimitExceeded { .. }
            ),
            "expected BacktrackLimitExceeded, got {:?}",
            error.kind()
        );
        assert_eq!(
            error.to_string(),
            "Error executing regex: Max limit for backtracking count exceeded"
        );
    }

    #[cfg(panic = "unwind")]
    #[test]
    fn empty_string_with_large_bounded_quantifier_regex() {
        // Recovery for https://github.com/rust-lang/regex/issues/1344.
        let schema = json!({"type": "string", "pattern": r"^.{0,404600}$"});
        let validator = crate::options()
            .with_pattern_options(PatternOptions::regex().size_limit(1_000_000_000))
            .build(&schema)
            .expect("Schema should be valid");

        assert!(validator.is_valid(&json!("x")));
        assert!(!validator.is_valid(&json!("")));
        let empty = json!("");
        let error = validator
            .validate(&empty)
            .expect_err("expected a validation error");
        assert_eq!(
            error.to_string(),
            "Regex engine failed to evaluate pattern '^.{0,404600}$'"
        );
    }
}

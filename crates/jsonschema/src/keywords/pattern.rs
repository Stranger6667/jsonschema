use std::sync::Arc;

use crate::{
    compiler,
    error::ValidationError,
    keywords::CompilationResult,
    options::PatternEngineOptions,
    paths::{LazyEvaluationPath, LazyLocation, Location, RefTracker},
    regex::{analyze_pattern, is_ecma_whitespace, PatternOptimization, RegexEngine, RegexError},
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

    fn schema_path(&self) -> &Location {
        &self.location
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::String(_))
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

    fn schema_path(&self) -> &Location {
        &self.location
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::String(_))
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

    fn schema_path(&self) -> &Location {
        &self.location
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::String(_))
    }
}

/// Validator for `^\S*$` — rejects any string containing ECMA-262 whitespace.
pub(crate) struct NoWhitespacePatternValidator {
    pattern: String,
    location: Location,
}

impl Validate for NoWhitespacePatternValidator {
    fn schema_path(&self) -> &Location {
        &self.location
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::String(_))
    }

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
    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::String(_))
    }
    fn schema_path(&self) -> &Location {
        &self.location
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
    fn trace_prefix_pattern_has_schema_path() {
        // ^foo compiles to PrefixPatternValidator
        let schema = serde_json::json!({"pattern": "^foo"});
        let validator = crate::validator_for(&schema).unwrap();
        let instance = serde_json::json!("foobar");
        let mut schema_locations: Vec<String> = Vec::new();
        let _ = validator.trace(&instance, &mut |ctx| {
            schema_locations.push(ctx.schema_location.as_str().to_string());
        });
        assert!(
            schema_locations.iter().any(|s| s == "/pattern"),
            "expected /pattern in {schema_locations:?}"
        );
    }

    #[test]
    fn trace_exact_pattern_has_schema_path() {
        // ^\$ref$ compiles to ExactPatternValidator
        let schema = serde_json::json!({"pattern": r"^\$ref$"});
        let validator = crate::validator_for(&schema).unwrap();
        let instance = serde_json::json!("$ref");
        let mut schema_locations: Vec<String> = Vec::new();
        let _ = validator.trace(&instance, &mut |ctx| {
            schema_locations.push(ctx.schema_location.as_str().to_string());
        });
        assert!(
            schema_locations.iter().any(|s| s == "/pattern"),
            "expected /pattern in {schema_locations:?}"
        );
    }

    #[test]
    fn trace_alternation_pattern_has_schema_path() {
        // ^(get|put|post)$ compiles to AlternationPatternValidator
        let schema = serde_json::json!({"pattern": r"^(get|put|post)$"});
        let validator = crate::validator_for(&schema).unwrap();
        let instance = serde_json::json!("get");
        let mut schema_locations: Vec<String> = Vec::new();
        let _ = validator.trace(&instance, &mut |ctx| {
            schema_locations.push(ctx.schema_location.as_str().to_string());
        });
        assert!(
            schema_locations.iter().any(|s| s == "/pattern"),
            "expected /pattern in {schema_locations:?}"
        );
    }

    #[test]
    fn trace_no_whitespace_pattern_has_schema_path() {
        // NoWhitespacePatternValidator is used for hostname/uri format checks internally,
        // but can also be triggered via pattern. Use a direct no-whitespace check.
        let schema = serde_json::json!({"pattern": r"^\S*$"});
        let validator = crate::validator_for(&schema).unwrap();
        let instance = serde_json::json!("hello");
        let mut schema_locations: Vec<String> = Vec::new();
        let _ = validator.trace(&instance, &mut |ctx| {
            schema_locations.push(ctx.schema_location.as_str().to_string());
        });
        assert!(
            schema_locations.iter().any(|s| s == "/pattern"),
            "expected /pattern in {schema_locations:?}"
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

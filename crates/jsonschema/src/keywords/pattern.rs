use crate::LazyInstance;
use std::{borrow::Cow, sync::Arc};

use crate::{
    compiler,
    error::ValidationError,
    keywords::CompilationResult,
    options::PatternEngineOptions,
    paths::{LazyEvaluationPath, LazyLocation, Location, RefTracker},
    regex::{analyze_pattern, contains_ecma_whitespace, PatternOptimization, RegexEngine},
    types::JsonType,
    unfinished_matches::MatchSite,
    validator::{Validate, ValidationContext},
    Json, Node,
};
use serde_json::{Map, Value};

/// Validator for patterns that are simple prefixes (optimized path).
pub(crate) struct PrefixPatternValidator {
    prefix: String,
    pattern: String,
    location: Location,
}

impl<F: Json> Validate<F> for PrefixPatternValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(item) = instance.as_string() {
            item.starts_with(&self.prefix)
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(item) = instance.as_string() {
            if !item.starts_with(&self.prefix) {
                return Err(ValidationError::pattern(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance.lazy_value(),
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

impl<F: Json> Validate<F> for ExactPatternValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(item) = instance.as_string() {
            item.as_ref() == self.exact
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(item) = instance.as_string() {
            if item.as_ref() != self.exact {
                return Err(ValidationError::pattern(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance.lazy_value(),
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

impl<F: Json> Validate<F> for AlternationPatternValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(item) = instance.as_string() {
            self.alternatives
                .iter()
                .any(|a| a.as_str() == item.as_ref())
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(item) = instance.as_string() {
            if !self
                .alternatives
                .iter()
                .any(|a| a.as_str() == item.as_ref())
            {
                return Err(ValidationError::pattern(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance.lazy_value(),
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

impl<F: Json> Validate<F> for NoWhitespacePatternValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(item) = instance.as_string() {
            !contains_ecma_whitespace(&item)
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(item) = instance.as_string() {
            if contains_ecma_whitespace(&item) {
                return Err(ValidationError::pattern(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance.lazy_value(),
                    self.pattern.clone(),
                ));
            }
        }
        Ok(())
    }
}

pub(crate) struct PatternValidator<R> {
    regex: Arc<R>,
    /// Original schema pattern, kept for error messages. The compiled `regex` stores the
    /// ECMA->Rust translated form (e.g. `\S` expanded into a verbose class), which is unreadable.
    pattern: String,
    location: Location,
}

impl<R: RegexEngine, F: Json> Validate<F> for PatternValidator<R> {
    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(item) = instance.as_string() {
            if !self.is_match(&item, ctx, || {
                MatchSite::located(&self.location, tracker, location)
            }) {
                return Err(ValidationError::pattern(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance.lazy_value(),
                    self.pattern.clone(),
                ));
            }
        }
        Ok(())
    }

    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(item) = instance.as_string() {
            return self.is_match(&item, ctx, || MatchSite::schema(&self.location));
        }
        true
    }
}

impl<R: RegexEngine> PatternValidator<R> {
    #[inline]
    fn is_match(
        &self,
        item: &str,
        ctx: &mut ValidationContext,
        site: impl FnOnce() -> MatchSite,
    ) -> bool {
        ctx.is_match(&*self.regex, item, || site().pattern(&self.pattern))
    }
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
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
                    pattern: item.clone(),
                    location: ctx.location().join("pattern"),
                })))
            }
            PatternEngineOptions::Regex { .. } => {
                let Ok(regex) = ctx.get_or_compile_standard_regex(item) else {
                    return Some(Err(invalid_regex(ctx, schema)));
                };
                Some(Ok(Box::new(PatternValidator {
                    regex,
                    pattern: item.clone(),
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
            LazyInstance::Ready(Cow::Borrowed(schema)),
            JsonType::String,
        )))
    }
}

fn invalid_regex<'a, F: Json>(
    ctx: &compiler::Context<F>,
    schema: &'a Value,
) -> ValidationError<'a> {
    ValidationError::format(
        ctx.location().join("pattern"),
        LazyEvaluationPath::SameAsSchemaPath,
        Location::new(),
        LazyInstance::Ready(Cow::Borrowed(schema)),
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

    #[test_case(r"^[a-z&&^b]+$", "d"; "class intersection syntax is literal")]
    #[test_case(r"^[a-z--b]$", "0"; "adjacent ranges with doubled hyphen")]
    #[test_case(r"^[\w--z]$", "-"; "word class escape before doubled hyphen")]
    #[test_case(r"^[\d--z]$", "-"; "digit class escape before doubled hyphen")]
    #[test_case(r"^[a~~b]$", "~"; "class symmetric difference syntax is literal")]
    #[test_case(r"^[[a]]$", "a]"; "nested class opener is literal")]
    fn class_set_syntax(pattern: &str, text: &str) {
        let validator = crate::validator_for(&json!({"pattern": pattern})).unwrap();
        assert!(validator.is_valid(&json!(text)));
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

    #[test_case(r"^\s$", "\u{3000}", true; "space ideographic")]
    #[test_case(r"^\s$", "\u{2028}", true; "space line separator")]
    #[test_case(r"^\s$", "\u{2000}", true; "space en quad")]
    #[test_case(r"^\s$", "\u{1680}", true; "space ogham")]
    #[test_case(r"^\s$", "\u{202f}", true; "space narrow no-break")]
    #[test_case(r"^\s$", "\u{205f}", true; "space medium mathematical")]
    #[test_case(r"^\s$", "\u{0085}", false; "space next line rejected")]
    #[test_case(r"^\S$", "\u{3000}", false; "non-space ideographic")]
    #[test_case(r"^\S$", "\u{2028}", false; "non-space line separator")]
    #[test_case(r"^\S$", "\u{0085}", true; "non-space next line")]
    #[test_case(r"^[\s]$", "\u{3000}", true; "set space ideographic")]
    #[test_case(r"^[^\s]$", "\u{3000}", false; "negated set space ideographic")]
    #[test_case(r"^[^\s]$", "\u{0085}", true; "negated set next line")]
    #[test_case(r"^[a\s]$", "\u{2028}", true; "mixed set line separator")]
    fn space_class_unicode(pattern: &str, text: &str, is_matching: bool) {
        let schema = json!({"pattern": pattern});
        let text = json!(text);
        let fancy = crate::options()
            .with_pattern_options(PatternOptions::fancy_regex())
            .build(&schema)
            .unwrap();
        let regex = crate::options()
            .with_pattern_options(PatternOptions::regex())
            .build(&schema)
            .unwrap();
        for validator in [&fancy, &regex] {
            assert_eq!(validator.is_valid(&text), is_matching);
            assert_eq!(validator.validate(&text).is_ok(), is_matching);
        }
    }

    // Error messages must show the original schema pattern, not the ECMA->Rust translated form
    // (e.g. `\S` expanded into a verbose Unicode class).
    fn assert_original_pattern_in_error(validator: &crate::Validator) {
        let instance = json!("");
        let error = validator
            .validate(&instance)
            .expect_err("expected a validation error");
        assert_eq!(error.to_string(), r#""" does not match "^[\S]{1,5}$""#);
    }

    #[test]
    fn original_pattern_in_error() {
        let schema = json!({"pattern": r"^[\S]{1,5}$"});
        assert_original_pattern_in_error(
            &crate::options()
                .with_pattern_options(PatternOptions::fancy_regex())
                .build(&schema)
                .expect("Schema should be valid"),
        );
        assert_original_pattern_in_error(
            &crate::options()
                .with_pattern_options(PatternOptions::regex())
                .build(&schema)
                .expect("Schema should be valid"),
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

    #[test]
    fn malformed() {
        tests_util::assert_compile_error(
            &json!({"pattern": 5}),
            "5 is not of type \"string\"",
            "/pattern",
        );
    }

    #[test]
    fn unsupported_by_regex_engine() {
        tests_util::assert_compile_error_with(
            &crate::options().with_pattern_options(PatternOptions::regex()),
            &json!({"pattern": "(?<=a)b"}),
            "\"(?<=a)b\" is not a \"regex\"",
            "/pattern",
        );
    }
}

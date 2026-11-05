use crate::LazyInstance;
use std::{borrow::Cow, sync::Arc};

use crate::{
    compiler,
    error::ValidationError,
    keywords::CompilationResult,
    options::PatternEngineOptions,
    paths::{LazyEvaluationPath, LazyLocation, Location, RefTracker},
    regex::{
        analyze_pattern, contains_ecma_whitespace, AllMatch, Part, PatternOptimization,
        RegexEngine, SequenceMatcher,
    },
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
            if R::MATCH_OUT_OF_LINE {
                return self.is_match_out_of_line(&item, ctx);
            }
            return self.is_match(&item, ctx, || MatchSite::schema(&self.location));
        }
        true
    }
}

impl<R: RegexEngine> PatternValidator<R> {
    #[inline(never)]
    fn is_match_out_of_line(&self, item: &str, ctx: &mut ValidationContext) -> bool {
        self.is_match(item, ctx, || MatchSite::schema(&self.location))
    }

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
        if let Some(sequence) = ctx.get_or_compile_sequence(item) {
            let location = ctx.location().join("pattern");
            let options = ctx.config().pattern_options();
            if !options.has_limits() {
                return Some(Ok(Box::new(PatternValidator {
                    regex: sequence,
                    pattern: item.clone(),
                    location,
                })));
            }
            let sequence = Arc::clone(&sequence.sequence);
            return Some(match options {
                PatternEngineOptions::FancyRegex { .. } => match ctx.get_or_compile_regex(item) {
                    Ok(fallback) => Ok(Box::new(PatternValidator {
                        regex: Arc::new(SequenceMatcher { sequence, fallback }),
                        pattern: item.clone(),
                        location,
                    })),
                    Err(()) => Err(invalid_regex(ctx, schema)),
                },
                PatternEngineOptions::Regex { .. } => {
                    match ctx.get_or_compile_standard_regex(item) {
                        Ok(fallback) => Ok(Box::new(PatternValidator {
                            regex: Arc::new(SequenceMatcher { sequence, fallback }),
                            pattern: item.clone(),
                            location,
                        })),
                        Err(()) => Err(invalid_regex(ctx, schema)),
                    }
                }
            });
        }
        // Fall back to regex compilation
        match ctx.config().pattern_options() {
            PatternEngineOptions::FancyRegex { .. } => {
                if let Some(regex) = all_match(ctx, item) {
                    return Some(Ok(Box::new(PatternValidator {
                        regex: Arc::new(regex),
                        pattern: item.clone(),
                        location: ctx.location().join("pattern"),
                    })));
                }
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

/// Leading lookaheads as separate lookaround-free regexes, when every part compiles as one.
fn all_match<F: Json>(ctx: &compiler::Context<F>, pattern: &str) -> Option<AllMatch> {
    jsonschema_regex::split_leading_lookaheads(pattern)?
        .iter()
        .map(|part| match ctx.get_or_compile_sequence(part) {
            Some(sequence) => Some(Part::Sequence(sequence)),
            None => ctx
                .get_or_compile_standard_regex(part)
                .ok()
                .map(Part::Regex),
        })
        .collect::<Option<Box<[_]>>>()
        .map(AllMatch)
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
    use serde_json::{json, Value};
    use test_case::test_case;

    // Leading lookaheads must all hold at the start, then the rest must match from there.
    #[test_case(r"^(?=[^!*,;{}[\]~\n]+$)(?=(.*\w)).+$", "word", true; "class and word")]
    #[test_case(r"^(?=[^!*,;{}[\]~\n]+$)(?=(.*\w)).+$", "a,b", false; "excluded character")]
    #[test_case(r"^(?=[^!*,;{}[\]~\n]+$)(?=(.*\w)).+$", "--", false; "no word character")]
    #[test_case(r"^(?=[^!*,;{}[\]~\n]+$)(?=(.*\w)).+$", "a\nb", false; "newline")]
    #[test_case(r"^(?=[^!*,;{}[\]~\n]+$)(?=(.*\w)).+$", "", false; "empty")]
    #[test_case(r"^(?=[^!*,;{}[\]~\n]+$)(?=(.*\w)).+$", "é-1", true; "non-ascii before a digit")]
    #[test_case(r"^(?=!+[^!*,;{}[\]~\n]+$)(?=(.*\w)).+$", "!!word", true; "negated glob")]
    #[test_case(r"^(?=!+[^!*,;{}[\]~\n]+$)(?=(.*\w)).+$", "word", false; "negated glob without bang")]
    #[test_case(r"^(?=!+[^!*,;{}[\]~\n]+$)(?=(.*\w)).+$", "!!", false; "only bangs")]
    #[test_case(r"^(?=a|b)c", "c", false; "lookahead alternation fails")]
    #[test_case(r"^(?=a|c)c", "cd", true; "lookahead alternation holds")]
    #[test_case(r"^(?=.*\d)(?=.*[a-z])\w{4,}$", "ab12", true; "password style")]
    #[test_case(r"^(?=.*\d)(?=.*[a-z])\w{4,}$", "abcd", false; "password style missing digit")]
    #[test_case(r"^(?=(a))\1b", "aab", false; "backreference to a lookahead group")]
    #[test_case(r"^(?=(a))\1b", "ab", true; "backreference to a lookahead group holds")]
    #[test_case(r"^(?=.{1,262144}$)[a-z]", "ab", true; "huge count in a lookahead")]
    #[test_case(r"^(?=.{1,262144}$)[a-z]", "a\nb", false; "huge count in a lookahead on a line feed")]
    #[test_case(r"^(?=.*[A-Z])(?=.*\d)[\s\S]{8,204800}$", "Abcdefg1", true; "huge count after lookaheads")]
    #[test_case(r"^(?=.*[A-Z])(?=.*\d)[\s\S]{8,204800}$", "Abcdef1", false; "huge count after lookaheads under min")]
    fn leading_lookaheads_match(pattern: &str, text: &str, is_matching: bool) {
        let schema = json!({"pattern": pattern});
        let validator = crate::validator_for(&schema).unwrap();
        assert_eq!(validator.is_valid(&json!(text)), is_matching);
        assert_eq!(validator.validate(&json!(text)).is_ok(), is_matching);
    }

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
    #[test_case(r"^.$", "\r", false; "dot carriage return")]
    #[test_case(r"^.$", "\u{2028}", false; "dot line separator")]
    #[test_case(r"^.$", "\u{2029}", false; "dot paragraph separator")]
    #[test_case(r"^a.b$", "a\u{85}b", true; "dot next line")]
    #[test_case(r"^[.]$", "\r", false; "dot in set is literal")]
    fn unicode_classes(pattern: &str, text: &str, is_matching: bool) {
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

    // The engine unrolls a bounded count into as many copies of the class, past the default size
    // limit; anchored at both ends the count is checked by counting characters, and at one edge
    // it collapses to its minimum, which matches the same strings.
    #[test_case(r"^.{0,262144}$", "", true; "zero minimum on empty")]
    #[test_case(r"^.{0,262144}$", "a\nb", false; "line feed under dot")]
    #[test_case(r"^.{0,262144}$", "a\u{85}b", true; "next line under dot")]
    #[test_case(r"^[\s\S]{1,204800}$", "a\nb", true; "every character")]
    #[test_case(r"^[\s\S]{1,204800}$", "", false; "every character under min")]
    #[test_case(r"^[\S\s]{0,2147483647}$", "x", true; "largest count")]
    #[test_case(r"^[\x09\x0a\x0d -ÿ]{1,2097152}$", "ÿ\t", true; "range class at its last character")]
    #[test_case(r"^[\x09\x0a\x0d -ÿ]{1,2097152}$", "Ā", false; "range class past its last character")]
    #[test_case(r"[^\p{C}]{1,2048}", "\u{1}a", true; "unanchored count collapses")]
    #[test_case(r"[^\p{C}]{1,2048}", "\u{1}\n", false; "unanchored count rejects controls")]
    #[test_case(r"^.{1,2097152}", "", false; "anchored start on empty")]
    #[test_case(r"^.{1,2097152}", "\na", false; "anchored start on a line feed")]
    #[test_case(r"^.{1,2097152}", "a", true; "anchored start on a letter")]
    #[test_case(r"^(.){0,262144}$", "ab", true; "capture group around the class")]
    #[test_case(r"^(.{0,262144})$", "a\nb", false; "capture group around the count")]
    fn huge_counts_at_default_limits(pattern: &str, text: &str, is_matching: bool) {
        let schema = json!({"pattern": pattern});
        let text = json!(text);
        let fancy = crate::options()
            .with_pattern_options(PatternOptions::fancy_regex())
            .build(&schema)
            .expect("builds within the default size limit");
        let regex = crate::options()
            .with_pattern_options(PatternOptions::regex())
            .build(&schema)
            .expect("builds within the default size limit");
        for validator in [&fancy, &regex] {
            assert_eq!(validator.is_valid(&text), is_matching);
            assert_eq!(validator.validate(&text).is_ok(), is_matching);
        }
    }

    // A zero minimum drops its atom from the search, but the engine still rejects the atom.
    #[test_case(r"^[\p{Lettr}]{0,64}"; "unknown property in a class")]
    #[test_case(r"x\p{Bogus}{0,3}"; "unknown property")]
    fn unknown_property_under_a_zero_minimum(pattern: &str) {
        let schema = json!({"pattern": pattern});
        assert!(crate::options()
            .with_pattern_options(PatternOptions::fancy_regex())
            .build(&schema)
            .is_err());
        assert!(crate::options()
            .with_pattern_options(PatternOptions::regex())
            .build(&schema)
            .is_err());
    }

    #[test]
    fn huge_count_error_shows_the_pattern() {
        let validator = crate::validator_for(&json!({"pattern": r"^.{0,262144}$"})).unwrap();
        let instance = json!("a\n");
        let error = validator.validate(&instance).expect_err("line feed");
        assert_eq!(error.to_string(), r#""a\n" does not match "^.{0,262144}$""#);
        assert_eq!(error.schema_path().as_str(), "/pattern");
    }

    // Patterns answered without the regex engine: a character count, an ECMA whitespace scan and
    // a list of alternatives. `evaluate` reaches the keyword whatever the instance, so all three
    // entry points must agree on every instance type.
    #[test_case(r"^.{0,262144}$", &json!(42), true; "repeated class accepts numbers")]
    #[test_case(r"^.{0,262144}$", &json!("ab"), true; "repeated class accepts a string in the class")]
    #[test_case(r"^.{0,262144}$", &json!("a\n"), false; "repeated class rejects a line feed")]
    #[test_case(r"^\S*$", &json!(null), true; "no whitespace accepts null")]
    #[test_case(r"^\S*$", &json!("ab"), true; "no whitespace accepts a string without spaces")]
    #[test_case(r"^\S*$", &json!("a\u{a0}b"), false; "no whitespace rejects a no-break space")]
    #[test_case(r"^(ab|cd)$", &json!(["ab"]), true; "alternation accepts arrays")]
    #[test_case(r"^(ab|cd)$", &json!("cd"), true; "alternation accepts a listed string")]
    #[test_case(r"^(ab|cd)$", &json!("abcd"), false; "alternation rejects an unlisted string")]
    fn patterns_without_the_engine_agree_across_entry_points(
        pattern: &str,
        instance: &Value,
        expected: bool,
    ) {
        let validator = crate::validator_for(&json!({"pattern": pattern})).unwrap();
        assert_eq!(
            (
                validator.is_valid(instance),
                validator.validate(instance).is_ok(),
                validator.evaluate(instance).flag().valid,
            ),
            (expected, expected, expected)
        );
    }

    // Error messages must show the original schema pattern, not the ECMA->Rust translated form
    // (e.g. `\S` expanded into a verbose Unicode class).
    fn assert_original_pattern_in_error(validator: &crate::Validator) {
        let instance = json!("");
        let error = validator
            .validate(&instance)
            .expect_err("expected a validation error");
        assert_eq!(error.to_string(), r#""" does not match "^[\S]+$""#);
    }

    #[test]
    fn original_pattern_in_error() {
        let schema = json!({"pattern": r"^[\S]+$"});
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
        let schema = json!({"type": "string", "pattern": r"^x?.{0,404600}$"});
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
            "Regex engine failed to evaluate pattern '^x?.{0,404600}$'"
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
        let schema = json!({"type": "string", "pattern": r"^x?.{0,404600}$"});
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
            "Regex engine failed to evaluate pattern '^x?.{0,404600}$'"
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

    fn build_with_cache(
        cache: &crate::PatternCache,
        options: PatternOptions<crate::FancyRegex>,
        schema: &Value,
    ) -> bool {
        crate::options()
            .with_pattern_options(options)
            .with_pattern_cache(cache.clone())
            .build(schema)
            .is_ok()
    }

    // One pattern under `pattern`, a `properties` subschema and `patternProperties`, each built
    // with `cache` and given an instance it accepts; `cached` reads what the build left in `cache`.
    fn build_three_ways<T>(
        cache: &crate::PatternCache,
        pattern: &str,
        cached: impl Fn() -> std::sync::Arc<T>,
    ) -> Vec<std::sync::Arc<T>> {
        let mut compiled = Vec::new();
        for schema in [
            json!({"pattern": pattern}),
            json!({"properties": {"name": {"pattern": pattern}}}),
            json!({"patternProperties": {(pattern): {"type": "string"}}}),
        ] {
            let validator = crate::options()
                .with_pattern_cache(cache.clone())
                .build(&schema)
                .expect("valid schema");
            let instance = if schema.get("pattern").is_some() {
                json!("abx")
            } else {
                json!({"name": "abx", "abx": "s"})
            };
            assert!(validator.is_valid(&instance));
            compiled.push(cached());
        }
        compiled
    }

    #[test]
    fn shared_pattern_cache_compiles_each_pattern_once() {
        let cache = crate::PatternCache::new();
        let pattern = "[a-c]+x$";
        let compiled = build_three_ways(&cache, pattern, || {
            cache
                .fancy(crate::options::PatternEngineOptions::default(), pattern)
                .expect("cached")
                .expect("compiled")
        });
        assert!(compiled
            .windows(2)
            .all(|pair| std::sync::Arc::ptr_eq(&pair[0], &pair[1])));
        assert_eq!(cache.len(), 1);
        // Later builds skip the sequence attempt too.
        assert!(cache.sequence(pattern).expect("attempted").is_err());
    }

    #[test]
    fn shared_pattern_cache_compiles_each_sequence_once() {
        let cache = crate::PatternCache::new();
        let pattern = "^[a-c]+x$";
        let compiled = build_three_ways(&cache, pattern, || {
            cache
                .sequence(pattern)
                .expect("cached")
                .expect("a sequence")
        });
        assert!(compiled
            .windows(2)
            .all(|pair| std::sync::Arc::ptr_eq(&pair[0], &pair[1])));
        // The engine is built only for a subject the sequence cannot decide.
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn shared_pattern_cache_keeps_failed_compiles() {
        let cache = crate::PatternCache::new();
        let tiny = || PatternOptions::fancy_regex().size_limit(10);
        let schema = json!({"pattern": "^[a-c]{1,1000}x$"});
        assert!(!build_with_cache(&cache, tiny(), &schema));
        assert!(!build_with_cache(&cache, tiny(), &schema));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn shared_pattern_cache_reports_cached_failure_without_compiling() {
        let cache = crate::PatternCache::new();
        cache.insert_fancy(
            crate::options::PatternEngineOptions::default(),
            "[a-c]+x$",
            Err(()),
        );
        assert!(!build_with_cache(
            &cache,
            PatternOptions::fancy_regex(),
            &json!({"pattern": "[a-c]+x$"})
        ));
    }

    #[test]
    fn shared_pattern_cache_is_keyed_by_engine_options() {
        let cache = crate::PatternCache::new();
        let schema = json!({"pattern": "^[a-c]{1,1000}x$"});
        assert!(!build_with_cache(
            &cache,
            PatternOptions::fancy_regex().size_limit(10),
            &schema
        ));
        assert!(build_with_cache(
            &cache,
            PatternOptions::fancy_regex(),
            &schema
        ));
        let validator = crate::options()
            .with_pattern_options(PatternOptions::regex())
            .with_pattern_cache(cache.clone())
            .build(&schema)
            .expect("valid schema");
        assert!(validator.is_valid(&json!("abx")));
        assert!(!validator.is_valid(&json!("x")));
        assert_eq!(cache.len(), 3);
    }
}

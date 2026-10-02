pub(crate) trait RegexEngine: Sized + Send + Sync {
    type Error: RegexError;
    fn is_match(&self, text: &str) -> Result<bool, Self::Error>;
}

/// Reason a regex match failed, distinguishing real engine errors from recovered panics.
#[derive(Debug)]
pub(crate) enum RegexFailureReason {
    /// Real `fancy-regex` runtime error (e.g., the configured backtrack limit was hit).
    FancyRegex(fancy_regex::Error),
    /// Engine panicked during matching and was recovered via `catch_unwind`.
    Panicked,
}

pub(crate) trait RegexError {
    /// The compiled pattern that failed.
    fn pattern(&self) -> &str;
    fn into_failure_reason(self) -> RegexFailureReason;
}

/// Failure mode for the `fancy-regex` backend: either a real engine error or a recovered panic.
#[derive(Debug)]
pub(crate) enum FancyRegexError {
    Engine {
        error: fancy_regex::Error,
        pattern: Box<str>,
    },
    Panicked {
        pattern: Box<str>,
    },
}

impl RegexError for FancyRegexError {
    fn pattern(&self) -> &str {
        match self {
            Self::Engine { pattern, .. } | Self::Panicked { pattern } => pattern,
        }
    }

    fn into_failure_reason(self) -> RegexFailureReason {
        match self {
            Self::Engine { error, .. } => RegexFailureReason::FancyRegex(error),
            Self::Panicked { .. } => RegexFailureReason::Panicked,
        }
    }
}

impl RegexEngine for fancy_regex::Regex {
    type Error = FancyRegexError;

    fn is_match(&self, text: &str) -> Result<bool, Self::Error> {
        // `regex-automata` 0.4 panics on some patterns (https://github.com/rust-lang/regex/issues/1344); catch to surface a regular error instead of aborting the host process.
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fancy_regex::Regex::is_match(self, text)
        })) {
            Ok(Ok(matched)) => Ok(matched),
            Ok(Err(error)) => Err(FancyRegexError::Engine {
                error,
                pattern: self.as_str().into(),
            }),
            Err(_) => Err(FancyRegexError::Panicked {
                pattern: self.as_str().into(),
            }),
        }
    }
}

/// Marker error for `regex::Regex::is_match` panics. The underlying `is_match` is otherwise infallible.
#[derive(Debug)]
pub(crate) struct RegexBackendPanic {
    pattern: Box<str>,
}

impl RegexError for RegexBackendPanic {
    fn pattern(&self) -> &str {
        &self.pattern
    }

    fn into_failure_reason(self) -> RegexFailureReason {
        RegexFailureReason::Panicked
    }
}

impl RegexEngine for regex::Regex {
    type Error = RegexBackendPanic;

    fn is_match(&self, text: &str) -> Result<bool, Self::Error> {
        // Same panic as fancy-regex (https://github.com/rust-lang/regex/issues/1344); see that impl for context.
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            regex::Regex::is_match(self, text)
        }))
        .map_err(|_| RegexBackendPanic {
            pattern: self.as_str().into(),
        })
    }
}

/// Uninhabited error type for literal matchers — matching never fails.
#[derive(Debug)]
pub(crate) enum LiteralMatchError {}

impl RegexError for LiteralMatchError {
    fn pattern(&self) -> &str {
        match *self {}
    }

    fn into_failure_reason(self) -> RegexFailureReason {
        match self {}
    }
}

/// [`RegexEngine`] for literal patterns — either `starts_with` (prefix) or `==` (exact).
pub(crate) enum LiteralMatcher {
    Prefix {
        literal: String,
    },
    Exact {
        exact: String,
    },
    /// `^(a|b|c)$` — linear scan over a small sorted array.
    Alternation {
        alternatives: Vec<String>,
    },
    /// `^\S*$` — no ECMA-262 whitespace characters.
    NoWhitespace,
}

impl RegexEngine for LiteralMatcher {
    type Error = LiteralMatchError;

    #[inline]
    fn is_match(&self, text: &str) -> Result<bool, Self::Error> {
        match self {
            Self::Prefix { literal } => Ok(text.starts_with(literal.as_str())),
            Self::Exact { exact } => Ok(text == exact.as_str()),
            Self::Alternation { alternatives } => {
                Ok(alternatives.iter().any(|a| a.as_str() == text))
            }
            Self::NoWhitespace => Ok(!contains_ecma_whitespace(text)),
        }
    }
}

/// Result of analyzing a regex pattern for literal-match optimizations.
#[derive(Debug, PartialEq)]
pub(crate) enum PatternOptimization {
    /// `^prefix` — use `starts_with(prefix)`.
    Prefix(String),
    /// `^exact$` — use `== exact`.
    Exact(String),
    /// `^(a|b|c)$` — linear scan over a small sorted array.
    Alternation(Vec<String>),
    /// `^\S*$` — no ECMA-262 whitespace characters.
    NoWhitespace,
}

pub(crate) use jsonschema_regex::contains_ecma_whitespace;

/// Build a fancy-regex matcher, applying engine limits; `Err(())` on a rejected pattern.
pub(crate) fn build_fancy_regex(
    translated: &str,
    backtrack_limit: Option<usize>,
    size_limit: Option<usize>,
    dfa_size_limit: Option<usize>,
) -> Result<fancy_regex::Regex, ()> {
    let mut builder = fancy_regex::RegexBuilder::new(translated);
    if let Some(limit) = backtrack_limit {
        builder.backtrack_limit(limit);
    }
    if let Some(limit) = size_limit {
        builder.delegate_size_limit(limit);
    }
    if let Some(limit) = dfa_size_limit {
        builder.delegate_dfa_size_limit(limit);
    }
    builder.build().map_err(|_| ())
}

/// Build a standard-regex matcher, applying engine limits; `Err(())` on a rejected pattern.
pub(crate) fn build_standard_regex(
    translated: &str,
    size_limit: Option<usize>,
    dfa_size_limit: Option<usize>,
) -> Result<regex::Regex, ()> {
    let mut builder = regex::RegexBuilder::new(translated);
    if let Some(limit) = size_limit {
        builder.size_limit(limit);
    }
    if let Some(limit) = dfa_size_limit {
        builder.dfa_size_limit(limit);
    }
    builder.build().map_err(|_| ())
}

/// Analyze a pattern and return a [`PatternOptimization`] if one applies, or `None` if a full
/// regex engine is required. Shares [`jsonschema_regex::analyze_pattern`] with the codegen backend.
pub(crate) fn analyze_pattern(pattern: &str) -> Option<PatternOptimization> {
    Some(match jsonschema_regex::analyze_pattern(pattern)? {
        jsonschema_regex::PatternAnalysis::Prefix(prefix) => {
            PatternOptimization::Prefix(prefix.into_owned())
        }
        jsonschema_regex::PatternAnalysis::Exact(exact) => {
            PatternOptimization::Exact(exact.into_owned())
        }
        jsonschema_regex::PatternAnalysis::Alternation(alternatives) => {
            PatternOptimization::Alternation(alternatives)
        }
        jsonschema_regex::PatternAnalysis::NoWhitespace => PatternOptimization::NoWhitespace,
    })
}

#[cfg(test)]
mod tests {
    use super::{analyze_pattern, PatternOptimization};
    use crate::PatternOptions;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(r"^\S*$", Some(PatternOptimization::NoWhitespace) ; "no whitespace sentinel")]
    #[test_case(
        r"^(get|put|post)$",
        Some(PatternOptimization::Alternation(vec!["get".into(), "post".into(), "put".into()])) ;
        "sorted alternation"
    )]
    #[test_case(r"^(a|b|c^)$", None ; "invalid char in alternative")]
    #[test_case(r"^(x-foo|x-bar)$", Some(PatternOptimization::Alternation(vec!["x-bar".into(), "x-foo".into()])) ; "alternation with dash")]
    #[test_case(r"^(single)$", Some(PatternOptimization::Alternation(vec!["single".into()])) ; "single alternative")]
    #[allow(clippy::needless_pass_by_value)]
    fn test_analyze_pattern_new(pattern: &str, expected: Option<PatternOptimization>) {
        assert_eq!(analyze_pattern(pattern), expected);
    }

    // With `backtrack_limit(1)` the engine cannot finish matching these against "abc":
    // `(?<=ab)c` would match it and `(?<=x)c` would not.
    const MATCHES: &str = "(?<=ab)c";
    const MISSES: &str = "(?<=x)c";
    const UNDECIDED: &str = "Error executing regex: Max limit for backtracking count exceeded";

    type Outcomes = (bool, Result<(), String>, Vec<String>, (bool, Vec<String>));

    fn outcomes(schema: &Value, instance: &Value) -> Outcomes {
        let validator = crate::options()
            .with_pattern_options(PatternOptions::fancy_regex().backtrack_limit(1))
            .build(schema)
            .expect("schema compiles");
        (
            validator.is_valid(instance),
            validator
                .validate(instance)
                .map_err(|error| error.to_string()),
            validator
                .iter_errors(instance)
                .map(|error| error.to_string())
                .collect(),
            {
                let evaluation = validator.evaluate(instance);
                (
                    evaluation.is_valid(),
                    evaluation
                        .iter_errors()
                        .map(|entry| entry.error.to_string())
                        .collect(),
                )
            },
        )
    }

    fn undecided() -> Outcomes {
        (
            false,
            Err(UNDECIDED.to_owned()),
            vec![UNDECIDED.to_owned()],
            (false, vec![UNDECIDED.to_owned()]),
        )
    }

    fn valid() -> Outcomes {
        (true, Ok(()), Vec::new(), (true, Vec::new()))
    }

    #[test_case(json!({"not": {"pattern": MATCHES}}), json!("abc") ; "not over a match")]
    #[test_case(json!({"not": {"pattern": MISSES}}), json!("abc") ; "not over a miss")]
    #[test_case(json!({"if": {"pattern": MATCHES}, "then": {"maxLength": 1}}), json!("abc") ; "if picks the branch")]
    #[test_case(json!({"oneOf": [{"pattern": MATCHES}, {"type": "string"}]}), json!("abc") ; "oneOf counts the branch")]
    #[test_case(json!({"patternProperties": {MATCHES: {"type": "integer"}}}), json!({"abc": "x"}) ; "patternProperties applies the subschema")]
    #[test_case(
        json!({"patternProperties": {MATCHES: true}, "additionalProperties": false}),
        json!({"abc": 1}) ;
        "additionalProperties excludes the key"
    )]
    #[test_case(
        json!({"patternProperties": {MATCHES: true}, "unevaluatedProperties": false}),
        json!({"abc": 1}) ;
        "unevaluatedProperties excludes the key"
    )]
    #[test_case(
        json!({"properties": {"abc": true}, "patternProperties": {MATCHES: {"type": "integer"}}, "additionalProperties": false}),
        json!({"abc": "x"}) ;
        "patternProperties over a declared property with additionalProperties false"
    )]
    #[test_case(
        json!({"properties": {"abc": true}, "patternProperties": {MATCHES: {"type": "integer"}}, "additionalProperties": {}}),
        json!({"abc": "x"}) ;
        "patternProperties over a declared property with additionalProperties schema"
    )]
    #[test_case(
        json!({"patternProperties": {MATCHES: {"type": "integer"}, "^z": true}}),
        json!({"abc": "x"}) ;
        "patternProperties with several patterns"
    )]
    #[test_case(
        json!({"patternProperties": {MATCHES: {"type": "integer"}}, "additionalProperties": {"type": "string"}}),
        json!({"abc": 1}) ;
        "additionalProperties schema"
    )]
    #[test_case(
        json!({
            "properties": {"q": true},
            "patternProperties": {MATCHES: {"type": "integer"}},
            "additionalProperties": {"type": "string"}
        }),
        json!({"abc": 1}) ;
        "undeclared property with additionalProperties schema"
    )]
    #[test_case(
        json!({"properties": {"q": true}, "patternProperties": {MATCHES: true}, "additionalProperties": false}),
        json!({"abc": 1}) ;
        "undeclared property with additionalProperties false"
    )]
    #[allow(clippy::needless_pass_by_value)]
    fn unfinished_match_that_decides_the_outcome_is_an_error(schema: Value, instance: Value) {
        assert_eq!(outcomes(&schema, &instance), undecided());
    }

    #[test_case(json!({"anyOf": [{"pattern": MATCHES}, {"type": "string"}]}) ; "anyOf with an accepting branch")]
    #[test_case(json!({"anyOf": [{"pattern": MATCHES}, {"not": {"pattern": MATCHES}}]}) ; "anyOf over a match and its negation")]
    #[test_case(
        json!({"if": {"pattern": MATCHES}, "then": {"minLength": 1}, "else": {"minLength": 1}}) ;
        "if with equal branches"
    )]
    #[allow(clippy::needless_pass_by_value)]
    fn unfinished_match_that_does_not_decide_the_outcome_is_ignored(schema: Value) {
        assert_eq!(outcomes(&schema, &json!("abc")), valid());
    }

    #[test]
    fn unfinished_match_keeps_the_errors_every_outcome_shares() {
        let schema = json!({"pattern": MATCHES, "maxLength": 1});
        let error = r#""abc" is longer than 1 character"#.to_owned();
        assert_eq!(
            outcomes(&schema, &json!("abc")),
            (
                false,
                Err(error.clone()),
                vec![error.clone()],
                (false, vec![error])
            )
        );
    }

    #[test]
    fn too_many_unfinished_matches_is_an_error() {
        let branches: Vec<Value> = "defghijk"
            .chars()
            .map(|letter| json!({"anyOf": [{"pattern": format!("(?<={letter})c")}, {"type": "string"}]}))
            .collect();
        assert_eq!(
            outcomes(&json!({"allOf": branches}), &json!("abc")),
            undecided()
        );
    }

    #[test_case(
        json!({"properties": {"x": {"pattern": MATCHES}}}),
        json!({"x": "abc"}),
        ("/properties/x/pattern", "/properties/x/pattern", "/x", json!("abc")) ;
        "pattern"
    )]
    #[test_case(
        json!({"$defs": {"s": {"pattern": MATCHES}}, "properties": {"x": {"$ref": "#/$defs/s"}}}),
        json!({"x": "abc"}),
        ("/$defs/s/pattern", "/properties/x/$ref/pattern", "/x", json!("abc")) ;
        "pattern behind a reference"
    )]
    #[test_case(
        json!({"properties": {"o": {"patternProperties": {MATCHES: {"type": "integer"}}}}}),
        json!({"o": {"abc": "x"}}),
        (
            "/properties/o/patternProperties/(?<=ab)c",
            "/properties/o/patternProperties/(?<=ab)c",
            "/o",
            json!({"abc": "x"}),
        ) ;
        "patternProperties"
    )]
    #[test_case(
        json!({"properties": {"o": {"patternProperties": {MATCHES: true}, "additionalProperties": false}}}),
        json!({"o": {"abc": 1}}),
        (
            "/properties/o/patternProperties/(?<=ab)c",
            "/properties/o/patternProperties/(?<=ab)c",
            "/o",
            json!({"abc": 1}),
        ) ;
        "additionalProperties"
    )]
    #[test_case(
        json!({"properties": {"o": {"patternProperties": {MATCHES: true}, "unevaluatedProperties": false}}}),
        json!({"o": {"abc": 1}}),
        (
            "/properties/o/patternProperties/(?<=ab)c",
            "/properties/o/patternProperties/(?<=ab)c",
            "/o",
            json!({"abc": 1}),
        ) ;
        "unevaluatedProperties"
    )]
    #[allow(clippy::needless_pass_by_value)]
    fn unfinished_match_error_points_at_the_match(
        schema: Value,
        instance: Value,
        expected: (&str, &str, &str, Value),
    ) {
        let validator = crate::options()
            .with_pattern_options(PatternOptions::fancy_regex().backtrack_limit(1))
            .build(&schema)
            .expect("schema compiles");
        let error = validator.validate(&instance).expect_err("undecided");
        assert_eq!(
            (
                error.schema_path().to_string(),
                error.evaluation_path().to_string(),
                error.instance_path().to_string(),
                error.instance().clone().into_owned(),
                error.to_string(),
            ),
            (
                expected.0.to_owned(),
                expected.1.to_owned(),
                expected.2.to_owned(),
                expected.3,
                UNDECIDED.to_owned(),
            )
        );
    }
}

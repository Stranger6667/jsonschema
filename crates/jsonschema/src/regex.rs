use std::sync::Arc;

pub(crate) trait RegexEngine: Sized + Send + Sync {
    type Error: RegexError;
    fn is_match(&self, text: &str) -> Result<bool, Self::Error>;
    /// The compiled pattern its errors name, for engines that can fail.
    fn pattern(&self) -> Option<&str> {
        None
    }
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

    fn pattern(&self) -> Option<&str> {
        Some(self.as_str())
    }

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

    fn pattern(&self) -> Option<&str> {
        Some(self.as_str())
    }

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

/// Matches when every part does: the parts [`jsonschema_regex::split_leading_lookaheads`] makes
/// of a pattern, each compiled without lookaround support.
pub(crate) struct AllMatch(pub(crate) Box<[Arc<regex::Regex>]>);

impl RegexEngine for AllMatch {
    type Error = RegexBackendPanic;

    #[inline]
    fn is_match(&self, text: &str) -> Result<bool, Self::Error> {
        for part in &self.0 {
            if !RegexEngine::is_match(&**part, text)? {
                return Ok(false);
            }
        }
        Ok(true)
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
#[derive(Clone)]
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

impl From<PatternOptimization> for LiteralMatcher {
    fn from(optimization: PatternOptimization) -> Self {
        match optimization {
            PatternOptimization::Prefix(literal) => LiteralMatcher::Prefix { literal },
            PatternOptimization::Exact(exact) => LiteralMatcher::Exact { exact },
            PatternOptimization::Alternation(alternatives) => {
                LiteralMatcher::Alternation { alternatives }
            }
            PatternOptimization::NoWhitespace => LiteralMatcher::NoWhitespace,
        }
    }
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
}

use std::sync::{Arc, OnceLock};

pub(crate) trait RegexEngine: Sized + Send + Sync {
    type Error: RegexError;
    /// Whether callers keep the match out of line. An inlined sequence match costs the callers'
    /// non-string exits their prologue-free path.
    const MATCH_OUT_OF_LINE: bool = false;
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

/// A compiled `pattern`: characters counted for `^X{m,n}$` over one class, the engine otherwise.
#[derive(Debug, Clone)]
pub(crate) enum Compiled<R> {
    RepeatedClass(RepeatedClass),
    Engine(R),
}

impl<R: RegexEngine> RegexEngine for Compiled<R> {
    type Error = R::Error;

    #[inline]
    fn is_match(&self, text: &str) -> Result<bool, Self::Error> {
        match self {
            Self::RepeatedClass(class) => Ok(class.is_match(text)),
            Self::Engine(regex) => regex.is_match(text),
        }
    }

    fn pattern(&self) -> Option<&str> {
        match self {
            Self::RepeatedClass(_) => None,
            Self::Engine(regex) => regex.pattern(),
        }
    }
}

impl<R: RegexEngine> RegexEngine for Arc<R> {
    type Error = R::Error;

    fn pattern(&self) -> Option<&str> {
        (**self).pattern()
    }

    #[inline]
    fn is_match(&self, text: &str) -> Result<bool, Self::Error> {
        (**self).is_match(text)
    }
}

/// A [`jsonschema_regex::Sequence`], with `fallback` deciding the subjects it cannot.
#[derive(Debug)]
pub(crate) struct SequenceMatcher<R> {
    pub(crate) sequence: Arc<jsonschema_regex::Sequence>,
    pub(crate) fallback: R,
}

impl<R: RegexEngine> RegexEngine for SequenceMatcher<R> {
    type Error = R::Error;
    const MATCH_OUT_OF_LINE: bool = true;

    fn pattern(&self) -> Option<&str> {
        self.fallback.pattern()
    }

    #[inline]
    fn is_match(&self, text: &str) -> Result<bool, Self::Error> {
        match self.sequence.is_match(text.as_bytes()) {
            Some(matched) => Ok(matched),
            None => self.fallback.is_match(text),
        }
    }
}

impl From<RegexBackendPanic> for FancyRegexError {
    fn from(panic: RegexBackendPanic) -> Self {
        Self::Panicked {
            pattern: panic.pattern,
        }
    }
}

/// A `regex::Regex` built on its first match. Only for patterns `Sequence::compile` accepts, whose
/// size budget keeps them far below the engine's default size limit.
#[derive(Debug)]
pub(crate) struct LazyRegex {
    translated: Arc<str>,
    regex: OnceLock<regex::Regex>,
}

impl LazyRegex {
    pub(crate) fn new(translated: Arc<str>) -> Self {
        Self {
            translated,
            regex: OnceLock::new(),
        }
    }
}

impl RegexEngine for LazyRegex {
    type Error = RegexBackendPanic;

    fn pattern(&self) -> Option<&str> {
        Some(&self.translated)
    }

    fn is_match(&self, text: &str) -> Result<bool, Self::Error> {
        let regex = self.regex.get_or_init(|| {
            regex::Regex::new(&self.translated).expect("sequence patterns build without limits")
        });
        RegexEngine::is_match(regex, text)
    }
}

pub(crate) enum Part {
    Sequence(Arc<SequenceMatcher<LazyRegex>>),
    Regex(Arc<Compiled<regex::Regex>>),
}

/// Matches when every part does: the parts [`jsonschema_regex::split_leading_lookaheads`] makes
/// of a pattern, each compiled without lookaround support.
pub(crate) struct AllMatch(pub(crate) Box<[Part]>);

impl RegexEngine for AllMatch {
    type Error = RegexBackendPanic;

    #[inline]
    fn is_match(&self, text: &str) -> Result<bool, Self::Error> {
        for part in &self.0 {
            let matched = match part {
                Part::Sequence(sequence) => sequence.is_match(text)?,
                Part::Regex(regex) => RegexEngine::is_match(&**regex, text)?,
            };
            if !matched {
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
use jsonschema_regex::{RepeatedClass, Search};

/// `pattern`, translated as `translated`, as a character count or compiled by `build`.
fn compile<R>(
    pattern: &str,
    translated: &str,
    build: impl FnOnce(&str) -> Result<R, ()>,
) -> Result<Compiled<R>, ()> {
    match jsonschema_regex::search_form(pattern, translated) {
        Some(Search::RepeatedClass(class)) => Ok(Compiled::RepeatedClass(class)),
        Some(Search::Collapsed(collapsed)) => build(&collapsed).map(Compiled::Engine),
        None => build(translated).map(Compiled::Engine),
    }
}

/// Build a fancy-regex matcher for `pattern`, translated as `translated`, applying engine limits;
/// `Err(())` on a rejected pattern.
pub(crate) fn build_fancy_regex(
    pattern: &str,
    translated: &str,
    backtrack_limit: Option<usize>,
    size_limit: Option<usize>,
    dfa_size_limit: Option<usize>,
) -> Result<Compiled<fancy_regex::Regex>, ()> {
    compile(pattern, translated, |translated| {
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
    })
}

/// Build a standard-regex matcher for `pattern`, translated as `translated`, applying engine
/// limits; `Err(())` on a rejected pattern.
pub(crate) fn build_standard_regex(
    pattern: &str,
    translated: &str,
    size_limit: Option<usize>,
    dfa_size_limit: Option<usize>,
) -> Result<Compiled<regex::Regex>, ()> {
    compile(pattern, translated, |translated| {
        let mut builder = regex::RegexBuilder::new(translated);
        if let Some(limit) = size_limit {
            builder.size_limit(limit);
        }
        if let Some(limit) = dfa_size_limit {
            builder.dfa_size_limit(limit);
        }
        builder.build().map_err(|_| ())
    })
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

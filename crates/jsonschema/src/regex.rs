pub(crate) trait RegexEngine: Sized + Send + Sync {
    type Error: RegexError;
    fn is_match(&self, text: &str) -> Result<bool, Self::Error>;

    fn pattern(&self) -> &str;
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
    fn into_failure_reason(self) -> RegexFailureReason;
}

/// Failure mode for the `fancy-regex` backend: either a real engine error or a recovered panic.
#[derive(Debug)]
pub(crate) enum FancyRegexError {
    Engine(fancy_regex::Error),
    Panicked,
}

impl RegexError for FancyRegexError {
    fn into_failure_reason(self) -> RegexFailureReason {
        match self {
            Self::Engine(e) => RegexFailureReason::FancyRegex(e),
            Self::Panicked => RegexFailureReason::Panicked,
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
            Ok(Err(e)) => Err(FancyRegexError::Engine(e)),
            Err(_) => Err(FancyRegexError::Panicked),
        }
    }

    fn pattern(&self) -> &str {
        self.as_str()
    }
}

/// Marker error for `regex::Regex::is_match` panics. The underlying `is_match` is otherwise infallible.
#[derive(Debug)]
pub(crate) struct RegexBackendPanic;

impl RegexError for RegexBackendPanic {
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
        .map_err(|_| RegexBackendPanic)
    }

    fn pattern(&self) -> &str {
        self.as_str()
    }
}

/// Uninhabited error type for literal matchers — matching never fails.
#[derive(Debug)]
pub(crate) enum LiteralMatchError {}

impl RegexError for LiteralMatchError {
    fn into_failure_reason(self) -> RegexFailureReason {
        match self {}
    }
}

/// [`RegexEngine`] for literal patterns — either `starts_with` (prefix) or `==` (exact).
pub(crate) enum LiteralMatcher {
    Prefix {
        literal: String,
        original: String,
    },
    Exact {
        exact: String,
        original: String,
    },
    /// `^(a|b|c)$` — linear scan over a small sorted array.
    Alternation {
        alternatives: Vec<String>,
        original: String,
    },
    /// `^\S*$` — no ECMA-262 whitespace characters.
    NoWhitespace {
        original: String,
    },
}

impl RegexEngine for LiteralMatcher {
    type Error = LiteralMatchError;

    #[inline]
    fn is_match(&self, text: &str) -> Result<bool, Self::Error> {
        match self {
            Self::Prefix { literal, .. } => Ok(text.starts_with(literal.as_str())),
            Self::Exact { exact, .. } => Ok(text == exact.as_str()),
            Self::Alternation { alternatives, .. } => {
                Ok(alternatives.iter().any(|a| a.as_str() == text))
            }
            Self::NoWhitespace { .. } => Ok(!text.chars().any(is_ecma_whitespace)),
        }
    }

    fn pattern(&self) -> &str {
        match self {
            Self::Prefix { original, .. }
            | Self::Exact { original, .. }
            | Self::Alternation { original, .. }
            | Self::NoWhitespace { original } => original.as_str(),
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

/// Returns `true` for ECMA-262 whitespace characters (`\s` in ECMA regex).
///
/// This is the union of ASCII whitespace, `\u{00a0}` (non-breaking space), and the Unicode
/// space separator category characters recognized by the spec.
#[inline]
pub(crate) fn is_ecma_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\x0b' | '\x0c' | '\r' | ' ' | '\u{00a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// Parse a single literal alternative, accepting the same character set as `analyze_pattern`.
fn parse_literal_part(s: &str) -> Option<String> {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next()? {
                c @ ('/' | '-' | '_' | '$' | '.') => result.push(c),
                _ => return None,
            }
        } else if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/') {
            result.push(c);
        } else {
            return None;
        }
    }
    Some(result)
}

/// Split `inner` by `|` and validate each alternative is a safe literal.
/// Returns `None` if any alternative contains characters that require a regex engine.
fn parse_literal_alternation(inner: &str) -> Option<Vec<String>> {
    let mut alternatives: Vec<String> = inner
        .split('|')
        .map(parse_literal_part)
        .collect::<Option<Vec<_>>>()?;
    alternatives.sort();
    Some(alternatives)
}

/// Analyze a pattern and return a [`PatternOptimization`] if one applies, or `None` if a full
/// regex engine is required.
///
/// Accepts unescaped alphanumeric chars, `-`, `_`, `/` and the safe escape sequences
/// `\/` → `/`, `\-` → `-`, `\_` → `_`, `\$` → `$`, `\.` → `.` in the literal body.
/// A trailing `$` anchor (unescaped) promotes the result to [`PatternOptimization::Exact`].
pub(crate) fn analyze_pattern(pattern: &str) -> Option<PatternOptimization> {
    // Fast path: exact no-whitespace sentinel.
    if pattern == r"^\S*$" {
        return Some(PatternOptimization::NoWhitespace);
    }
    // Fast path: `^(a|b|c)$` alternation.
    if let Some(inner) = pattern
        .strip_prefix("^(")
        .and_then(|s| s.strip_suffix(")$"))
    {
        let alternatives = parse_literal_alternation(inner)?;
        return Some(PatternOptimization::Alternation(alternatives));
    }
    let suffix = pattern.strip_prefix('^')?;
    let mut literal = String::new();
    let mut chars = suffix.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            // `\/` is a common ECMA idiom for a literal `/`; accept a small set of
            // safe escapes that map 1-to-1 to their unescaped character.
            match chars.next()? {
                c @ ('/' | '-' | '_' | '$' | '.') => literal.push(c),
                _ => return None,
            }
        } else if c == '$' {
            // Unescaped `$` is only valid as the very last character (end anchor).
            if chars.peek().is_none() {
                return Some(PatternOptimization::Exact(literal));
            }
            return None;
        } else if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/') {
            literal.push(c);
        } else {
            return None;
        }
    }
    Some(PatternOptimization::Prefix(literal))
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

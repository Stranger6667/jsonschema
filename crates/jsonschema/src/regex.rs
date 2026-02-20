pub(crate) trait RegexEngine: Sized + Send + Sync {
    type Error: RegexError;
    fn is_match(&self, text: &str) -> Result<bool, Self::Error>;

    fn pattern(&self) -> &str;
}

impl RegexEngine for fancy_regex::Regex {
    type Error = fancy_regex::Error;

    fn is_match(&self, text: &str) -> Result<bool, Self::Error> {
        fancy_regex::Regex::is_match(self, text)
    }

    fn pattern(&self) -> &str {
        self.as_str()
    }
}

impl RegexEngine for regex::Regex {
    type Error = regex::Error;

    fn is_match(&self, text: &str) -> Result<bool, Self::Error> {
        Ok(regex::Regex::is_match(self, text))
    }

    fn pattern(&self) -> &str {
        self.as_str()
    }
}

pub(crate) trait RegexError {
    fn into_backtrack_error(self) -> Option<fancy_regex::Error>;
}

impl RegexError for fancy_regex::Error {
    fn into_backtrack_error(self) -> Option<fancy_regex::Error> {
        Some(self)
    }
}

impl RegexError for regex::Error {
    fn into_backtrack_error(self) -> Option<fancy_regex::Error> {
        None
    }
}

/// Infallible error for literal matchers — matching never fails.
#[derive(Debug)]
pub(crate) struct LiteralMatchError;

impl RegexError for LiteralMatchError {
    fn into_backtrack_error(self) -> Option<fancy_regex::Error> {
        None
    }
}

/// [`RegexEngine`] for literal patterns — either `starts_with` (prefix) or `==` (exact).
pub(crate) enum LiteralMatcher {
    Prefix { literal: String, original: String },
    Exact { exact: String, original: String },
}

impl RegexEngine for LiteralMatcher {
    type Error = LiteralMatchError;

    #[inline]
    fn is_match(&self, text: &str) -> Result<bool, Self::Error> {
        match self {
            Self::Prefix { literal, .. } => Ok(text.starts_with(literal.as_str())),
            Self::Exact { exact, .. } => Ok(text == exact.as_str()),
        }
    }

    fn pattern(&self) -> &str {
        match self {
            Self::Prefix { original, .. } | Self::Exact { original, .. } => original.as_str(),
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
}

/// Analyze a pattern and return a [`PatternOptimization`] if one applies, or `None` if a full
/// regex engine is required.
///
/// Accepts unescaped alphanumeric chars, `-`, `_`, `/` and the safe escape sequences
/// `\/` → `/`, `\-` → `-`, `\_` → `_`, `\$` → `$`, `\.` → `.` in the literal body.
/// A trailing `$` anchor (unescaped) promotes the result to [`PatternOptimization::Exact`].
pub(crate) fn analyze_pattern(pattern: &str) -> Option<PatternOptimization> {
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

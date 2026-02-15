use std::borrow::Cow;

use regex_syntax::ast::{
    self, parse::Parser, Ast, ClassPerl, ClassPerlKind, ClassSetItem, ErrorKind, Literal,
    LiteralKind, Span, SpecialLiteralKind, Visitor,
};

/// Convert ECMA Script 262 regex to Rust regex on the best effort basis.
///
/// NOTE: Patterns with look arounds and backreferences are not supported.
///
/// # Errors
///
/// Errors are returned on unsupported or invalid regular expressions.
#[allow(clippy::result_unit_err)]
pub fn to_rust_regex(pattern: &str) -> Result<Cow<'_, str>, ()> {
    let mut pattern = Cow::Borrowed(pattern);
    let mut ast = loop {
        match Parser::new().parse(&pattern) {
            Ok(ast) => break ast,
            Err(error) if *error.kind() == ErrorKind::EscapeUnrecognized => {
                let Span { start, end } = error.span();
                let source = error.pattern();
                if &source[start.offset..end.offset] == r"\c" {
                    if let Some(letter) = &source[end.offset..].chars().next() {
                        if letter.is_ascii_alphabetic() {
                            let start = start.offset;
                            let end = end.offset + 1;
                            let replacement = ((*letter as u8) % 32) as char;
                            match pattern {
                                Cow::Borrowed(_) => {
                                    let prefix = &source[..start];
                                    let suffix = &source[end..];
                                    pattern = Cow::Owned(format!("{prefix}{replacement}{suffix}"));
                                }
                                Cow::Owned(ref mut buffer) => {
                                    let mut char_buffer = [0; 4];
                                    let replacement = replacement.encode_utf8(&mut char_buffer);
                                    buffer.replace_range(start..end, replacement);
                                }
                            }
                            continue;
                        }
                    }
                }
                return Err(());
            }
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::UnsupportedLookAround | ErrorKind::UnsupportedBackreference
                ) =>
            {
                // Can't translate patterns with look arounds & backreferences
                return Ok(pattern);
            }
            Err(_) => {
                return Err(());
            }
        };
    };
    let mut has_changes;
    loop {
        let translator = Ecma262Translator::new(pattern);
        (pattern, has_changes) = ast::visit(&ast, translator).map_err(|_| ())?;
        if !has_changes {
            return Ok(pattern);
        }
        match Parser::new().parse(&pattern) {
            Ok(updated_ast) => {
                ast = updated_ast;
            }
            Err(_) => {
                return Err(());
            }
        }
    }
}

struct Ecma262Translator<'a> {
    pattern: Cow<'a, str>,
    offset: usize,
    has_changes: bool,
}

impl<'a> Ecma262Translator<'a> {
    fn new(input: Cow<'a, str>) -> Self {
        Self {
            pattern: input,
            offset: 0,
            has_changes: false,
        }
    }

    fn replace_impl(&mut self, span: &Span, replacement: &str) {
        let Span { start, end } = span;
        match self.pattern {
            Cow::Borrowed(pattern) => {
                let prefix = &pattern[..start.offset];
                let suffix = &pattern[end.offset..];
                self.pattern = Cow::Owned(format!("{prefix}{replacement}{suffix}"));
            }
            Cow::Owned(ref mut buffer) => {
                buffer.replace_range(
                    start.offset + self.offset..end.offset + self.offset,
                    replacement,
                );
            }
        }
        self.offset += replacement.len() - (end.offset - start.offset);
        self.has_changes = true;
    }

    fn replace(&mut self, cls: &ClassPerl) {
        match cls.kind {
            ClassPerlKind::Digit => {
                let replacement = if cls.negated { "[^0-9]" } else { "[0-9]" };
                self.replace_impl(&cls.span, replacement);
            }
            ClassPerlKind::Word => {
                let replacement = if cls.negated {
                    "[^A-Za-z0-9_]"
                } else {
                    "[A-Za-z0-9_]"
                };
                self.replace_impl(&cls.span, replacement);
            }
            ClassPerlKind::Space => {
                let replacement = &if cls.negated {
                    "[^ \t\n\r\u{000b}\u{000c}\u{00a0}\u{feff}\u{2003}\u{2029}]"
                } else {
                    "[ \t\n\r\u{000b}\u{000c}\u{00a0}\u{feff}\u{2003}\u{2029}]"
                };
                self.replace_impl(&cls.span, replacement);
            }
        }
    }
}

impl<'a> Visitor for Ecma262Translator<'a> {
    type Output = (Cow<'a, str>, bool);
    type Err = ast::Error;

    fn finish(self) -> Result<Self::Output, Self::Err> {
        Ok((self.pattern, self.has_changes))
    }

    fn visit_class_set_item_pre(&mut self, item: &ast::ClassSetItem) -> Result<(), Self::Err> {
        if let ClassSetItem::Perl(cls) = item {
            self.replace(cls);
        }
        Ok(())
    }
    fn visit_post(&mut self, ast: &Ast) -> Result<(), Self::Err> {
        if self.has_changes {
            return Ok(());
        }
        match ast {
            Ast::ClassPerl(perl) => {
                self.replace(perl);
            }
            Ast::Literal(literal) => {
                if let Literal {
                    kind: LiteralKind::Special(SpecialLiteralKind::Bell),
                    ..
                } = literal.as_ref()
                {
                    // Not possible to create a custom error, hence throw an arbitrary one from a
                    // known invalid pattern.
                    return Parser::new().parse("[").map(|_| ());
                }
            }
            _ => (),
        }
        Ok(())
    }
}

/// The result of analyzing a regex pattern for literal-match optimizations.
#[derive(Debug, PartialEq)]
pub enum PatternAnalysis<'a> {
    /// `^prefix` — use `starts_with(prefix)`.
    Prefix(Cow<'a, str>),
    /// `^exact$` — use `== exact`.
    Exact(Cow<'a, str>),
    /// `^(a|b|c)$` — linear scan over a small sorted set of literals.
    Alternation(Vec<String>),
}

/// Parse a single literal alternative for use inside `^(a|b|c)$`.
/// Accepts alphanumeric chars, `-`, `_`, `/` and the safe escapes `\/` → `/`, `\$` → `$`.
fn parse_literal_alternative(s: &str) -> Option<String> {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('/') => result.push('/'),
                Some('$') => result.push('$'),
                _ => return None,
            },
            c if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/') => result.push(c),
            _ => return None,
        }
    }
    Some(result)
}

/// Analyze a regex pattern and return a [`PatternAnalysis`] if a literal optimization applies.
///
/// Handles:
/// - `^prefix` → [`PatternAnalysis::Prefix`] (use `starts_with`)
/// - `^exact$` → [`PatternAnalysis::Exact`] (use `==`)
/// - `^(a|b|c)$` → [`PatternAnalysis::Alternation`] (linear scan, sorted)
///
/// Returns `None` if a full regex engine is required.
#[must_use]
pub fn analyze_pattern(pattern: &str) -> Option<PatternAnalysis<'_>> {
    // Fast path: `^(a|b|c)$` alternation.
    if let Some(inner) = pattern
        .strip_prefix("^(")
        .and_then(|s| s.strip_suffix(")$"))
    {
        let mut alternatives: Vec<String> = inner
            .split('|')
            .map(parse_literal_alternative)
            .collect::<Option<_>>()?;
        alternatives.sort_unstable();
        return Some(PatternAnalysis::Alternation(alternatives));
    }

    let suffix = pattern.strip_prefix('^')?;

    // Fast path: no backslashes — borrow directly from the input.
    if !suffix.contains('\\') {
        // Trailing `$` is the end-of-string anchor → Exact match.
        if let Some(body) = suffix.strip_suffix('$') {
            return if body
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/'))
            {
                Some(PatternAnalysis::Exact(Cow::Borrowed(body)))
            } else {
                None
            };
        }
        return if suffix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/'))
        {
            Some(PatternAnalysis::Prefix(Cow::Borrowed(suffix)))
        } else {
            None
        };
    }

    // Slow path: unescape `\/` → `/` and `\$` → `$`; a bare `$` at end means Exact.
    let mut result = String::with_capacity(suffix.len());
    let mut chars = suffix.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('/') => result.push('/'),
                Some('$') => result.push('$'),
                _ => return None,
            },
            '$' => {
                // End-of-string anchor — valid only as the very last character.
                if chars.peek().is_none() {
                    return Some(PatternAnalysis::Exact(Cow::Owned(result)));
                }
                return None;
            }
            c if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/') => result.push(c),
            _ => return None,
        }
    }
    Some(PatternAnalysis::Prefix(Cow::Owned(result)))
}

/// Try to extract a simple prefix from a pattern like `^prefix`.
/// Only matches patterns with alphanumeric characters, hyphens, underscores, and forward slashes.
/// The escaped form `\/` is also accepted and normalised to `/`.
#[must_use]
pub fn pattern_as_prefix(pattern: &str) -> Option<Cow<'_, str>> {
    match analyze_pattern(pattern) {
        Some(PatternAnalysis::Prefix(p)) => Some(p),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test_case(r"\d", "[0-9]"; "digit class")]
    #[test_case(r"\D", "[^0-9]"; "non-digit class")]
    #[test_case(r"\w", "[A-Za-z0-9_]"; "word class")]
    #[test_case(r"\W", "[^A-Za-z0-9_]"; "non-word class")]
    #[test_case(r"[\d]", "[[0-9]]"; "digit class in character set")]
    #[test_case(r"[\D]", "[[^0-9]]"; "non-digit class in character set")]
    #[test_case(r"[\w]", "[[A-Za-z0-9_]]"; "word class in character set")]
    #[test_case(r"[\W]", "[[^A-Za-z0-9_]]"; "non-word class in character set")]
    #[test_case(r"\d+\w*", "[0-9]+[A-Za-z0-9_]*"; "combination of digit and word classes")]
    #[test_case(r"\D*\W+", "[^0-9]*[^A-Za-z0-9_]+"; "combination of non-digit and non-word classes")]
    #[test_case(r"[\d\w]", "[[0-9][A-Za-z0-9_]]"; "digit and word classes in character set")]
    #[test_case(r"[^\d\w]", "[^[0-9][A-Za-z0-9_]]"; "negated digit and word classes in character set")]
    #[test_case(r"[\d\w\d\w]", "[[0-9][A-Za-z0-9_][0-9][A-Za-z0-9_]]"; "multiple replacements")]
    #[test_case(r"\cA\cB\cC", "\x01\x02\x03"; "multiple control characters")]
    #[test_case(r"foo\cIbar\cXbaz", "foo\x09bar\x18baz"; "control characters mixed with text")]
    #[test_case(r"\ca\cb\cc", "\x01\x02\x03"; "lowercase control characters")]
    fn test_ecma262_to_rust_regex(input: &str, expected: &str) {
        let result = to_rust_regex(input).unwrap();
        assert_eq!(result, expected);
    }

    #[test_case(r"\c"; "incomplete control character")]
    #[test_case(r"\c?"; "invalid control character")]
    #[test_case(r"\mA"; "another invalid control character")]
    #[test_case(r"[a-z"; "unclosed character class")]
    #[test_case(r"(abc"; "unclosed parenthesis")]
    #[test_case(r"abc)"; "unmatched closing parenthesis")]
    #[test_case(r"a{3,2}"; "invalid quantifier range")]
    #[test_case(r"\"; "trailing backslash")]
    #[test_case(r"[a-\w]"; "invalid character range")]
    fn test_invalid_regex(input: &str) {
        let result = to_rust_regex(input);
        assert!(result.is_err(), "Expected error for input: {input}");
    }

    #[test_case("^foo", Some("foo"))]
    #[test_case("^x-", Some("x-"))]
    #[test_case("^eo_band", Some("eo_band"))]
    #[test_case("^path/to", Some("path/to"))]
    #[test_case("^ABC123", Some("ABC123"))]
    #[test_case("^\\/", Some("/"); "escaped slash prefix")]
    #[test_case("^\\/path", Some("/path"); "escaped slash with suffix")]
    #[test_case("^\\$ref", Some("$ref"); "escaped dollar ref")]
    #[test_case("^\\$defs", Some("$defs"); "escaped dollar defs")]
    #[test_case("foo", None; "no anchor")]
    #[test_case("^foo$", None; "end anchor")]
    #[test_case("^\\$ref$", None; "exact match dollar ref is not a prefix")]
    #[test_case("^foo.*", None; "contains dot")]
    #[test_case("^foo+", None; "contains plus")]
    #[test_case("^foo?", None; "contains question")]
    #[test_case("^[a-z]", None; "contains bracket")]
    #[test_case("^foo|bar", None; "contains pipe")]
    #[test_case("^foo(bar)", None; "contains parens")]
    #[test_case("^foo\\d", None; "contains backslash-d")]
    fn test_pattern_as_prefix(pattern: &str, expected: Option<&str>) {
        assert_eq!(pattern_as_prefix(pattern).as_deref(), expected);
    }

    #[test_case("^foo", PatternAnalysis::Prefix("foo".into()) ; "prefix")]
    #[test_case("^x-", PatternAnalysis::Prefix("x-".into()) ; "x_prefix")]
    #[test_case("^\\$ref", PatternAnalysis::Prefix("$ref".into()) ; "escaped dollar ref prefix")]
    #[test_case("^foo$", PatternAnalysis::Exact("foo".into()) ; "exact fast path")]
    #[test_case("^\\$ref$", PatternAnalysis::Exact("$ref".into()) ; "exact escaped dollar ref")]
    #[test_case(
        "^(get|put|post|delete|options|head|patch|trace)$",
        PatternAnalysis::Alternation(vec![
            "delete".into(), "get".into(), "head".into(), "options".into(),
            "patch".into(), "post".into(), "put".into(), "trace".into(),
        ]) ; "http methods alternation"
    )]
    #[test_case(
        "^(a|b|c)$",
        PatternAnalysis::Alternation(vec!["a".into(), "b".into(), "c".into()]) ; "simple alternation sorted"
    )]
    fn test_analyze_pattern(pattern: &str, expected: PatternAnalysis<'_>) {
        assert_eq!(analyze_pattern(pattern), Some(expected));
    }

    #[test_case("foo" ; "no anchor")]
    #[test_case("^foo.*" ; "contains dot")]
    #[test_case("^foo+" ; "contains plus")]
    #[test_case("^[a-z]" ; "contains bracket")]
    #[test_case("^(a|b^)$" ; "invalid char in alternation")]
    fn test_analyze_pattern_none(pattern: &str) {
        assert_eq!(analyze_pattern(pattern), None);
    }
}

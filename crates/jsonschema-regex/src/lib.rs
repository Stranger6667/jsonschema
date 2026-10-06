mod syntax;

use std::{borrow::Cow, fmt::Write};

use regex_syntax::{
    ast::{
        self, parse::Parser, Ast, ClassPerl, ClassPerlKind, ClassSetItem, ErrorKind, Literal,
        LiteralKind, Span, SpecialLiteralKind, Visitor,
    },
    hir::{Class, Hir, HirKind},
};

pub use syntax::is_valid_ecma_regex;

/// Convert ECMA Script 262 regex to Rust regex on the best effort basis.
///
/// NOTE: Patterns with look arounds and backreferences are not supported.
///
/// # Errors
///
/// Errors are returned on unsupported or invalid regular expressions.
#[allow(clippy::result_unit_err)]
pub fn to_rust_regex(pattern: &str) -> Result<Cow<'_, str>, ()> {
    let mut pattern = escape_class_set_syntax(pattern);
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

#[derive(Clone, Copy, PartialEq)]
enum ClassState {
    AtItemStart,
    AfterRangeStart,
    ExpectRangeEnd,
}

impl ClassState {
    fn consume_atom(&mut self) {
        *self = match self {
            Self::ExpectRangeEnd => Self::AtItemStart,
            Self::AtItemStart | Self::AfterRangeStart => Self::AfterRangeStart,
        };
    }
}

fn escape_class_set_syntax(pattern: &str) -> Cow<'_, str> {
    let mut escapes = Vec::new();
    let mut in_class = false;
    let mut escaped = false;
    let mut class_state = ClassState::AtItemStart;
    let mut previous_atom_is_class_escape = false;
    let mut class_initial = false;
    let mut chars = pattern.char_indices().peekable();
    while let Some((offset, c)) = chars.next() {
        if escaped {
            escaped = false;
            if in_class {
                class_state.consume_atom();
                previous_atom_is_class_escape = matches!(c, 'd' | 'D' | 's' | 'S' | 'w' | 'W');
                class_initial = false;
            }
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if c == '[' {
            if in_class {
                escapes.push(offset);
                class_state.consume_atom();
                previous_atom_is_class_escape = false;
                class_initial = false;
            } else {
                in_class = true;
                class_state = ClassState::AtItemStart;
                previous_atom_is_class_escape = false;
                class_initial = true;
            }
            continue;
        }
        if c == ']' && in_class {
            in_class = false;
            class_state = ClassState::AtItemStart;
            previous_atom_is_class_escape = false;
            continue;
        }
        if !in_class {
            continue;
        }
        if c == '^' && class_initial {
            class_initial = false;
            continue;
        }
        if c == '-'
            && previous_atom_is_class_escape
            && chars.peek().is_some_and(|(_, next)| *next == '-')
        {
            // Without the Unicode flag, a class escape cannot participate in a range. Annex B
            // treats both hyphens in forms such as `[\w--z]` as literals.
            escapes.push(offset);
            let (next_offset, _) = chars.next().expect("the next character was peeked");
            escapes.push(next_offset);
            class_state.consume_atom();
            previous_atom_is_class_escape = false;
            class_initial = false;
            continue;
        }
        if c == '-'
            && class_state == ClassState::AfterRangeStart
            && chars.peek().is_some_and(|(_, next)| *next != ']')
        {
            // This hyphen is an ECMAScript range delimiter. Keep it unescaped so ranges such as
            // the `--b` part of `[a-z--b]` preserve their meaning.
            class_state = ClassState::ExpectRangeEnd;
            previous_atom_is_class_escape = false;
            continue;
        }
        if matches!(c, '&' | '~') && chars.peek().is_some_and(|(_, next)| *next == c) {
            escapes.push(offset);
            let (next_offset, _) = chars.next().expect("the next character was peeked");
            escapes.push(next_offset);
            class_state.consume_atom();
        } else if c == '-'
            && (pattern.as_bytes().get(offset.wrapping_sub(1)) == Some(&b'-')
                || chars.peek().is_some_and(|(_, next)| *next == '-'))
        {
            // Only escape literal hyphens. At least one hyphen in every valid adjacent pair is a
            // class atom, and escaping that atom is sufficient to avoid Rust's `--` operator.
            escapes.push(offset);
        }
        class_state.consume_atom();
        previous_atom_is_class_escape = false;
        class_initial = false;
    }
    if escapes.is_empty() {
        return Cow::Borrowed(pattern);
    }
    let mut translated = String::with_capacity(pattern.len() + escapes.len());
    let mut escapes = escapes.into_iter().peekable();
    for (offset, c) in pattern.char_indices() {
        if escapes.next_if_eq(&offset).is_some() {
            translated.push('\\');
        }
        translated.push(c);
    }
    Cow::Owned(translated)
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
                let mut replacement = String::from(if cls.negated { "[^" } else { "[" });
                for (start, end) in ECMA_WHITESPACE_RANGES {
                    // Writing to a `String` cannot fail.
                    let _ = write!(
                        replacement,
                        "\\x{{{:x}}}-\\x{{{:x}}}",
                        start as u32, end as u32
                    );
                }
                replacement.push(']');
                self.replace_impl(&cls.span, &replacement);
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
    /// `^prefix` -> use `starts_with(prefix)`.
    Prefix(Cow<'a, str>),
    /// `^exact$` -> use `== exact`.
    Exact(Cow<'a, str>),
    /// `^(a|b|c)$` -> linear scan over a small sorted set of literals.
    Alternation(Vec<String>),
    /// `^\S*$` -> no ECMA-262 whitespace character (see [`is_ecma_whitespace`]).
    NoWhitespace,
}

/// Inclusive ranges of ECMA-262 `\s`: `WhiteSpace` (TAB, VT, FF, every `Zs`, BOM) and `LineTerminator`.
const ECMA_WHITESPACE_RANGES: [(char, char); 10] = [
    ('\t', '\r'),
    (' ', ' '),
    ('\u{00a0}', '\u{00a0}'),
    ('\u{1680}', '\u{1680}'),
    ('\u{2000}', '\u{200a}'),
    ('\u{2028}', '\u{2029}'),
    ('\u{202f}', '\u{202f}'),
    ('\u{205f}', '\u{205f}'),
    ('\u{3000}', '\u{3000}'),
    ('\u{feff}', '\u{feff}'),
];

/// Returns `true` for ECMA-262 whitespace characters (`\s` in ECMA regex): the union of ASCII
/// whitespace, `\u{00a0}`, and the Unicode space-separator category recognized by the spec.
#[inline]
#[must_use]
pub fn is_ecma_whitespace(c: char) -> bool {
    ECMA_WHITESPACE_RANGES
        .iter()
        .any(|&(start, end)| start <= c && c <= end)
}

/// Whether `input` holds any character `\s` matches.
///
/// All-ASCII input is scanned as bytes, which vectorizes; anything else is decoded, as before.
/// `is_ascii` stops at the first non-ASCII byte, so input taking the decode path pays little for
/// the check.
#[must_use]
#[inline]
pub fn contains_ecma_whitespace(input: &str) -> bool {
    if input.is_ascii() {
        input
            .as_bytes()
            .iter()
            .any(|byte| matches!(byte, b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | b' '))
    } else {
        input.chars().any(is_ecma_whitespace)
    }
}

/// Maps a backslash escape to its literal character, or `None` if the escape requires a regex engine.
fn safe_escape(escaped: char) -> Option<char> {
    matches!(escaped, '/' | '-' | '_' | '$' | '.').then_some(escaped)
}

/// Parse a single literal alternative for use inside `^(a|b|c)$`.
/// Accepts alphanumeric chars, `-`, `_`, `/` and the safe escapes handled by [`safe_escape`].
fn parse_literal_alternative(s: &str) -> Option<String> {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => result.push(safe_escape(chars.next()?)?),
            c if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/') => result.push(c),
            _ => return None,
        }
    }
    Some(result)
}

/// Analyze a regex pattern and return a [`PatternAnalysis`] if a literal optimization applies.
///
/// Handles:
/// - `^prefix` -> [`PatternAnalysis::Prefix`] (use `starts_with`)
/// - `^exact$` -> [`PatternAnalysis::Exact`] (use `==`)
/// - `^(a|b|c)$` -> [`PatternAnalysis::Alternation`] (linear scan, sorted)
/// - `^\S*$` -> [`PatternAnalysis::NoWhitespace`] (scan for [`is_ecma_whitespace`])
///
/// Accepts unescaped alphanumeric chars, `-`, `_`, `/` and the safe escapes `\/`, `\-`, `\_`, `\$`, `\.`.
/// Returns `None` if a full regex engine is required.
#[must_use]
pub fn analyze_pattern(pattern: &str) -> Option<PatternAnalysis<'_>> {
    if pattern == r"^\S*$" {
        return Some(PatternAnalysis::NoWhitespace);
    }

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

    // Fast path: no backslashes, borrow directly from the input.
    if !suffix.contains('\\') {
        // Trailing `$` is the end-of-string anchor -> Exact match.
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

    // Slow path: unescape via `safe_escape`; a bare `$` at end means Exact.
    let mut result = String::with_capacity(suffix.len());
    let mut chars = suffix.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => result.push(safe_escape(chars.next()?)?),
            '$' => {
                // End-of-string anchor: valid only as the last character.
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

/// A string `pattern` matches, or `None` where the syntax alone is not enough to build one.
///
/// A `pattern` is an unanchored search, so a look-around contributes nothing to the string built
/// here, and the caller checks the result against the pattern anyway.
#[must_use]
pub fn pattern_witness(pattern: &str) -> Option<String> {
    let hir = regex_syntax::parse(&to_rust_regex(pattern).ok()?).ok()?;
    let mut witness = String::new();
    write_witness(&hir, &mut witness).then_some(witness)
}

/// The most repetitions of one sub-expression written out.
const WITNESS_REPETITIONS: u32 = 64;

/// The longest witness built. Without a cap, nested repetitions multiply the length.
const WITNESS_LENGTH: usize = 256;

/// Appends a string matching `hir`, reporting whether one was found. On failure `witness` keeps a
/// partial string, so a caller trying another branch truncates back to its own length first.
fn write_witness(hir: &Hir, witness: &mut String) -> bool {
    if witness.len() > WITNESS_LENGTH {
        return false;
    }
    match hir.kind() {
        HirKind::Empty | HirKind::Look(_) => true,
        HirKind::Literal(literal) => match std::str::from_utf8(&literal.0) {
            Ok(text) => {
                witness.push_str(text);
                true
            }
            Err(_) => false,
        },
        HirKind::Class(class) => {
            let character = match class {
                Class::Unicode(class) => pick(class.ranges().iter().map(|r| (r.start(), r.end()))),
                Class::Bytes(class) => pick(
                    class
                        .ranges()
                        .iter()
                        .map(|r| (char::from(r.start()), char::from(r.end()))),
                ),
            };
            character.is_some_and(|character| {
                witness.push(character);
                true
            })
        }
        HirKind::Repetition(repetition) => {
            if repetition.min > WITNESS_REPETITIONS {
                return false;
            }
            (0..repetition.min).all(|_| write_witness(&repetition.sub, witness))
        }
        HirKind::Capture(capture) => write_witness(&capture.sub, witness),
        HirKind::Concat(parts) => parts.iter().all(|part| write_witness(part, witness)),
        HirKind::Alternation(branches) => {
            let start = witness.len();
            branches.iter().any(|branch| {
                witness.truncate(start);
                write_witness(branch, witness)
            })
        }
    }
}

/// A character from a class, preferring `a` over whatever the class starts with.
fn pick(ranges: impl Iterator<Item = (char, char)>) -> Option<char> {
    let mut first = None;
    for (start, end) in ranges {
        if (start..=end).contains(&'a') {
            return Some('a');
        }
        first.get_or_insert(start);
    }
    first
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
    #[test_case(r"^[a-z&&^b]+$", r"^[a-z\&\&^b]+$"; "class intersection syntax is literal")]
    #[test_case(r"^[a-z--b]$", r"^[a-z\--b]$"; "class difference syntax preserves ranges")]
    #[test_case(r"^[\w--z]$", r"^[[A-Za-z0-9_]\-\-z]$"; "word class escape before doubled hyphen")]
    #[test_case(r"^[\d--z]$", r"^[[0-9]\-\-z]$"; "digit class escape before doubled hyphen")]
    #[test_case(r"^[a~~b]$", r"^[a\~\~b]$"; "class symmetric difference syntax is literal")]
    #[test_case(r"^[[a]]$", r"^[\[a]]$"; "nested class opener is literal")]
    #[test_case(r"[a-z]", r"[a-z]"; "ordinary class is unchanged")]
    #[test_case(r"[\[\]]", r"[\[\]]"; "escaped brackets are unchanged")]
    #[test_case(r"[^&]", r"[^&]"; "single ampersand is unchanged")]
    #[test_case(r"a&&b", r"a&&b"; "operator syntax outside a class is unchanged")]
    fn test_ecma262_to_rust_regex(input: &str, expected: &str) {
        let result = to_rust_regex(input).unwrap();
        assert_eq!(result, expected);
    }

    const ECMA_WHITESPACE: [char; 25] = [
        '\t', '\n', '\u{b}', '\u{c}', '\r', ' ', '\u{a0}', '\u{1680}', '\u{2000}', '\u{2003}',
        '\u{200a}', '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}', '\u{feff}',
        '\u{2001}', '\u{2002}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}',
        '\u{2009}',
    ];
    const NOT_ECMA_WHITESPACE: [char; 7] = [
        '\u{85}', '\u{1c}', '\u{200b}', '\u{180e}', 'a', '0', '\u{2010}',
    ];

    #[test_case(r"\s", true; "space class")]
    #[test_case(r"\S", false; "non-space class")]
    #[test_case(r"[\s]", true; "space class in set")]
    #[test_case(r"[^\s]", false; "negated set of space class")]
    #[test_case(r"[a\s]", true; "space class with literal in set")]
    #[test_case(r"[\S]", false; "non-space class in set")]
    fn space_class_matches_ecma_whitespace(pattern: &str, matches_whitespace: bool) {
        let regex =
            regex::Regex::new(&format!("^(?:{})$", to_rust_regex(pattern).unwrap())).unwrap();
        for c in ECMA_WHITESPACE {
            assert_eq!(
                regex.is_match(&c.to_string()),
                matches_whitespace,
                "{pattern} on U+{:04X}",
                c as u32
            );
        }
        for c in NOT_ECMA_WHITESPACE {
            let expected = if matches_whitespace {
                c == 'a' && pattern.starts_with("[a")
            } else {
                true
            };
            assert_eq!(
                regex.is_match(&c.to_string()),
                expected,
                "{pattern} on U+{:04X}",
                c as u32
            );
        }
    }

    #[test]
    fn whitespace_test_set_agrees_with_predicate() {
        assert!(ECMA_WHITESPACE.into_iter().all(is_ecma_whitespace));
        assert!(!NOT_ECMA_WHITESPACE.into_iter().any(is_ecma_whitespace));
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
    #[test_case(r"[z--]"; "invalid range ending in a hyphen")]
    fn test_invalid_regex(input: &str) {
        let result = to_rust_regex(input);
        assert!(result.is_err(), "Expected error for input: {input}");
    }

    #[test]
    fn doubled_hyphen_preserves_ecmascript_range_semantics() {
        let translated = to_rust_regex(r"^[a-z--b]$").expect("valid ECMAScript regex");
        let regex = regex::Regex::new(&translated).expect("translation compiles");

        assert!(regex.is_match("0"));
        assert!(regex.is_match("A"));
        assert!(regex.is_match("z"));
        assert!(!regex.is_match("{"));
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
    #[test_case(
        "^(a\\/b|c)$",
        PatternAnalysis::Alternation(vec!["a/b".into(), "c".into()]) ; "alternation with escaped slash"
    )]
    #[test_case(
        "^(x\\$y|z)$",
        PatternAnalysis::Alternation(vec!["x$y".into(), "z".into()]) ; "alternation with escaped dollar"
    )]
    #[test_case("^a\\.b", PatternAnalysis::Prefix("a.b".into()) ; "escaped dot prefix")]
    #[test_case("^a\\.b$", PatternAnalysis::Exact("a.b".into()) ; "escaped dot exact")]
    #[test_case("^a\\-b\\_c", PatternAnalysis::Prefix("a-b_c".into()) ; "escaped dash underscore prefix")]
    #[test_case(
        "^(a\\.b|c\\-d)$",
        PatternAnalysis::Alternation(vec!["a.b".into(), "c-d".into()]) ; "alternation with escaped dot and dash"
    )]
    #[test_case(r"^\S*$", PatternAnalysis::NoWhitespace ; "no whitespace")]
    fn test_analyze_pattern(pattern: &str, expected: PatternAnalysis<'_>) {
        assert_eq!(analyze_pattern(pattern), Some(expected));
    }

    #[test_case(' ' ; "space")]
    #[test_case('\t' ; "tab")]
    #[test_case('\u{00a0}' ; "nbsp")]
    #[test_case('\u{2003}' ; "em space")]
    #[test_case('\u{3000}' ; "ideographic space")]
    #[test_case('\u{feff}' ; "bom")]
    fn test_is_ecma_whitespace(c: char) {
        assert!(is_ecma_whitespace(c));
    }

    #[test_case('a' ; "letter")]
    #[test_case('0' ; "digit")]
    #[test_case('\u{200b}' ; "zero width space")]
    fn test_not_ecma_whitespace(c: char) {
        assert!(!is_ecma_whitespace(c));
    }

    #[test_case("foo" ; "no anchor")]
    #[test_case("^foo.*" ; "contains dot")]
    #[test_case("^foo+" ; "contains plus")]
    #[test_case("^[a-z]" ; "contains bracket")]
    #[test_case("^(a|b^)$" ; "invalid char in alternation")]
    #[test_case("^(a\\db)$" ; "invalid escape in alternation")]
    #[test_case("^a\\-$b" ; "escaped then dollar not at end")]
    fn test_analyze_pattern_none(pattern: &str) {
        assert_eq!(analyze_pattern(pattern), None);
    }

    #[test_case("=", "="; "bare literal")]
    #[test_case("b$", "b"; "tail anchored literal")]
    #[test_case("^abc", "abc"; "prefix")]
    #[test_case("^abc$", "abc"; "exact")]
    #[test_case("^(red|green)$", "red"; "alternation")]
    #[test_case(r"^\S*$", ""; "star repetition")]
    #[test_case("", ""; "empty pattern")]
    #[test_case("a.b", "aab"; "wildcard")]
    #[test_case("a+", "a"; "plus repetition")]
    #[test_case(r"\d{3}", "000"; "counted character class")]
    #[test_case("^[a-z]+-[0-9]+$", "a-0"; "classes around a literal")]
    fn test_pattern_witness(pattern: &str, expected: &str) {
        let witness = pattern_witness(pattern).expect("has a witness");
        assert_eq!(witness, expected);
        let regex = regex::Regex::new(&to_rust_regex(pattern).expect("translates")).expect("valid");
        assert!(
            regex.is_match(&witness),
            "`{witness}` does not match `{pattern}`"
        );
    }

    #[test_case(r"(?=a)b"; "look ahead the translation rejects")]
    #[test_case("a{100}"; "more repetitions than are written out")]
    #[test_case("((a{64}){64}){64}"; "nested repetitions multiplying past the length cap")]
    fn test_pattern_without_a_witness(pattern: &str) {
        assert_eq!(pattern_witness(pattern), None);
    }
    #[test_case("" ; "empty")]
    #[test_case("plain" ; "ascii without whitespace")]
    #[test_case("\u{00e9}t\u{00e9}" ; "non-ascii without whitespace")]
    #[test_case("\u{4e2d}\u{6587}" ; "multi byte without whitespace")]
    fn no_whitespace(input: &str) {
        assert!(!contains_ecma_whitespace(input));
    }

    #[test_case(" " ; "space")]
    #[test_case("a\tb" ; "tab")]
    #[test_case("a\nb" ; "newline")]
    #[test_case("a\u{000b}b" ; "vertical tab")]
    #[test_case("a\u{000c}b" ; "form feed")]
    #[test_case("a\rb" ; "carriage return")]
    #[test_case("a\u{00a0}b" ; "no break space")]
    #[test_case("a\u{1680}b" ; "ogham space mark")]
    #[test_case("a\u{2000}b" ; "en quad")]
    #[test_case("a\u{200a}b" ; "hair space")]
    #[test_case("a\u{2028}b" ; "line separator")]
    #[test_case("a\u{2029}b" ; "paragraph separator")]
    #[test_case("a\u{202f}b" ; "narrow no break space")]
    #[test_case("a\u{205f}b" ; "medium mathematical space")]
    #[test_case("a\u{3000}b" ; "ideographic space")]
    #[test_case("a\u{feff}b" ; "zero width no break space")]
    fn has_whitespace(input: &str) {
        assert!(contains_ecma_whitespace(input));
    }

    // A byte scan must not mistake a continuation byte for whitespace.
    #[test_case("\u{0a20}" ; "leading byte matches newline value")]
    #[test_case("\u{2820}" ; "continuation byte matches space value")]
    fn multi_byte_is_not_whitespace(input: &str) {
        assert!(!contains_ecma_whitespace(input));
        assert_eq!(
            contains_ecma_whitespace(input),
            input.chars().any(is_ecma_whitespace)
        );
    }
    // The byte scan must agree with a decode for every character there is.
    #[test]
    fn agrees_with_decoding_on_every_scalar_value() {
        let mut buffer = [0_u8; 4];
        for value in 0..=0x0010_FFFF_u32 {
            let Some(c) = char::from_u32(value) else {
                continue;
            };
            let text = c.encode_utf8(&mut buffer);
            assert_eq!(
                contains_ecma_whitespace(text),
                text.chars().any(is_ecma_whitespace),
                "{c:?} (U+{value:04X})"
            );
        }
    }

    // Bytes of a multi-byte encoding are all >= 0x80, so no combination can spell an ASCII
    // whitespace byte; these sequences check that directly.
    #[test]
    fn agrees_with_decoding_on_mixed_sequences() {
        let alphabet = [
            'a',
            ' ',
            '\t',
            '\n',
            '\r',
            '\u{000b}',
            '\u{000c}',
            '\u{00a0}',
            '\u{0a20}',
            '\u{2820}',
            '\u{2000}',
            '\u{200a}',
            '\u{2028}',
            '\u{3000}',
            '\u{feff}',
            '\u{4e2d}',
            '\u{10348}',
        ];
        let mut text = String::new();
        for first in alphabet {
            for second in alphabet {
                for third in alphabet {
                    text.clear();
                    text.push(first);
                    text.push(second);
                    text.push(third);
                    assert_eq!(
                        contains_ecma_whitespace(&text),
                        text.chars().any(is_ecma_whitespace),
                        "{text:?}"
                    );
                }
            }
        }
    }
}

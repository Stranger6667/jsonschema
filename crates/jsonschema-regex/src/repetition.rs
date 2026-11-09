//! The engine compiles `X{m,n}` into `n` copies of `X`, so a large count over a wide class is slow
//! to build or exceeds the size limit. A `pattern` only asks whether a string matches, so
//! `^X{m,n}$` can count characters instead, and a count at either edge can drop to its minimum.

use regex_syntax::{
    ast::{self, parse::Parser, Ast, RepetitionKind, RepetitionRange},
    hir::{translate::Translator, Class, Hir, HirKind, Look},
};

use crate::is_valid_ecma_regex;

/// A cheaper stand-in for a translated `pattern` in an unanchored `is_match`.
#[derive(Debug, PartialEq, Eq)]
pub enum Search {
    /// `^X{m,n}$` over one character class: count the characters.
    RepeatedClass(RepeatedClass),
    /// The translation with each count at an edge reduced to its minimum.
    Collapsed(String),
}

/// A cheaper form of `translated`, the [`crate::to_rust_regex`] of `pattern`, for an unanchored
/// `is_match` only: wrapped in `^...$`, it matches other strings.
///
/// A pattern ECMA-262 rejects in Unicode mode gets `None`: the engines read some of them, such as
/// `a{1, 3}`, differently.
#[must_use]
pub fn search_form(pattern: &str, translated: &str) -> Option<Search> {
    // The translation adds braces of its own (`\x{2028}`), so look for a count in the source.
    if !pattern.contains('{') || !is_valid_ecma_regex(pattern) {
        return None;
    }
    let ast = Parser::new().parse(translated).ok()?;
    // The engine must accept the whole pattern, including any atom a collapse drops.
    let hir = Translator::new().translate(translated, &ast).ok()?;
    if let Some(class) = RepeatedClass::from_hir(&hir) {
        return Some(Search::RepeatedClass(class));
    }
    collapse_edge_counts(translated, &ast).map(Search::Collapsed)
}

/// `pattern` with each bounded count at an edge reduced to its minimum, or `None` if it has none.
///
/// A `pattern` matches a string when it matches some substring. When nothing follows `X{m,n}`,
/// every match of `A X{m,n}` starts with a match of `A X{m}`, and every match of `A X{m}` is a
/// match of `A X{m,n}`. A count that nothing precedes works the same from the other end. Groups
/// and alternation branches keep a count at the edge; a quantified group does not, because its
/// inner count repeats with it.
pub(crate) fn collapse_edge_counts(pattern: &str, ast: &Ast) -> Option<String> {
    let mut edits = Vec::new();
    edge_counts(ast, Edge::Head, &mut edits);
    edge_counts(ast, Edge::Tail, &mut edits);
    if edits.is_empty() {
        return None;
    }
    // A pattern that is a single count gets the same edit from both edges.
    edits.sort_unstable();
    edits.dedup();
    let mut collapsed = String::with_capacity(pattern.len());
    let mut copied = 0;
    for edit in &edits {
        // The head walk follows first items and the tail walk last items, so their edits land in
        // different subtrees or coincide.
        debug_assert!(edit.start >= copied, "overlapping edits in {pattern:?}");
        collapsed.push_str(&pattern[copied..edit.start]);
        collapsed.push_str(&edit.replacement);
        copied = edit.end;
    }
    collapsed.push_str(&pattern[copied..]);
    Some(collapsed)
}

#[derive(Clone, Copy)]
enum Edge {
    Head,
    Tail,
}

/// Replace `pattern[start..end]` with `replacement`.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Edit {
    start: usize,
    end: usize,
    replacement: String,
}

fn edge_counts(ast: &Ast, edge: Edge, edits: &mut Vec<Edit>) {
    match ast {
        Ast::Concat(concat) => {
            let item = match edge {
                Edge::Head => concat.asts.first(),
                Edge::Tail => concat.asts.last(),
            };
            if let Some(item) = item {
                edge_counts(item, edge, edits);
            }
        }
        Ast::Alternation(alternation) => {
            for branch in &alternation.asts {
                edge_counts(branch, edge, edits);
            }
        }
        Ast::Group(group) => edge_counts(&group.ast, edge, edits),
        Ast::Repetition(repetition) => {
            if let RepetitionKind::Range(RepetitionRange::Bounded(min, max)) = repetition.op.kind {
                if max > min {
                    edits.push(collapse_to_minimum(repetition, min));
                }
            }
        }
        _ => {}
    }
}

/// `X{m,n}` as `X{m}`: nothing for `m = 0`, `X` for `m = 1`.
fn collapse_to_minimum(repetition: &ast::Repetition, min: u32) -> Edit {
    let (span, replacement) = match min {
        0 => (&repetition.span, String::new()),
        1 => (&repetition.op.span, String::new()),
        _ => (&repetition.op.span, format!("{{{min}}}")),
    };
    Edit {
        start: span.start.offset,
        end: span.end.offset,
        replacement,
    }
}

/// `^X{m,n}$` over one character class `X`: a string matches when it has `m` to `n` characters,
/// all in `X`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepeatedClass {
    /// One bit per ASCII code point in `X`.
    ascii: u128,
    /// For each block of 256 code points in the BMP, the position of its bits in `blocks`.
    block_index: Box<[u8; 256]>,
    /// The distinct blocks, one bit per code point in `X`.
    blocks: Box<[Block]>,
    /// Members of `X` past the BMP, as sorted and disjoint ranges.
    astral: Box<[(char, char)]>,
    min: usize,
    max: usize,
}

type Block = [u64; 4];

const BMP_END: u32 = 0xFFFF;

impl RepeatedClass {
    /// The matcher for a translated pattern shaped `^X{m,n}$`, or `None` for any other shape. The
    /// anchors must match the ends of the text (no `(?m)`), and capture groups may wrap `X` or the
    /// count.
    #[must_use]
    pub fn from_translated(translated: &str) -> Option<Self> {
        Self::from_hir(&regex_syntax::parse(translated).ok()?)
    }

    /// The HIR has already applied flags such as `(?i)` and `(?m)`, and keeps only capture groups.
    fn from_hir(hir: &Hir) -> Option<Self> {
        let HirKind::Concat(items) = hir.kind() else {
            return None;
        };
        let [start, repeated, end] = items.as_slice() else {
            return None;
        };
        if !matches!(start.kind(), HirKind::Look(Look::Start))
            || !matches!(end.kind(), HirKind::Look(Look::End))
        {
            return None;
        }
        let HirKind::Repetition(repetition) = uncaptured(repeated).kind() else {
            return None;
        };
        let HirKind::Class(Class::Unicode(class)) = uncaptured(&repetition.sub).kind() else {
            return None;
        };
        let mut bmp = [[0_u64; 4]; 256];
        let mut astral = Vec::new();
        for range in class.ranges() {
            let (start, end) = (u32::from(range.start()), u32::from(range.end()));
            if end > BMP_END {
                astral.push((range.start().max('\u{10000}'), range.end()));
            }
            // A word at a time; a range past the BMP sets none.
            let end = end.min(BMP_END);
            for word in start / 64..=end / 64 {
                let low = if word == start / 64 { start % 64 } else { 0 };
                let high = if word == end / 64 { end % 64 } else { 63 };
                bmp[(word / 4) as usize][(word % 4) as usize] |=
                    (u64::MAX << low) & (u64::MAX >> (63 - high));
            }
        }
        let mut block_index = Box::new([0_u8; 256]);
        let mut blocks: Vec<Block> = Vec::new();
        let mut previous = 0;
        for (block, position) in bmp.iter().zip(block_index.iter_mut()) {
            // Neighbouring blocks often hold the same script or the same gap.
            let found = if blocks.get(previous) == Some(block) {
                previous
            } else if let Some(found) = blocks.iter().position(|known| known == block) {
                found
            } else {
                blocks.push(*block);
                blocks.len() - 1
            };
            *position = u8::try_from(found).ok()?;
            previous = found;
        }
        Some(Self {
            ascii: u128::from(bmp[0][0]) | (u128::from(bmp[0][1]) << 64),
            block_index,
            blocks: blocks.into_boxed_slice(),
            astral: astral.into_boxed_slice(),
            min: usize::try_from(repetition.min).ok()?,
            max: usize::try_from(repetition.max?).ok()?,
        })
    }

    /// Whether `text` matches the pattern.
    #[must_use]
    pub fn is_match(&self, text: &str) -> bool {
        let mut count = 0;
        for c in text.chars() {
            if !self.contains(c) {
                return false;
            }
            count += 1;
            if count > self.max {
                return false;
            }
        }
        count >= self.min
    }

    fn contains(&self, c: char) -> bool {
        let code = u32::from(c);
        if code < 0x80 {
            return (self.ascii >> code) & 1 == 1;
        }
        if code <= BMP_END {
            let block = &self.blocks[usize::from(self.block_index[(code >> 8) as usize])];
            return (block[((code >> 6) & 3) as usize] >> (code & 63)) & 1 == 1;
        }
        let next = self.astral.partition_point(|&(_, end)| end < c);
        self.astral.get(next).is_some_and(|&(start, _)| start <= c)
    }
}

/// `hir` without the capture groups around it.
fn uncaptured(mut hir: &Hir) -> &Hir {
    while let HirKind::Capture(capture) = hir.kind() {
        hir = &capture.sub;
    }
    hir
}

#[cfg(test)]
mod tests {
    use super::{collapse_edge_counts, search_form, RepeatedClass, Search};
    use crate::to_rust_regex;
    use regex_syntax::ast::parse::Parser;
    use test_case::test_case;

    fn search(pattern: &str) -> Option<Search> {
        search_form(pattern, &to_rust_regex(pattern).expect("translates"))
    }

    // Callers may anchor the translation or search it, so counts stay as written.
    #[test_case("a{1,3}"; "count at an edge")]
    #[test_case(r"[^\p{C}]{1,2048}"; "count over a property class")]
    fn translation_keeps_counts(pattern: &str) {
        assert_eq!(to_rust_regex(pattern).unwrap(), pattern);
    }

    #[test_case(r"^.{1,2097152}", r"^[^\n\r\x{2028}\x{2029}]"; "dot under a huge count")]
    #[test_case(r"[^\p{C}]{1,2048}", r"[^\p{C}]"; "negated property class")]
    #[test_case(r"\d{1,5}", "[0-9]"; "translated class escape")]
    #[test_case(r"[a-zA-Z0-9_=-]{1,4096}", "[a-zA-Z0-9_=-]"; "ascii class")]
    fn search_collapses_edge_counts(pattern: &str, expected: &str) {
        assert_eq!(
            search(pattern),
            Some(Search::Collapsed(expected.to_string()))
        );
    }

    #[test_case(r"^.{0,262144}$"; "dot")]
    #[test_case(r"^(.{0,262144})$"; "capture group around the count")]
    #[test_case(r"^((.{0,262144}))$"; "nested capture groups around the count")]
    #[test_case(r"^([\s\S]{1,204800})$"; "capture group around every character")]
    #[test_case(r"^(?:.{1,262144}$)"; "end anchor inside a non-capturing group")]
    fn search_counts_repeated_classes(pattern: &str) {
        assert!(
            matches!(search(pattern), Some(Search::RepeatedClass(_))),
            "{pattern}"
        );
    }

    // Nothing to rewrite.
    #[test_case(r"^\S+@\S+$"; "no count")]
    #[test_case(r"xa{1,3}b"; "count in the middle")]
    #[test_case(r"^(ab){1,3}$"; "anchored count over two characters")]
    #[test_case(r"^[ab]{1,3}\b"; "word boundary after an anchored count")]
    // ECMA-262 rejects these in Unicode mode, and the engines read them differently.
    #[test_case(r"^[0-9]{1, 3}$"; "space inside the braces")]
    #[test_case(r"[0-9]{2, 4}"; "space inside the braces at an edge")]
    #[test_case(r"[ab]{2}{1,3}"; "count on a count")]
    #[test_case(r"^a{1,3}{1}$"; "count on an anchored count")]
    #[test_case(r"b+{1,3}"; "count on a plus")]
    // The engine rejects an atom that a zero minimum would drop.
    #[test_case(r"x\p{Bogus}{0,3}"; "unknown property under a zero minimum")]
    #[test_case(r"^[\p{Lettr}]{0,64}"; "unknown property in a class under a zero minimum")]
    fn search_declines(pattern: &str) {
        assert_eq!(search(pattern), None);
    }

    // Inputs are translated patterns, in Rust syntax.
    #[test_case("x{1,3}", Some("x"); "single atom")]
    #[test_case("[ab]{2,4}", Some("[ab]{2}"); "class above one")]
    #[test_case("^x{1,3}", Some("^x"); "anchored start")]
    #[test_case("x{0,3}", Some(""); "zero minimum alone")]
    #[test_case("ax{0,3}", Some("a"); "zero minimum at the tail")]
    #[test_case("x{0,3}a", Some("a"); "zero minimum at the head")]
    #[test_case("(?:ab){1,3}", Some("(?:ab)"); "non-capturing group atom")]
    #[test_case("(a|bc){0,2}", Some(""); "capturing group atom with alternation")]
    #[test_case("^x{2,3}|y{1,2}", Some("^x{2}|y"); "every top-level branch")]
    #[test_case("(x{1,3})", Some("(x)"); "inside a trailing group")]
    #[test_case("(?:x{1,3}|y{2,5})z", Some("(?:x|y{2})z"); "inside a leading group with branches")]
    #[test_case("x{1,3}?", Some("x"); "lazy")]
    #[test_case("x{2,3}?y", Some("x{2}y"); "lazy at the head")]
    #[test_case("x{1,3}y{2,4}", Some("xy{2}"); "both edges")]
    #[test_case("(x{1,3}){1,2}", Some("(x{1,3})"); "nested, outer count only")]
    #[test_case(r"\x{41}{1,3}", Some(r"\x{41}"); "braced escape atom")]
    #[test_case("x{1,3}$", Some("x$"); "head before an end anchor")]
    #[test_case(r"x{1,3}\b", Some(r"x\b"); "head before a word boundary")]
    #[test_case("ax{1,3}$", None; "end anchor follows")]
    #[test_case(r"ax{1,3}\b", None; "word boundary follows")]
    #[test_case("^x{1,3}$", None; "anchored both ends")]
    #[test_case("(x{1,3}){2}", None; "inside a counted group")]
    #[test_case("(x{1,3})+", None; "inside a repeated group")]
    #[test_case("ax{1,3}b", None; "in the middle")]
    #[test_case("x{3}", None; "exact count")]
    #[test_case("x{1,}", None; "open count")]
    #[test_case("x{2,2}", None; "range of one count")]
    #[test_case(r"\{1,3}", None; "escaped brace")]
    #[test_case("[{1,3}]", None; "braces in a class")]
    #[test_case("x{1,3}(?=y)", None; "lookahead does not parse")]
    #[test_case(r"(x)\1{1,3}", None; "backreference does not parse")]
    #[test_case("", None; "empty")]
    #[test_case("x*y+z?", None; "uncounted repetitions")]
    fn collapse(pattern: &str, expected: Option<&str>) {
        assert_eq!(collapsed(pattern).as_deref(), expected);
    }

    fn collapsed(pattern: &str) -> Option<String> {
        let ast = Parser::new().parse(pattern).ok()?;
        collapse_edge_counts(pattern, &ast)
    }

    fn strings_up_to(alphabet: &[char], length: usize) -> Vec<String> {
        let mut all = vec![String::new()];
        let mut layer = vec![String::new()];
        for _ in 0..length {
            layer = layer
                .iter()
                .flat_map(|prefix| {
                    alphabet.iter().map(move |c| {
                        let mut text = prefix.clone();
                        text.push(*c);
                        text
                    })
                })
                .collect();
            all.extend(layer.iter().cloned());
        }
        all
    }

    const ALPHABET: [char; 5] = ['a', 'b', 'c', '\n', '\u{1F600}'];

    // The collapsed and original translations agree on every string up to length 6.
    #[test_case("a{1,3}"; "single letter")]
    #[test_case("[ab]{2,4}"; "class above one")]
    #[test_case("^.{1,3}"; "anchored dot")]
    #[test_case("^.{2,4}"; "anchored dot above one")]
    #[test_case("(?:ab){1,3}"; "group atom")]
    #[test_case("(a|bc){0,2}"; "capturing alternation atom zero min")]
    #[test_case("c[ab]{0,3}"; "letter then zero min class")]
    #[test_case("^a{2,3}|b{1,2}"; "two branches")]
    #[test_case("(?:[ab]{1,3})"; "class in a non-capturing group")]
    #[test_case("([ab]{2,3})"; "class in a capture group")]
    #[test_case("[ab]{1,3}c"; "class then letter")]
    #[test_case("[ab]{2,3}$"; "class then end anchor")]
    #[test_case("(a|b)[ab]{2,3}"; "group then class")]
    #[test_case("[ab]{1,3}?"; "lazy class")]
    #[test_case("[ab]{0,3}?c"; "lazy zero min class then letter")]
    #[test_case("[^a]{1,3}"; "negated class")]
    #[test_case("(?i)A{1,3}"; "case-insensitive letter")]
    #[test_case("a{2,3}(?:b|c{1,2})"; "letter then group with inner count")]
    #[test_case("(?:[ab]{1,3}){2,3}"; "counted group with inner count")]
    #[test_case("(a{1,2}){1,2}"; "capture group with inner count")]
    #[test_case(r"\s{1,2}a"; "space class then letter")]
    #[test_case(".{0,2}a.{0,2}"; "dot both sides of a letter")]
    #[test_case("a{1,3}b{1,3}"; "two adjacent counts")]
    #[test_case("[ab]{3,5}"; "class from three")]
    #[test_case("^[ab]{3,5}"; "anchored class from three")]
    #[test_case("[ab]{3,5}c"; "class from three then letter")]
    #[test_case("a{0,2}b{2,4}"; "zero min then two")]
    fn collapsed_translation_matches_the_same_strings(pattern: &str) {
        let translated = to_rust_regex(pattern).unwrap();
        let original = regex::Regex::new(&translated).unwrap();
        let collapsed = regex::Regex::new(&collapsed(&translated).expect("collapses")).unwrap();
        for text in strings_up_to(&ALPHABET, 6) {
            assert_eq!(
                original.is_match(&text),
                collapsed.is_match(&text),
                "{pattern} on {text:?}"
            );
        }
    }

    fn repeated_class(pattern: &str) -> Option<RepeatedClass> {
        RepeatedClass::from_translated(&to_rust_regex(pattern).expect("translates"))
    }

    #[test_case(r"^[ab]{1,3}"; "no end anchor")]
    #[test_case(r"[ab]{1,3}$"; "no start anchor")]
    #[test_case(r"^[ab]+$"; "open count")]
    #[test_case(r"^[ab]{1,}$"; "open braced count")]
    #[test_case(r"(?m)^[ab]{1,3}$"; "line anchors")]
    #[test_case(r"^[ab]{1,3}c$"; "more after the class")]
    #[test_case(r"^c[ab]{1,3}$"; "more before the class")]
    #[test_case(r"^(?:ab){1,3}$"; "two characters repeated")]
    #[test_case(r"^(ab){1,3}$"; "captured two characters repeated")]
    #[test_case(r"^[ab]{1,3}|c$"; "alternation")]
    #[test_case(r"^$"; "no repetition")]
    #[test_case(r"^[ab]$"; "no count")]
    #[test_case(r"^\b[ab]{1,3}$"; "word boundary")]
    #[test_case(r"^[ab]{1,3}(?=a)$"; "lookahead")]
    #[test_case(r"^[ab]{1,3}\b"; "word boundary after the class")]
    #[test_case(r"\b[ab]{1,3}$"; "word boundary before the class")]
    #[test_case(r"^(?-u:[ab]){1,3}$"; "byte class")]
    #[test_case(r"^a{1,3}$"; "one character")]
    #[test_case(r"^[^\s\S]{0,2}$"; "empty class")]
    fn repeated_class_declined(pattern: &str) {
        assert!(repeated_class(pattern).is_none(), "{pattern}");
    }

    #[test_case(r"^[ab]{2,3}$", "", false; "empty under min")]
    #[test_case(r"^[ab]{2,3}$", "a", false; "one under min")]
    #[test_case(r"^[ab]{2,3}$", "ab", true; "at min")]
    #[test_case(r"^[ab]{2,3}$", "aba", true; "at max")]
    #[test_case(r"^[ab]{2,3}$", "abab", false; "one over max")]
    #[test_case(r"^[ab]{2,3}$", "abc", false; "outside the class")]
    #[test_case(r"^[ab]{0,2}$", "", true; "empty at zero min")]
    #[test_case(r"^.{0,2}$", "a\n", false; "line feed under dot")]
    #[test_case(r"^.{0,2}$", "a\u{2028}", false; "line separator under dot")]
    #[test_case(r"^.{0,2}$", "\u{1F600}\u{1F600}", true; "astral characters count once")]
    #[test_case(r"^.{0,2}$", "\u{1F600}\u{1F600}\u{1F600}", false; "three astral characters")]
    #[test_case(r"^[\s\S]{1,2}$", "\n", true; "line feed under every character")]
    #[test_case(r"^[^\p{C}]{1,2}$", "\u{85}", false; "control character under negated property")]
    #[test_case(r"^[^\p{C}]{1,2}$", "é", true; "letter under negated property")]
    #[test_case(r"(?i)^[a-b]{1,2}$", "AB", true; "case-insensitive")]
    #[test_case(r"^[ab]{0,2147483647}$", "abab", true; "largest count")]
    fn repeated_class_matches(pattern: &str, text: &str, expected: bool) {
        assert_eq!(
            repeated_class(pattern)
                .expect("repeated class")
                .is_match(text),
            expected
        );
    }

    const CLASS_ALPHABET: [char; 10] = [
        'a',
        'b',
        '0',
        '\t',
        '\n',
        '\r',
        '\u{2028}',
        '\u{85}',
        'é',
        '\u{1F600}',
    ];

    // The matcher and the engine agree on every string up to length 4.
    #[test_case(r"^[ab]{1,3}$"; "class")]
    #[test_case(r"^.{0,2}$"; "dot")]
    #[test_case(r"^[^a]{2,3}$"; "negated class")]
    #[test_case(r"^(.){1,2}$"; "capture group around the class")]
    #[test_case(r"^[\s\S]{0,3}$"; "every character")]
    #[test_case(r"^[^\p{C}]{1,3}$"; "negated property")]
    #[test_case(r"^\w{2}$"; "exact count")]
    #[test_case(r"^[ab]{1,3}?$"; "lazy")]
    #[test_case(r"^\S{1,3}$"; "non-space")]
    #[test_case(r"^\s{0,3}$"; "space")]
    #[test_case(r"(?i)^[a-b]{1,3}$"; "case-insensitive")]
    #[test_case(r"^\d{0,2}$"; "digit")]
    #[test_case(r"^[\x09\x0a\x0d -ÿ]{1,3}$"; "range up to a two-byte character")]
    #[test_case(r"^[^\n]{1,3}$"; "negated line feed")]
    #[test_case(r"^(?:[ab]){0,2}$"; "non-capturing group around the class")]
    fn repeated_class_agrees_with_the_engine(pattern: &str) {
        let class = repeated_class(pattern).expect("repeated class");
        let regex = regex::Regex::new(&to_rust_regex(pattern).unwrap()).unwrap();
        for text in strings_up_to(&CLASS_ALPHABET, 4) {
            assert_eq!(
                class.is_match(&text),
                regex.is_match(&text),
                "{pattern} on {text:?}"
            );
        }
    }

    // Longer strings over two letters reach bounds the wider alphabet cannot.
    #[test_case(r"^[ab]{3,6}$"; "class from three")]
    #[test_case(r"^[ab]{0,5}$"; "class from zero")]
    #[test_case(r"^[^b]{4,4}$"; "negated letter exact")]
    #[test_case(r"^.{5,7}$"; "dot from five")]
    #[test_case(r"^[ab]{7,7}$"; "class exact seven")]
    fn repeated_class_agrees_with_the_engine_on_long_strings(pattern: &str) {
        let class = repeated_class(pattern).expect("repeated class");
        let regex = regex::Regex::new(&to_rust_regex(pattern).unwrap()).unwrap();
        for text in strings_up_to(&['a', 'b'], 8) {
            assert_eq!(
                class.is_match(&text),
                regex.is_match(&text),
                "{pattern} on {text:?}"
            );
        }
    }

    // Inputs are translated classes, in Rust syntax. Every character of the BMP, and every 17th
    // past it.
    #[test_case(r"\p{L}"; "letters")]
    #[test_case(r"[^\p{C}]"; "negated property")]
    #[test_case(r"[\p{L}\p{N}]"; "letters and numbers")]
    #[test_case(r"\w"; "word characters")]
    #[test_case(r"[^\n\r\x{2028}\x{2029}]"; "dot")]
    #[test_case(r"[\x{7E}-\x{81}\x{FF}-\x{101}\x{13F}-\x{140}\x{FFFF}-\x{10001}]"; "ranges across block and plane edges")]
    #[test_case(r"[\x{10000}-\x{10FFFF}]"; "only past the BMP")]
    #[test_case(r"[a-z]"; "only ascii")]
    fn repeated_class_agrees_with_the_engine_on_every_character(class: &str) {
        let pattern = format!("^{class}{{1,2}}$");
        let repeated = RepeatedClass::from_translated(&pattern).expect("repeated class");
        let regex = regex::Regex::new(&pattern).unwrap();
        let codes = (0..=0xFFFF).chain((0x1_0000..=0x10_FFFF).step_by(17));
        for c in codes.filter_map(char::from_u32) {
            let text = c.to_string();
            assert_eq!(
                repeated.is_match(&text),
                regex.is_match(&text),
                "{class} on {c:?}"
            );
        }
    }
}

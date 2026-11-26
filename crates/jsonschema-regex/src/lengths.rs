//! How long a string holding a match of a pattern can be.
use regex_syntax::hir::{self, Hir, HirKind, Look, Visitor};

/// The lengths, in Unicode scalar values, of the strings a search for a pattern finds a match in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchLengths {
    /// No shorter string holds a match.
    pub minimum: u64,
    /// No longer string holds a match, or `None` where no such length is known.
    pub maximum: Option<u64>,
}

impl SearchLengths {
    /// The lengths for a pattern already translated by [`crate::to_rust_regex`], or `None` where
    /// the pattern is not one the `regex` syntax reads.
    ///
    /// A search finds a match anywhere in a string, so the shortest match bounds the string from
    /// below, and the longest one bounds it from above only where every match spans the whole
    /// string: `^` and `$` hold at both ends without `(?m)`.
    #[must_use]
    pub fn from_translated(translated: &str) -> Option<Self> {
        let hir = regex_syntax::parse(translated).ok()?;
        let matched = hir::visit(&hir, MatchLengths::default()).ok()?;
        let properties = hir.properties();
        let spans_whole_string = properties.look_set_prefix().contains(Look::Start)
            && properties.look_set_suffix().contains(Look::End);
        Some(Self {
            minimum: matched.minimum,
            maximum: matched.maximum.filter(|_| spans_whole_string),
        })
    }
}

/// The lengths of the text one part of a pattern matches.
#[derive(Debug, Clone, Copy)]
struct Matched {
    minimum: u64,
    /// `None` where the part has no longest match, or none this walk can tell.
    maximum: Option<u64>,
}

impl Matched {
    fn exactly(length: u64) -> Self {
        Self {
            minimum: length,
            maximum: Some(length),
        }
    }
}

/// Lengths of the parts visited so far, children before the part holding them.
#[derive(Default)]
struct MatchLengths {
    finished: Vec<Matched>,
}

impl MatchLengths {
    fn take(&mut self, count: usize) -> std::vec::Drain<'_, Matched> {
        self.finished.drain(self.finished.len() - count..)
    }
}

impl Visitor for MatchLengths {
    type Output = Matched;
    type Err = ();

    fn finish(mut self) -> Result<Matched, ()> {
        self.finished.pop().ok_or(())
    }

    fn visit_post(&mut self, hir: &Hir) -> Result<(), ()> {
        let matched = match hir.kind() {
            // The parser keeps literals valid UTF-8, where every scalar value has one byte that is
            // not a continuation byte.
            HirKind::Literal(literal) => Matched::exactly(
                literal
                    .0
                    .iter()
                    .filter(|byte| (**byte & 0xC0) != 0x80)
                    .count() as u64,
            ),
            // One scalar value: a class of bytes holds only ASCII when the parser demands UTF-8.
            HirKind::Class(_) => Matched::exactly(1),
            HirKind::Empty
            | HirKind::Look(
                Look::Start
                | Look::End
                | Look::StartLF
                | Look::EndLF
                | Look::StartCRLF
                | Look::EndCRLF
                | Look::WordAscii
                | Look::WordAsciiNegate
                | Look::WordUnicode
                | Look::WordUnicodeNegate,
            ) => Matched::exactly(0),
            // Written with escapes another engine may read as literal characters, which would make
            // its matches longer.
            HirKind::Look(_) => Matched {
                minimum: 0,
                maximum: None,
            },
            HirKind::Capture(_) => return Ok(()),
            HirKind::Repetition(repetition) => {
                let repeated = self.finished.pop().ok_or(())?;
                Matched {
                    minimum: repeated.minimum.saturating_mul(u64::from(repetition.min)),
                    maximum: match (repeated.maximum, repetition.max) {
                        (Some(0), _) => Some(0),
                        (Some(length), Some(count)) => length.checked_mul(u64::from(count)),
                        (Some(_), None) | (None, _) => None,
                    },
                }
            }
            HirKind::Concat(parts) => {
                self.take(parts.len())
                    .fold(Matched::exactly(0), |total, part| Matched {
                        minimum: total.minimum.saturating_add(part.minimum),
                        maximum: total
                            .maximum
                            .zip(part.maximum)
                            .and_then(|(total, part)| total.checked_add(part)),
                    })
            }
            HirKind::Alternation(branches) => self
                .take(branches.len())
                .reduce(|widest, branch| Matched {
                    minimum: widest.minimum.min(branch.minimum),
                    maximum: widest
                        .maximum
                        .zip(branch.maximum)
                        .map(|(widest, branch)| widest.max(branch)),
                })
                .ok_or(())?,
        };
        self.finished.push(matched);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SearchLengths;
    use crate::to_rust_regex;
    use test_case::test_case;

    fn lengths(pattern: &str) -> Option<SearchLengths> {
        SearchLengths::from_translated(&to_rust_regex(pattern).expect("translates"))
    }

    #[test_case("^VC-[0-9]{4}$", 7, Some(7) ; "literal and counted class")]
    #[test_case("^$", 0, Some(0) ; "empty string only")]
    #[test_case("^a?b{2,5}$", 2, Some(6) ; "optional and ranged counts")]
    #[test_case("^(?:ab|abcdef)$", 2, Some(6) ; "alternatives of different lengths")]
    #[test_case("^(?:^ab$|^abcdef$)$", 2, Some(6) ; "anchors repeated inside alternatives")]
    #[test_case("^ab$|^abcdef$", 2, Some(6) ; "every alternative anchored")]
    #[test_case("^(a{2}){3}$", 6, Some(6) ; "nested counts")]
    #[test_case("^(?<year>[0-9]{4})-(?<month>[0-9]{2})$", 7, Some(7) ; "named groups")]
    #[test_case(r"^\bab\b$", 2, Some(2) ; "word boundaries")]
    #[test_case(r"^\d\w\s.$", 4, Some(4) ; "class escapes and dot")]
    #[test_case("^[^a]$", 1, Some(1) ; "negated class")]
    #[test_case("^a{2,3}?$", 2, Some(3) ; "lazy count")]
    #[test_case("^\u{1F600}{2}$", 2, Some(2) ; "literal outside the Basic Multilingual Plane")]
    #[test_case("^\u{e9}\u{4e2d}$", 2, Some(2) ; "multi-byte literals")]
    #[test_case(r"^\u{1F600}$", 1, Some(1) ; "escaped code point")]
    #[test_case("^(?i)stra\u{df}e$", 6, Some(6) ; "case-insensitive literal")]
    fn whole_string(pattern: &str, minimum: u64, maximum: Option<u64>) {
        assert_eq!(lengths(pattern), Some(SearchLengths { minimum, maximum }));
    }

    #[test_case("abc", 3 ; "unanchored")]
    #[test_case("^abc", 3 ; "anchored at the start only")]
    #[test_case("abc$", 3 ; "anchored at the end only")]
    #[test_case("^a+$", 1 ; "unbounded repetition")]
    #[test_case("^a{2,}$", 2 ; "open count")]
    #[test_case("^(a*){3}$", 0 ; "unbounded repetition under a count")]
    #[test_case("^a|b$", 1 ; "each alternative anchored at one end")]
    #[test_case("^(?:a|b$)", 1 ; "one alternative anchored at the end")]
    #[test_case("(^)?a$", 1 ; "optional start anchor")]
    #[test_case("^a$|b", 1 ; "one alternative unanchored")]
    #[test_case("", 0 ; "empty pattern")]
    #[test_case("a*", 0 ; "matches the empty string")]
    fn no_longest_string(pattern: &str, minimum: u64) {
        assert_eq!(
            lengths(pattern),
            Some(SearchLengths {
                minimum,
                maximum: None
            })
        );
    }

    #[test_case("(?m)^ab$", 2, None ; "multi-line anchors")]
    #[test_case(r"\Aab\z", 2, Some(2) ; "absolute anchors")]
    #[test_case("(?m)\\Aab\\z", 2, Some(2) ; "absolute anchors in multi-line mode")]
    #[test_case(r"^\<ab\>$", 2, None ; "word start and end assertions")]
    #[test_case("^(?-u:[a-z]){3}$", 3, Some(3) ; "class of bytes")]
    #[test_case("^a{4294967295}{4294967295}{4294967295}$", u64::MAX, None ; "lengths past the counter")]
    fn engine_syntax(translated: &str, minimum: u64, maximum: Option<u64>) {
        assert_eq!(
            SearchLengths::from_translated(translated),
            Some(SearchLengths { minimum, maximum })
        );
    }

    #[test_case("^(?=a)a$" ; "lookahead")]
    #[test_case("^(?<!b)a$" ; "lookbehind")]
    #[test_case(r"^(a)\1$" ; "back reference")]
    #[test_case("^(?>a)$" ; "atomic group")]
    #[test_case("^a{2" ; "unterminated count")]
    fn not_derived(translated: &str) {
        assert_eq!(SearchLengths::from_translated(translated), None);
    }

    // Every string a pattern matches lies inside the lengths derived for it.
    #[test_case("^VC-[0-9]{4}$", "VC-1234" ; "exact length")]
    #[test_case("^(?:ab|abcdef)$", "ab" ; "shorter alternative")]
    #[test_case("^(?:ab|abcdef)$", "abcdef" ; "longer alternative")]
    #[test_case("b{2,3}", "bb" ; "unanchored count alone")]
    #[test_case("b{2,3}", "abbbbbbbbc" ; "unanchored count inside a longer string")]
    #[test_case("^.{1,2}$", "\u{1F600}" ; "dot over one character of four bytes")]
    #[test_case("^.{1,2}$", "\u{1F600}\u{1F600}" ; "dot over two characters of four bytes")]
    #[test_case("^[\u{e9}\u{1F600}]{3}$", "\u{e9}\u{1F600}\u{e9}" ; "class over characters of several bytes")]
    #[test_case(r"^\w+$", "a_1" ; "unbounded word characters")]
    #[test_case("^(?i)\u{212a}$", "k" ; "case folding to one byte")]
    #[test_case("^(?i)k$", "\u{212a}" ; "case folding to three bytes")]
    fn matches_lie_inside(pattern: &str, text: &str) {
        let translated = to_rust_regex(pattern).expect("translates");
        let regex = regex::Regex::new(&translated).expect("compiles");
        assert!(regex.is_match(text));
        let lengths = SearchLengths::from_translated(&translated).expect("derived");
        let length = text.chars().count() as u64;
        assert!(lengths.minimum <= length);
        assert!(lengths.maximum.is_none_or(|maximum| length <= maximum));
    }
}

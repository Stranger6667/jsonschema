use regex_syntax::hir::{Class, Hir, HirKind, Look};

const MAX_SLOTS: usize = 63;
const MAX_SKIP_RUN: u32 = 8;
const MAX_SINGLE_BOUND: u32 = 1000;
/// Class ranges times repetitions over the whole pattern. Keeps the fallback regex for any
/// accepted pattern far below the `regex` crate's default size limit (Unicode `\w`, about 770
/// ranges, compiles to about 45 KB).
const MAX_RANGE_BUDGET: u64 = 16_384;
/// Branches an alternation expands into.
const MAX_BRANCHES: usize = 8;

/// An anchored sequence of ASCII character classes, each matched once, optionally or any number of
/// times, or a few alternatives of one, decided without a regex engine for ASCII subjects.
#[derive(Debug, Clone)]
pub struct Sequence {
    form: Form,
}

#[derive(Debug, Clone)]
enum Form {
    /// One class repeated between `min` and `max` times, up to the end of input.
    Single {
        members: Box<[bool; 128]>,
        min: usize,
        max: Option<usize>,
    },
    /// One state bit per slot boundary. Branches sit side by side, each followed by its accepting
    /// bit; no byte fills that bit, so a state never moves from one branch into the next.
    Parallel {
        /// The slots each byte can fill.
        table: Box<[u64; 128]>,
        start: u64,
        star: u64,
        skippable: u64,
        longest_skip_run: u32,
        accept: u64,
        anchored_end: bool,
    },
}

struct Atom {
    /// Bit `b` set when ASCII byte `b` is in the class.
    class: u128,
    /// Ranges of the full (Unicode) class, which the fallback regex compiles.
    ranges: u64,
    min: u32,
    max: Option<u32>,
}

impl Sequence {
    /// Compile a pattern already translated by [`crate::to_rust_regex`]. `None` when it is not an
    /// anchored sequence of literals, classes and their repetitions, or a few alternatives of one,
    /// within the size limits.
    #[must_use]
    pub fn compile(translated: &str) -> Option<Sequence> {
        // Skip parsing what is never a sequence: no leading `^` or quantified groups.
        if !translated.starts_with('^')
            || translated
                .as_bytes()
                .windows(2)
                .any(|pair| pair[0] == b')' && matches!(pair[1], b'?' | b'*' | b'+' | b'{'))
        {
            return None;
        }
        let hir = regex_syntax::Parser::new().parse(translated).ok()?;
        let mut items = Vec::new();
        flatten(&hir, &mut items);
        let mut budget = 0;
        let mut anchored_end = None;
        let mut branches = Vec::new();
        for branch in expand(&items)? {
            let (end, atoms) = branch_atoms(&branch, &mut budget)?;
            // One state decides every branch, so they all end the same way.
            if *anchored_end.get_or_insert(end) != end {
                return None;
            }
            branches.push(atoms);
        }
        if budget > MAX_RANGE_BUDGET {
            return None;
        }
        let anchored_end = anchored_end?;
        if let [atoms] = branches.as_slice() {
            if let [atom] = atoms.as_slice() {
                if anchored_end
                    && atom.min <= MAX_SINGLE_BOUND
                    && atom.max.is_none_or(|max| max <= MAX_SINGLE_BOUND)
                {
                    let mut members = Box::new([false; 128]);
                    for (byte, member) in members.iter_mut().enumerate() {
                        *member = atom.class >> byte & 1 == 1;
                    }
                    return Some(Sequence {
                        form: Form::Single {
                            members,
                            min: atom.min as usize,
                            max: atom.max.map(|max| max as usize),
                        },
                    });
                }
            }
        }
        parallel(&branches, anchored_end).map(|form| Sequence { form })
    }

    /// `None` when the answer needs the regex engine: a non-ASCII byte was reached before the
    /// outcome was known.
    #[must_use]
    #[inline]
    pub fn is_match(&self, text: &[u8]) -> Option<bool> {
        match &self.form {
            Form::Single { members, min, max } => {
                if text.len() < *min {
                    return Some(false);
                }
                for &byte in text {
                    if !*members.get(byte as usize)? {
                        return Some(false);
                    }
                }
                Some(max.is_none_or(|max| text.len() <= max))
            }
            Form::Parallel {
                table,
                start,
                star,
                skippable,
                longest_skip_run,
                accept,
                anchored_end,
            } => {
                let close = |mut state: u64| {
                    for _ in 0..*longest_skip_run {
                        state |= (state & skippable) << 1;
                    }
                    state
                };
                let mut state = close(*start);
                for &byte in text {
                    if !anchored_end && state & accept != 0 {
                        return Some(true);
                    }
                    let hit = state & table.get(byte as usize)?;
                    state = close(((hit & !star) << 1) | (hit & star));
                    if state == 0 {
                        return Some(false);
                    }
                }
                Some(state & accept != 0)
            }
        }
    }
}

// One branch: whether it is anchored at the end, and its literals, classes and repetitions.
fn branch_atoms(items: &[&Hir], budget: &mut u64) -> Option<(bool, Vec<Atom>)> {
    let (first, rest) = items.split_first()?;
    if !matches!(first.kind(), HirKind::Look(Look::Start)) {
        return None;
    }
    let (anchored_end, middle) = match rest.split_last() {
        Some((last, middle)) if matches!(last.kind(), HirKind::Look(Look::End)) => (true, middle),
        _ => (false, rest),
    };
    let mut atoms = Vec::new();
    for item in middle {
        push_atoms(item, &mut atoms)?;
    }
    *budget = atoms
        .iter()
        .map(|atom| {
            let repetitions = atom.max.map_or(u64::from(atom.min) + 1, u64::from);
            atom.ranges.saturating_mul(repetitions.max(1))
        })
        .fold(*budget, u64::saturating_add);
    Some((anchored_end, atoms))
}

// Every way through the alternations among `items`, as a list of items without any.
fn expand<'h>(items: &[&'h Hir]) -> Option<Vec<Vec<&'h Hir>>> {
    let mut branches = vec![Vec::new()];
    for &item in items {
        let HirKind::Alternation(alternatives) = item.kind() else {
            for branch in &mut branches {
                branch.push(item);
            }
            continue;
        };
        let mut next = Vec::new();
        for alternative in alternatives {
            let mut sub = Vec::new();
            flatten(alternative, &mut sub);
            for tail in expand(&sub)? {
                for head in &branches {
                    if next.len() == MAX_BRANCHES {
                        return None;
                    }
                    next.push(head.iter().chain(&tail).copied().collect());
                }
            }
        }
        branches = next;
    }
    Some(branches)
}

fn flatten<'h>(hir: &'h Hir, items: &mut Vec<&'h Hir>) {
    match hir.kind() {
        HirKind::Concat(subs) => subs.iter().for_each(|sub| flatten(sub, items)),
        HirKind::Capture(capture) => flatten(&capture.sub, items),
        HirKind::Empty => {}
        _ => items.push(hir),
    }
}

fn push_atoms(hir: &Hir, atoms: &mut Vec<Atom>) -> Option<()> {
    match hir.kind() {
        HirKind::Literal(literal) => {
            for c in std::str::from_utf8(&literal.0).ok()?.chars() {
                atoms.push(Atom {
                    class: char_class(c),
                    ranges: 1,
                    min: 1,
                    max: Some(1),
                });
            }
        }
        HirKind::Class(class) => {
            let (class, ranges) = class_members(class)?;
            atoms.push(Atom {
                class,
                ranges,
                min: 1,
                max: Some(1),
            });
        }
        HirKind::Repetition(repetition) => {
            // `compile` declines quantified groups before parsing, so no capture wraps `sub`.
            let (class, ranges) = match repetition.sub.kind() {
                HirKind::Class(class) => class_members(class)?,
                HirKind::Literal(literal) => {
                    let mut chars = std::str::from_utf8(&literal.0).ok()?.chars();
                    let (Some(c), None) = (chars.next(), chars.next()) else {
                        return None;
                    };
                    (char_class(c), 1)
                }
                _ => return None,
            };
            atoms.push(Atom {
                class,
                ranges,
                min: repetition.min,
                max: repetition.max,
            });
        }
        _ => return None,
    }
    Some(())
}

fn char_class(c: char) -> u128 {
    if c.is_ascii() {
        1 << c as u32
    } else {
        0
    }
}

/// The ASCII members of `class` and the number of ranges in the full class; `None` for a byte
/// class.
fn class_members(class: &Class) -> Option<(u128, u64)> {
    let Class::Unicode(class) = class else {
        return None;
    };
    let mut members = 0u128;
    for range in class.ranges() {
        for c in range.start() as u32..=(range.end() as u32).min(127) {
            members |= 1 << c;
        }
    }
    Some((members, class.ranges().len() as u64))
}

// Slots laid out so far, in bit order.
struct Layout {
    table: Box<[u64; 128]>,
    star: u64,
    skippable: u64,
    slots: usize,
    run: u32,
    longest_skip_run: u32,
}

impl Layout {
    fn push(&mut self, class: u128, repeat: Repeat) -> Option<()> {
        if self.slots == MAX_SLOTS {
            return None;
        }
        let bit = 1u64 << self.slots;
        for (byte, slot) in self.table.iter_mut().enumerate() {
            if class >> byte & 1 == 1 {
                *slot |= bit;
            }
        }
        if matches!(repeat, Repeat::Once) {
            self.run = 0;
        } else {
            self.skippable |= bit;
            self.run += 1;
            self.longest_skip_run = self.longest_skip_run.max(self.run);
            if self.run > MAX_SKIP_RUN {
                return None;
            }
        }
        if matches!(repeat, Repeat::Star) {
            self.star |= bit;
        }
        self.slots += 1;
        Some(())
    }
}

fn parallel(branches: &[Vec<Atom>], anchored_end: bool) -> Option<Form> {
    let mut layout = Layout {
        table: Box::new([0u64; 128]),
        star: 0,
        skippable: 0,
        slots: 0,
        run: 0,
        longest_skip_run: 0,
    };
    let (mut start, mut accept) = (0u64, 0u64);
    for atoms in branches {
        if layout.slots > MAX_SLOTS {
            return None;
        }
        start |= 1 << layout.slots;
        layout.run = 0;
        for atom in atoms {
            if atom.min as usize > MAX_SLOTS {
                return None;
            }
            for _ in 0..atom.min {
                layout.push(atom.class, Repeat::Once)?;
            }
            match atom.max {
                None => layout.push(atom.class, Repeat::Star)?,
                Some(max) => {
                    if (max - atom.min) as usize > MAX_SLOTS {
                        return None;
                    }
                    for _ in atom.min..max {
                        layout.push(atom.class, Repeat::Optional)?;
                    }
                }
            }
        }
        if layout.slots > MAX_SLOTS {
            return None;
        }
        accept |= 1 << layout.slots;
        layout.slots += 1;
    }
    Some(Form::Parallel {
        table: layout.table,
        start,
        star: layout.star,
        skippable: layout.skippable,
        longest_skip_run: layout.longest_skip_run,
        accept,
        anchored_end,
    })
}

#[derive(Clone, Copy)]
enum Repeat {
    Once,
    Optional,
    Star,
}

#[cfg(test)]
mod tests {
    use super::Sequence;
    use crate::to_rust_regex;
    use test_case::test_case;

    fn compile(pattern: &str) -> Option<Sequence> {
        Sequence::compile(&to_rust_regex(pattern).expect("translates"))
    }

    #[test_case(r"^[a-z]+$", "abc", Some(true); "class plus")]
    #[test_case(r"^[a-z]+$", "", Some(false); "class plus empty")]
    #[test_case(r"^[a-z]+$", "aB", Some(false); "class plus outside")]
    #[test_case(r"^#[0-9a-fA-F]{6}$", "#00ffAA", Some(true); "hex colour")]
    #[test_case(r"^#[0-9a-fA-F]{6}$", "#00ffA", Some(false); "hex colour short")]
    #[test_case(r"^#[0-9a-fA-F]{6}$", "#00ffAA0", Some(false); "hex colour long")]
    #[test_case(r"^[a-z]{1,2}$", "ab", Some(true); "bounded at max")]
    #[test_case(r"^[a-z]{1,2}$", "abc", Some(false); "bounded over max")]
    #[test_case(r"^.+$", "a b", Some(true); "dot plus")]
    #[test_case(r"^[^\n\r]+$", "a\rb", Some(false); "negated class")]
    #[test_case(r"^a?b*c$", "c", Some(true); "skip both")]
    #[test_case(r"^a?b*c$", "abbbc", Some(true); "take both")]
    #[test_case(r"^a?b*c$", "aac", Some(false); "optional taken twice")]
    #[test_case(r"^a?b*c$", "ab", Some(false); "missing last")]
    #[test_case(r"^(a)$", "a", Some(true); "group around one class")]
    #[test_case(r"^()a$", "a", Some(true); "empty group")]
    #[test_case(r"^()a$", "", Some(false); "empty group consumes nothing")]
    #[test_case(r"^$", "", Some(true); "empty pattern on empty")]
    #[test_case(r"^$", "a", Some(false); "empty pattern on text")]
    #[test_case(r"^[a-z]{2}", "abz9", Some(true); "unanchored end")]
    #[test_case(r"^[a-z]{2}", "a9", Some(false); "unanchored end too short")]
    #[test_case(r"^[a-z][0-9]*x$", "a12x", Some(true); "star between")]
    #[test_case(r"^[a-z]{2,}$", "abcdefghijk", Some(true); "open bound")]
    #[test_case(r"^[a-z]{2,}$", "a", Some(false); "open bound under")]
    #[test_case(r"^\d+[:-]\d+$", "8080:80", Some(true); "port mapping")]
    #[test_case(r"^\d+[:-]\d+$", "8080:", Some(false); "port mapping missing")]
    #[test_case(r"^\d+$", "\u{661}", None; "non-ascii digit needs the engine")]
    #[test_case(r"^\s$", "\u{a0}", None; "non-ascii whitespace needs the engine")]
    #[test_case(r"^\s$", "\t", Some(true); "ascii whitespace")]
    #[test_case(r"^.$", "\r", Some(false); "dot excludes carriage return")]
    #[test_case(r"^abc$", "abc\n", Some(false); "end means end of input")]
    #[test_case(r"^[a-z]+$", "café", None; "non-ascii subject")]
    #[test_case(r"^[a-zé]+$", "abc", Some(true); "non-ascii class member")]
    #[test_case(r"^[a-zé]+$", "café", None; "non-ascii class member on non-ascii")]
    #[test_case(r"^[a-z]{2}", "ab\u{e9}", Some(true); "match before non-ascii")]
    #[test_case(r"^[a-z]{1,100}$", &"a".repeat(100), Some(true); "single class long at max")]
    #[test_case(r"^[a-z]{1,100}$", &"a".repeat(101), Some(false); "single class long over max")]
    #[test_case(r"^[a-z]{1,1000}$", "a", Some(true); "single class at cap")]
    #[test_case(r"^\p{L}+[0-9]$", "ab1", Some(true); "wide class within the range budget")]
    #[test_case(r"^[1-5](?:[0-9]{2}|XX)$", "200", Some(true); "alternation first branch")]
    #[test_case(r"^[1-5](?:[0-9]{2}|XX)$", "4XX", Some(true); "alternation second branch")]
    #[test_case(r"^[1-5](?:[0-9]{2}|XX)$", "6XX", Some(false); "alternation shared head fails")]
    #[test_case(r"^[1-5](?:[0-9]{2}|XX)$", "20", Some(false); "alternation too short")]
    #[test_case(r"^[1-5](?:[0-9]{2}|XX)$", "2XXX", Some(false); "alternation too long")]
    #[test_case(r"^(?:ab|cde)f$", "cdef", Some(true); "alternation of different lengths")]
    #[test_case(r"^(ab|cd)$", "cd", Some(true); "capturing alternation")]
    #[test_case(r"^a(?:b(?:cc|dd)|ee)$", "abdd", Some(true); "nested alternation")]
    #[test_case(r"^a(?:b(?:cc|dd)|ee)$", "aee", Some(true); "nested alternation outer branch")]
    #[test_case(r"^a(?:b(?:cc|dd)|ee)$", "abee", Some(false); "nested alternation mixed")]
    #[test_case(r"^(?:ab|cd)", "cdz", Some(true); "alternation with unanchored end")]
    #[test_case(r"^(?:\d\d|xx)$", "\u{661}\u{661}", None; "undecided branch needs the engine")]
    #[test_case(r"^(?:\d\d|xx)$", "xx", Some(true); "decided branch wins over undecided")]
    #[test_case(r"^(?:a|bc)$", "abc", Some(false); "no state crosses into the next branch")]
    fn matches(pattern: &str, text: &str, expected: Option<bool>) {
        let sequence = compile(pattern).expect("compiles");
        assert_eq!(sequence.is_match(text.as_bytes()), expected);
    }

    #[test_case(r"[a-z]+"; "unanchored start")]
    #[test_case(r"^a|b$"; "alternation")]
    #[test_case(r"^(ab){2}$"; "quantified group")]
    #[test_case(r"^(?:\.[a-z]+)*$"; "quantified sequence group")]
    #[test_case(r"a^b"; "anchor in the middle")]
    #[test_case(r"^(a)+$"; "quantified group around one class")]
    #[test_case(r"^[a-z][a-z0-9-_]{1,63}$"; "62 optional slots in a row")]
    #[test_case(r"^a{32}b{32}$"; "64 slots")]
    #[test_case(r"^*abc"; "quantified start anchor")]
    #[test_case(r"^a\b*c$"; "quantified assertion")]
    #[test_case(r"^(?-u)[a-z]+$"; "byte class")]
    #[test_case(r"^a?b?c?d?e?f?g?h?i?$"; "nine skippable slots")]
    #[test_case(r"^[a-z]{1,1001}$"; "single class over cap")]
    #[test_case(r"^[a-z]{1001,}$"; "single class open over cap")]
    #[test_case(r"^\bx$"; "word boundary")]
    #[test_case(r"^\p{L}{1,1000}$"; "wide class over the range budget")]
    #[test_case(r"^(?:aa|bb)(?:cc|dd)(?:ee|ff)(?:gg|hh)$"; "sixteen branches")]
    #[test_case(r"^ab|cd$"; "branch without the start anchor")]
    #[test_case(r"^(?:ab$|cd)"; "branches ending differently")]
    fn declined(pattern: &str) {
        assert!(compile(pattern).is_none());
    }

    // 63 slots, the most the state holds: the final boundary is its top bit.
    #[test_case(30, Some(false); "one short")]
    #[test_case(31, Some(true); "every slot")]
    #[test_case(32, Some(false); "one over")]
    fn sixty_three_slots(trailing: usize, expected: Option<bool>) {
        let sequence = compile(r"^a{32}b{31}$").expect("63 slots fit");
        let text = format!("{}{}", "a".repeat(32), "b".repeat(trailing));
        assert_eq!(sequence.is_match(text.as_bytes()), expected);
    }
}

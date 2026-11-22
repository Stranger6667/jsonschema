//! JSON text representation: validates a document's bytes in place, without building a tree.
//!
//! [`JsonText::parse`] checks the whole document once, as `serde_json` would, and records one
//! token per key and value. Nodes then read the text through those tokens without scanning it.

#[cfg(target_arch = "aarch64")]
use std::arch::aarch64::{
    vceqq_u8, vcleq_u8, vdupq_n_u8, vget_lane_u64, vld1q_u8, vorrq_u8, vreinterpret_u64_u8,
    vreinterpretq_u16_u8, vshrn_n_u16,
};
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::{
    _mm_cmpeq_epi8, _mm_loadu_si128, _mm_min_epu8, _mm_movemask_epi8, _mm_or_si128, _mm_set1_epi8,
};
use std::{borrow::Cow, sync::OnceLock};

use serde_json::{Number, Value};

use crate::{cmp, types::JsonType, Array, Json, LazyInstance, Node, NodeIdentity, Object};

// `serde_json` rejects a document nested this deep.
const RECURSION_LIMIT: usize = 128;

// Past this many digits an integer may overflow `f64`.
const SAFE_NUMBER_LENGTH: usize = 300;

// Up to this many keys, comparing every pair beats sorting fingerprints.
const PAIRWISE_KEYS: usize = 8;

// Token kinds for integers read while checking, with the magnitude in `aux`, and for keys a later
// occurrence of the same key replaces. No JSON value starts with any of these bytes.
const INTEGER: u8 = b'i';
const NEGATIVE_INTEGER: u8 = b'j';
const SHADOWED_KEY: u8 = b's';

// High bit of `Token::aux`: a string holding an escape.
const FLAG: usize = 1 << (usize::BITS - 1);

/// One key or value. Strings keep their closing quote in `aux`, numbers and literals their end.
/// A container takes two tokens: its opening bracket with the index past its last descendant,
/// then its closing bracket with its member count.
#[derive(Clone, Copy)]
struct Token {
    at: usize,
    aux: usize,
    // The value's first byte, which tells its type.
    kind: u8,
    // For the first of repeated keys: the index of the value its last occurrence holds.
    target: usize,
}

pub struct JsonText;

/// Backs the nodes [`Json::with_string_node`] makes, reused across calls.
#[derive(Default)]
pub struct NameBuffer {
    text: String,
    tokens: Vec<Token>,
}

/// A checked JSON document.
pub struct JsonTextDocument<'a> {
    text: &'a str,
    tokens: Vec<Token>,
}

/// One value inside a [`JsonTextDocument`].
#[derive(Clone, Copy)]
pub struct JsonTextNode<'a> {
    document: &'a JsonTextDocument<'a>,
    index: usize,
}

impl JsonText {
    /// Check `bytes` as `serde_json::from_slice` would.
    ///
    /// # Errors
    ///
    /// The error `serde_json` reports when `bytes` is not one JSON document.
    pub fn parse(bytes: &[u8]) -> Result<JsonTextDocument<'_>, serde_json::Error> {
        std::str::from_utf8(bytes)
            .ok()
            .and_then(|text| {
                let mut tokens = Vec::with_capacity(bytes.len() / 4 + 2);
                check(text, &mut tokens).map(|()| JsonTextDocument { text, tokens })
            })
            .ok_or_else(|| {
                serde_json::from_slice::<Value>(bytes)
                    .err()
                    .unwrap_or_else(|| {
                        serde_json::Error::io(std::io::Error::other(
                            "the JSON text check rejected a document serde_json accepts",
                        ))
                    })
            })
    }
}

impl JsonTextDocument<'_> {
    #[must_use]
    pub fn root(&self) -> JsonTextNode<'_> {
        JsonTextNode {
            document: self,
            index: 0,
        }
    }
}

#[inline]
fn skip_whitespace(bytes: &[u8], mut at: usize) -> usize {
    while matches!(bytes.get(at), Some(b' ' | b'\n' | b'\t' | b'\r')) {
        at += 1;
    }
    at
}

/// A container being checked.
struct Open {
    index: usize,
    object: bool,
    len: usize,
    escaped_keys: bool,
}

fn check(text: &str, tokens: &mut Vec<Token>) -> Option<()> {
    let bytes = text.as_bytes();
    let mut stack: Vec<Open> = Vec::with_capacity(16);
    let mut keys: Vec<usize> = Vec::with_capacity(64);
    let mut fingerprints: Vec<u64> = Vec::with_capacity(64);
    let mut at = skip_whitespace(bytes, 0);
    'value: loop {
        match *bytes.get(at)? {
            open @ (b'{' | b'[') => {
                if stack.len() + 1 >= RECURSION_LIMIT {
                    return None;
                }
                let object = open == b'{';
                stack.push(Open {
                    index: tokens.len(),
                    object,
                    len: 0,
                    escaped_keys: false,
                });
                tokens.push(Token {
                    at,
                    aux: 0,
                    kind: open,
                    target: 0,
                });
                tokens.push(Token {
                    at: 0,
                    aux: 0,
                    kind: open,
                    target: 0,
                });
                at = skip_whitespace(bytes, at + 1);
                let close = if object { b'}' } else { b']' };
                if bytes.get(at) == Some(&close) {
                    let open = stack.pop().expect("just pushed");
                    close_container(text, tokens, &mut keys, &mut fingerprints, &open, at);
                    at += 1;
                } else {
                    if object {
                        let top = stack.last_mut().expect("just pushed");
                        at = check_key(bytes, at, tokens, &mut top.escaped_keys)?;
                    }
                    continue 'value;
                }
            }
            b'"' => {
                let (close, escaped) = check_string(bytes, at)?;
                tokens.push(Token {
                    at,
                    aux: close | if escaped { FLAG } else { 0 },
                    kind: b'"',
                    target: 0,
                });
                at = close + 1;
            }
            b't' => at = check_literal(bytes, at, b"true", tokens)?,
            b'f' => at = check_literal(bytes, at, b"false", tokens)?,
            b'n' => at = check_literal(bytes, at, b"null", tokens)?,
            b'-' | b'0'..=b'9' => {
                let (token, end) = check_number(bytes, at)?;
                tokens.push(token);
                at = end;
            }
            _ => return None,
        }
        // A value just ended: continue its container, or close it.
        loop {
            at = skip_whitespace(bytes, at);
            let Some(top) = stack.last_mut() else {
                return (at == bytes.len()).then_some(());
            };
            top.len += 1;
            match *bytes.get(at)? {
                b',' => {
                    at = skip_whitespace(bytes, at + 1);
                    if top.object {
                        at = check_key(bytes, at, tokens, &mut top.escaped_keys)?;
                    }
                    continue 'value;
                }
                b'}' if top.object => {}
                b']' if !top.object => {}
                _ => return None,
            }
            let open = stack.pop().expect("checked above");
            close_container(text, tokens, &mut keys, &mut fingerprints, &open, at);
            at += 1;
        }
    }
}

fn close_container(
    text: &str,
    tokens: &mut [Token],
    keys: &mut Vec<usize>,
    fingerprints: &mut Vec<u64>,
    open: &Open,
    close: usize,
) {
    let next = tokens.len();
    tokens[open.index].aux = next;
    let len = if open.object && open.len > 1 {
        keys.clear();
        let mut index = open.index + 2;
        while index < next {
            keys.push(index);
            index = skip(tokens, index + 1);
        }
        mark_repeated_keys(text, tokens, keys, open.escaped_keys, fingerprints)
    } else {
        open.len
    };
    tokens[open.index + 1] = Token {
        at: close,
        aux: len,
        kind: tokens[open.index].kind,
        target: 0,
    };
}

/// The index past the value at `index`.
fn skip(tokens: &[Token], index: usize) -> usize {
    if matches!(tokens[index].kind, b'{' | b'[') {
        tokens[index].aux
    } else {
        index + 1
    }
}

/// Equal keys share one; different keys rarely do. A key is its bytes in `start..end`.
fn fingerprint(bytes: &[u8], start: usize, end: usize) -> u64 {
    let word = |at: usize| {
        if let Some(chunk) = bytes[at..].first_chunk::<8>() {
            u64::from_le_bytes(*chunk)
        } else {
            let mut chunk = [0_u8; 8];
            chunk[..bytes.len() - at].copy_from_slice(&bytes[at..]);
            u64::from_le_bytes(chunk)
        }
    };
    let len = end - start;
    let head = if len >= 8 {
        word(start)
    } else {
        word(start) & ((1_u64 << (8 * len)) - 1)
    };
    let tail = if len > 8 { word(end - 8) } else { 0 };
    (head ^ tail.rotate_left(29) ^ len as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// Marks every later occurrence of a repeated key and points its first occurrence at the last
/// value, as `json.loads` keeps the first position and the last value. Returns the distinct count.
fn mark_repeated_keys(
    text: &str,
    tokens: &mut [Token],
    keys: &[usize],
    escaped: bool,
    fingerprints: &mut Vec<u64>,
) -> usize {
    let bytes = text.as_bytes();
    if !escaped {
        fingerprints.clear();
        fingerprints.extend(keys.iter().map(|key| {
            let token = tokens[*key];
            fingerprint(bytes, token.at + 1, token.aux)
        }));
        if keys.len() <= PAIRWISE_KEYS {
            let raw = |tokens: &[Token], key: usize| {
                let token = tokens[key];
                &bytes[token.at + 1..token.aux]
            };
            let mut shadowed = 0;
            for first in 0..keys.len() {
                if tokens[keys[first]].kind == SHADOWED_KEY {
                    continue;
                }
                let mut last = None;
                for later in first + 1..keys.len() {
                    if fingerprints[first] == fingerprints[later]
                        && raw(tokens, keys[first]) == raw(tokens, keys[later])
                    {
                        tokens[keys[later]].kind = SHADOWED_KEY;
                        shadowed += 1;
                        last = Some(later);
                    }
                }
                if let Some(last) = last {
                    point_at(tokens, keys[first], keys[last]);
                }
            }
            return keys.len() - shadowed;
        }
        fingerprints.sort_unstable();
        if fingerprints.windows(2).all(|pair| pair[0] != pair[1]) {
            return keys.len();
        }
    }
    let mut first_of: ahash::AHashMap<Cow<'_, str>, usize> =
        ahash::AHashMap::with_capacity(keys.len());
    let mut shadowed = 0;
    for &key in keys {
        match first_of.entry(string_value(text, tokens[key])) {
            std::collections::hash_map::Entry::Occupied(first) => {
                tokens[key].kind = SHADOWED_KEY;
                shadowed += 1;
                point_at(tokens, *first.get(), key);
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(key);
            }
        }
    }
    keys.len() - shadowed
}

fn point_at(tokens: &mut [Token], first: usize, later: usize) {
    tokens[first].target = later + 1;
}

#[inline]
fn check_key(
    bytes: &[u8],
    at: usize,
    tokens: &mut Vec<Token>,
    escaped_keys: &mut bool,
) -> Option<usize> {
    if bytes.get(at) != Some(&b'"') {
        return None;
    }
    let (close, escaped) = check_string(bytes, at)?;
    *escaped_keys |= escaped;
    tokens.push(Token {
        at,
        aux: close | if escaped { FLAG } else { 0 },
        kind: b'"',
        target: 0,
    });
    let colon = skip_whitespace(bytes, close + 1);
    if bytes.get(colon) != Some(&b':') {
        return None;
    }
    Some(skip_whitespace(bytes, colon + 1))
}

const ONES: u64 = 0x0101_0101_0101_0101;
const HIGH: u64 = 0x8080_8080_8080_8080;

/// Bytes of `word` that are `"`, `\` or a control character have their high bit set; the lowest
/// set bit is exact, those above it may be false positives.
fn special_bytes(word: u64) -> u64 {
    let quote = word ^ (ONES * u64::from(b'"'));
    let backslash = word ^ (ONES * u64::from(b'\\'));
    ((quote.wrapping_sub(ONES) & !quote)
        | (backslash.wrapping_sub(ONES) & !backslash)
        | (word.wrapping_sub(ONES * 0x20) & !word))
        & HIGH
}

/// The offset of the closing quote, and whether the string holds an escape.
#[inline]
fn check_string(bytes: &[u8], open: usize) -> Option<(usize, bool)> {
    let mut at = open + 1;
    let mut escaped = false;
    loop {
        at = skip_plain_bytes(bytes, at);
        match *bytes.get(at)? {
            b'"' => return Some((at, escaped)),
            b'\\' => {
                escaped = true;
                at = match *bytes.get(at + 1)? {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => at + 2,
                    b'u' => check_unicode_escape(bytes, at)?,
                    _ => return None,
                };
            }
            0x00..=0x1F => return None,
            _ => at += 1,
        }
    }
}

/// The offset of the first `"`, `\\` or control character from `at`, or of the last few bytes,
/// too few to read as a word.
#[inline]
fn skip_plain_bytes(bytes: &[u8], mut at: usize) -> usize {
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    while let Some(chunk) = bytes[at..].first_chunk::<16>() {
        if let Some(offset) = first_special_lane(chunk) {
            return at + offset;
        }
        at += 16;
    }
    while let Some(chunk) = bytes[at..].first_chunk::<8>() {
        let special = special_bytes(u64::from_le_bytes(*chunk));
        if special != 0 {
            return at + (special.trailing_zeros() / 8) as usize;
        }
        at += 8;
    }
    at
}

/// The offset of the first byte of `chunk` that is `"`, `\\` or a control character.
#[cfg(target_arch = "x86_64")]
#[inline]
fn first_special_lane(chunk: &[u8; 16]) -> Option<usize> {
    // SAFETY: SSE2 is part of the x86_64 baseline, and the load reads the sixteen bytes of `chunk`.
    #[allow(unsafe_code)]
    let mask = unsafe {
        let lanes = _mm_loadu_si128(chunk.as_ptr().cast());
        let quote = _mm_cmpeq_epi8(lanes, _mm_set1_epi8(i8::from_ne_bytes(*b"\"")));
        let backslash = _mm_cmpeq_epi8(lanes, _mm_set1_epi8(i8::from_ne_bytes(*b"\\")));
        // A byte at most 0x1F equals its minimum with 0x1F.
        let control = _mm_cmpeq_epi8(_mm_min_epu8(lanes, _mm_set1_epi8(0x1F)), lanes);
        _mm_movemask_epi8(_mm_or_si128(_mm_or_si128(quote, backslash), control))
    };
    // The mask fills only the low sixteen bits, so it is never negative.
    (mask != 0).then(|| mask.unsigned_abs().trailing_zeros() as usize)
}

/// The offset of the first byte of `chunk` that is `"`, `\\` or a control character.
#[cfg(target_arch = "aarch64")]
#[inline]
fn first_special_lane(chunk: &[u8; 16]) -> Option<usize> {
    // SAFETY: NEON is part of the aarch64 baseline, and the load reads the sixteen bytes of `chunk`.
    #[allow(unsafe_code)]
    let mask = unsafe {
        let lanes = vld1q_u8(chunk.as_ptr());
        let quote = vceqq_u8(lanes, vdupq_n_u8(b'"'));
        let backslash = vceqq_u8(lanes, vdupq_n_u8(b'\\'));
        let control = vcleq_u8(lanes, vdupq_n_u8(0x1F));
        let special = vorrq_u8(vorrq_u8(quote, backslash), control);
        // Narrowing each byte pair by four bits keeps four bits per byte in one word.
        let nibbles = vshrn_n_u16::<4>(vreinterpretq_u16_u8(special));
        vget_lane_u64::<0>(vreinterpret_u64_u8(nibbles))
    };
    (mask != 0).then(|| (mask.trailing_zeros() / 4) as usize)
}

/// `\uXXXX` at `at`; surrogates must pair, as `serde_json` requires for strings.
fn check_unicode_escape(bytes: &[u8], at: usize) -> Option<usize> {
    let unit = hex4(bytes, at + 2)?;
    if (0xDC00..=0xDFFF).contains(&unit) {
        return None;
    }
    if !(0xD800..=0xDBFF).contains(&unit) {
        return Some(at + 6);
    }
    if bytes.get(at + 6) != Some(&b'\\') || bytes.get(at + 7) != Some(&b'u') {
        return None;
    }
    (0xDC00..=0xDFFF)
        .contains(&hex4(bytes, at + 8)?)
        .then_some(at + 12)
}

fn hex4(bytes: &[u8], at: usize) -> Option<u32> {
    let digits = bytes.get(at..at + 4)?;
    digits.iter().try_fold(0, |value, digit| {
        Some(value * 16 + char::from(*digit).to_digit(16)?)
    })
}

fn check_literal(
    bytes: &[u8],
    at: usize,
    literal: &[u8],
    tokens: &mut Vec<Token>,
) -> Option<usize> {
    let end = at + literal.len();
    if bytes.get(at..end)? != literal {
        return None;
    }
    tokens.push(Token {
        at,
        aux: end,
        kind: literal[0],
        target: 0,
    });
    Some(end)
}

/// Bytes of `word` that are not ASCII digits have their high bit set; the lowest set bit is exact,
/// those above it may be false positives.
fn non_digit_bytes(word: u64) -> u64 {
    (word.wrapping_add(ONES * 0x46) | word.wrapping_sub(ONES * 0x30)) & HIGH
}

/// The offset past the ASCII digits starting at `at`.
fn digit_run_end(bytes: &[u8], mut at: usize) -> usize {
    while let Some(chunk) = bytes[at..].first_chunk::<8>() {
        let non_digit = non_digit_bytes(u64::from_le_bytes(*chunk));
        if non_digit != 0 {
            return at + (non_digit.trailing_zeros() / 8) as usize;
        }
        at += 8;
    }
    while matches!(bytes.get(at), Some(b'0'..=b'9')) {
        at += 1;
    }
    at
}

/// The number's token, holding its value when it is an integer that fits, and its end.
fn check_number(bytes: &[u8], start: usize) -> Option<(Token, usize)> {
    let negative = bytes.get(start) == Some(&b'-');
    let mut at = start + usize::from(negative);
    let integer_start = at;
    let mut magnitude = 0_u64;
    match bytes.get(at)? {
        b'0' => at += 1,
        b'1'..=b'9' => {
            while let Some(digit @ b'0'..=b'9') = bytes.get(at) {
                magnitude = magnitude
                    .wrapping_mul(10)
                    .wrapping_add(u64::from(digit - b'0'));
                at += 1;
            }
        }
        _ => return None,
    }
    // Nineteen digits always fit a `u64`.
    let mut integer = at - integer_start <= 19;
    if bytes.get(at) == Some(&b'.') {
        if !matches!(bytes.get(at + 1), Some(b'0'..=b'9')) {
            return None;
        }
        at = digit_run_end(bytes, at + 1);
        integer = false;
    }
    let mut exponent = false;
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        exponent = true;
        integer = false;
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        if !matches!(bytes.get(at), Some(b'0'..=b'9')) {
            return None;
        }
        while matches!(bytes.get(at), Some(b'0'..=b'9')) {
            at += 1;
        }
    }
    // `serde_json` rejects a number past `f64` unless its `arbitrary_precision` keeps the digits,
    // which any crate in the build can turn on, so it decides.
    if exponent || at - start > SAFE_NUMBER_LENGTH {
        let literal = std::str::from_utf8(&bytes[start..at]).ok()?;
        literal.parse::<Number>().ok()?;
    }
    if integer {
        // `serde_json` reads `-0` as a float.
        let kind = match (negative, magnitude) {
            (false, _) => Some(INTEGER),
            (true, 1..=0x8000_0000_0000_0000) => Some(NEGATIVE_INTEGER),
            (true, _) => None,
        };
        if let (Some(kind), Ok(aux)) = (kind, usize::try_from(magnitude)) {
            return Some((
                Token {
                    at: start,
                    aux,
                    kind,
                    target: 0,
                },
                at,
            ));
        }
    }
    Some((
        Token {
            at: start,
            aux: at,
            kind: bytes[start],
            target: 0,
        },
        at,
    ))
}

/// A checked string's value.
fn string_value(text: &str, token: Token) -> Cow<'_, str> {
    let content = &text[token.at + 1..token.aux & !FLAG];
    if token.aux & FLAG == 0 {
        Cow::Borrowed(content)
    } else {
        Cow::Owned(Unescaped(content.chars()).collect())
    }
}

/// The characters a checked string's content stands for.
#[derive(Clone)]
struct Unescaped<'a>(std::str::Chars<'a>);

impl Unescaped<'_> {
    fn hex4(&mut self) -> u32 {
        (0..4).fold(0, |value, _| {
            value * 16
                + self
                    .0
                    .next()
                    .and_then(|digit| digit.to_digit(16))
                    .expect("a checked escape")
        })
    }
}

impl Iterator for Unescaped<'_> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        let character = self.0.next()?;
        if character != '\\' {
            return Some(character);
        }
        Some(match self.0.next()? {
            'b' => '\u{8}',
            'f' => '\u{c}',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'u' => {
                let unit = self.hex4();
                let code = if (0xD800..0xDC00).contains(&unit) {
                    // The low half follows as another `\uXXXX`.
                    self.0.nth(1);
                    0x1_0000 + ((unit - 0xD800) << 10) + (self.hex4() - 0xDC00)
                } else {
                    unit
                };
                char::from_u32(code).expect("a checked escape")
            }
            other => other,
        })
    }
}

/// The checked string's content with escapes still written out.
fn raw_content(text: &str, token: Token) -> &str {
    &text[token.at + 1..token.aux & !FLAG]
}

/// A checked number, read the way `serde_json` reads it.
fn number_value(literal: &str) -> Number {
    literal.parse().expect("a checked number")
}

impl Json for JsonText {
    type Node<'a> = JsonTextNode<'a>;
    type PreparedKey = String;
    type StringBuffer = NameBuffer;

    // A lookup walks the object, so one members pass always beats several lookups.
    const KEYS_PER_LOOKUP: usize = 1 << 16;
    const MEMBERS_PER_PROPERTY: usize = 1 << 16;
    const ACYCLIC: bool = true;

    fn prepare_key(key: &str) -> String {
        key.to_owned()
    }

    fn with_string_node<T>(
        NameBuffer { text, tokens }: &mut NameBuffer,
        string: &str,
        f: impl FnOnce(JsonTextNode<'_>) -> T,
    ) -> T {
        text.clear();
        text.push('"');
        let mut escaped = false;
        for character in string.chars() {
            match character {
                '"' => text.push_str("\\\""),
                '\\' => text.push_str("\\\\"),
                '\u{0}'..='\u{1f}' => {
                    use std::fmt::Write as _;
                    write!(text, "\\u{:04x}", u32::from(character))
                        .expect("write to String never fails");
                }
                _ => {
                    text.push(character);
                    continue;
                }
            }
            escaped = true;
        }
        text.push('"');
        let mut owned = std::mem::take(tokens);
        owned.clear();
        owned.push(Token {
            at: 0,
            aux: (text.len() - 1) | if escaped { FLAG } else { 0 },
            kind: b'"',
            target: 0,
        });
        let document = JsonTextDocument {
            text,
            tokens: owned,
        };
        let result = f(document.root());
        *tokens = document.tokens;
        result
    }
}

impl<'a> JsonTextNode<'a> {
    fn integer_end(bytes: &[u8], at: usize) -> usize {
        let mut end = at + 1;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        end
    }

    fn token(&self) -> Token {
        self.document.tokens[self.index]
    }

    fn first(&self) -> u8 {
        self.token().kind
    }

    /// One past the value's last byte.
    fn end(&self) -> usize {
        let token = self.token();
        match token.kind {
            b'{' | b'[' => self.document.tokens[self.index + 1].at + 1,
            b'"' => (token.aux & !FLAG) + 1,
            INTEGER | NEGATIVE_INTEGER => {
                Self::integer_end(self.document.text.as_bytes(), token.at)
            }
            _ => token.aux,
        }
    }

    /// The member or element count; a repeated key counts once.
    fn count(&self) -> usize {
        self.document.tokens[self.index + 1].aux
    }

    fn walker(&self) -> Walker<'a> {
        Walker {
            document: self.document,
            index: self.index + 2,
            end: self.token().aux,
        }
    }

    fn number(&self) -> Option<Number> {
        let token = self.token();
        match token.kind {
            INTEGER => Some(Number::from(token.aux as u64)),
            NEGATIVE_INTEGER => Some(Number::from(0_i64.wrapping_sub_unsigned(token.aux as u64))),
            b'-' | b'0'..=b'9' => Some(number_value(&self.document.text[token.at..token.aux])),
            _ => None,
        }
    }
}

impl<'a> Node<'a, JsonText> for JsonTextNode<'a> {
    type Object = JsonTextObject<'a>;
    type Array = JsonTextArray<'a>;
    type Number = Number;

    fn as_object(&self) -> Option<JsonTextObject<'a>> {
        (self.first() == b'{').then_some(JsonTextObject { node: *self })
    }

    fn as_array(&self) -> Option<JsonTextArray<'a>> {
        (self.first() == b'[').then_some(JsonTextArray { node: *self })
    }

    fn as_string(&self) -> Option<Cow<'a, str>> {
        (self.first() == b'"').then(|| string_value(self.document.text, self.token()))
    }

    fn as_number(&self) -> Option<Number> {
        self.number()
    }

    fn as_boolean(&self) -> Option<bool> {
        match self.first() {
            b't' => Some(true),
            b'f' => Some(false),
            _ => None,
        }
    }

    fn is_null(&self) -> bool {
        self.first() == b'n'
    }

    fn is_number(&self) -> bool {
        matches!(
            self.first(),
            b'-' | b'0'..=b'9' | INTEGER | NEGATIVE_INTEGER
        )
    }

    fn is_string(&self) -> bool {
        self.first() == b'"'
    }

    fn json_type(&self) -> JsonType {
        match self.first() {
            b'{' => JsonType::Object,
            b'[' => JsonType::Array,
            b'"' => JsonType::String,
            b't' | b'f' => JsonType::Boolean,
            b'n' => JsonType::Null,
            _ => JsonType::Number,
        }
    }

    fn string_length(&self) -> Option<u64> {
        if self.first() != b'"' {
            return None;
        }
        let token = self.token();
        Some(if token.aux & FLAG == 0 {
            bytecount::num_chars(&self.document.text.as_bytes()[token.at + 1..token.aux]) as u64
        } else {
            Unescaped(raw_content(self.document.text, token).chars()).count() as u64
        })
    }

    fn equals_value(&self, expected: &Value) -> bool {
        match expected {
            Value::Null => self.is_null(),
            Value::Bool(boolean) => self.as_boolean() == Some(*boolean),
            Value::Number(number) => self
                .number()
                .is_some_and(|got| cmp::equal_numbers(&got, number)),
            Value::String(string) => self
                .as_string()
                .is_some_and(|got| got.as_ref() == string.as_str()),
            Value::Array(items) => self.as_array().is_some_and(|array| {
                array.len() == items.len()
                    && array
                        .elements()
                        .zip(items)
                        .all(|(element, item)| element.equals_value(item))
            }),
            Value::Object(map) => self.as_object().is_some_and(|object| {
                object.len() == map.len()
                    && object.members().all(|(name, value)| {
                        map.get(name.as_ref())
                            .is_some_and(|expected| value.equals_value(expected))
                    })
            }),
        }
    }

    // Most failures never read the instance back, so this keeps the text, not a built value.
    fn lazy_value(&self) -> LazyInstance<'a> {
        LazyInstance::Deferred {
            bytes: &self.document.text.as_bytes()[self.token().at..self.end()],
            tag: 0,
            make: parse_checked,
            cell: OnceLock::new(),
        }
    }

    fn to_value(&self) -> Cow<'a, Value> {
        Cow::Owned(
            serde_json::from_str(&self.document.text[self.token().at..self.end()])
                .expect("a checked value"),
        )
    }

    fn identity(&self) -> Option<NodeIdentity> {
        Some(NodeIdentity::new(
            self.document.text.as_ptr() as usize + self.token().at,
        ))
    }
}

/// JSON equality without building either value; numbers compare by value.
fn equal(left: JsonTextNode<'_>, right: JsonTextNode<'_>) -> bool {
    let (left_kind, right_kind) = (left.first(), right.first());
    match (left_kind, right_kind) {
        (b'"', b'"') => {
            let (left_token, right_token) = (left.token(), right.token());
            if (left_token.aux | right_token.aux) & FLAG == 0 {
                left.document.text.as_bytes()[left_token.at..left_token.aux]
                    == right.document.text.as_bytes()[right_token.at..right_token.aux]
            } else {
                Unescaped(raw_content(left.document.text, left_token).chars()).eq(Unescaped(
                    raw_content(right.document.text, right_token).chars(),
                ))
            }
        }
        (b'{', b'{') => {
            let (left, right) = (
                JsonTextObject { node: left },
                JsonTextObject { node: right },
            );
            if left.len() != right.len() {
                return false;
            }
            // Equal objects usually list their keys in one order: walk both in step, and index the
            // rest of `right` only once the orders diverge. Keys are distinct, so the counts match.
            let (mut left, mut right) = (left.members(), right.members());
            while let (Some((name, value)), Some((other_name, other))) = (left.next(), right.next())
            {
                if name != other_name {
                    let mut rest: ahash::AHashMap<Cow<'_, str>, JsonTextNode<'_>> =
                        std::iter::once((other_name, other)).chain(right).collect();
                    return std::iter::once((name, value))
                        .chain(left)
                        .all(|(name, value)| {
                            rest.remove(&name).is_some_and(|other| equal(value, other))
                        });
                }
                if !equal(value, other) {
                    return false;
                }
            }
            true
        }
        (b'[', b'[') => {
            left.count() == right.count()
                && left
                    .walker()
                    .zip_elements(right.walker())
                    .all(|(left, right)| equal(left, right))
        }
        (b't' | b'f' | b'n', _) => left_kind == right_kind,
        _ => match (left.number(), right.number()) {
            (Some(left), Some(right)) => cmp::equal_numbers(&left, &right),
            _ => false,
        },
    }
}

fn parse_checked(bytes: &[u8], _: u32) -> Value {
    serde_json::from_slice(bytes).expect("a checked value")
}

/// Steps through an object's members or an array's elements.
#[derive(Clone)]
struct Walker<'a> {
    document: &'a JsonTextDocument<'a>,
    index: usize,
    end: usize,
}

impl<'a> Walker<'a> {
    fn value(&mut self, index: usize) -> JsonTextNode<'a> {
        self.index = skip(&self.document.tokens, index);
        JsonTextNode {
            document: self.document,
            index,
        }
    }

    fn element(&mut self) -> Option<JsonTextNode<'a>> {
        (self.index < self.end).then(|| self.value(self.index))
    }

    fn zip_elements<'b>(
        mut self,
        mut other: Walker<'b>,
    ) -> impl Iterator<Item = (JsonTextNode<'a>, JsonTextNode<'b>)> {
        std::iter::from_fn(move || Some((self.element()?, other.element()?)))
    }

    /// The next member's key token and its value; a repeated key appears once, with its last value.
    fn member(&mut self) -> Option<(Token, JsonTextNode<'a>)> {
        while self.index < self.end {
            let key = self.document.tokens[self.index];
            let value = self.value(self.index + 1);
            if key.kind == SHADOWED_KEY {
                continue;
            }
            if key.target == 0 {
                return Some((key, value));
            }
            return Some((
                key,
                JsonTextNode {
                    document: self.document,
                    index: key.target,
                },
            ));
        }
        None
    }
}

pub struct JsonTextObject<'a> {
    node: JsonTextNode<'a>,
}

pub struct JsonTextArray<'a> {
    node: JsonTextNode<'a>,
}

fn name_is(text: &str, token: Token, key: &str) -> bool {
    if token.aux & FLAG == 0 {
        &text.as_bytes()[token.at + 1..token.aux] == key.as_bytes()
    } else {
        Unescaped(raw_content(text, token).chars()).eq(key.chars())
    }
}

/// Members in document order; a repeated key appears once, where it first occurs.
pub struct JsonTextMembers<'a> {
    walker: Walker<'a>,
}

impl<'a> Iterator for JsonTextMembers<'a> {
    type Item = (Cow<'a, str>, JsonTextNode<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        let (key, value) = self.walker.member()?;
        Some((string_value(self.walker.document.text, key), value))
    }
}

impl<'a> Object<'a, JsonText> for JsonTextObject<'a> {
    type Node = JsonTextNode<'a>;
    type MemberName = Cow<'a, str>;
    type MembersIter = JsonTextMembers<'a>;

    fn len(&self) -> usize {
        self.node.count()
    }

    fn get(&self, key: &String) -> Option<JsonTextNode<'a>> {
        self.find(key)
    }

    fn members(&self) -> JsonTextMembers<'a> {
        JsonTextMembers {
            walker: self.node.walker(),
        }
    }
}

impl<'a> JsonTextObject<'a> {
    fn find(&self, key: &str) -> Option<JsonTextNode<'a>> {
        let text = self.node.document.text;
        let mut walker = self.node.walker();
        while let Some((name, value)) = walker.member() {
            if name_is(text, name, key) {
                return Some(value);
            }
        }
        None
    }
}

pub struct JsonTextElements<'a> {
    walker: Walker<'a>,
}

impl<'a> Iterator for JsonTextElements<'a> {
    type Item = JsonTextNode<'a>;

    fn next(&mut self) -> Option<JsonTextNode<'a>> {
        self.walker.element()
    }
}

impl<'a> Array<'a, JsonText> for JsonTextArray<'a> {
    type Node = JsonTextNode<'a>;
    type ElementsIter = JsonTextElements<'a>;

    fn len(&self) -> usize {
        self.node.count()
    }

    fn elements(&self) -> JsonTextElements<'a> {
        JsonTextElements {
            walker: self.node.walker(),
        }
    }

    fn is_unique(&self) -> bool {
        let len = self.node.count();
        if len <= 1 {
            return true;
        }
        if len <= crate::unique::ITEMS_SIZE_THRESHOLD {
            // Pairwise over a stack copy: most arrays are short.
            let mut items = [self.node; crate::unique::ITEMS_SIZE_THRESHOLD];
            for (slot, element) in items.iter_mut().zip(self.elements()) {
                *slot = element;
            }
            let items = &items[..len];
            return items
                .iter()
                .enumerate()
                .all(|(index, left)| items[index + 1..].iter().all(|right| !equal(*left, *right)));
        }
        // Strings and literals hash by their values; anything else is built first.
        let text = self.node.document.text;
        let mut seen = ahash::AHashSet::with_capacity(len);
        for element in self.elements() {
            let token = element.token();
            let key = match token.kind {
                b'"' => string_value(text, token),
                b't' | b'f' | b'n' => Cow::Borrowed(""),
                _ => {
                    let values: Vec<Cow<'a, Value>> =
                        self.elements().map(|element| element.to_value()).collect();
                    return crate::unique::is_unique(&values);
                }
            };
            if !seen.insert((token.kind, key)) {
                return false;
            }
        }
        true
    }
}

//! Postgres `jsonb` representation: reads stored bytes without materializing a document.
// Numeric digits are bounded base-10000 groups and the varlena layout is fixed-width; the casts
// here stay within those bounds.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap
)]

use std::{
    borrow::Cow,
    fmt::Write as _,
    hash::{Hash, Hasher},
    sync::OnceLock,
};

use ahash::AHasher;
use serde_json::{Map, Value};

use crate::{
    cmp, types::JsonType, Array, Json, JsonNumber, LazyInstance, Node, NodeIdentity, Object,
};

const JB_CMASK: u32 = 0x0FFF_FFFF;
const JB_FSCALAR: u32 = 0x1000_0000;
const JB_FOBJECT: u32 = 0x2000_0000;

const JENTRY_OFFLENMASK: u32 = 0x0FFF_FFFF;
const JENTRY_TYPEMASK: u32 = 0x7000_0000;
const JENTRY_HAS_OFF: u32 = 0x8000_0000;

const JENTRY_ISSTRING: u32 = 0x0000_0000;
const JENTRY_ISNUMERIC: u32 = 0x1000_0000;
const JENTRY_ISBOOL_FALSE: u32 = 0x2000_0000;
const JENTRY_ISBOOL_TRUE: u32 = 0x3000_0000;
const JENTRY_ISNULL: u32 = 0x4000_0000;
const JENTRY_ISCONTAINER: u32 = 0x5000_0000;

/// How deep a document may nest before it cannot be reported back as a `serde_json::Value`.
///
/// Reading a `jsonb` datum never recurses, so navigation and `is_valid` are bounded only by what
/// Postgres itself accepts. A `Value` is a recursive type whose `Drop` and `Serialize` do recurse,
/// so building one past this depth would overflow the stack while it is dropped rather than here.
const MATERIALIZATION_NESTING_LIMIT: usize = 255;

const NUMERIC_SIGN_MASK: u16 = 0xC000;
const NUMERIC_NEG: u16 = 0x4000;
const NUMERIC_SHORT: u16 = 0x8000;
const NUMERIC_DSCALE_MASK: u16 = 0x3FFF;
const NUMERIC_SHORT_SIGN_MASK: u16 = 0x2000;
const NUMERIC_SHORT_DSCALE_MASK: u16 = 0x1F80;
const NUMERIC_SHORT_DSCALE_SHIFT: u16 = 7;
const NUMERIC_SHORT_WEIGHT_SIGN_MASK: u16 = 0x0040;
const NUMERIC_SHORT_WEIGHT_MASK: u16 = 0x003F;

pub struct Jsonb;

/// One `jsonb` value: its own bytes, plus the `JEntry` type bits naming what they hold.
#[derive(Clone, Copy)]
pub struct JsonbNode<'a> {
    data: &'a [u8],
    kind: u32,
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    let mut buffer = [0_u8; 4];
    buffer.copy_from_slice(&bytes[at..at + 4]);
    u32::from_ne_bytes(buffer)
}

fn read_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_ne_bytes([bytes[at], bytes[at + 1]])
}

fn align_up(offset: u32) -> u32 {
    offset.next_multiple_of(4)
}

/// The `JsonbContainer`-relative bytes of a `numeric` varlena, with its own header stripped.
fn varlena_body(bytes: &[u8]) -> &[u8] {
    let short = if cfg!(target_endian = "big") {
        bytes[0] & 0x80 == 0x80
    } else {
        bytes[0] & 0x01 == 0x01
    };
    if short {
        &bytes[1..]
    } else {
        &bytes[4..]
    }
}

/// The header word, and how many `JEntry`s follow it.
fn header(container: &[u8]) -> (u32, usize) {
    let header = read_u32(container, 0);
    let count = header & JB_CMASK;
    let entries = if header & JB_FOBJECT == 0 {
        count as usize
    } else {
        count as usize * 2
    };
    (header, entries)
}

fn data_start(entries: usize) -> usize {
    4 + 4 * entries
}

impl Jsonb {
    /// A node over a `JsonbContainer`, with the enclosing varlena header stripped.
    ///
    /// # Panics
    ///
    /// On anything but a container exactly as Postgres writes one.
    #[must_use]
    pub fn root(container: &[u8]) -> JsonbNode<'_> {
        let (flags, entries) = header(container);
        if flags & JB_FSCALAR == 0 {
            return JsonbNode {
                data: container,
                kind: JENTRY_ISCONTAINER,
            };
        }
        // A top-level scalar is a one-element pseudo-array.
        let entry = read_u32(container, 4);
        child(container, data_start(entries), 4, entry, 0)
    }
}

fn child(
    container: &[u8],
    data_start: usize,
    entry_at: usize,
    entry: u32,
    offset: u32,
) -> JsonbNode<'_> {
    let kind = entry & JENTRY_TYPEMASK;
    let end = data_start + (offset + (entry & JENTRY_OFFLENMASK)) as usize;
    let start = match kind {
        JENTRY_ISNUMERIC | JENTRY_ISCONTAINER => data_start + align_up(offset) as usize,
        _ => data_start + offset as usize,
    };
    JsonbNode {
        // A zero-length child borrows from its own JEntry so its address remains unique.
        data: if start == end {
            &container[entry_at..entry_at]
        } else {
            &container[start..end]
        },
        kind,
    }
}

// The offset of entry `index`: sum lengths backwards until an entry carrying its own end offset
// is consumed. `JB_OFFSET_STRIDE` bounds this at 32 steps.
fn entry_offset(container: &[u8], index: usize) -> u32 {
    entry_offset_after(container, index, 0, 0)
}

// `entry_offset` resumed from a known point: `checkpoint_offset` must equal
// `entry_offset(container, checkpoint)`.
fn entry_offset_after(
    container: &[u8],
    index: usize,
    checkpoint: usize,
    checkpoint_offset: u32,
) -> u32 {
    let mut offset = 0;
    for previous in (checkpoint..index).rev() {
        let entry = read_u32(container, 4 + 4 * previous);
        offset += entry & JENTRY_OFFLENMASK;
        if entry & JENTRY_HAS_OFF != 0 {
            return offset;
        }
    }
    offset + checkpoint_offset
}

fn entry_length(container: &[u8], index: usize, offset: u32) -> u32 {
    let entry = read_u32(container, 4 + 4 * index);
    let field = entry & JENTRY_OFFLENMASK;
    if index > 0 && entry & JENTRY_HAS_OFF != 0 {
        field - offset
    } else {
        field
    }
}

impl<'a> JsonbNode<'a> {
    fn container_flags(&self) -> u32 {
        read_u32(self.data, 0)
    }

    /// The node's own bytes at the node's lifetime, not the shorter one `&self` carries.
    fn bytes(self) -> &'a [u8] {
        self.data
    }
}

impl Json for Jsonb {
    type Node<'a> = JsonbNode<'a>;
    type PreparedKey = Box<[u8]>;
    type StringBuffer = Vec<u8>;

    fn prepare_key(key: &str) -> Box<[u8]> {
        key.as_bytes().into()
    }

    fn with_string_node<T>(
        buffer: &mut Vec<u8>,
        string: &str,
        f: impl FnOnce(JsonbNode<'_>) -> T,
    ) -> T {
        buffer.clear();
        buffer.reserve(string.len().max(1));
        buffer.extend_from_slice(string.as_bytes());
        f(JsonbNode {
            data: &*buffer,
            kind: JENTRY_ISSTRING,
        })
    }
}

impl<'a> Node<'a, Jsonb> for JsonbNode<'a> {
    type Object = JsonbObject<'a>;
    type Array = JsonbArray<'a>;
    type Number = JsonbNumber<'a>;

    fn as_object(&self) -> Option<JsonbObject<'a>> {
        if self.kind != JENTRY_ISCONTAINER {
            return None;
        }
        let flags = self.container_flags();
        if flags & JB_FOBJECT == 0 {
            return None;
        }
        Some(JsonbObject {
            container: (*self).bytes(),
            count: (flags & JB_CMASK) as usize,
        })
    }

    fn as_array(&self) -> Option<JsonbArray<'a>> {
        if self.kind != JENTRY_ISCONTAINER {
            return None;
        }
        let flags = self.container_flags();
        if flags & JB_FOBJECT != 0 {
            return None;
        }
        Some(JsonbArray {
            container: (*self).bytes(),
            count: (flags & JB_CMASK) as usize,
        })
    }

    fn as_string(&self) -> Option<Cow<'a, str>> {
        if self.kind == JENTRY_ISSTRING {
            // `from_utf8_lossy` walks `Utf8Chunks` to locate an invalid run; under a UTF-8 server
            // encoding there never is one, so validate first and keep it as the fallback.
            let bytes = (*self).bytes();
            Some(match std::str::from_utf8(bytes) {
                Ok(text) => Cow::Borrowed(text),
                Err(_) => String::from_utf8_lossy(bytes),
            })
        } else {
            None
        }
    }

    fn as_number(&self) -> Option<JsonbNumber<'a>> {
        if self.kind == JENTRY_ISNUMERIC {
            Some(JsonbNumber::parse((*self).bytes()))
        } else {
            None
        }
    }

    fn as_boolean(&self) -> Option<bool> {
        match self.kind {
            JENTRY_ISBOOL_TRUE => Some(true),
            JENTRY_ISBOOL_FALSE => Some(false),
            _ => None,
        }
    }

    fn is_null(&self) -> bool {
        self.kind == JENTRY_ISNULL
    }

    fn is_string(&self) -> bool {
        self.kind == JENTRY_ISSTRING
    }

    fn string_length(&self) -> Option<u64> {
        if self.kind != JENTRY_ISSTRING {
            return None;
        }
        // Counting non-continuation bytes is the character count only for valid UTF-8. An
        // invalid run is what `as_string` replaces, and one run can stand for several
        // replacement characters, so a lossy decode is what the length has to agree with.
        Some(match std::str::from_utf8(self.data) {
            Ok(_) => bytecount::num_chars(self.data) as u64,
            Err(_) => String::from_utf8_lossy(self.data).chars().count() as u64,
        })
    }

    fn is_number(&self) -> bool {
        self.kind == JENTRY_ISNUMERIC
    }

    fn json_type(&self) -> JsonType {
        match self.kind {
            JENTRY_ISSTRING => JsonType::String,
            JENTRY_ISNUMERIC => JsonType::Number,
            JENTRY_ISBOOL_FALSE | JENTRY_ISBOOL_TRUE => JsonType::Boolean,
            JENTRY_ISNULL => JsonType::Null,
            _ if self.container_flags() & JB_FOBJECT == 0 => JsonType::Array,
            _ => JsonType::Object,
        }
    }

    fn equals_value(&self, expected: &Value) -> bool {
        match expected {
            Value::Null => self.is_null(),
            Value::Bool(boolean) => self.as_boolean() == Some(*boolean),
            Value::Number(number) => self
                .as_number()
                .is_some_and(|got| cmp::equal_numbers(&got, number)),
            Value::String(string) => self
                .as_string()
                .is_some_and(|got| got.as_ref() == string.as_str()),
            Value::Array(items) => self.as_array().is_some_and(|array| {
                array.len() == items.len()
                    && array
                        .elements()
                        .zip(items)
                        .all(|(element, expected)| element.equals_value(expected))
            }),
            Value::Object(map) => self.as_object().is_some_and(|object| {
                let mut members = object.members();
                // A stored key that is not UTF-8 matches no key of a `serde_json` object, whose
                // keys are all `String`.
                object.len() == map.len()
                    && std::iter::from_fn(|| members.next_raw()).all(|(key, value)| {
                        std::str::from_utf8(key)
                            .ok()
                            .and_then(|key| map.get(key))
                            .is_some_and(|expected| value.equals_value(expected))
                    })
            }),
        }
    }

    /// # Panics
    ///
    /// If the value nests deeper than [`MATERIALIZATION_NESTING_LIMIT`]. Validating such a
    /// document is fine; only reporting one back as a `serde_json::Value` is not, so
    /// `is_valid` answers where `validate` and `iter_errors` panic.
    fn to_value(&self) -> Cow<'a, Value> {
        Cow::Owned(materialize(*self))
    }

    // Most failures never read the instance back, so copy two fields instead of the document.
    //
    // # Panics
    //
    // On resolution, under the same depth limit as `to_value`.
    fn lazy_value(&self) -> LazyInstance<'a> {
        LazyInstance::Deferred {
            bytes: self.data,
            tag: self.kind,
            make: rebuild_and_materialize,
            cell: OnceLock::new(),
        }
    }

    fn identity(&self) -> Option<NodeIdentity> {
        let tag = self.kind | (self.data.len() as u32 & JENTRY_OFFLENMASK);
        Some(NodeIdentity::tagged(self.data.as_ptr() as usize, tag))
    }
}

/// One container being rebuilt, holding what is left to visit and what has been built so far.
enum Frame<'a> {
    Array(Cursor<'a>, Vec<Value>),
    Object(JsonbMembers<'a>, Map<String, Value>, Option<String>),
}

/// Postgres stores nesting far deeper than a recursive walk survives on a backend's stack, so
/// the containers are rebuilt against an explicit stack.
fn materialize(root: JsonbNode<'_>) -> Value {
    fn scalar(node: &JsonbNode<'_>) -> Option<Value> {
        match node.kind {
            JENTRY_ISNULL => Some(Value::Null),
            JENTRY_ISBOOL_TRUE => Some(Value::Bool(true)),
            JENTRY_ISBOOL_FALSE => Some(Value::Bool(false)),
            JENTRY_ISSTRING => Some(Value::String(
                node.as_string().expect("a string").into_owned(),
            )),
            JENTRY_ISNUMERIC => Some(Value::Number(
                node.as_number().expect("a number").to_number().into_owned(),
            )),
            _ => None,
        }
    }

    fn open<'a>(node: &JsonbNode<'a>, depth: usize) -> Frame<'a> {
        assert!(
            depth < MATERIALIZATION_NESTING_LIMIT,
            "JSONB value exceeds maximum materialization nesting depth ({MATERIALIZATION_NESTING_LIMIT})"
        );
        if node.container_flags() & JB_FOBJECT == 0 {
            Frame::Array(node.as_array().expect("an array").elements(), Vec::new())
        } else {
            Frame::Object(
                node.as_object().expect("an object").members(),
                Map::new(),
                None,
            )
        }
    }

    let Some(value) = scalar(&root) else {
        let mut stack = vec![open(&root, 0)];
        loop {
            let depth = stack.len();
            let finished = match stack.last_mut().expect("a frame") {
                Frame::Array(elements, built) => match elements.next() {
                    Some(element) => {
                        if let Some(value) = scalar(&element) {
                            built.push(value);
                            None
                        } else {
                            let frame = open(&element, depth);
                            stack.push(frame);
                            continue;
                        }
                    }
                    None => Some(()),
                },
                Frame::Object(members, built, key) => match members.next() {
                    Some((name, member)) => {
                        if let Some(value) = scalar(&member) {
                            built.insert(name.into_owned(), value);
                            None
                        } else {
                            *key = Some(name.into_owned());
                            let frame = open(&member, depth);
                            stack.push(frame);
                            continue;
                        }
                    }
                    None => Some(()),
                },
            };
            if finished.is_none() {
                continue;
            }
            let value = match stack.pop().expect("a frame") {
                Frame::Array(_, built) => Value::Array(built),
                Frame::Object(_, built, _) => Value::Object(built),
            };
            match stack.last_mut() {
                None => return value,
                Some(Frame::Array(_, built)) => built.push(value),
                Some(Frame::Object(_, built, key)) => {
                    let name = key.take().expect("a key for the container just closed");
                    built.insert(name, value);
                }
            }
        }
    };
    value
}

// `JsonbNode` is exactly `(data, kind)`, so it round-trips losslessly.
fn rebuild_and_materialize(bytes: &[u8], tag: u32) -> Value {
    JsonbNode {
        data: bytes,
        kind: tag,
    }
    .to_value()
    .into_owned()
}

pub struct JsonbObject<'a> {
    container: &'a [u8],
    count: usize,
}

pub struct JsonbArray<'a> {
    container: &'a [u8],
    count: usize,
}

pub struct JsonbNumber<'a> {
    digits: &'a [u8],
    weight: i32,
    dscale: u32,
    negative: bool,
}

impl<'a> JsonbNumber<'a> {
    /// `bytes` is the numeric's own varlena, header included.
    fn parse(bytes: &'a [u8]) -> JsonbNumber<'a> {
        let body = varlena_body(bytes);
        let header = read_u16(body, 0);
        if header & NUMERIC_SIGN_MASK == NUMERIC_SHORT {
            let weight = i32::from(header & NUMERIC_SHORT_WEIGHT_MASK);
            JsonbNumber {
                digits: &body[2..],
                weight: if header & NUMERIC_SHORT_WEIGHT_SIGN_MASK == 0 {
                    weight
                } else {
                    weight - 64
                },
                dscale: u32::from(
                    (header & NUMERIC_SHORT_DSCALE_MASK) >> NUMERIC_SHORT_DSCALE_SHIFT,
                ),
                negative: header & NUMERIC_SHORT_SIGN_MASK != 0,
            }
        } else {
            JsonbNumber {
                digits: &body[4..],
                weight: i32::from(i16::from_ne_bytes([body[2], body[3]])),
                dscale: u32::from(header & NUMERIC_DSCALE_MASK),
                negative: header & NUMERIC_SIGN_MASK == NUMERIC_NEG,
            }
        }
    }

    fn count(&self) -> usize {
        self.digits.len() / 2
    }

    /// The base-10000 digit at `index`, or zero past the stored ones.
    fn digit(&self, index: i32) -> u32 {
        let Ok(index) = usize::try_from(index) else {
            return 0;
        };
        if index >= self.count() {
            return 0;
        }
        let at = index * 2;
        u32::from(u16::from_ne_bytes([self.digits[at], self.digits[at + 1]]))
    }

    /// Magnitude as a `u128`, or `None` when it does not fit or has a fraction.
    fn magnitude(&self) -> Option<u128> {
        if !self.is_integer() {
            return None;
        }
        let mut value: u128 = 0;
        for index in 0..=self.weight.max(0) {
            value = value.checked_mul(10_000)?;
            value = value.checked_add(u128::from(self.digit(index)))?;
        }
        Some(value)
    }

    /// Exact equality, digit group by digit group, however far the value exceeds `f64`.
    fn equals(&self, other: &JsonbNumber<'_>) -> bool {
        // Postgres normalises numerics, so a non-zero value has no leading or trailing zero
        // group and zero has no digits at all. Two non-zero values therefore disagree the
        // moment their sign or weight does, without walking the digits between them.
        if self.count() > 0
            && other.count() > 0
            && (self.negative != other.negative || self.weight != other.weight)
        {
            return false;
        }
        let high = self.weight.max(other.weight);
        let low =
            (self.weight - self.count() as i32 + 1).min(other.weight - other.count() as i32 + 1);
        let mut any_nonzero = false;
        for exponent in (low..=high).rev() {
            let left = self.digit(self.weight - exponent);
            let right = other.digit(other.weight - exponent);
            if left != right {
                return false;
            }
            any_nonzero |= left != 0;
        }
        !any_nonzero || self.negative == other.negative
    }
}

/// Equality for everything a container holds directly.
fn equal_leaves(left: JsonbNode<'_>, right: JsonbNode<'_>) -> bool {
    match (left.kind, right.kind) {
        (JENTRY_ISNULL, JENTRY_ISNULL)
        | (JENTRY_ISBOOL_FALSE, JENTRY_ISBOOL_FALSE)
        | (JENTRY_ISBOOL_TRUE, JENTRY_ISBOOL_TRUE) => true,
        (JENTRY_ISNUMERIC, JENTRY_ISNUMERIC) => left
            .as_number()
            .expect("number")
            .equals(&right.as_number().expect("number")),
        // The stored bytes, so a lossy decode cannot fold two strings together.
        (JENTRY_ISSTRING, JENTRY_ISSTRING) => left.data == right.data,
        _ => false,
    }
}

/// Structural equality between two nodes: numbers compare exactly and objects ignore key order.
/// Nesting can run deeper than a backend's stack, so pairs still to compare go on a worklist,
/// which only a container ever needs.
fn equal_nodes(left: JsonbNode<'_>, right: JsonbNode<'_>) -> bool {
    if left.kind != JENTRY_ISCONTAINER || right.kind != JENTRY_ISCONTAINER {
        return equal_leaves(left, right);
    }
    let mut pending = vec![(left, right)];
    while let Some((left, right)) = pending.pop() {
        if left.kind != JENTRY_ISCONTAINER || right.kind != JENTRY_ISCONTAINER {
            if !equal_leaves(left, right) {
                return false;
            }
            continue;
        }
        let equal = match (left.as_array(), right.as_array()) {
            (Some(left), Some(right)) => {
                left.len() == right.len() && {
                    pending.extend(left.elements().zip(right.elements()));
                    true
                }
            }
            (None, None) => {
                let (left, right) = (
                    left.as_object().expect("object"),
                    right.as_object().expect("object"),
                );
                if left.len() != right.len() {
                    return false;
                }
                // Postgres orders keys by length then bytes, so equal key sets arrive in the
                // same order and one walk settles both the keys and the pairing.
                let (mut left, mut right) = (left.members(), right.members());
                loop {
                    match (left.next_raw(), right.next_raw()) {
                        (Some((left_key, left_value)), Some((right_key, right_value))) => {
                            if left_key != right_key {
                                return false;
                            }
                            pending.push((left_value, right_value));
                        }
                        (None, None) => break true,
                        _ => return false,
                    }
                }
            }
            _ => false,
        };
        if !equal {
            return false;
        }
    }
    true
}

/// Mirrors `equal_nodes`, so equal nodes always hash equal.
fn hash_node<H: Hasher>(node: JsonbNode<'_>, state: &mut H) {
    match node.kind {
        JENTRY_ISNULL => state.write_u32(3_221_225_473), // chosen randomly
        JENTRY_ISBOOL_FALSE | JENTRY_ISBOOL_TRUE => node.as_boolean().expect("boolean").hash(state),
        JENTRY_ISNUMERIC => hash_number(&node.as_number().expect("number"), state),
        JENTRY_ISSTRING => node.data.hash(state),
        // Nesting can run deeper than a backend's stack, so each container is folded against an
        // explicit stack. Writing the length keeps `[]` apart from `[[]]`.
        _ => state.write_u64(hash_container(node)),
    }
}

/// One container being folded: what is left to visit, and the hash accumulated so far.
enum HashFrame<'a> {
    Array(Cursor<'a>, AHasher),
    // Members are combined with `^` so key order cannot change the result.
    Object(JsonbMembers<'a>, u64, Option<Cow<'a, str>>),
}

fn hash_container(root: JsonbNode<'_>) -> u64 {
    fn open<'a>(node: &JsonbNode<'a>) -> HashFrame<'a> {
        if let Some(array) = node.as_array() {
            let mut hasher = AHasher::default();
            hasher.write_usize(array.len());
            HashFrame::Array(array.elements(), hasher)
        } else {
            HashFrame::Object(node.as_object().expect("object").members(), 0, None)
        }
    }

    let mut stack = vec![open(&root)];
    loop {
        let next = match stack.last_mut().expect("a frame") {
            HashFrame::Array(elements, hasher) => elements.next().map(|element| {
                if element.kind != JENTRY_ISCONTAINER {
                    hash_node(element, hasher);
                    return None;
                }
                Some(element)
            }),
            HashFrame::Object(members, combined, key) => members.next().map(|(name, member)| {
                if member.kind != JENTRY_ISCONTAINER {
                    let mut hasher = AHasher::default();
                    name.hash(&mut hasher);
                    hash_node(member, &mut hasher);
                    *combined ^= hasher.finish();
                    return None;
                }
                *key = Some(name);
                Some(member)
            }),
        };
        match next {
            Some(Some(container)) => {
                let frame = open(&container);
                stack.push(frame);
            }
            Some(None) => {}
            None => {
                let finished = match stack.pop().expect("a frame") {
                    HashFrame::Array(_, hasher) => hasher.finish(),
                    HashFrame::Object(_, combined, _) => combined,
                };
                match stack.last_mut() {
                    None => return finished,
                    Some(HashFrame::Array(_, hasher)) => hasher.write_u64(finished),
                    Some(HashFrame::Object(_, combined, key)) => {
                        let mut hasher = AHasher::default();
                        key.take().expect("a key").hash(&mut hasher);
                        hasher.write_u64(finished);
                        *combined ^= hasher.finish();
                    }
                }
            }
        }
    }
}

/// Trimmed to the leading..trailing nonzero group, so redundant zero groups do not change it.
fn hash_number<H: Hasher>(number: &JsonbNumber<'_>, state: &mut H) {
    let mut first = None;
    let mut last = None;
    for index in 0..number.count() as i32 {
        if number.digit(index) != 0 {
            first.get_or_insert(index);
            last = Some(index);
        }
    }
    let Some(first) = first else {
        state.write_u32(0); // canonical zero
        return;
    };
    number.negative.hash(state);
    (number.weight - first).hash(state);
    for index in first..=last.expect("first implies last") {
        number.digit(index).hash(state);
    }
}

struct HashedNode<'a>(JsonbNode<'a>);

impl PartialEq for HashedNode<'_> {
    fn eq(&self, other: &Self) -> bool {
        equal_nodes(self.0, other.0)
    }
}

impl Eq for HashedNode<'_> {}

impl Hash for HashedNode<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_node(self.0, state);
    }
}

/// Keys and values live in separate runs of the entry array, so each side gets its own cursor.
pub struct JsonbMembers<'a> {
    keys: Cursor<'a>,
    values: Cursor<'a>,
}

impl<'a> Iterator for JsonbMembers<'a> {
    type Item = (Cow<'a, str>, JsonbNode<'a>);

    fn next(&mut self) -> Option<(Cow<'a, str>, JsonbNode<'a>)> {
        let (key, value) = self.next_raw()?;
        Some((String::from_utf8_lossy(key), value))
    }
}

impl<'a> JsonbMembers<'a> {
    /// The next member with its key as stored.
    ///
    /// Decoding a key lossily maps every invalid byte onto U+FFFD, so distinct keys would
    /// compare equal; equality walks the stored bytes instead.
    fn next_raw(&mut self) -> Option<(&'a [u8], JsonbNode<'a>)> {
        let key = self.keys.next()?;
        let value = self.values.next()?;
        Some((key.bytes(), value))
    }
}

impl<'a> Object<'a, Jsonb> for JsonbObject<'a> {
    type Node = JsonbNode<'a>;
    type MemberName = Cow<'a, str>;
    type MembersIter = JsonbMembers<'a>;

    fn len(&self) -> usize {
        self.count
    }

    #[allow(clippy::borrowed_box)]
    fn get(&self, key: &Box<[u8]>) -> Option<JsonbNode<'a>> {
        let key: &[u8] = key;
        let data_start = data_start(self.count * 2);
        let mut low = 0;
        let mut high = self.count;
        while low < high {
            let middle = usize::midpoint(low, high);
            let offset = entry_offset(self.container, middle);
            let length = entry_length(self.container, middle, offset);
            let at = data_start + offset as usize;
            let candidate = &self.container[at..at + length as usize];
            // Postgres orders keys by length first, then by bytes.
            match (candidate.len(), candidate).cmp(&(key.len(), key)) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => {
                    let index = self.count + middle;
                    // The value's entries follow the key's, so resume from the key's offset.
                    let offset = entry_offset_after(self.container, index, middle, offset);
                    let length = entry_length(self.container, index, offset);
                    let kind = read_u32(self.container, 4 + 4 * index) & JENTRY_TYPEMASK;
                    return Some(child(
                        self.container,
                        data_start,
                        4 + 4 * index,
                        kind | length,
                        offset,
                    ));
                }
            }
        }
        None
    }

    fn members(&self) -> JsonbMembers<'a> {
        let data_start = data_start(self.count * 2);
        JsonbMembers {
            keys: Cursor::new(self.container, data_start, 0, self.count),
            values: Cursor::new(self.container, data_start, self.count, self.count * 2),
        }
    }
}

/// A sequential walk over a run of entries, carrying the running offset so each step is O(1).
pub struct Cursor<'a> {
    container: &'a [u8],
    data_start: usize,
    index: usize,
    last: usize,
    offset: u32,
}

impl<'a> Cursor<'a> {
    fn new(container: &'a [u8], data_start: usize, first: usize, last: usize) -> Self {
        Cursor {
            container,
            data_start,
            index: first,
            last,
            offset: entry_offset(container, first),
        }
    }
}

impl<'a> Iterator for Cursor<'a> {
    type Item = JsonbNode<'a>;

    fn next(&mut self) -> Option<JsonbNode<'a>> {
        if self.index >= self.last {
            return None;
        }
        let length = entry_length(self.container, self.index, self.offset);
        let kind = read_u32(self.container, 4 + 4 * self.index) & JENTRY_TYPEMASK;
        let node = child(
            self.container,
            self.data_start,
            4 + 4 * self.index,
            kind | length,
            self.offset,
        );
        self.index += 1;
        self.offset += length;
        Some(node)
    }
}

impl<'a> Array<'a, Jsonb> for JsonbArray<'a> {
    type Node = JsonbNode<'a>;
    type ElementsIter = Cursor<'a>;

    fn len(&self) -> usize {
        self.count
    }

    fn elements(&self) -> Cursor<'a> {
        Cursor::new(self.container, data_start(self.count), 0, self.count)
    }

    fn is_unique(&self) -> bool {
        // Elements arrive through a cursor, so one walk up front beats re-walking it per index.
        let items: Vec<JsonbNode<'a>> = self.elements().collect();
        crate::unique::is_unique_by(
            items.len(),
            |index| items[index],
            |left, right| equal_nodes(*left, *right),
            |index| HashedNode(items[index]),
        )
    }
}

impl JsonNumber for JsonbNumber<'_> {
    fn as_u64(&self) -> Option<u64> {
        if self.negative {
            return None;
        }
        u64::try_from(self.magnitude()?).ok()
    }

    fn as_i64(&self) -> Option<i64> {
        let magnitude = self.magnitude()?;
        if self.negative {
            i64::try_from(magnitude).map_or_else(
                |_| (magnitude == 1_u128 << 63).then_some(i64::MIN),
                |value| Some(-value),
            )
        } else {
            i64::try_from(magnitude).ok()
        }
    }

    fn as_f64(&self) -> Option<f64> {
        // Integers convert directly; anything else rounds through the digits, as its text would.
        if let Some(magnitude) = self.magnitude() {
            let value = magnitude as f64;
            return Some(if self.negative { -value } else { value });
        }
        // Scientific notation over the stored groups, rather than `as_str`'s expansion: that runs
        // to one character per decimal place, and a weight Postgres accepts reaches tens of
        // thousands of them. Parsing is left to round the result.
        let mut text = String::new();
        if self.negative {
            text.push('-');
        }
        for index in 0..self.count() as i32 {
            let digit = self.digit(index);
            if index == 0 {
                write!(text, "{digit}").expect("write to String never fails");
            } else {
                write!(text, "{digit:04}").expect("write to String never fails");
            }
        }
        // Every group past the stored ones is zero, so they only place the decimal point.
        let exponent = 4 * (self.weight - self.count() as i32 + 1);
        write!(text, "e{exponent}").expect("write to String never fails");
        // Callers read `None` as "past binary64"; rounding up to infinity would answer keywords
        // as if the value were one.
        text.parse().ok().filter(|value: &f64| value.is_finite())
    }

    fn as_str(&self) -> Cow<'_, str> {
        let mut out = String::new();
        if self.negative {
            out.push('-');
        }
        if self.weight < 0 {
            out.push('0');
        } else {
            for index in 0..=self.weight {
                let digit = self.digit(index);
                if index == 0 {
                    write!(out, "{digit}").expect("write to String never fails");
                } else {
                    write!(out, "{digit:04}").expect("write to String never fails");
                }
            }
        }
        if self.dscale > 0 {
            out.push('.');
            let mut written = 0;
            let mut index = self.weight + 1;
            while written < self.dscale {
                let group = format!("{:04}", self.digit(index));
                let take = 4.min(self.dscale - written) as usize;
                out.push_str(&group[..take]);
                written += take as u32;
                index += 1;
            }
        }
        Cow::Owned(out)
    }

    fn to_number(&self) -> Cow<'_, serde_json::Number> {
        // Past `f64` the digits are already lost, so this keeps the sign and the largest
        // magnitude a `Number` can carry rather than reporting an unrelated value.
        serde_json::from_str(&self.as_str()).map_or_else(
            |_| {
                let saturated = if self.negative { -f64::MAX } else { f64::MAX };
                Cow::Owned(
                    serde_json::Number::from_f64(saturated).expect("f64::MAX is representable"),
                )
            },
            Cow::Owned,
        )
    }

    // `dscale` is how many fraction digits Postgres displays, so it records how the value was
    // written even when every one of them is zero. An exponent does not survive the datum, so
    // `1e2` arrives indistinguishable from `100`.
    fn is_written_as_integer(&self) -> bool {
        self.dscale == 0
    }

    fn is_negative(&self) -> bool {
        self.negative
    }

    fn is_integer(&self) -> bool {
        // Fraction digits are the ones past `weight`; `dscale` is display only.
        let first = (self.weight + 1).max(0);
        (first..self.count() as i32).all(|index| self.digit(index) == 0)
    }
}

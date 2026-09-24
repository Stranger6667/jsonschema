//! Postgres `jsonb` representation: reads stored bytes in place.
// Digits are bounded base-10000 groups and the layout is fixed-width.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap
)]

#[cfg(feature = "jsonb-testkit")]
pub mod encode;

use std::{
    borrow::Cow,
    cell::RefCell,
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
pub(crate) const JB_FSCALAR: u32 = 0x1000_0000;
pub(crate) const JB_FOBJECT: u32 = 0x2000_0000;

pub(crate) const JENTRY_OFFLENMASK: u32 = 0x0FFF_FFFF;
pub(crate) const JENTRY_TYPEMASK: u32 = 0x7000_0000;
pub(crate) const JENTRY_HAS_OFF: u32 = 0x8000_0000;

pub(crate) const JENTRY_ISSTRING: u32 = 0x0000_0000;
pub(crate) const JENTRY_ISNUMERIC: u32 = 0x1000_0000;
pub(crate) const JENTRY_ISBOOL_FALSE: u32 = 0x2000_0000;
pub(crate) const JENTRY_ISBOOL_TRUE: u32 = 0x3000_0000;
pub(crate) const JENTRY_ISNULL: u32 = 0x4000_0000;
pub(crate) const JENTRY_ISCONTAINER: u32 = 0x5000_0000;

const NUMERIC_SIGN_MASK: u16 = 0xC000;
pub(crate) const NUMERIC_NEG: u16 = 0x4000;
pub(crate) const NUMERIC_SHORT: u16 = 0x8000;
const NUMERIC_DSCALE_MASK: u16 = 0x3FFF;
pub(crate) const NUMERIC_SHORT_SIGN_MASK: u16 = 0x2000;
const NUMERIC_SHORT_DSCALE_MASK: u16 = 0x1F80;
pub(crate) const NUMERIC_SHORT_DSCALE_SHIFT: u16 = 7;
pub(crate) const NUMERIC_SHORT_WEIGHT_SIGN_MASK: u16 = 0x0040;
pub(crate) const NUMERIC_SHORT_WEIGHT_MASK: u16 = 0x003F;

// `serde_json`'s own parser depth: `Value`'s `Clone`, `Drop` and `Display` recurse, so a deeper
// one can overflow the caller's stack.
const RECURSION_LIMIT: usize = 128;

thread_local! {
    static PENDING_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn record(message: impl FnOnce() -> String) {
    PENDING_ERROR.with(|slot| {
        slot.borrow_mut().get_or_insert_with(message);
    });
}

/// Take the error recorded while reporting an instance back as a `serde_json::Value`, if any.
#[must_use]
pub fn take_pending_error() -> Option<String> {
    PENDING_ERROR.with(|slot| slot.borrow_mut().take())
}

pub struct Jsonb;

/// One `jsonb` value: its own bytes and its `JEntry` type bits.
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

/// A `numeric` varlena without its own header.
fn varlena_body(bytes: &[u8]) -> &[u8] {
    #[cfg(target_endian = "big")]
    let short = bytes[0] & 0x80 == 0x80;
    #[cfg(target_endian = "little")]
    let short = bytes[0] & 0x01 == 0x01;
    if short {
        &bytes[1..]
    } else {
        &bytes[4..]
    }
}

/// The header word and the number of `JEntry`s after it.
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
    /// A node over a `JsonbContainer` without its varlena header.
    ///
    /// # Panics
    ///
    /// On anything but a container as Postgres writes one.
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
        // A zero-length child borrows from its own JEntry to keep a unique address.
        data: if start == end {
            &container[entry_at..entry_at]
        } else {
            &container[start..end]
        },
        kind,
    }
}

// Sums lengths backwards to the nearest entry that stores its end offset; `JB_OFFSET_STRIDE`
// bounds this at 32 steps.
fn entry_offset(container: &[u8], index: usize) -> u32 {
    entry_offset_after(container, index, 0, 0)
}

// `checkpoint_offset` must equal `entry_offset(container, checkpoint)`.
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

impl JsonbNode<'_> {
    fn container_flags(&self) -> u32 {
        read_u32(self.data, 0)
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
            container: self.data,
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
            container: self.data,
            count: (flags & JB_CMASK) as usize,
        })
    }

    fn as_string(&self) -> Option<Cow<'a, str>> {
        if self.kind == JENTRY_ISSTRING {
            // Validating first skips `from_utf8_lossy`'s chunk walk on the common path.
            Some(match std::str::from_utf8(self.data) {
                Ok(text) => Cow::Borrowed(text),
                Err(_) => String::from_utf8_lossy(self.data),
            })
        } else {
            None
        }
    }

    fn as_number(&self) -> Option<JsonbNumber<'a>> {
        if self.kind == JENTRY_ISNUMERIC {
            Some(JsonbNumber::parse(self.data))
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
        // Must agree with `as_string`, whose lossy decode can turn one invalid run into several
        // replacement characters.
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

    // A `const` built in code has no depth cap, so this walks rather than recurses.
    fn equals_value(&self, expected: &Value) -> bool {
        let mut pending = vec![(*self, expected)];
        while let Some((node, expected)) = pending.pop() {
            let equal = match expected {
                Value::Null => node.is_null(),
                Value::Bool(boolean) => node.as_boolean() == Some(*boolean),
                Value::Number(number) => node
                    .as_number()
                    .is_some_and(|got| cmp::equal_numbers(&got, number)),
                Value::String(string) => node
                    .as_string()
                    .is_some_and(|got| got.as_ref() == string.as_str()),
                Value::Array(items) => node.as_array().is_some_and(|array| {
                    array.len() == items.len() && {
                        pending.extend(array.elements().zip(items));
                        true
                    }
                }),
                Value::Object(map) => node.as_object().is_some_and(|object| {
                    if object.len() != map.len() {
                        return false;
                    }
                    // A key that is not UTF-8 matches no `String` key.
                    let mut members = object.members();
                    while let Some((key, value)) = members.next_raw() {
                        let Some(expected) =
                            std::str::from_utf8(key).ok().and_then(|key| map.get(key))
                        else {
                            return false;
                        };
                        pending.push((value, expected));
                    }
                    true
                }),
            };
            if !equal {
                return false;
            }
        }
        true
    }

    fn to_value(&self) -> Cow<'a, Value> {
        Cow::Owned(materialize(*self))
    }

    // Most failures never read the instance back, so this copies two fields, not the document.
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

/// A container being rebuilt: what is left to visit and what is built so far.
enum Frame<'a> {
    Array(Cursor<'a>, Vec<Value>),
    // The last field is the name of the member being built.
    Object(JsonbMembers<'a>, Map<String, Value>, String),
}

impl<'a> Frame<'a> {
    fn open(node: &JsonbNode<'a>) -> Self {
        match node.as_array() {
            Some(array) => Frame::Array(array.elements(), Vec::new()),
            None => Frame::Object(
                node.as_object().expect("an object").members(),
                Map::new(),
                String::new(),
            ),
        }
    }

    fn next(&mut self) -> Option<JsonbNode<'a>> {
        match self {
            Frame::Array(elements, _) => elements.next(),
            Frame::Object(members, _, name) => members.next().map(|(key, value)| {
                *name = key.into_owned();
                value
            }),
        }
    }

    fn push(&mut self, value: Value) {
        match self {
            Frame::Array(_, built) => built.push(value),
            Frame::Object(_, built, name) => {
                built.insert(std::mem::take(name), value);
            }
        }
    }

    fn finish(self) -> Value {
        match self {
            Frame::Array(_, built) => Value::Array(built),
            Frame::Object(_, built, _) => Value::Object(built),
        }
    }
}

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

// Postgres nests deeper than a recursive walk survives, so containers go on an explicit stack.
// Past `RECURSION_LIMIT` a container reports as `null` and records a pending error.
fn materialize(root: JsonbNode<'_>) -> Value {
    if let Some(value) = scalar(&root) {
        return value;
    }
    let mut stack = vec![Frame::open(&root)];
    loop {
        let depth = stack.len();
        let top = stack.last_mut().expect("a frame");
        if let Some(node) = top.next() {
            if let Some(value) = scalar(&node) {
                top.push(value);
            } else if depth < RECURSION_LIMIT {
                stack.push(Frame::open(&node));
            } else {
                record(|| format!("Exceeded maximum nesting depth ({RECURSION_LIMIT})"));
                top.push(Value::Null);
            }
            continue;
        }
        let value = stack.pop().expect("a frame").finish();
        match stack.last_mut() {
            Some(parent) => parent.push(value),
            None => return value,
        }
    }
}

fn rebuild_and_materialize(bytes: &[u8], tag: u32) -> Value {
    materialize(JsonbNode {
        data: bytes,
        kind: tag,
    })
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

// Exactly representable in `f64`.
const POWERS_OF_TEN: [f64; 23] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
    1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
];

impl<'a> JsonbNumber<'a> {
    /// `bytes` is the numeric's varlena, header included.
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

    /// `None` when it has a fraction or does not fit.
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

    fn signed(&self, value: f64) -> f64 {
        if self.negative {
            -value
        } else {
            value
        }
    }

    // Exact at any magnitude.
    fn equals(&self, other: &JsonbNumber<'_>) -> bool {
        // Postgres strips leading and trailing zero groups, and zero has none, so two non-zero
        // values with a different sign or weight differ.
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

fn equal_leaves(left: JsonbNode<'_>, right: JsonbNode<'_>) -> bool {
    match (left.kind, right.kind) {
        (JENTRY_ISNULL, JENTRY_ISNULL)
        | (JENTRY_ISBOOL_FALSE, JENTRY_ISBOOL_FALSE)
        | (JENTRY_ISBOOL_TRUE, JENTRY_ISBOOL_TRUE) => true,
        (JENTRY_ISNUMERIC, JENTRY_ISNUMERIC) => left
            .as_number()
            .expect("number")
            .equals(&right.as_number().expect("number")),
        // Stored bytes, so a lossy decode cannot fold two strings together.
        (JENTRY_ISSTRING, JENTRY_ISSTRING) => left.data == right.data,
        _ => false,
    }
}

fn equal_nodes(left: JsonbNode<'_>, right: JsonbNode<'_>) -> bool {
    equal_nodes_into(left, right, &mut Vec::new())
}

// Iterative, with the caller's worklist so a run of comparisons allocates once.
fn equal_nodes_into<'a>(
    left: JsonbNode<'a>,
    right: JsonbNode<'a>,
    pending: &mut Vec<(JsonbNode<'a>, JsonbNode<'a>)>,
) -> bool {
    if left.kind != JENTRY_ISCONTAINER || right.kind != JENTRY_ISCONTAINER {
        return equal_leaves(left, right);
    }
    pending.push((left, right));
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
                // Keys are sorted, so equal key sets line up.
                let (mut left, mut right) = (left.members(), right.members());
                while let (Some((left_key, left_value)), Some((right_key, right_value))) =
                    (left.next_raw(), right.next_raw())
                {
                    if left_key != right_key {
                        return false;
                    }
                    pending.push((left_value, right_value));
                }
                true
            }
            _ => false,
        };
        if !equal {
            return false;
        }
    }
    true
}

/// Agrees with `equal_nodes`.
fn hash_node<H: Hasher>(node: JsonbNode<'_>, state: &mut H) {
    match node.kind {
        JENTRY_ISNULL => state.write_u32(3_221_225_473), // chosen randomly
        JENTRY_ISBOOL_FALSE | JENTRY_ISBOOL_TRUE => node.as_boolean().expect("boolean").hash(state),
        JENTRY_ISNUMERIC => hash_number(&node.as_number().expect("number"), state),
        JENTRY_ISSTRING => node.data.hash(state),
        _ => state.write_u64(hash_container(node)),
    }
}

enum HashFrame<'a> {
    // Starts with the length, keeping `[]` apart from `[[]]`.
    Array(Cursor<'a>, AHasher),
    // Members combine with `^`, so key order does not matter.
    Object(JsonbMembers<'a>, u64, Option<Cow<'a, str>>),
}

// Iterative, like `materialize`.
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

// Over the leading..trailing non-zero groups only.
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

/// Keys and values live in separate runs of entries, so each gets its own cursor.
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
    // Stored bytes: a lossy decode would make distinct keys equal.
    fn next_raw(&mut self) -> Option<(&'a [u8], JsonbNode<'a>)> {
        let key = self.keys.next()?;
        let value = self.values.next()?;
        Some((key.data, value))
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
            // Postgres orders keys by length, then bytes.
            match (candidate.len(), candidate).cmp(&(key.len(), key)) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => {
                    let index = self.count + middle;
                    // The value's entries follow the keys', so resume from the key's offset.
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

/// Walks a run of entries, carrying the running offset so each step is O(1).
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
        if self.count <= 1 {
            return true;
        }
        // One walk of the cursor up front instead of one per index.
        let items: Vec<JsonbNode<'a>> = self.elements().collect();
        if items.len() <= crate::unique::ITEMS_SIZE_THRESHOLD {
            // Pairwise is cheaper than hashing at this size.
            let mut pending = Vec::new();
            for (index, left) in items.iter().enumerate() {
                for right in &items[index + 1..] {
                    pending.clear();
                    if equal_nodes_into(*left, *right, &mut pending) {
                        return false;
                    }
                }
            }
            return true;
        }
        let mut seen = ahash::AHashSet::with_capacity(items.len());
        items.into_iter().all(|item| seen.insert(HashedNode(item)))
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
        if let Some(magnitude) = self.magnitude() {
            return Some(self.signed(magnitude as f64));
        }
        let count = self.count();
        let exponent = 4 * (self.weight - count as i32 + 1);
        // Four groups fit a `u64`; an exact mantissa over an exact power of ten rounds correctly
        // in one division. A non-integer always has a negative exponent.
        if count <= 4 {
            let mantissa = (0..count as i32).fold(0_u64, |acc, index| {
                acc * 10_000 + u64::from(self.digit(index))
            });
            let power = POWERS_OF_TEN.get(exponent.unsigned_abs() as usize);
            if let (true, Some(power)) = (mantissa <= 1 << 53, power) {
                return Some(self.signed(mantissa as f64 / power));
            }
        }
        // Scientific notation: `as_str` would write every decimal place.
        let mut text = String::new();
        if self.negative {
            text.push('-');
        }
        for index in 0..count as i32 {
            write!(text, "{:04}", self.digit(index)).expect("write to String never fails");
        }
        write!(text, "e{exponent}").expect("write to String never fails");
        // `None` means past binary64, not infinity.
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
            write!(out, "{}", self.digit(0)).expect("write to String never fails");
            for index in 1..=self.weight {
                write!(out, "{:04}", self.digit(index)).expect("write to String never fails");
            }
        }
        if self.dscale > 0 {
            out.push('.');
            let end = out.len() + self.dscale as usize;
            let mut index = self.weight + 1;
            while out.len() < end {
                write!(out, "{:04}", self.digit(index)).expect("write to String never fails");
                index += 1;
            }
            out.truncate(end);
        }
        Cow::Owned(out)
    }

    fn to_number(&self) -> Cow<'_, serde_json::Number> {
        if let Ok(number) = serde_json::from_str(&self.as_str()) {
            return Cow::Owned(number);
        }
        record(|| "Number out of range".to_string());
        let saturated = self.signed(f64::MAX);
        Cow::Owned(serde_json::Number::from_f64(saturated).expect("f64::MAX is finite"))
    }

    // `dscale` keeps how many fraction digits were written, zeros included. An exponent does
    // not survive, so `1e2` reads as `100`.
    fn is_written_as_integer(&self) -> bool {
        self.dscale == 0
    }

    fn is_integer(&self) -> bool {
        // `dscale` is display only; fraction digits are the groups past `weight`.
        let first = (self.weight + 1).max(0);
        (first..self.count() as i32).all(|index| self.digit(index) == 0)
    }
}

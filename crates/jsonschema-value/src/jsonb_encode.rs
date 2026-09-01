//! Builds Postgres `jsonb` bytes, for tests and fixture generation.
// Test helpers: the casts are to known-small fields, and a malformed fixture should just panic.
#![allow(
    clippy::must_use_candidate,
    clippy::missing_panics_doc,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use serde_json::Value;

pub const JB_FSCALAR: u32 = 0x1000_0000;
const JB_FOBJECT: u32 = 0x2000_0000;
pub const JB_FARRAY: u32 = 0x4000_0000;

const JENTRY_OFFLENMASK: u32 = 0x0FFF_FFFF;
const JENTRY_TYPEMASK: u32 = 0x7000_0000;
pub const JENTRY_HAS_OFF: u32 = 0x8000_0000;

const JENTRY_ISSTRING: u32 = 0x0000_0000;
pub const JENTRY_ISNUMERIC: u32 = 0x1000_0000;
const JENTRY_ISBOOL_FALSE: u32 = 0x2000_0000;
const JENTRY_ISBOOL_TRUE: u32 = 0x3000_0000;
const JENTRY_ISNULL: u32 = 0x4000_0000;
const JENTRY_ISCONTAINER: u32 = 0x5000_0000;

const JB_OFFSET_STRIDE: usize = 32;

const NUMERIC_NEG: u16 = 0x4000;
const NUMERIC_SHORT: u16 = 0x8000;
const NUMERIC_SHORT_SIGN_MASK: u16 = 0x2000;
const NUMERIC_SHORT_DSCALE_SHIFT: u16 = 7;
const NUMERIC_SHORT_WEIGHT_SIGN_MASK: u16 = 0x0040;
const NUMERIC_SHORT_WEIGHT_MASK: u16 = 0x003F;

// Which numeric header to write. Postgres picks `Short` whenever the weight and dscale fit.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NumericForm {
    Short,
    Long,
    // A 1-byte varlena header, not the short numeric header.
    ShortVarlena,
}
/// The stored varlena, minus its header: a `JsonbContainer`.
///
/// Postgres writes either a 1-byte header carrying the size, or a 4-byte one, and marks which
/// in the low bit (high bit on big-endian).
#[must_use]
pub fn strip_varlena(bytes: &[u8]) -> &[u8] {
    let word = || u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let (header, size) = if cfg!(target_endian = "big") {
        if bytes[0] & 0x80 == 0x80 {
            (1, usize::from(bytes[0] & 0x7F))
        } else {
            (4, word() as usize)
        }
    } else if bytes[0] & 0x01 == 0x01 {
        (1, usize::from(bytes[0] >> 1) & 0x7F)
    } else {
        (4, (word() >> 2) as usize)
    };
    &bytes[header..size]
}

/// Bytes from the hex `psql` prints for a `bytea`.
///
/// # Panics
///
/// If `text` is not an even-length run of hex digits.
#[must_use]
pub fn decode_hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).expect("hex"))
        .collect()
}

pub fn encode(value: &Value) -> Vec<u8> {
    encode_with(value, NumericForm::Short)
}

pub fn encode_with(value: &Value, form: NumericForm) -> Vec<u8> {
    match value {
        Value::Array(_) | Value::Object(_) => encode_container(value, form),
        scalar => {
            let mut data = Vec::new();
            let entry = encode_value(scalar, &mut data, form);
            let entry = stride_entry(entry, 0, entry & JENTRY_OFFLENMASK);
            assemble(JB_FARRAY | JB_FSCALAR | 1, &[entry], &data)
        }
    }
}

pub fn assemble(flags: u32, entries: &[u32], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 4 * entries.len() + data.len());
    out.extend_from_slice(&flags.to_ne_bytes());
    for entry in entries {
        out.extend_from_slice(&entry.to_ne_bytes());
    }
    out.extend_from_slice(data);
    out
}

// A JEntry's length shares its word with the type bits, so an oversized payload would re-tag the
// value instead of overflowing.
fn entry(kind: u32, length: u32) -> u32 {
    assert!(
        length <= JENTRY_OFFLENMASK,
        "payload of {length} bytes does not fit a JEntry"
    );
    kind | length
}

fn pad_to_int(data: &mut Vec<u8>) -> u32 {
    let pad = (4 - data.len() % 4) % 4;
    data.resize(data.len() + pad, 0);
    pad as u32
}

fn encode_string(bytes: &[u8], data: &mut Vec<u8>) -> u32 {
    data.extend_from_slice(bytes);
    entry(JENTRY_ISSTRING, bytes.len() as u32)
}

fn encode_value(value: &Value, data: &mut Vec<u8>, form: NumericForm) -> u32 {
    match value {
        Value::Null => JENTRY_ISNULL,
        Value::Bool(true) => JENTRY_ISBOOL_TRUE,
        Value::Bool(false) => JENTRY_ISBOOL_FALSE,
        Value::String(string) => encode_string(string.as_bytes(), data),
        Value::Number(number) => {
            let pad = pad_to_int(data);
            let numeric = encode_numeric(&number.to_string(), form);
            data.extend_from_slice(&numeric);
            entry(JENTRY_ISNUMERIC, pad + numeric.len() as u32)
        }
        container => {
            let pad = pad_to_int(data);
            let nested = encode_container(container, form);
            data.extend_from_slice(&nested);
            entry(JENTRY_ISCONTAINER, pad + nested.len() as u32)
        }
    }
}

// Every 32nd entry stores the running end offset instead of its own length.
fn stride_entry(entry: u32, index: usize, total: u32) -> u32 {
    if index % JB_OFFSET_STRIDE == 0 {
        (entry & JENTRY_TYPEMASK) | total | JENTRY_HAS_OFF
    } else {
        entry
    }
}

fn encode_container(value: &Value, form: NumericForm) -> Vec<u8> {
    let mut entries = Vec::new();
    let mut data = Vec::new();
    let mut total = 0_u32;
    match value {
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let entry = encode_value(item, &mut data, form);
                total += entry & JENTRY_OFFLENMASK;
                entries.push(stride_entry(entry, index, total));
            }
            assemble(JB_FARRAY | items.len() as u32, &entries, &data)
        }
        Value::Object(members) => {
            let mut pairs: Vec<(&String, &Value)> = members.iter().collect();
            // Postgres stores keys shortest-first, then by byte order.
            pairs.sort_by(|left, right| {
                (left.0.len(), left.0.as_bytes()).cmp(&(right.0.len(), right.0.as_bytes()))
            });
            for (index, (key, _)) in pairs.iter().enumerate() {
                let entry = encode_string(key.as_bytes(), &mut data);
                total += entry & JENTRY_OFFLENMASK;
                entries.push(stride_entry(entry, index, total));
            }
            for (index, (_, member)) in pairs.iter().enumerate() {
                let entry = encode_value(member, &mut data, form);
                total += entry & JENTRY_OFFLENMASK;
                entries.push(stride_entry(entry, index + pairs.len(), total));
            }
            assemble(JB_FOBJECT | pairs.len() as u32, &entries, &data)
        }
        other => panic!("not a container: {other}"),
    }
}

// Decimal text to a Postgres `numeric` varlena: base-10000 digits, a weight naming the group that
// holds the units place, and a dscale naming how many fraction digits are displayed.
pub fn encode_numeric(text: &str, form: NumericForm) -> Vec<u8> {
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (mantissa, exponent) = match rest.find(['e', 'E']) {
        Some(at) => (
            &rest[..at],
            rest[at + 1..].parse::<i32>().expect("exponent parses"),
        ),
        None => (rest, 0),
    };
    let (integer_text, fraction_text) = match mantissa.find('.') {
        Some(at) => (&mantissa[..at], &mantissa[at + 1..]),
        None => (mantissa, ""),
    };

    let dscale = (fraction_text.len() as i32 - exponent).max(0) as u16;
    let significant: Vec<u8> = integer_text
        .bytes()
        .chain(fraction_text.bytes())
        .map(|byte| byte - b'0')
        .collect();
    // Digits before the decimal point, which may fall outside the written ones.
    let point = integer_text.len() as i32 + exponent;

    let mut integer_digits: Vec<u8> = Vec::new();
    let mut fraction_digits: Vec<u8> = Vec::new();
    if point <= 0 {
        fraction_digits.resize((-point) as usize, 0);
        fraction_digits.extend_from_slice(&significant);
    } else if point as usize >= significant.len() {
        integer_digits.extend_from_slice(&significant);
        integer_digits.resize(point as usize, 0);
    } else {
        integer_digits.extend_from_slice(&significant[..point as usize]);
        fraction_digits.extend_from_slice(&significant[point as usize..]);
    }

    // Group into base-10000 aligned on the decimal point.
    let leading = (4 - integer_digits.len() % 4) % 4;
    let mut aligned = vec![0_u8; leading];
    aligned.extend_from_slice(&integer_digits);
    let integer_groups = aligned.len() / 4;
    aligned.extend_from_slice(&fraction_digits);
    aligned.resize(aligned.len().next_multiple_of(4), 0);

    let mut digits: Vec<i16> = aligned
        .chunks_exact(4)
        .map(|group| {
            group
                .iter()
                .fold(0_i16, |value, digit| value * 10 + i16::from(*digit))
        })
        .collect();
    let mut weight = integer_groups as i32 - 1;
    while digits.first() == Some(&0) {
        digits.remove(0);
        weight -= 1;
    }
    while digits.last() == Some(&0) {
        digits.pop();
    }
    // Postgres stores zero without a sign and with weight 0.
    let (negative, weight) = if digits.is_empty() {
        (false, 0)
    } else {
        (negative, weight)
    };

    let fits_short = dscale <= 0x3F && (-64..=63).contains(&weight);
    // `Short` is a request, not a promise: Postgres also falls back when the value does not fit.
    let use_long = form == NumericForm::Long || (form == NumericForm::Short && !fits_short);

    let mut body: Vec<u8> = Vec::new();
    if use_long {
        let sign = if negative { NUMERIC_NEG } else { 0 };
        body.extend_from_slice(&(sign | dscale).to_ne_bytes());
        body.extend_from_slice(&(weight as i16).to_ne_bytes());
    } else {
        assert!(
            fits_short,
            "dscale {dscale} or weight {weight} needs the long form"
        );
        let sign = if negative { NUMERIC_SHORT_SIGN_MASK } else { 0 };
        let weight_sign = if weight < 0 {
            NUMERIC_SHORT_WEIGHT_SIGN_MASK
        } else {
            0
        };
        let header = NUMERIC_SHORT
            | sign
            | (dscale << NUMERIC_SHORT_DSCALE_SHIFT)
            | weight_sign
            | (weight as u16 & NUMERIC_SHORT_WEIGHT_MASK);
        body.extend_from_slice(&header.to_ne_bytes());
    }
    for digit in &digits {
        body.extend_from_slice(&digit.to_ne_bytes());
    }

    let mut out = Vec::new();
    if form == NumericForm::ShortVarlena {
        assert!(
            body.len() < 0x7F,
            "value too large for a short varlena header"
        );
        let size = (body.len() + 1) as u8;
        out.push(varlena_header_1b(size));
    } else {
        out.extend_from_slice(&varlena_header_4b((body.len() + 4) as u32));
    }
    out.extend_from_slice(&body);
    out
}

// An array of numerics from decimal text rather than `serde_json::Value`, so a magnitude
// `serde_json::Number` cannot hold (e.g. beyond `f64`'s range) can still be encoded.
pub fn encode_numeric_text_array(texts: &[&str], form: NumericForm) -> Vec<u8> {
    let mut entries = Vec::new();
    let mut data = Vec::new();
    let mut total = 0_u32;
    for (index, text) in texts.iter().enumerate() {
        let pad = pad_to_int(&mut data);
        let numeric = encode_numeric(text, form);
        data.extend_from_slice(&numeric);
        let entry = entry(JENTRY_ISNUMERIC, pad + numeric.len() as u32);
        total += entry & JENTRY_OFFLENMASK;
        entries.push(stride_entry(entry, index, total));
    }
    assemble(JB_FARRAY | texts.len() as u32, &entries, &data)
}
/// A one-member object whose key is exactly `key`, valid UTF-8 or not.
///
/// Postgres writes keys in the server encoding, so a `SQL_ASCII` database stores whatever bytes
/// it was handed. Equality has to hold for those, which a lossy decode would fold together.
#[must_use]
pub fn encode_raw_key_object(key: &[u8], value: bool) -> Vec<u8> {
    let mut data = Vec::new();
    let key_entry = encode_string(key, &mut data);
    let kind = if value {
        JENTRY_ISBOOL_TRUE
    } else {
        JENTRY_ISBOOL_FALSE
    };
    let entries = [
        stride_entry(key_entry, 0, key_entry & JENTRY_OFFLENMASK),
        stride_entry(entry(kind, 0), 1, key_entry & JENTRY_OFFLENMASK),
    ];
    assemble(JB_FOBJECT | 1, &entries, &data)
}

// The same, for a single-member object.
pub fn encode_numeric_text_object(key: &str, text: &str, form: NumericForm) -> Vec<u8> {
    let mut entries = Vec::new();
    let mut data = Vec::new();
    let key_entry = encode_string(key.as_bytes(), &mut data);
    entries.push(stride_entry(key_entry, 0, key_entry & JENTRY_OFFLENMASK));
    let pad = pad_to_int(&mut data);
    let numeric = encode_numeric(text, form);
    data.extend_from_slice(&numeric);
    let value_entry = entry(JENTRY_ISNUMERIC, pad + numeric.len() as u32);
    let total = (key_entry & JENTRY_OFFLENMASK) + (value_entry & JENTRY_OFFLENMASK);
    entries.push(stride_entry(value_entry, 1, total));
    assemble(JB_FOBJECT | 1, &entries, &data)
}

// The header flag bits sit at the opposite end of the byte on a big-endian build.
fn varlena_header_4b(size: u32) -> [u8; 4] {
    if cfg!(target_endian = "big") {
        size.to_ne_bytes()
    } else {
        (size << 2).to_ne_bytes()
    }
}

fn varlena_header_1b(size: u8) -> u8 {
    if cfg!(target_endian = "big") {
        0x80 | size
    } else {
        (size << 1) | 0x01
    }
}

/// `depth` nested single-element arrays around an empty one, built without recursing.
pub fn encode_nested_arrays(depth: usize) -> Vec<u8> {
    let mut bytes = assemble(JB_FARRAY, &[], &[]);
    for _ in 0..depth {
        let entry = stride_entry(
            entry(JENTRY_ISCONTAINER, bytes.len() as u32),
            0,
            bytes.len() as u32,
        );
        bytes = assemble(JB_FARRAY | 1, &[entry], &bytes);
    }
    bytes
}

/// An array holding each of `children` as a nested container.
pub fn encode_array_of(children: &[Vec<u8>]) -> Vec<u8> {
    let mut entries = Vec::new();
    let mut data = Vec::new();
    let mut total = 0_u32;
    for (index, child) in children.iter().enumerate() {
        let pad = pad_to_int(&mut data);
        data.extend_from_slice(child);
        let entry = entry(JENTRY_ISCONTAINER, pad + child.len() as u32);
        total += entry & JENTRY_OFFLENMASK;
        entries.push(stride_entry(entry, index, total));
    }
    assemble(JB_FARRAY | children.len() as u32, &entries, &data)
}

/// A top-level scalar holding one numeric written as decimal text.
pub fn encode_numeric_text_scalar(text: &str, form: NumericForm) -> Vec<u8> {
    let numeric = encode_numeric(text, form);
    let entry = stride_entry(
        entry(JENTRY_ISNUMERIC, numeric.len() as u32),
        0,
        numeric.len() as u32,
    );
    assemble(JB_FARRAY | JB_FSCALAR | 1, &[entry], &numeric)
}

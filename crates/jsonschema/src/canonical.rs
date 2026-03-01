use serde::{
    ser::{self, Serialize, SerializeMap, SerializeSeq},
    Serializer,
};
use serde_json::{
    ser::{CompactFormatter, Formatter},
    Number, Value,
};
use std::{cell::RefCell, io, mem};

const I64_UPPER_EXCLUSIVE_F64: f64 = 9_223_372_036_854_775_808.0;
const I64_LOWER_INCLUSIVE_F64: f64 = -9_223_372_036_854_775_808.0;
const U64_UPPER_EXCLUSIVE_F64: f64 = 18_446_744_073_709_551_616.0;
const RECURSION_LIMIT: u16 = 255;
const MAX_SCRATCH_POOL_SIZE: usize = 8;
const MAX_SCRATCH_CAPACITY: usize = 16_384;
#[cfg(feature = "arbitrary-precision")]
const SERDE_JSON_NUMBER_TOKEN: &str = "$serde_json::private::Number";

/// Canonical JSON serialization helpers for stable schema processing.
pub mod json {
    use super::{initial_output_capacity, CanonicalFormatter, CanonicalValue};
    use serde::Serialize;
    use serde_json::Value;
    use std::{cell::RefCell, fmt, io};

    /// Error returned by [`to_string`].
    #[derive(Debug)]
    pub struct Error {
        inner: serde_json::Error,
    }

    impl fmt::Display for Error {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.inner.fmt(f)
        }
    }

    impl std::error::Error for Error {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.inner)
        }
    }

    impl From<serde_json::Error> for Error {
        fn from(inner: serde_json::Error) -> Self {
            Error { inner }
        }
    }

    /// Serialize a JSON value into a deterministic canonical JSON string.
    ///
    /// This is intended to provide a stable representation for schema-oriented
    /// canonicalization/normalization workflows, including deduplication of
    /// equivalent JSON Schemas and downstream processing that relies on a
    /// single deterministic form.
    ///
    /// # Rules
    ///
    /// - Object keys are emitted in lexicographic order.
    /// - Integer-valued floats are emitted as integers (`1.0` becomes `1`).
    /// - Output is always compact (no extra whitespace).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use serde_json::json;
    ///
    /// let schema = json!({"b": 1, "a": {"y": 2, "x": 3}});
    /// let canonical = jsonschema::canonical::json::to_string(&schema).unwrap();
    /// assert_eq!(canonical, r#"{"a":{"x":3,"y":2},"b":1}"#);
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails, for example when the input exceeds
    /// the canonical serializer recursion limit.
    pub fn to_string(value: &Value) -> Result<String, Error> {
        let mut output = Vec::with_capacity(initial_output_capacity(value));
        let formatter = CanonicalFormatter {
            default: serde_json::ser::CompactFormatter,
        };
        let scratch_pool = RefCell::new(Vec::new());
        let mut serializer = serde_json::Serializer::with_formatter(&mut output, formatter);
        CanonicalValue::new(value, 0, &scratch_pool)
            .serialize(&mut serializer)
            .map_err(Error::from)?;
        String::from_utf8(output).map_err(|err| {
            serde_json::Error::io(io::Error::new(io::ErrorKind::InvalidData, err)).into()
        })
    }
}

#[inline]
fn initial_output_capacity(value: &Value) -> usize {
    const MIN_CAPACITY: usize = 16;
    const MAX_PREALLOC: usize = 1 << 20; // 1 MiB

    let estimated = match value {
        Value::Object(map) => map.len().saturating_mul(24).saturating_add(2),
        Value::Array(items) => items.len().saturating_mul(12).saturating_add(2),
        Value::String(s) => s.len().saturating_add(2),
        Value::Number(_) => 32,
        Value::Bool(_) => 8,
        Value::Null => 4,
    };

    estimated.clamp(MIN_CAPACITY, MAX_PREALLOC)
}

/// A formatter that emits integer-valued floats as integers.
#[derive(Default)]
struct CanonicalFormatter {
    default: CompactFormatter,
}

impl Formatter for CanonicalFormatter {
    #[inline]
    fn write_f64<W: io::Write + ?Sized>(&mut self, writer: &mut W, value: f64) -> io::Result<()> {
        if value.fract() == 0.0 {
            if (0.0..U64_UPPER_EXCLUSIVE_F64).contains(&value) {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let int = value as u64;
                return self.default.write_u64(writer, int);
            }
            if (I64_LOWER_INCLUSIVE_F64..I64_UPPER_EXCLUSIVE_F64).contains(&value) {
                #[allow(clippy::cast_possible_truncation)]
                let int = value as i64;
                return self.default.write_i64(writer, int);
            }
            let integer = format!("{value:.0}");
            return writer.write_all(integer.as_bytes());
        }

        self.default.write_f64(writer, value)
    }
}

struct CanonicalValue<'value> {
    value: &'value Value,
    recursion_depth: u16,
    scratch_pool: &'value RefCell<Vec<Vec<ObjectEntry<'value>>>>,
}

struct ObjectEntry<'value> {
    key: &'value str,
    value: &'value Value,
}

struct ObjectEntryScratch<'value, 'pool> {
    entries: Vec<ObjectEntry<'value>>,
    pool: &'pool RefCell<Vec<Vec<ObjectEntry<'value>>>>,
}

impl<'value, 'pool> ObjectEntryScratch<'value, 'pool> {
    fn with_capacity(pool: &'pool RefCell<Vec<Vec<ObjectEntry<'value>>>>, capacity: usize) -> Self {
        let mut entries = pool.borrow_mut().pop().unwrap_or_default();
        entries.clear();
        if entries.capacity() < capacity {
            entries.reserve(capacity - entries.capacity());
        }
        Self { entries, pool }
    }

    #[inline]
    fn entries_mut(&mut self) -> &mut Vec<ObjectEntry<'value>> {
        &mut self.entries
    }

    #[inline]
    fn entries(&self) -> &[ObjectEntry<'value>] {
        &self.entries
    }
}

impl Drop for ObjectEntryScratch<'_, '_> {
    fn drop(&mut self) {
        self.entries.clear();
        if self.entries.capacity() > MAX_SCRATCH_CAPACITY {
            return;
        }
        let mut pool = self.pool.borrow_mut();
        if pool.len() < MAX_SCRATCH_POOL_SIZE {
            pool.push(mem::take(&mut self.entries));
        }
    }
}

impl<'value> CanonicalValue<'value> {
    #[inline]
    const fn new(
        value: &'value Value,
        recursion_depth: u16,
        scratch_pool: &'value RefCell<Vec<Vec<ObjectEntry<'value>>>>,
    ) -> Self {
        CanonicalValue {
            value,
            recursion_depth,
            scratch_pool,
        }
    }
}

#[cfg(feature = "arbitrary-precision")]
struct BorrowedNumber<'a>(&'a str);

#[cfg(feature = "arbitrary-precision")]
impl Serialize for BorrowedNumber<'_> {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut serialized = serializer.serialize_struct(SERDE_JSON_NUMBER_TOKEN, 1)?;
        serialized.serialize_field(SERDE_JSON_NUMBER_TOKEN, self.0)?;
        serialized.end()
    }
}

#[cfg(feature = "arbitrary-precision")]
fn canonical_integral_number(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    let mut idx = 0;
    let negative = if bytes[idx] == b'-' {
        idx += 1;
        true
    } else {
        false
    };
    if idx == bytes.len() {
        return None;
    }

    let int_start = idx;
    if bytes[idx] == b'0' {
        idx += 1;
        if idx < bytes.len() && bytes[idx].is_ascii_digit() {
            return None;
        }
    } else if bytes[idx].is_ascii_digit() {
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
    } else {
        return None;
    }
    let int_end = idx;

    let mut frac_start = idx;
    let mut frac_end = idx;
    if idx < bytes.len() && bytes[idx] == b'.' {
        idx += 1;
        frac_start = idx;
        if idx == bytes.len() || !bytes[idx].is_ascii_digit() {
            return None;
        }
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
        frac_end = idx;
    }
    let has_fraction = frac_end > frac_start;

    let mut exponent: i64 = 0;
    let has_exponent = if idx < bytes.len() && matches!(bytes[idx], b'e' | b'E') {
        idx += 1;
        let mut exp_negative = false;
        if idx < bytes.len() && matches!(bytes[idx], b'+' | b'-') {
            exp_negative = bytes[idx] == b'-';
            idx += 1;
        }
        if idx == bytes.len() || !bytes[idx].is_ascii_digit() {
            return None;
        }
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            exponent = exponent
                .saturating_mul(10)
                .saturating_add(i64::from(bytes[idx] - b'0'));
            idx += 1;
        }
        if exp_negative {
            exponent = -exponent;
        }
        true
    } else {
        false
    };

    if idx != bytes.len() {
        return None;
    }

    if !has_fraction && !has_exponent {
        if negative && &raw[int_start..int_end] == "0" {
            return Some(String::from("0"));
        }
        return Some(raw.to_owned());
    }

    let int_digits = &raw[int_start..int_end];
    let frac_digits = &raw[frac_start..frac_end];
    let mut digits = Vec::with_capacity(int_digits.len() + frac_digits.len());
    digits.extend_from_slice(int_digits.as_bytes());
    digits.extend_from_slice(frac_digits.as_bytes());

    let frac_len = i64::try_from(frac_digits.len()).unwrap_or(i64::MAX);
    let shift = exponent.saturating_sub(frac_len);

    let kept_len = if shift >= 0 {
        let extra_zeros = usize::try_from(shift).ok()?;
        digits.resize(digits.len().saturating_add(extra_zeros), b'0');
        digits.len()
    } else {
        let drop_len = usize::try_from(shift.unsigned_abs()).ok()?;
        if drop_len > digits.len() {
            if digits.iter().all(|&byte| byte == b'0') {
                0
            } else {
                return None;
            }
        } else {
            let kept = digits.len() - drop_len;
            if digits[kept..].iter().all(|&byte| byte == b'0') {
                kept
            } else {
                return None;
            }
        }
    };

    let prefix = &digits[..kept_len];
    let first_non_zero = prefix.iter().position(|&byte| byte != b'0');
    let Some(first_non_zero) = first_non_zero else {
        return Some(String::from("0"));
    };

    let mut output = String::with_capacity(
        prefix
            .len()
            .saturating_sub(first_non_zero)
            .saturating_add(usize::from(negative)),
    );
    if negative {
        output.push('-');
    }
    for byte in &prefix[first_non_zero..] {
        output.push(char::from(*byte));
    }
    Some(output)
}

fn serialize_number<S>(number: &Number, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    #[cfg(feature = "arbitrary-precision")]
    {
        if let Some(integer) = canonical_integral_number(number.as_str()) {
            return serializer.serialize_some(&BorrowedNumber(&integer));
        }
    }
    number.serialize(serializer)
}

impl Serialize for CanonicalValue<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.value {
            Value::Null => serializer.serialize_unit(),
            Value::Bool(value) => serializer.serialize_bool(*value),
            Value::Number(number) => serialize_number(number, serializer),
            Value::String(value) => serializer.serialize_str(value),
            Value::Array(items) => {
                if self.recursion_depth == RECURSION_LIMIT {
                    return Err(ser::Error::custom("Recursion limit reached"));
                }
                let mut sequence = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    sequence.serialize_element(&CanonicalValue::new(
                        item,
                        self.recursion_depth + 1,
                        self.scratch_pool,
                    ))?;
                }
                sequence.end()
            }
            Value::Object(map) => {
                if self.recursion_depth == RECURSION_LIMIT {
                    return Err(ser::Error::custom("Recursion limit reached"));
                }
                let mut output = serializer.serialize_map(Some(map.len()))?;
                // Canonical output must be independent from serde_json's internal map backend.
                // Downstream crates can enable serde_json/preserve_order transitively, so we
                // always sort object keys here instead of depending on iteration order.
                let mut scratch = ObjectEntryScratch::with_capacity(self.scratch_pool, map.len());
                {
                    let entries = scratch.entries_mut();
                    for (key, value) in map {
                        entries.push(ObjectEntry {
                            key: key.as_str(),
                            value,
                        });
                    }
                    entries.sort_unstable_by(|left, right| {
                        left.key.as_bytes().cmp(right.key.as_bytes())
                    });
                }
                for entry in scratch.entries() {
                    output.serialize_entry(
                        entry.key,
                        &CanonicalValue::new(
                            entry.value,
                            self.recursion_depth + 1,
                            self.scratch_pool,
                        ),
                    )?;
                }
                output.end()
            }
        }
    }
}

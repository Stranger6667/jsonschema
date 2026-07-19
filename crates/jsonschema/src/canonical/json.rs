#[cfg(feature = "arbitrary-precision")]
use std::borrow::Cow;
use std::{cell::RefCell, fmt, io, mem};

#[cfg(feature = "arbitrary-precision")]
use serde::ser::SerializeStruct;
use serde::{
    ser::{self, Serialize, SerializeMap, SerializeSeq},
    Serializer,
};
use serde_json::{
    ser::{CompactFormatter, Formatter},
    Number, Value,
};

const I64_UPPER_EXCLUSIVE_F64: f64 = 9_223_372_036_854_775_808.0;
const I64_LOWER_INCLUSIVE_F64: f64 = -9_223_372_036_854_775_808.0;
const U64_UPPER_EXCLUSIVE_F64: f64 = 18_446_744_073_709_551_616.0;
const RECURSION_LIMIT: u16 = 255;
const MAX_SCRATCH_POOL_SIZE: usize = 8;
const MAX_SCRATCH_CAPACITY: usize = 16_384;
/// `serde_json`'s private token routing a raw digit string into an exact `Number`.
/// Not public API: shared with the bindings' (de)serializers.
#[cfg(feature = "arbitrary-precision")]
#[doc(hidden)]
pub const SERDE_JSON_NUMBER_TOKEN: &str = "$serde_json::private::Number";
#[cfg(feature = "arbitrary-precision")]
const MAX_EXPANDED_INTEGER_DIGITS: usize = 1 << 20;

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
/// Provides a stable representation for schema canonicalization: deduplicating equivalent JSON Schemas and downstream
/// processing that relies on a single deterministic form.
///
/// # Rules
///
/// - Object keys are emitted in lexicographic order.
/// - Integer-valued floats are emitted as integers (`1.0` becomes `1`).
/// - With `arbitrary-precision`, one normal form per value: plain integer/decimal while the expansion fits the digit
///   cap, normalized scientific (`d[.rest]e{E}`) past it. The IR relies on this for value text equality and type scanning.
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
/// Returns an error if serialization fails, e.g. when the input exceeds the recursion limit.
#[allow(
    clippy::missing_panics_doc,
    reason = "the canonical serializer emits valid UTF-8 by construction; the expect is unreachable"
)]
pub fn to_string(value: &Value) -> Result<String, Error> {
    let mut output = Vec::with_capacity(initial_output_capacity(value));
    let formatter = CanonicalFormatter {
        default: CompactFormatter,
    };
    let scratch_pool = RefCell::new(Vec::new());
    let mut serializer = serde_json::Serializer::with_formatter(&mut output, formatter);
    CanonicalValue::new(value, 0, &scratch_pool)
        .serialize(&mut serializer)
        .map_err(Error::from)?;
    Ok(String::from_utf8(output).expect("canonical serializer emits valid UTF-8"))
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

#[derive(Default)]
struct CanonicalFormatter {
    default: CompactFormatter,
}

/// A formatter that emits integer-valued floats as integers.
impl Formatter for CanonicalFormatter {
    #[inline]
    fn write_f64<W: io::Write + ?Sized>(&mut self, writer: &mut W, value: f64) -> io::Result<()> {
        if value.fract() == 0.0 {
            if (0.0..U64_UPPER_EXCLUSIVE_F64).contains(&value) {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "guarded by the `0.0..U64_UPPER_EXCLUSIVE_F64` range and zero fractional part"
                )]
                let integer = value as u64;
                return self.default.write_u64(writer, integer);
            }
            if (I64_LOWER_INCLUSIVE_F64..I64_UPPER_EXCLUSIVE_F64).contains(&value) {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "guarded by the `I64_LOWER_INCLUSIVE_F64..I64_UPPER_EXCLUSIVE_F64` range and zero fractional part"
                )]
                let integer = value as i64;
                return self.default.write_i64(writer, integer);
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

/// A valid JSON-number token that serializes as an exact `serde_json` number.
/// Not public API: bindings emit their normalized number texts through this so the
/// `serde_json` private-token contract lives in one place.
#[cfg(feature = "arbitrary-precision")]
#[doc(hidden)]
pub struct NumberToken<'a>(pub &'a str);

#[cfg(feature = "arbitrary-precision")]
impl Serialize for NumberToken<'_> {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut serialized = serializer.serialize_struct(SERDE_JSON_NUMBER_TOKEN, 1)?;
        serialized.serialize_field(SERDE_JSON_NUMBER_TOKEN, self.0)?;
        serialized.end()
    }
}

#[cfg(feature = "arbitrary-precision")]
fn push_digits(output: &mut String, digits: &[u8]) {
    output.push_str(std::str::from_utf8(digits).expect("ASCII digits"));
}

/// Whether the canonical spelling of a valid JSON-number token stays plain (non-scientific).
/// Past-cap values take the scientific normal form, which the runtime validator's exact numeric
/// comparisons decline, so documents carrying them as instance data must stay raw.
#[cfg(feature = "arbitrary-precision")]
#[doc(hidden)]
#[inline]
#[must_use]
pub(crate) fn number_spelling_stays_plain(raw: &str) -> bool {
    let canonical = canonical_number(raw);
    let text = canonical.as_deref().unwrap_or(raw);
    !text.bytes().any(|byte| matches!(byte, b'e' | b'E'))
}

/// Canonical text for a valid JSON-number token (borrowed when the token is already canonical);
/// `None` when the canonical spelling is unrepresentable (the decimal shift overflows `usize` or
/// the exponent does not parse) and the caller must keep the raw text.
/// Not public API: bindings normalize their native number texts through this so every surface
/// shares one normal form.
#[cfg(feature = "arbitrary-precision")]
#[doc(hidden)]
#[must_use]
pub fn canonical_number(raw: &str) -> Option<Cow<'_, str>> {
    // `raw` is always a valid JSON-number token (no leading `-0`), so the parse assumes that shape rather than re-validating it.
    let bytes = raw.as_bytes();

    let mut idx = 0;
    let negative = bytes[idx] == b'-';
    if negative {
        idx += 1;
    }

    let integer_start = idx;
    if bytes[idx] == b'0' {
        idx += 1;
    } else {
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
    }
    let integer_end = idx;

    let mut fraction_start = idx;
    let mut fraction_end = idx;
    if idx < bytes.len() && bytes[idx] == b'.' {
        idx += 1;
        fraction_start = idx;
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
        fraction_end = idx;
    }
    let has_fraction = fraction_end > fraction_start;

    let mut exponent: i64 = 0;
    let mut exponent_digits = "";
    let has_exponent = if idx < bytes.len() && matches!(bytes[idx], b'e' | b'E') {
        idx += 1;
        // Keep a leading `-` in the slice so `BigInt::parse_bytes` sees the sign; a `+` is dropped.
        let sign_start = idx;
        let mut exponent_negative = false;
        if idx < bytes.len() && matches!(bytes[idx], b'+' | b'-') {
            exponent_negative = bytes[idx] == b'-';
            idx += 1;
        }
        let digits_start = idx;
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            // Saturating: only steers the coarse branch below; the scientific form recomputes exactly.
            exponent = exponent
                .saturating_mul(10)
                .saturating_add(i64::from(bytes[idx] - b'0'));
            idx += 1;
        }
        exponent_digits = if exponent_negative {
            exponent = -exponent;
            &raw[sign_start..idx]
        } else {
            &raw[digits_start..idx]
        };
        true
    } else {
        false
    };

    let integer_within_cap = integer_end - integer_start <= MAX_EXPANDED_INTEGER_DIGITS;

    if !has_fraction && !has_exponent && integer_within_cap {
        if negative && integer_end - integer_start == 1 && bytes[integer_start] == b'0' {
            return Some(Cow::Borrowed("0"));
        }
        return Some(Cow::Borrowed(raw));
    }

    // Plain decimal with no exponent or trailing fraction zero is already canonical (grammar rejects leading integer zeros).
    if !has_exponent && integer_within_cap && bytes[fraction_end - 1] != b'0' {
        return Some(Cow::Borrowed(raw));
    }

    let integer_digits = &raw[integer_start..integer_end];
    let fraction_digits = &raw[fraction_start..fraction_end];
    let mut digits = Vec::with_capacity(integer_digits.len() + fraction_digits.len());
    digits.extend_from_slice(integer_digits.as_bytes());
    digits.extend_from_slice(fraction_digits.as_bytes());

    // Leading zeros (only from a `0.xxx` integer part) would skew the expansion-size cap below, splitting equal values across branches.
    let leading_zeros = digits.iter().take_while(|&&byte| byte == b'0').count();
    digits.drain(..leading_zeros);
    if digits.is_empty() {
        return Some(Cow::Borrowed("0"));
    }

    let parts = NumberParts {
        fraction_len: fraction_digits.len(),
        exponent_digits,
        negative,
    };

    let fraction_len = i64::try_from(parts.fraction_len).unwrap_or(i64::MAX);
    let shift = exponent.saturating_sub(fraction_len);

    let prefix_len = if shift >= 0 {
        let extra_zeros = usize::try_from(shift).ok()?;
        let expanded_len = digits.len().checked_add(extra_zeros)?;
        if expanded_len > MAX_EXPANDED_INTEGER_DIGITS {
            return canonical_scientific_number(&digits, &parts).map(Cow::Owned);
        }
        digits.resize(expanded_len, b'0');
        digits.len()
    } else {
        let drop_len = usize::try_from(shift.unsigned_abs()).ok()?;
        if drop_len > digits.len() {
            return canonical_fractional_number(&digits, drop_len, &parts).map(Cow::Owned);
        }
        let prefix_len = digits.len() - drop_len;
        if digits[prefix_len..].iter().all(|&byte| byte == b'0') {
            prefix_len
        } else {
            return canonical_fractional_number(&digits, drop_len, &parts).map(Cow::Owned);
        }
    };

    // `digits[0] != b'0'` after the strip above, and neither branch can yield an empty prefix.
    let prefix = &digits[..prefix_len];
    let mut output = String::with_capacity(prefix.len() + usize::from(negative));
    if negative {
        output.push('-');
    }
    push_digits(&mut output, prefix);
    Some(Cow::Owned(output))
}

/// Facets of a parsed number literal needed past the digit buffer: the exact exponent for the scientific
/// form (the `i64` working exponent saturates; the slice keeps a leading `-`) and the sign.
#[cfg(feature = "arbitrary-precision")]
struct NumberParts<'a> {
    fraction_len: usize,
    exponent_digits: &'a str,
    negative: bool,
}

/// Plain-decimal normal form for a non-integral value: `digits` (leading zeros pre-stripped) sit `point_offset` places right of the point.
///
/// Distinct spellings of one number must share one text (`CanonicalJson` equality is text equality). Past
/// `MAX_EXPANDED_INTEGER_DIGITS` the scientific form takes over; trailing zeros are stripped before the cap check so spellings never straddle it.
#[cfg(feature = "arbitrary-precision")]
fn canonical_fractional_number(
    digits: &[u8],
    point_offset: usize,
    parts: &NumberParts<'_>,
) -> Option<String> {
    let last_non_zero = digits.iter().rposition(|&byte| byte != b'0')?;
    // Stripped trailing zeros all sit right of the point: a non-zero digit exists within the last `point_offset` positions, so the offset stays positive.
    let point_offset = point_offset.checked_sub(digits.len() - 1 - last_non_zero)?;
    if point_offset > MAX_EXPANDED_INTEGER_DIGITS {
        return canonical_scientific_number(digits, parts);
    }
    let stripped = &digits[..=last_non_zero];
    let mut output = String::with_capacity(
        stripped
            .len()
            .max(point_offset)
            .saturating_add(2)
            .saturating_add(usize::from(parts.negative)),
    );
    if parts.negative {
        output.push('-');
    }
    if stripped.len() > point_offset {
        let integer_len = stripped.len() - point_offset;
        push_digits(&mut output, &stripped[..integer_len]);
        output.push('.');
        push_digits(&mut output, &stripped[integer_len..]);
    } else {
        output.push_str("0.");
        output.extend(std::iter::repeat_n('0', point_offset - stripped.len()));
        push_digits(&mut output, stripped);
    }
    Some(output)
}

/// Scientific normal form `d[.rest]e{E}` for values whose plain expansion exceeds `MAX_EXPANDED_INTEGER_DIGITS`.
///
/// `digits` (leading zeros pre-stripped) carry `digits x 10^(exponent - fraction_len)`, so `E = exponent + len(digits) -
/// fraction_len - 1`, computed exactly over the literal's exponent digits (the `i64` working exponent saturates).
#[cfg(feature = "arbitrary-precision")]
fn canonical_scientific_number(digits: &[u8], parts: &NumberParts<'_>) -> Option<String> {
    use num_bigint::BigInt;

    // Also reached from the plain-decimal re-dispatch past the expansion cap, which has no exponent; an empty
    // slice would make `parse_bytes` return `None` and drop the value to non-canonical raw text.
    let mut exponent = if parts.exponent_digits.is_empty() {
        BigInt::from(0)
    } else {
        BigInt::parse_bytes(parts.exponent_digits.as_bytes(), 10)?
    };
    exponent += BigInt::from(i64::try_from(digits.len()).ok()?);
    exponent -= BigInt::from(i64::try_from(parts.fraction_len).ok()?) + BigInt::from(1);

    let last_non_zero = digits.iter().rposition(|&byte| byte != b'0')?;
    let significand = &digits[..=last_non_zero];
    let exponent = exponent.to_string();
    let mut output = String::with_capacity(
        significand
            .len()
            .saturating_add(exponent.len())
            .saturating_add(3),
    );
    if parts.negative {
        output.push('-');
    }
    output.push(char::from(significand[0]));
    if significand.len() > 1 {
        output.push('.');
        push_digits(&mut output, &significand[1..]);
    }
    output.push('e');
    output.push_str(&exponent);
    Some(output)
}

// `Number::from_f64` formats to shortest-roundtrip text before this runs, so the double nearest
// `1e300` and exactly `1e300` collide here. The bindings convert native floats exactly and do not.
fn serialize_number<S>(number: &Number, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    #[cfg(feature = "arbitrary-precision")]
    {
        if let Some(text) = canonical_number(number.as_str()) {
            return serializer.serialize_some(&NumberToken(&text));
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
                // Always sort keys: downstream crates can enable `serde_json/preserve_order` transitively.
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

#[cfg(test)]
mod tests {
    use serde_json::{json, Map, Value};
    use test_case::test_case;

    use super::to_string;

    #[test]
    fn canonical_string_is_stable_for_equivalent_schemas() {
        let left: Value =
            serde_json::from_str(r#"{"b":1,"a":{"z":3,"x":1,"y":2},"c":[{"d":4,"b":2}]}"#).unwrap();
        let right: Value =
            serde_json::from_str(r#"{"c":[{"b":2,"d":4}],"a":{"y":2,"x":1,"z":3},"b":1}"#).unwrap();

        assert_eq!(to_string(&left).unwrap(), to_string(&right).unwrap());
    }

    #[test_case("null"; "null")]
    #[test_case("true"; "bool_true")]
    #[test_case("false"; "bool_false")]
    #[test_case(r#""hello""#; "simple")]
    #[test_case(r#""line\nbreak""#; "escaped_newline")]
    fn scalar_literals_roundtrip(raw: &str) {
        let value: Value = serde_json::from_str(raw).unwrap();
        assert_eq!(to_string(&value).unwrap(), raw);
    }

    #[test]
    fn canonical_output_is_idempotent() {
        let value: Value =
            serde_json::from_str(r#"{"z":{"b":1,"a":2},"a":[3,2,1],"f":1.0,"v":1.5}"#).unwrap();

        let first = to_string(&value).unwrap();
        let parsed: Value = serde_json::from_str(&first).unwrap();

        assert_eq!(to_string(&parsed).unwrap(), first);
    }

    fn nest_arrays(depth: usize) -> Value {
        let mut value = Value::Null;
        for _ in 0..depth {
            value = Value::Array(vec![value]);
        }
        value
    }

    fn nest_objects(depth: usize) -> Value {
        let mut value = Value::Null;
        for _ in 0..depth {
            value = Value::Object([("a".to_string(), value)].into_iter().collect());
        }
        value
    }

    #[test_case(&nest_arrays(300) ; "arrays")]
    #[test_case(&nest_objects(300) ; "objects")]
    fn recursion_limit_error_exposes_source_and_message(value: &Value) {
        let error = to_string(value).expect_err("should exceed the recursion limit");
        assert!(std::error::Error::source(&error).is_some());
        assert!(error.to_string().contains("Recursion"));
    }

    #[test]
    fn oversized_object_scratch_is_not_pooled() {
        // A scratch buffer whose capacity exceeds MAX_SCRATCH_CAPACITY is dropped instead of pooled.
        let mut map = Map::new();
        for index in 0..20_000 {
            map.insert(format!("k{index:05}"), json!(index));
        }
        let out = to_string(&Value::Object(map)).unwrap();
        assert!(out.starts_with(r#"{"k00000":0,"k00001":1,"#));
    }

    #[cfg(not(feature = "arbitrary-precision"))]
    #[test_case(&json!(1.0), "1" ; "small unsigned integral")]
    #[test_case(&json!(-5.0), "-5" ; "small signed integral")]
    #[test_case(&json!(1.5), "1.5" ; "non integral falls through")]
    fn write_f64_formats(value: &Value, expected: &str) {
        assert_eq!(to_string(value).unwrap(), expected);
    }

    #[cfg(not(feature = "arbitrary-precision"))]
    #[test_case(1e30 ; "above u64 max")]
    #[test_case(-1e30 ; "below i64 min")]
    fn large_integral_floats_have_no_exponent(value: f64) {
        let out = to_string(&json!(value)).unwrap();
        assert!(
            !out.contains('.') && !out.contains('e') && !out.contains('E'),
            "{out}"
        );
    }

    #[test]
    fn large_integer_valued_float_uses_integer_form() {
        let value: Value = serde_json::from_str("1e300").unwrap();

        #[cfg(feature = "arbitrary-precision")]
        let expected = {
            let mut output = String::with_capacity(301);
            output.push('1');
            output.push_str(&"0".repeat(300));
            output
        };
        #[cfg(not(feature = "arbitrary-precision"))]
        let expected = format!("{:.0}", 1e300_f64);

        assert_eq!(to_string(&value).unwrap(), expected);
    }

    #[cfg(feature = "arbitrary-precision")]
    fn canonical(raw: &str) -> String {
        to_string(&serde_json::from_str::<Value>(raw).unwrap()).unwrap()
    }

    // Plain expansion past the digit cap switches to scientific normal form.
    #[cfg(feature = "arbitrary-precision")]
    #[test_case("1e1048577", "1e1048577" ; "single digit significand")]
    #[test_case("12e1048576", "1.2e1048577" ; "multi digit significand")]
    #[test_case("-1e1048576", "-1e1048576" ; "negative significand")]
    // Huge negative exponent drops past the digit buffer into the fractional-then-scientific path.
    #[test_case("1e-1048577", "1e-1048577" ; "fractional past cap")]
    // All-zero fractions collapse to "0"; trailing-zero fractions expand back to a (signed) integer.
    #[test_case("0.0", "0" ; "all zero collapses")]
    #[test_case("-1.0", "-1" ; "negative integral expansion")]
    fn scientific_normal_form(raw: &str, expected: &str) {
        assert_eq!(canonical(raw), expected);
    }

    // Within-cap spellings share one plain-decimal normal form.
    #[cfg(feature = "arbitrary-precision")]
    #[test_case("1e+3", "1000" ; "positive exponent expands")]
    #[test_case("12e2", "1200" ; "multi digit positive exponent expands")]
    #[test_case("100E-2", "1" ; "negative exponent integral")]
    #[test_case("1e-2", "0.01" ; "negative exponent zero padded")]
    #[test_case("3.14e-3", "0.00314" ; "fraction with exponent zero padded")]
    #[test_case("3.1400e-3", "0.00314" ; "trailing zeros stripped before padding")]
    #[test_case("1.50", "1.5" ; "trailing fraction zero stripped")]
    #[test_case("0.15e1", "1.5" ; "point shifts right")]
    #[test_case("-5e-1", "-0.5" ; "negative fraction")]
    #[test_case("0e+10", "0" ; "zero with positive exponent")]
    #[test_case("-0E-1000", "0" ; "negative zero with huge negative exponent")]
    // No exponent, already canonical, non-zero trailing digit: takes the zero-copy borrow path.
    #[test_case("1.25", "1.25" ; "fractional decimal")]
    fn plain_decimal_normal_form(raw: &str, expected: &str) {
        assert_eq!(canonical(raw), expected);
    }
}

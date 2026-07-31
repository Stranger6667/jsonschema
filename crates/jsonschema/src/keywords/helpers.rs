#![allow(clippy::float_cmp, clippy::cast_sign_loss)]

use std::borrow::Cow;

use serde_json::{Map, Value};

use crate::{compiler, paths::Location, types::JsonType, Json, ValidationError};

/// A non-negative bound on a collection size, saturating at `u64::MAX`.
///
/// Sizes are `usize`-bounded, so a bound at or past `u64::MAX` rules out nothing that `u64::MAX`
/// already allows: `max*` still accepts every instance that can exist, `min*` still rejects every
/// one. Saturating keeps the runtime check a plain `u64` compare rather than a bignum.
///
/// `None` for anything that is not a non-negative integer under this draft; the caller turns that
/// into a compilation error.
pub(crate) fn size_limit<F: Json>(ctx: &compiler::Context<F>, schema: &Value) -> Option<u64> {
    if let Some(limit) = schema.as_u64() {
        return Some(limit);
    }
    if is_integer_past_u64(schema) {
        return Some(u64::MAX);
    }
    // Draft 4 spells a bound as an integer literal; later drafts also read integer-valued floats.
    if ctx.supports_integer_valued_numbers() {
        // Anything past `u64` was handled above, so the cast cannot lose the value.
        #[allow(clippy::cast_possible_truncation)]
        if let Some(limit) = schema
            .as_f64()
            .filter(|limit| *limit >= 0.0 && limit.trunc() == *limit)
        {
            return Some(limit as u64);
        }
    }
    None
}

/// Whether the value is an integer too large for `u64`.
#[cfg(feature = "arbitrary-precision")]
fn is_integer_past_u64(schema: &Value) -> bool {
    // The raw JSON text survives here, so a bare digit run is exactly an integer literal, and
    // `as_u64` already claimed every one that fits.
    match schema {
        Value::Number(number) => number.as_str().bytes().all(|byte| byte.is_ascii_digit()),
        _ => false,
    }
}

/// Whether the value is an integer too large for `u64`.
#[cfg(not(feature = "arbitrary-precision"))]
fn is_integer_past_u64(schema: &Value) -> bool {
    // Without the raw text such a value is an `f64`, and every `f64` at or past `2^64` is an exact
    // integer - no float in that range carries a fractional part - so the draft gate is moot.
    schema
        .as_f64()
        .is_some_and(|value| value >= crate::canonical::json::U64_UPPER_EXCLUSIVE_F64)
}

/// Extract a u64 value from a schema map, returning a compilation error if invalid.
///
/// This is a defensive check - normally caught by metaschema validation.
#[inline]
pub(crate) fn map_get_u64<'a, F: Json>(
    m: &'a Map<String, Value>,
    ctx: &compiler::Context<F>,
    keyword: &str,
) -> Option<Result<u64, ValidationError<'a>>> {
    let schema_value = m.get(keyword)?;
    Some(match size_limit(ctx, schema_value) {
        Some(limit) => Ok(limit),
        None => Err(fail_on_non_positive_integer(
            schema_value,
            ctx.location().join(keyword),
        )),
    })
}

/// Create a compilation error for schema values that must be non-negative integers.
///
/// This is a defensive check - normally caught by metaschema validation.
pub(crate) fn fail_on_non_positive_integer(
    schema_value: &Value,
    schema_path: Location,
) -> ValidationError<'_> {
    if schema_value.is_i64() {
        // Negative integer
        ValidationError::minimum(
            schema_path.clone(),
            schema_path,
            Location::new(),
            Cow::Borrowed(schema_value),
            0.into(),
        )
    } else {
        // Wrong type (string, object, etc.)
        ValidationError::single_type_error(
            schema_path.clone(),
            schema_path,
            Location::new(),
            Cow::Borrowed(schema_value),
            JsonType::Integer,
        )
    }
}

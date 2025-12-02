#![allow(clippy::float_cmp, clippy::cast_sign_loss)]

use serde_json::{Map, Value};

use crate::{compiler, paths::Location, types::JsonType, ValidationError};

/// Extract a u64 value from a schema map, returning a compilation error if invalid.
///
/// This is a defensive check - normally caught by metaschema validation.
#[inline]
pub(crate) fn map_get_u64<'a>(
    m: &'a Map<String, Value>,
    ctx: &compiler::Context,
    keyword: &str,
) -> Option<Result<u64, ValidationError<'a>>> {
    let schema_value = m.get(keyword)?;
    match schema_value.as_u64() {
        Some(n) => Some(Ok(n)),
        None if schema_value.is_i64() => {
            // Negative integer
            let schema_path = ctx.location().join(keyword);
            Some(Err(ValidationError::minimum(
                schema_path.clone(),
                schema_path,
                Location::new(),
                schema_value,
                0.into(),
            )))
        }
        None => {
            if let Some(f) = schema_value.as_f64() {
                if f.trunc() == f {
                    // NOTE: Imprecise cast as big integers are not supported yet
                    #[allow(clippy::cast_possible_truncation)]
                    return Some(Ok(f as u64));
                }
            }
            // Wrong type (string, object, float, etc.)
            let schema_path = ctx.location().join(keyword);
            Some(Err(ValidationError::single_type_error(
                schema_path.clone(),
                schema_path,
                Location::new(),
                schema_value,
                JsonType::Integer,
            )))
        }
    }
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
            schema_value,
            0.into(),
        )
    } else {
        // Wrong type (string, object, etc.)
        ValidationError::single_type_error(
            schema_path.clone(),
            schema_path,
            Location::new(),
            schema_value,
            JsonType::Integer,
        )
    }
}

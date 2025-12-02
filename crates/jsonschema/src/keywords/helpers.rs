#![allow(clippy::float_cmp, clippy::cast_sign_loss)]

use serde_json::{Map, Value};

use crate::{compiler, paths::Location, types::JsonType, ValidationError};

#[inline]
pub(crate) fn map_get_u64<'a>(
    m: &'a Map<String, Value>,
    ctx: &compiler::Context,
    type_name: &str,
) -> Option<Result<u64, ValidationError<'a>>> {
    let value = m.get(type_name)?;
    match value.as_u64() {
        Some(n) => Some(Ok(n)),
        None if value.is_i64() => {
            let location = ctx.location().join(type_name);
            Some(Err(ValidationError::minimum(
                location.clone(),
                location,
                Location::new(),
                value,
                0.into(),
            )))
        }
        None => {
            if let Some(value) = value.as_f64() {
                if value.trunc() == value {
                    // NOTE: Imprecise cast as big integers are not supported yet
                    #[allow(clippy::cast_possible_truncation)]
                    return Some(Ok(value as u64));
                }
            }
            let location = ctx.location().join(type_name);
            Some(Err(ValidationError::single_type_error(
                location.clone(),
                location,
                Location::new(),
                value,
                JsonType::Integer,
            )))
        }
    }
}

/// Fail if the input value is not `u64`.
pub(crate) fn fail_on_non_positive_integer(
    value: &Value,
    instance_path: Location,
) -> ValidationError<'_> {
    if value.is_i64() {
        ValidationError::minimum(
            instance_path.clone(),
            instance_path,
            Location::new(),
            value,
            0.into(),
        )
    } else {
        ValidationError::single_type_error(
            instance_path.clone(),
            instance_path,
            Location::new(),
            value,
            JsonType::Integer,
        )
    }
}

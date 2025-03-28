use num_cmp::NumCmp;
use serde_json::{Map, Value};

use crate::{compiler, paths::Location, types::JsonType, ValidationError};

macro_rules! num_cmp {
    ($left:expr, $right:expr) => {
        if let Some(b) = $right.as_u64() {
            NumCmp::num_eq($left, b)
        } else if let Some(b) = $right.as_i64() {
            NumCmp::num_eq($left, b)
        } else {
            NumCmp::num_eq($left, $right.as_f64().expect("Always valid"))
        }
    };
}

#[inline]
pub(crate) fn equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::String(left), Value::String(right)) => left == right,
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::Null, Value::Null) => true,
        (Value::Number(left), Value::Number(right)) => {
            if let Some(a) = left.as_u64() {
                num_cmp!(a, right)
            } else if let Some(a) = left.as_i64() {
                num_cmp!(a, right)
            } else {
                let a = left.as_f64().expect("Always valid");
                num_cmp!(a, right)
            }
        }
        (Value::Array(left), Value::Array(right)) => equal_arrays(left, right),
        (Value::Object(left), Value::Object(right)) => equal_objects(left, right),
        (_, _) => false,
    }
}

#[inline]
pub(crate) fn equal_arrays(left: &[Value], right: &[Value]) -> bool {
    left.len() == right.len() && {
        let mut idx = 0_usize;
        while idx < left.len() {
            if !equal(&left[idx], &right[idx]) {
                return false;
            }
            idx += 1;
        }
        true
    }
}

#[inline]
pub(crate) fn equal_objects(left: &Map<String, Value>, right: &Map<String, Value>) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|((ka, va), (kb, vb))| ka == kb && equal(va, vb))
}

#[inline]
pub(crate) fn map_get_u64<'a>(
    m: &'a Map<String, Value>,
    ctx: &compiler::Context,
    type_name: &str,
) -> Option<Result<u64, ValidationError<'a>>> {
    let value = m.get(type_name)?;
    match value.as_u64() {
        Some(n) => Some(Ok(n)),
        None if value.is_i64() => Some(Err(ValidationError::minimum(
            Location::new(),
            ctx.location().clone(),
            value,
            0.into(),
        ))),
        None => {
            if let Some(value) = value.as_f64() {
                if value.trunc() == value {
                    // NOTE: Imprecise cast as big integers are not supported yet
                    #[allow(clippy::cast_possible_truncation)]
                    return Some(Ok(value as u64));
                }
            }
            Some(Err(ValidationError::single_type_error(
                Location::new(),
                ctx.location().clone(),
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
        ValidationError::minimum(Location::new(), instance_path, value, 0.into())
    } else {
        ValidationError::single_type_error(Location::new(), instance_path, value, JsonType::Integer)
    }
}

#[cfg(test)]
mod tests {
    use super::equal;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&json!(1), &json!(1.0))]
    #[test_case(&json!([2]), &json!([2.0]))]
    #[test_case(&json!([-3]), &json!([-3.0]))]
    #[test_case(&json!({"a": 1}), &json!({"a": 1.0}))]
    fn are_equal(left: &Value, right: &Value) {
        assert!(equal(left, right))
    }

    #[test_case(&json!(1), &json!(2.0))]
    #[test_case(&json!([]), &json!(["foo"]))]
    #[test_case(&json!([-3]), &json!([-4.0]))]
    #[test_case(&json!({"a": 1}), &json!({"a": 1.0, "b": 2}))]
    fn are_not_equal(left: &Value, right: &Value) {
        assert!(!equal(left, right))
    }
}

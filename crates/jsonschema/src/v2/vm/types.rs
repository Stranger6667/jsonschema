use serde_json::Value;

#[inline(always)]
pub(crate) fn is_integer(value: &Value) -> bool {
    if let Value::Number(n) = value {
        n.is_i64() || n.is_u64()
    } else {
        false
    }
}

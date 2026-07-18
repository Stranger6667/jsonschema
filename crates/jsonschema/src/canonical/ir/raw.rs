use std::{
    cmp::Ordering,
    hash::{Hash, Hasher},
    sync::Arc,
};

use serde_json::{Number, Value};

/// Verbatim schema document with document-identity Eq/Ord/Hash: `1` and `1.0` are distinct, unlike
/// JSON value equality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawJson(Arc<Value>);

impl RawJson {
    #[must_use]
    pub(crate) fn new(value: Value) -> Self {
        Self(Arc::new(value))
    }

    #[must_use]
    pub(crate) fn get(&self) -> &Value {
        &self.0
    }
}

impl Hash for RawJson {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_value(&self.0, state);
    }
}

impl PartialOrd for RawJson {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RawJson {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_values(&self.0, &other.0)
    }
}

fn hash_value<H: Hasher>(value: &Value, state: &mut H) {
    match value {
        Value::Null => state.write_u8(0),
        Value::Bool(item) => {
            state.write_u8(1);
            item.hash(state);
        }
        Value::Number(item) => {
            state.write_u8(2);
            state.write_u8(number_tag(item));
            if let Some(number) = item.as_u64() {
                number.hash(state);
            } else if let Some(number) = item.as_i64() {
                number.hash(state);
            } else if let Some(number) = item.as_f64() {
                number.to_bits().hash(state);
            } else {
                item.to_string().hash(state);
            }
        }
        Value::String(item) => {
            state.write_u8(3);
            item.hash(state);
        }
        Value::Array(items) => {
            state.write_u8(4);
            state.write_usize(items.len());
            for item in items {
                hash_value(item, state);
            }
        }
        Value::Object(items) => {
            state.write_u8(5);
            state.write_usize(items.len());
            for (key, item) in items {
                key.hash(state);
                hash_value(item, state);
            }
        }
    }
}

fn compare_values(left: &Value, right: &Value) -> Ordering {
    fn rank(value: &Value) -> u8 {
        match value {
            Value::Null => 0,
            Value::Bool(_) => 1,
            Value::Number(_) => 2,
            Value::String(_) => 3,
            Value::Array(_) => 4,
            Value::Object(_) => 5,
        }
    }
    match (left, right) {
        (Value::Bool(left), Value::Bool(right)) => left.cmp(right),
        (Value::Number(left), Value::Number(right)) => compare_numbers(left, right),
        (Value::String(left), Value::String(right)) => left.cmp(right),
        (Value::Array(left), Value::Array(right)) => {
            for (left, right) in left.iter().zip(right) {
                let ordering = compare_values(left, right);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            left.len().cmp(&right.len())
        }
        (Value::Object(left), Value::Object(right)) => {
            for ((left_key, left_value), (right_key, right_value)) in left.iter().zip(right) {
                let ordering = left_key
                    .cmp(right_key)
                    .then_with(|| compare_values(left_value, right_value));
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            left.len().cmp(&right.len())
        }
        (left, right) => rank(left).cmp(&rank(right)),
    }
}

fn number_tag(number: &Number) -> u8 {
    if number.as_u64().is_some() {
        0
    } else if number.as_i64().is_some() {
        1
    } else if number.as_f64().is_some() {
        2
    } else {
        // Under `arbitrary-precision`, numerals outside the f64 range.
        3
    }
}

// The f64-bits tie-break on exact text keeps `Ord` consistent with `Eq` under `arbitrary-precision`,
// where distinct numerals ("1.0" / "1.00") share one f64 approximation but compare unequal.
fn compare_numbers(left: &Number, right: &Number) -> Ordering {
    match (number_tag(left), number_tag(right)) {
        (0, 0) => left.as_u64().cmp(&right.as_u64()),
        (1, 1) => left.as_i64().cmp(&right.as_i64()),
        (2, 2) => left
            .as_f64()
            .map(f64::to_bits)
            .cmp(&right.as_f64().map(f64::to_bits))
            .then_with(|| left.to_string().cmp(&right.to_string())),
        (3, 3) => left.to_string().cmp(&right.to_string()),
        (left, right) => left.cmp(&right),
    }
}

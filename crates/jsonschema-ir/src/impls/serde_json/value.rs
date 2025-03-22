use serde_json::Value;

use crate::{value::Number, JsonValue};

impl From<Value> for JsonValue {
    fn from(value: Value) -> Self {
        match value {
            Value::Null => JsonValue::Null,
            Value::Bool(b) => JsonValue::Bool(b),
            Value::Number(num) => {
                if let Some(u) = num.as_u64() {
                    JsonValue::Number(Number::PositiveInteger(u))
                } else if let Some(i) = num.as_i64() {
                    JsonValue::Number(Number::NegativeInteger(i))
                } else if let Some(f) = num.as_f64() {
                    JsonValue::Number(Number::Float(f))
                } else {
                    panic!("Invalid number encountered in Value")
                }
            }
            Value::String(s) => JsonValue::String(s.into()),
            Value::Array(old) => {
                let new: Vec<JsonValue> = old.into_iter().map(JsonValue::from).collect();
                JsonValue::Array(new.into_boxed_slice())
            }
            Value::Object(old) => {
                let mut entries: Vec<(Box<str>, JsonValue)> = old
                    .into_iter()
                    .map(|(k, v)| (k.into(), JsonValue::from(v)))
                    .collect();
                entries.sort_by(|(k1, _), (k2, _)| k1.as_ref().cmp(k2.as_ref()));
                JsonValue::Object(entries.into_boxed_slice())
            }
        }
    }
}

impl PartialEq<Value> for JsonValue {
    fn eq(&self, other: &Value) -> bool {
        eq(other, self)
    }
}

impl PartialEq<JsonValue> for Value {
    fn eq(&self, other: &JsonValue) -> bool {
        eq(self, other)
    }
}

fn eq(lhs: &Value, rhs: &JsonValue) -> bool {
    match (lhs, rhs) {
        (Value::Null, JsonValue::Null) => true,
        (Value::Bool(l), JsonValue::Bool(r)) => l == r,
        (Value::Number(l), JsonValue::Number(r)) => compare_number(l, r),
        (Value::String(l), JsonValue::String(r)) => l.as_bytes() == r.as_bytes(),
        (Value::Array(l), JsonValue::Array(r)) => {
            if l.len() != r.len() {
                return false;
            }
            for (l, r) in l.iter().zip(r.iter()) {
                if !eq(l, r) {
                    return false;
                }
            }
            true
        }
        (Value::Object(l), JsonValue::Object(r)) => {
            if l.len() != r.len() {
                return false;
            }

            // NOTE: Map from `serde_json` is expected to be `BTreeMap` as this comparison depends
            // on iteration order.
            for ((lk, lv), (rk, rv)) in l.iter().zip(r.iter()) {
                if lk.as_bytes() == rk.as_bytes() && eq(lv, rv) {
                    continue;
                }
                return false;
            }
            true
        }
        _ => false,
    }
}

#[inline]
fn compare_number(lhs: &serde_json::Number, rhs: &Number) -> bool {
    match rhs {
        Number::PositiveInteger(u) => lhs.as_u64().map_or(false, |v| v == *u),
        Number::NegativeInteger(i) => lhs.as_i64().map_or(false, |v| v == *i),
        Number::Float(f) => lhs.as_f64().map_or(false, |v| v == *f),
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use serde_json::json;
    use test_case::test_case;

    #[test_case(json!(null), JsonValue::Null; "null")]
    #[test_case(json!(true), JsonValue::Bool(true); "bool")]
    #[test_case(json!(42u64), JsonValue::Number(Number::PositiveInteger(42)); "positive number")]
    #[test_case(json!(-42), JsonValue::Number(Number::NegativeInteger(-42)); "negative number")]
    #[test_case(json!(3.14), JsonValue::Number(Number::Float(3.14)); "float number")]
    #[test_case(
        json!("hello"),
        JsonValue::String("hello".into());
        "string"
    )]
    #[test_case(
        json!([1, 2, 3]),
        JsonValue::Array(Box::new([
            JsonValue::Number(Number::PositiveInteger(1)),
            JsonValue::Number(Number::PositiveInteger(2)),
            JsonValue::Number(Number::PositiveInteger(3)),
        ]));
        "array"
    )]
    #[test_case(
        json!({
            "a": 1,
            "b": "test",
            "c": true
        }),
        JsonValue::Object(vec![
            ("a".into(), JsonValue::Number(Number::PositiveInteger(1))),
            ("b".into(), JsonValue::String("test".into())),
            ("c".into(), JsonValue::Bool(true))
        ].into());
        "object"
    )]
    fn test_json_conversion(value: serde_json::Value, expected: JsonValue) {
        assert_eq!(JsonValue::from(value), expected);
    }

    #[test_case(json!(null), JsonValue::Null; "null equals")]
    #[test_case(json!(true), JsonValue::Bool(true); "bool equals")]
    #[test_case(json!(42), JsonValue::Number(Number::PositiveInteger(42)); "positive number equals")]
    #[test_case(json!(-42), JsonValue::Number(Number::NegativeInteger(-42)); "negative number equals")]
    #[test_case(json!(3.14), JsonValue::Number(Number::Float(3.14)); "float number equals")]
    #[test_case(
        json!("hello"),
        JsonValue::String("hello".into());
        "string equals"
    )]
    #[test_case(
        json!([1, 2, 3]),
        JsonValue::Array(Box::new([
            JsonValue::Number(Number::PositiveInteger(1)),
            JsonValue::Number(Number::PositiveInteger(2)),
            JsonValue::Number(Number::PositiveInteger(3)),
        ]));
        "array equals"
    )]
    #[test_case(
        json!({
            "b": "test",
            "a": 1,
            "c": true
        }),
        JsonValue::Object(vec![
            ("a".into(), JsonValue::Number(Number::PositiveInteger(1))),
            ("b".into(), JsonValue::String("test".into())),
            ("c".into(), JsonValue::Bool(true))
        ].into());
        "object equals"
    )]
    fn test_comparison_eq(serde_value: serde_json::Value, custom: JsonValue) {
        assert_eq!(serde_value, custom);
        assert_eq!(custom, serde_value);
    }

    #[test_case(json!(null), JsonValue::Bool(true); "null != bool")]
    #[test_case(json!(true), JsonValue::Bool(false); "bool not equal")]
    #[test_case(json!(42), JsonValue::Number(Number::NegativeInteger(-42)); "positive vs negative number not equal")]
    #[test_case(json!(3.14), JsonValue::Number(Number::Float(2.71)); "different floats not equal")]
    #[test_case(json!("hello"), JsonValue::String("world".into()); "different strings not equal")]
    #[test_case(
        json!([1, 2, 3]),
        JsonValue::Array(Box::new([
            JsonValue::Number(Number::PositiveInteger(1)),
            JsonValue::Number(Number::PositiveInteger(2)),
            JsonValue::Number(Number::PositiveInteger(4))
        ]));
        "different arrays not equal"
    )]
    #[test_case(
        json!({"a": 1}),
        JsonValue::Object(vec![
            ("a".into(), JsonValue::Number(Number::PositiveInteger(2)))
        ].into());
        "different object not equal"
    )]
    fn test_comparison_neq(serde_value: serde_json::Value, custom: JsonValue) {
        assert_ne!(serde_value, custom);
        assert_ne!(custom, serde_value);
    }
}

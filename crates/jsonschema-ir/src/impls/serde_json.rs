use serde_json::Value;
use strumbra::UniqueString;

use crate::{blocks::Block, value::Number, BlockId, IntoJsonSchema, JsonValue, ParseError, Schema};

impl<'a> IntoJsonSchema for &'a Value {
    fn parse(&self) -> Result<Schema, ParseError> {
        match self {
            Value::Bool(true) => todo!(),
            Value::Bool(false) => todo!(),
            Value::Object(map) => todo!(),
            _ => Err(ParseError::Invalid),
        }
    }
}

struct ParserContext {
    blocks: u32,
}

impl ParserContext {
    fn new() -> Self {
        Self { blocks: 0 }
    }
    fn new_block(&mut self) -> Block {
        let current = self.blocks;
        self.blocks += 1;
        let id = BlockId::new(current);
        Block::new(id)
    }
}

fn parse_impl(value: &Value) {}

impl From<Value> for JsonValue {
    fn from(value: Value) -> Self {
        match value {
            Value::Null => JsonValue::Null,
            Value::Bool(b) => JsonValue::Bool(b),
            Value::Number(num) => {
                if let Some(u) = num.as_u64() {
                    JsonValue::Number(Number::Positive(u))
                } else if let Some(i) = num.as_i64() {
                    JsonValue::Number(Number::Negative(i))
                } else if let Some(f) = num.as_f64() {
                    JsonValue::Number(Number::Float(f))
                } else {
                    panic!("Invalid number encountered in Value")
                }
            }
            Value::String(s) => JsonValue::String(s.try_into().expect("String is too long")),
            Value::Array(old) => {
                let new: Vec<JsonValue> = old.into_iter().map(JsonValue::from).collect();
                JsonValue::Array(new.into_boxed_slice())
            }
            Value::Object(old) => {
                let new = old
                    .into_iter()
                    .map(|(k, v)| {
                        (
                            k.try_into().expect("String is too long"),
                            JsonValue::from(v),
                        )
                    })
                    .collect();
                JsonValue::Object(new)
            }
        }
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
            for (key, lv) in l.iter() {
                match r.get(key.as_str()) {
                    Some(rv) if eq(lv, rv) => continue,
                    _ => return false,
                }
            }
            true
        }
        _ => false,
    }
}

fn compare_number(lhs: &serde_json::Number, rhs: &Number) -> bool {
    match rhs {
        Number::Positive(u) => lhs.as_u64().map_or(false, |v| v == *u),
        Number::Negative(i) => lhs.as_i64().map_or(false, |v| v == *i),
        Number::Float(f) => lhs.as_f64().map_or(false, |v| v == *f),
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use ahash::AHashMap;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(json!(true))]
    fn basic(input: Value) {
        let schema = crate::parse(&input).unwrap();
    }

    // TODO: Use test_case

    #[test]
    fn test_null() {
        let serde_value = serde_json::Value::Null;
        let custom_value: JsonValue = serde_value.into();
        assert_eq!(custom_value, JsonValue::Null);
    }

    #[test]
    fn test_bool() {
        let serde_value = json!(true);
        let custom_value: JsonValue = serde_value.into();
        assert_eq!(custom_value, JsonValue::Bool(true));
    }

    #[test]
    fn test_positive_number() {
        let serde_value = json!(42u64);
        let custom_value: JsonValue = serde_value.into();
        assert_eq!(custom_value, JsonValue::Number(Number::Positive(42)));
    }

    #[test]
    fn test_negative_number() {
        let serde_value = json!(-42);
        let custom_value: JsonValue = serde_value.into();
        assert_eq!(custom_value, JsonValue::Number(Number::Negative(-42)));
    }

    #[test]
    fn test_float_number() {
        let serde_value = json!(3.14);
        let custom_value: JsonValue = serde_value.into();
        assert_eq!(custom_value, JsonValue::Number(Number::Float(3.14)));
    }

    #[test]
    fn test_string() {
        let serde_value = json!("hello");
        let custom_value: JsonValue = serde_value.into();
        let expected = JsonValue::String("hello".try_into().unwrap());
        assert_eq!(custom_value, expected);
    }

    #[test]
    fn test_array() {
        let serde_value = json!([1, 2, 3]);
        let custom_value: JsonValue = serde_value.into();
        let expected = JsonValue::Array(Box::new([
            JsonValue::Number(Number::Positive(1)),
            JsonValue::Number(Number::Positive(2)),
            JsonValue::Number(Number::Positive(3)),
        ]));
        assert_eq!(custom_value, expected);
    }

    #[test]
    fn test_object() {
        let serde_value = json!({
            "a": 1,
            "b": "test",
            "c": true
        });
        let custom_value: JsonValue = serde_value.into();

        let mut expected_map = BTreeMap::new();
        expected_map.insert(
            "a".try_into().unwrap(),
            JsonValue::Number(Number::Positive(1)),
        );
        expected_map.insert(
            "b".try_into().unwrap(),
            JsonValue::String("test".try_into().unwrap()),
        );
        expected_map.insert("c".try_into().unwrap(), JsonValue::Bool(true));
        let expected = JsonValue::Object(expected_map);

        assert_eq!(custom_value, expected);
    }
}

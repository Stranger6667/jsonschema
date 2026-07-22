//! `serde_json::Value` representation: borrow-only accessors that monomorphize to direct `&Value` code.

use std::borrow::Cow;

use serde_json::{Map, Value};

use crate::{cmp, types::JsonType};

use super::{Array, Json, JsonNumber, Node, NodeIdentity, Object};

pub struct SerdeJson;

impl Json for SerdeJson {
    type Node<'a> = &'a Value;
    type PreparedKey = String;
    type StringBuffer = Value;

    fn prepare_key(key: &str) -> String {
        key.to_owned()
    }

    fn with_string_node<T>(buffer: &mut Value, string: &str, f: impl FnOnce(&Value) -> T) -> T {
        // Reuses the buffer's allocation across calls instead of building a fresh `String` per name.
        if let Value::String(existing) = buffer {
            existing.clear();
            existing.push_str(string);
        } else {
            *buffer = Value::String(string.to_owned());
        }
        f(buffer)
    }
}

impl JsonNumber for serde_json::Number {
    fn as_u64(&self) -> Option<u64> {
        serde_json::Number::as_u64(self)
    }
    fn as_i64(&self) -> Option<i64> {
        serde_json::Number::as_i64(self)
    }
    fn as_f64(&self) -> Option<f64> {
        serde_json::Number::as_f64(self)
    }
    fn as_str(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }
    fn to_number(&self) -> Cow<'_, serde_json::Number> {
        Cow::Borrowed(self)
    }
}

impl JsonNumber for &serde_json::Number {
    fn as_u64(&self) -> Option<u64> {
        serde_json::Number::as_u64(self)
    }
    fn as_i64(&self) -> Option<i64> {
        serde_json::Number::as_i64(self)
    }
    fn as_f64(&self) -> Option<f64> {
        serde_json::Number::as_f64(self)
    }
    fn as_str(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }
    fn to_number(&self) -> Cow<'_, serde_json::Number> {
        Cow::Borrowed(self)
    }
}

impl<'a> Node<'a, SerdeJson> for &'a Value {
    type Object = &'a Map<String, Value>;
    type Array = &'a [Value];
    type Number = &'a serde_json::Number;

    fn as_object(&self) -> Option<&'a Map<String, Value>> {
        match self {
            Value::Object(members) => Some(members),
            _ => None,
        }
    }

    fn as_array(&self) -> Option<&'a [Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    fn as_string(&self) -> Option<Cow<'a, str>> {
        match self {
            Value::String(string) => Some(Cow::Borrowed(string)),
            _ => None,
        }
    }

    fn as_number(&self) -> Option<&'a serde_json::Number> {
        match self {
            Value::Number(number) => Some(number),
            _ => None,
        }
    }

    fn as_boolean(&self) -> Option<bool> {
        match self {
            Value::Bool(boolean) => Some(*boolean),
            _ => None,
        }
    }

    fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    fn json_type(&self) -> JsonType {
        match self {
            Value::Null => JsonType::Null,
            Value::Bool(_) => JsonType::Boolean,
            Value::Number(_) => JsonType::Number,
            Value::String(_) => JsonType::String,
            Value::Array(_) => JsonType::Array,
            Value::Object(_) => JsonType::Object,
        }
    }

    fn string_length(&self) -> Option<u64> {
        match self {
            // SIMD-accelerated counting; the default `chars().count()` is measurably slower.
            Value::String(string) => Some(bytecount::num_chars(string.as_bytes()) as u64),
            _ => None,
        }
    }

    fn equals_value(&self, expected: &Value) -> bool {
        cmp::equal(self, expected)
    }

    fn to_value(&self) -> Cow<'a, Value> {
        Cow::Borrowed(self)
    }

    fn identity(&self) -> Option<NodeIdentity> {
        Some(NodeIdentity::new(std::ptr::from_ref::<Value>(self) as usize))
    }
}

pub struct SerdeMembersIter<'a>(serde_json::map::Iter<'a>);

impl<'a> Iterator for SerdeMembersIter<'a> {
    type Item = (&'a str, &'a Value);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(key, value)| (key.as_str(), value))
    }
}

impl<'a> Object<'a, SerdeJson> for &'a Map<String, Value> {
    type Node = &'a Value;
    type MemberName = &'a str;
    type MembersIter = SerdeMembersIter<'a>;

    fn len(&self) -> usize {
        Map::len(self)
    }

    fn get(&self, key: &String) -> Option<&'a Value> {
        (*self).get(key.as_str())
    }

    fn members(&self) -> SerdeMembersIter<'a> {
        SerdeMembersIter((*self).iter())
    }
}

impl<'a> Array<'a, SerdeJson> for &'a [Value] {
    type Node = &'a Value;
    type ElementsIter = std::slice::Iter<'a, Value>;

    fn len(&self) -> usize {
        <[Value]>::len(self)
    }

    fn elements(&self) -> std::slice::Iter<'a, Value> {
        (*self).iter()
    }

    fn is_unique(&self) -> bool {
        crate::unique::is_unique(self)
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use serde_json::{json, Value};
    use test_case::test_case;

    use super::{
        super::{Array, Json, JsonNumber, Node, Object},
        SerdeJson,
    };
    use crate::types::JsonType;

    // Generic on purpose: inherent `Value` methods shadow the trait on concrete `&Value`, and keyword code
    // only ever sees `F::Node<'_>`.
    fn assert_document_accessors<F: Json>(node: &F::Node<'_>) {
        let object = node.as_object().expect("object");

        let member = |name: &str| object.get(&F::prepare_key(name)).expect("present");

        assert!(member("object").as_object().is_some());
        assert_eq!(member("array").as_array().expect("array").len(), 3);
        assert_eq!(member("string").as_string().as_deref(), Some("héllo"));
        assert_eq!(member("string").json_type(), JsonType::String);
        assert_eq!(member("string").string_length(), Some(5));
        assert_eq!(
            member("integer").as_number().expect("number").as_u64(),
            Some(42)
        );
        assert_eq!(
            member("float").as_number().expect("number").as_f64(),
            Some(1.5)
        );
        assert_eq!(member("boolean").as_boolean(), Some(true));
        assert!(member("null").is_null());
        assert_eq!(member("null").json_type(), JsonType::Null);
        assert!(object.get(&F::prepare_key("missing")).is_none());
    }

    #[test]
    fn accessors_match_value_kinds() {
        let document = json!({
            "object": {"a": 1},
            "array": [1, 2, 3],
            "string": "héllo",
            "integer": 42,
            "float": 1.5,
            "boolean": true,
            "null": null
        });
        assert_document_accessors::<SerdeJson>(&&document);
    }

    #[test_case(&json!(1), &json!(1.0), true; "integer equals float")]
    #[test_case(&json!(1.0), &json!(1), true; "float equals integer")]
    #[test_case(&json!(1), &json!(2), false; "different integers")]
    #[test_case(&json!(true), &json!(1), false; "boolean is not a number")]
    #[test_case(&json!({"a": [1, {"b": 1.0}]}), &json!({"a": [1.0, {"b": 1}]}), true; "nested numeric equality")]
    #[test_case(&json!({"a": [1, {"b": 1.0}]}), &json!({"a": [1, {"b": 2}]}), false; "nested mismatch")]
    fn equals_value_follows_json_schema_semantics(left: &Value, right: &Value, expected: bool) {
        assert_eq!(left.equals_value(right), expected);
    }

    #[test_case("", 0; "empty")]
    #[test_case("héllo", 5; "multi-byte")]
    #[test_case("🦀🦀", 2; "astral plane")]
    fn string_length_counts_code_points(input: &str, expected: u64) {
        let value = json!(input);
        assert_eq!((&value).string_length(), Some(expected));
    }

    #[test]
    fn to_value_borrows() {
        let document = json!({"a": 1});
        let node = &document;
        assert!(matches!(node.to_value(), Cow::Borrowed(_)));
    }

    fn assert_identity_stability<F: Json>(node: &F::Node<'_>) {
        let child = node
            .as_object()
            .expect("object")
            .get(&F::prepare_key("a"))
            .expect("present");
        assert_eq!(node.identity(), node.identity());
        assert_ne!(node.identity(), child.identity());
    }

    #[test]
    fn identity_is_stable_per_node() {
        let document = json!({"a": {"b": 1}});
        assert_identity_stability::<SerdeJson>(&&document);
    }

    fn assert_iteration_order<F: Json>(node: &F::Node<'_>) {
        let object = node.as_object().expect("object");
        let names: Vec<_> = object
            .members()
            .map(|(name, _)| name.as_ref().to_owned())
            .collect();
        assert_eq!(names, ["a", "b"]);

        let items = object
            .get(&F::prepare_key("b"))
            .expect("present")
            .as_array()
            .expect("array");
        let collected: Vec<Option<u64>> = items
            .elements()
            .map(|item| item.as_number().and_then(|number| number.as_u64()))
            .collect();
        assert_eq!(collected, [Some(10), Some(20)]);
    }

    #[test]
    fn members_and_items_iterate_in_order() {
        let document = json!({"a": 1, "b": [10, 20]});
        assert_iteration_order::<SerdeJson>(&&document);
    }
}

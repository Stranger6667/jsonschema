//! `serde_json::Value` representation: borrow-only accessors that monomorphize to direct `&Value` code.

use std::borrow::Cow;

use serde_json::{Map, Value};

use crate::{cmp, types::JsonType};

use super::{Array, Json, Node, NodeIdentity, Object};

/// Whether `serde_json` keeps objects in insertion order (its `preserve_order` feature). Its map is then
/// a hash index over a vector, and scanning the vector beats hashing the key for small objects.
const INSERTION_ORDERED: bool = std::mem::size_of::<Map<String, Value>>()
    != std::mem::size_of::<std::collections::BTreeMap<String, Value>>();

pub struct SerdeJson;

impl Json for SerdeJson {
    type Node<'a> = &'a Value;
    type PreparedKey = String;
    type StringBuffer = Value;

    // `Map` is a `BTreeMap`, so a lookup orders keys and reaches `memcmp` on every probe.
    const KEYS_PER_LOOKUP: usize = 2;

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

    fn string_length_within(&self, min: u64, max: u64) -> Option<bool> {
        let Value::String(string) = self else {
            return None;
        };
        // A code point takes one to four bytes, so the count lies in `ceil(bytes / 4)..=bytes`.
        let bytes = string.len() as u64;
        let fewest = bytes.div_ceil(4);
        if bytes < min || fewest > max {
            return Some(false);
        }
        if min <= fewest && bytes <= max {
            return Some(true);
        }
        Some((min..=max).contains(&(bytecount::num_chars(string.as_bytes()) as u64)))
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
        if INSERTION_ORDERED && Map::len(self) <= 16 {
            return (*self)
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value);
        }
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

    use serde_json::{json, Map, Value};
    use test_case::test_case;

    use super::{
        super::{Array, Json, JsonNumber, Node, Object},
        SerdeJson, INSERTION_ORDERED,
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

    // Byte length alone decides some cases (`ab`, 2 bytes) and not others (`🦀🦀`, 8 bytes, 2 code points).
    #[test_case("", 0, 0, Some(true); "empty within zero")]
    #[test_case("", 1, 5, Some(false); "empty below minimum")]
    #[test_case("ab", 0, 2, Some(true); "ascii at maximum")]
    #[test_case("abc", 0, 2, Some(false); "ascii past maximum")]
    #[test_case("ab", 3, 9, Some(false); "ascii below minimum")]
    #[test_case("🦀🦀", 0, 2, Some(true); "astral at maximum")]
    #[test_case("🦀🦀🦀", 0, 2, Some(false); "astral past maximum")]
    #[test_case("🦀🦀", 3, 9, Some(false); "astral below minimum")]
    #[test_case("🦀🦀", 2, 2, Some(true); "astral exact")]
    #[test_case("héllo", 5, 5, Some(true); "mixed exact")]
    #[test_case("héllo", 6, 9, Some(false); "mixed bytes reach minimum, code points do not")]
    #[test_case("héllo", 0, 4, Some(false); "mixed bytes past maximum, code points too")]
    #[test_case("ééé", 0, 3, Some(true); "two-byte bytes past maximum, code points not")]
    #[test_case("abc", 0, u64::MAX, Some(true); "unbounded")]
    fn string_length_within_counts_code_points(
        input: &str,
        min: u64,
        max: u64,
        expected: Option<bool>,
    ) {
        let value = json!(input);
        assert_eq!((&value).string_length_within(min, max), expected);
    }

    #[test]
    fn string_length_within_non_string() {
        assert_eq!((&json!(5)).string_length_within(0, 1), None);
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
    fn insertion_order_detection() {
        let mut map = Map::new();
        map.insert("b".to_owned(), Value::Null);
        map.insert("a".to_owned(), Value::Null);
        assert_eq!(
            INSERTION_ORDERED,
            map.keys().next().map(String::as_str) == Some("b")
        );
    }

    #[test_case(3; "below the scan threshold")]
    #[test_case(40; "above the scan threshold")]
    fn get(size: usize) {
        let map: Map<String, Value> = (0..size)
            .map(|index| (format!("key{index}"), json!(index)))
            .collect();
        let object = &map;
        for index in 0..size {
            let found = Object::get(&object, &SerdeJson::prepare_key(&format!("key{index}")));
            assert_eq!(found, Some(&json!(index)));
        }
        for missing in ["key", "key100", "kez0", ""] {
            assert_eq!(Object::get(&object, &SerdeJson::prepare_key(missing)), None);
        }
    }

    #[test]
    fn members_and_items_iterate_in_order() {
        let document = json!({"a": 1, "b": [10, 20]});
        assert_iteration_order::<SerdeJson>(&&document);
    }
}

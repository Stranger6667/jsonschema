//! Checks a representation against the contract the validator relies on.

use serde_json::{json, Value};

use crate::{types::JsonType, Array, Json, JsonNumber, Node, Object};

/// The document to encode before calling [`assert_conformance`].
#[must_use]
pub fn document() -> Value {
    json!({
        "string": "héllo",
        "integer": 42,
        "float": 1.5,
        "boolean": true,
        "null": null,
        "array": [10, 20, 30],
        "duplicates": [1, {"a": 1}, {"a": 1}],
        "object": {"nested": "value"}
    })
}

/// Assert that `root`, encoding [`document`], satisfies the contract.
///
/// # Panics
///
/// Panics on the first violation.
pub fn assert_conformance<F: Json>(root: &F::Node<'_>) {
    let members = root.as_object().expect("root is an object");
    let member = |name: &str| {
        members
            .get(&F::prepare_key(name))
            .unwrap_or_else(|| panic!("member {name} is present"))
    };

    assert_eq!(members.len(), 8, "object length");
    assert!(
        members.get(&F::prepare_key("missing")).is_none(),
        "absent member reports None"
    );

    assert_types::<F>(&member);
    assert_scalars::<F>(&member);
    assert_numbers::<F>(&member);
    assert_containers::<F>(&member);
    assert_equality::<F>(&member);
    assert_identity::<F>(root, &member);
    assert_string_nodes::<F>();
}

// `propertyNames` runs property names through `with_string_node`.
fn assert_string_nodes<F: Json>() {
    let mut buffer = F::StringBuffer::default();
    for expected in ["héllo", "", "second"] {
        F::with_string_node(&mut buffer, expected, |node| {
            assert_eq!(node.json_type(), JsonType::String, "string node type");
            assert_eq!(
                node.as_string().as_deref(),
                Some(expected),
                "string node content"
            );
        });
    }
}

fn assert_types<'a, F: Json>(member: &impl Fn(&str) -> F::Node<'a>) {
    for (name, expected) in [
        ("string", JsonType::String),
        ("integer", JsonType::Number),
        ("float", JsonType::Number),
        ("boolean", JsonType::Boolean),
        ("null", JsonType::Null),
        ("array", JsonType::Array),
        ("object", JsonType::Object),
    ] {
        let node = member(name);
        assert_eq!(node.json_type(), expected, "json_type of {name}");
        assert_eq!(
            node.is_number(),
            expected == JsonType::Number,
            "is_number of {name} agrees with json_type"
        );
        assert_eq!(
            node.is_number(),
            node.as_number().is_some(),
            "is_number of {name} agrees with as_number"
        );
        assert_eq!(
            node.is_string(),
            expected == JsonType::String,
            "is_string of {name}"
        );
        assert_eq!(node.is_null(), expected == JsonType::Null, "is_null {name}");
        assert_eq!(
            node.as_object().is_some(),
            expected == JsonType::Object,
            "as_object of {name}"
        );
        assert_eq!(
            node.as_array().is_some(),
            expected == JsonType::Array,
            "as_array of {name}"
        );
    }
}

fn assert_scalars<'a, F: Json>(member: &impl Fn(&str) -> F::Node<'a>) {
    let string = member("string");
    assert_eq!(string.as_string().as_deref(), Some("héllo"), "as_string");
    // Code points, not bytes: "héllo" is 6 bytes.
    assert_eq!(string.string_length(), Some(5), "string_length");
    assert_eq!(
        member("integer").string_length(),
        None,
        "string_length None"
    );

    assert_eq!(
        member("integer").as_number().expect("number").as_u64(),
        Some(42),
        "integer as_u64"
    );
    assert_eq!(
        member("float").as_number().expect("number").as_f64(),
        Some(1.5),
        "float as_f64"
    );
    assert_eq!(member("boolean").as_boolean(), Some(true), "as_boolean");
    assert_eq!(member("string").as_boolean(), None, "as_boolean None");
}

// A number must be readable without building a `serde_json::Number`
fn assert_numbers<'a, F: Json>(member: &impl Fn(&str) -> F::Node<'a>) {
    let integer = member("integer").as_number().expect("integer");
    assert_eq!(integer.as_u64(), Some(42), "as_u64");
    assert_eq!(integer.as_i64(), Some(42), "as_i64");
    assert_eq!(integer.as_f64(), Some(42.0), "as_f64");
    assert!(integer.is_integer(), "integer is_integer");
    assert_eq!(
        integer.to_number().as_ref(),
        &serde_json::Number::from(42),
        "to_number"
    );

    let float = member("float").as_number().expect("float");
    assert_eq!(float.as_f64(), Some(1.5), "float as_f64");
    assert_eq!(float.as_u64(), None, "float as_u64");
    assert_eq!(float.as_i64(), None, "float as_i64");
    assert!(!float.is_integer(), "float is_integer");

    assert!(
        member("string").as_number().is_none(),
        "string is not a number"
    );
    assert!(
        member("boolean").as_number().is_none(),
        "boolean is not a number"
    );
}

fn assert_containers<'a, F: Json>(member: &impl Fn(&str) -> F::Node<'a>) {
    let array = member("array").as_array().expect("array");
    assert_eq!(array.len(), 3, "array length");
    let elements: Vec<Option<u64>> = array
        .elements()
        .map(|element| element.as_number().and_then(|number| number.as_u64()))
        .collect();
    assert_eq!(elements, [Some(10), Some(20), Some(30)], "element order");
    assert!(array.is_unique(), "distinct elements are unique");
    assert!(
        !member("duplicates").as_array().expect("array").is_unique(),
        "equal objects are not unique"
    );

    let object = member("object").as_object().expect("object");
    let names: Vec<String> = object
        .members()
        .map(|(name, _)| name.as_ref().to_owned())
        .collect();
    assert_eq!(names, ["nested"], "member names");
    assert!(!object.is_empty(), "is_empty agrees with len");
}

fn assert_equality<'a, F: Json>(member: &impl Fn(&str) -> F::Node<'a>) {
    assert!(
        member("integer").equals_value(&json!(42)),
        "equals_value on the same number"
    );
    // JSON Schema compares numbers mathematically.
    assert!(
        member("integer").equals_value(&json!(42.0)),
        "equals_value across numeric spelling"
    );
    assert!(
        !member("integer").equals_value(&json!(43)),
        "equals_value rejects a different number"
    );
    assert!(
        !member("boolean").equals_value(&json!(1)),
        "a boolean is not a number"
    );
    assert!(
        member("object").equals_value(&json!({"nested": "value"})),
        "equals_value on a nested object"
    );
    assert_eq!(
        member("object").to_value().as_ref(),
        &json!({"nested": "value"}),
        "to_value round-trips"
    );
}

fn assert_identity<'a, F: Json>(root: &F::Node<'a>, member: &impl Fn(&str) -> F::Node<'a>) {
    let child = member("object");
    let Some(root_identity) = root.identity() else {
        // Opting out is allowed, but then recursion is bounded only by the stack.
        return;
    };
    assert_eq!(Some(root_identity), root.identity(), "identity is stable");
    assert_ne!(
        Some(root_identity),
        child.identity(),
        "distinct live nodes have distinct identities"
    );
    assert_eq!(
        member("object").identity(),
        child.identity(),
        "one node reached through two handles has one identity"
    );
    assert_eq!(
        root.container_identity(),
        Some(root_identity),
        "containers keep their identity"
    );
}

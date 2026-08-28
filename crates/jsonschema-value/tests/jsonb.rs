#![cfg(feature = "jsonb-testkit")]
// The encoder mirrors Postgres varlena/JEntry bit widths; every cast here is to a known-small field.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
#[cfg(feature = "conformance")]
use jsonschema_value::conformance;

use hegel::{extras::serde_json as json_gs, TestCase};
use jsonschema_value::{
    cmp, types::JsonType, Array, Json, JsonNumber, Jsonb, JsonbNode, LazyInstance, Node, Object,
};
use serde_json::{json, Value};
use test_case::test_case;

mod common;
use common::{
    assemble, decode_hex, encode, encode_array_of, encode_nested_arrays, encode_numeric,
    encode_numeric_text_array, encode_numeric_text_object, strip_varlena, to_hex, NumericForm,
    JB_FARRAY, JB_FSCALAR, JENTRY_HAS_OFF, JENTRY_ISNUMERIC,
};

#[test_case(json!({"a": 1}), JsonType::Object; "object root")]
#[test_case(json!([1, 2]), JsonType::Array; "array root")]
#[test_case(json!("text"), JsonType::String; "scalar string root")]
#[test_case(json!(1), JsonType::Number; "scalar number root")]
#[test_case(json!(true), JsonType::Boolean; "scalar boolean root")]
#[test_case(json!(null), JsonType::Null; "scalar null root")]
#[allow(clippy::needless_pass_by_value)]
fn root_reports_its_type(value: Value, expected: JsonType) {
    let encoded = encode(&value);
    assert_eq!(Jsonb::root(&encoded).json_type(), expected);
}

#[test]
fn scalar_root_entry_carries_has_off() {
    let encoded = encode(&json!(1));
    let entry = u32::from_ne_bytes(encoded[4..8].try_into().expect("4 bytes"));
    assert_eq!(entry & JENTRY_HAS_OFF, JENTRY_HAS_OFF);
}

// Fixtures captured from a real PostgreSQL server; see tools/gen-jsonb-fixtures.sh. `jsonb` stores
// its integer fields in native byte order, so each byte order needs its own capture.
const CORPUS_LITTLE: &str = include_str!("fixtures/jsonb-corpus.tsv");
const CORPUS_BIG: &str = include_str!("fixtures/jsonb-corpus-be.tsv");

#[cfg(target_endian = "little")]
const CORPUS: &str = CORPUS_LITTLE;
#[cfg(target_endian = "big")]
const CORPUS: &str = CORPUS_BIG;

const READER_ONLY_MARKER: &str = "# reader-only below this line";

fn parse_corpus_line(line: &str) -> Option<(&str, &str, Vec<u8>)> {
    if line.starts_with('#') || line.is_empty() {
        return None;
    }
    let mut columns = line.split('\t');
    let input = columns.next().expect("input column");
    let text = columns.next().expect("text column");
    let hex = columns.next().expect("hex column");
    Some((input, text, decode_hex(hex)))
}

/// `(input, postgres_text, stored_bytes)` for every fixture our encoder must reproduce exactly.
fn strict_corpus() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    let mut rows = Vec::new();
    for line in CORPUS.lines() {
        if line.starts_with(READER_ONLY_MARKER) {
            break;
        }
        if let Some(row) = parse_corpus_line(line) {
            rows.push(row);
        }
    }
    rows
}

// Catches a capture regenerated for only one byte order, or taken on the wrong architecture.
#[test]
fn captures_mirror_each_other() {
    let little: Vec<_> = CORPUS_LITTLE
        .lines()
        .filter_map(parse_corpus_line)
        .collect();
    let big: Vec<_> = CORPUS_BIG.lines().filter_map(parse_corpus_line).collect();
    assert_eq!(
        little.len(),
        big.len(),
        "captures cover a different row count"
    );
    // Per row the bytes may legitimately match - a container whose every word is symmetric
    // reads the same either way - so the byte order shows up across the corpus, not row by row.
    let mut differing_bytes = false;
    for (left, right) in little.iter().zip(&big) {
        assert_eq!(left.0, right.0, "captures cover different inputs");
        differing_bytes |= left.2 != right.2;
        assert_eq!(left.1, right.1, "postgres renders {} differently", left.0);
    }
    assert!(
        differing_bytes,
        "the two captures are byte-identical, so one of them is not from the other byte order"
    );
}

#[test]
fn corpus_is_populated() {
    let rows = strict_corpus();
    assert!(
        rows.len() >= 30,
        "corpus has {} strict rows, expected at least 30",
        rows.len()
    );
}

// The encoder feeds the rest of the suite: if it drifts, every other test agrees with a wrong
// reader.
#[test]
fn encoder_reproduces_postgres_bytes() {
    for (input, text, stored) in strict_corpus() {
        let value: Value = serde_json::from_str(text).expect("postgres text parses");
        let ours = encode(&value);
        let theirs = strip_varlena(&stored);
        assert_eq!(
            ours,
            theirs,
            "encoder diverges from postgres for {input}\n  ours:   {}\n  theirs: {}",
            to_hex(&ours),
            to_hex(theirs),
        );
    }
}

// From decimal text rather than `serde_json`'s float formatting, so a case names its own digits.
fn encode_numeric_scalar(text: &str, form: NumericForm) -> Vec<u8> {
    let mut data = Vec::new();
    let numeric = encode_numeric(text, form);
    data.extend_from_slice(&numeric);
    let entry = JENTRY_ISNUMERIC | numeric.len() as u32;
    assemble(JB_FARRAY | JB_FSCALAR | 1, &[entry], &data)
}

#[test_case("0", Some(0), Some(0), 0.0, true; "zero")]
#[test_case("42", Some(42), Some(42), 42.0, true; "small integer")]
#[test_case("-42", None, Some(-42), -42.0, true; "negative integer")]
#[test_case("1.5", None, None, 1.5, false; "one fraction digit")]
#[test_case("-0.25", None, None, -0.25, false; "negative fraction")]
#[test_case("1.0", Some(1), Some(1), 1.0, true; "trailing zero is still an integer")]
#[test_case("10000", Some(10000), Some(10000), 10000.0, true; "group boundary")]
#[test_case("100000000", Some(100_000_000), Some(100_000_000), 100_000_000.0, true; "two groups")]
#[test_case("0.0001", None, None, 0.0001, false; "leading fraction zeros")]
#[test_case("18446744073709551615", Some(u64::MAX), None, 1.844_674_407_370_955_2e19, true; "u64 max")]
#[test_case("-9223372036854775808", None, Some(i64::MIN), -9.223_372_036_854_776e18, true; "i64 min")]
fn numeric_accessors(
    text: &str,
    as_u64: Option<u64>,
    as_i64: Option<i64>,
    as_f64: f64,
    is_integer: bool,
) {
    let encoded = encode_numeric_scalar(text, NumericForm::Long);
    let node = Jsonb::root(&encoded);
    let number = node.as_number().expect("number");
    assert_eq!(number.as_u64(), as_u64);
    assert_eq!(number.as_i64(), as_i64);
    assert_eq!(number.as_f64(), Some(as_f64));
    assert_eq!(number.is_integer(), is_integer);
}

// Rendering puts the point where `dscale` says, not where `serde_json` would.
#[test_case("0", "0"; "zero")]
#[test_case("42", "42"; "small integer")]
#[test_case("-42", "-42"; "negative integer")]
#[test_case("1.0", "1.0"; "trailing zero is displayed")]
#[test_case("1.5", "1.5"; "one fraction digit")]
#[test_case("-0.25", "-0.25"; "two fraction digits")]
#[test_case("0.0001", "0.0001"; "fraction shorter than a group")]
#[test_case("0.00001", "0.00001"; "fraction crossing a group")]
#[test_case("10000", "10000"; "group boundary")]
#[test_case("100000000", "100000000"; "two groups")]
#[test_case("1.23456789", "1.23456789"; "fraction spanning groups")]
// Postgres `numeric` has no signed zero.
#[test_case("-0.0", "0.0"; "negative zero loses its sign")]
fn numeric_text(text: &str, expected: &str) {
    let encoded = encode_numeric_scalar(text, NumericForm::Long);
    let node = Jsonb::root(&encoded);
    assert_eq!(node.as_number().expect("number").as_str(), expected);
}

// One scalar root per number, so neither value's bytes shift the other's layout.
#[test_case(NumericForm::Short; "short numeric header")]
#[test_case(NumericForm::Long; "long numeric header")]
#[test_case(NumericForm::ShortVarlena; "short varlena header")]
fn numeric_header_forms_agree(form: NumericForm) {
    let fraction_encoded = encode_numeric_scalar("1.5", form);
    let fraction = Jsonb::root(&fraction_encoded).as_number().expect("number");
    let integer_encoded = encode_numeric_scalar("42", form);
    let integer = Jsonb::root(&integer_encoded).as_number().expect("number");
    assert_eq!(fraction.as_f64(), Some(1.5));
    assert!(!fraction.is_integer());
    assert_eq!(integer.as_u64(), Some(42));
    assert!(integer.is_integer());
}

#[test]
fn wide_numbers_keep_their_digits() {
    let encoded = encode_numeric_scalar("1e100", NumericForm::Long);
    let number = Jsonb::root(&encoded).as_number().expect("number");
    assert!(number.is_integer());
    // Past `u128`, so the integer path declines and the decimal text carries the value.
    assert_eq!(number.as_u64(), None);
    assert_eq!(number.as_f64(), Some(1e100));
    let mut expected = String::from("1");
    expected.push_str(&"0".repeat(100));
    assert_eq!(number.as_str(), expected);
}

// `serde_json::Number` cannot hold this magnitude, so `equals_value` has to compare through
// `JsonNumber`.
#[test_case(&json!(0), false; "instance is not the schema's small integer")]
#[test_case(&json!(1), false; "instance is not another small integer")]
fn equals_value_number_beyond_f64(expected: &Value, want: bool) {
    let encoded = encode_numeric_scalar("1e400", NumericForm::Long);
    assert_eq!(Jsonb::root(&encoded).equals_value(expected), want);
}

#[test]
fn equals_value_number_beyond_f64_inside_an_object() {
    let encoded = encode_numeric_text_object("big", "1e400", NumericForm::Long);
    assert!(!Jsonb::root(&encoded).equals_value(&json!({"big": 0})));
}

#[test]
fn is_unique_tells_apart_two_numbers_beyond_f64() {
    let encoded = encode_numeric_text_array(&["1e400", "2e400"], NumericForm::Long);
    let array = Jsonb::root(&encoded).as_array().expect("array");
    assert!(array.is_unique());
}

// The same value twice must still read back as a duplicate.
#[test]
fn is_unique_still_finds_a_duplicate_beyond_f64() {
    let encoded = encode_numeric_text_array(&["1e400", "1e400"], NumericForm::Long);
    let array = Jsonb::root(&encoded).as_array().expect("array");
    assert!(!array.is_unique());
}

#[test_case(json!(42), &json!(42), true; "same integer")]
#[test_case(json!(42), &json!(42.0), true; "integer equals float")]
#[test_case(json!(42), &json!(43), false; "different integer")]
#[test_case(json!(true), &json!(1), false; "boolean is not a number")]
#[test_case(json!("x"), &json!("x"), true; "same string")]
#[test_case(json!("x"), &json!("y"), false; "different string")]
#[test_case(json!(null), &json!(null), true; "null equals null")]
#[test_case(json!([1, 2]), &json!([1, 2]), true; "same array")]
#[test_case(json!([1, 2]), &json!([1, 2, 3]), false; "array length differs")]
#[test_case(json!([1, {"a": 1.0}]), &json!([1, {"a": 1}]), true; "nested numeric equality")]
#[test_case(json!({"a": 1, "b": 2}), &json!({"b": 2, "a": 1}), true; "object ignores key order")]
#[test_case(json!({"a": 1}), &json!({"a": 1, "b": 2}), false; "expected has an extra key")]
#[allow(clippy::needless_pass_by_value)]
fn equals_value_semantics(instance: Value, expected: &Value, want: bool) {
    let encoded = encode(&instance);
    assert_eq!(Jsonb::root(&encoded).equals_value(expected), want);
}

#[test]
fn is_unique_large_array_uses_hashing() {
    let values: Vec<Value> = (0..20).map(Value::from).collect();
    let encoded = encode(&Value::Array(values));
    assert!(Jsonb::root(&encoded).as_array().expect("array").is_unique());
}

#[test]
fn is_unique_large_array_finds_a_duplicate() {
    let mut values: Vec<Value> = (0..20).map(Value::from).collect();
    values[19] = json!(0);
    let encoded = encode(&Value::Array(values));
    assert!(!Jsonb::root(&encoded).as_array().expect("array").is_unique());
}

#[test_case(json!("x"); "string")]
#[test_case(json!(true); "boolean")]
#[test_case(json!(null); "null")]
#[test_case(json!([]); "array")]
#[allow(clippy::needless_pass_by_value)]
fn non_numbers_have_no_number(value: Value) {
    let encoded = encode(&value);
    let node = Jsonb::root(&encoded);
    assert!(node.as_number().is_none());
    assert!(!node.is_number());
}

#[test_case(json!([1, 2]); "array")]
#[test_case(json!("x"); "scalar")]
#[allow(clippy::needless_pass_by_value)]
fn non_objects_have_no_object(value: Value) {
    let encoded = encode(&value);
    assert!(Jsonb::root(&encoded).as_object().is_none());
}

#[test_case(json!({"a": 1}); "object")]
#[test_case(json!("x"); "scalar")]
#[allow(clippy::needless_pass_by_value)]
fn non_arrays_have_no_array(value: Value) {
    let encoded = encode(&value);
    assert!(Jsonb::root(&encoded).as_array().is_none());
}

fn numbers_of(node: &JsonbNode<'_>) -> Vec<u64> {
    node.as_array()
        .expect("array")
        .elements()
        .map(|element| {
            element
                .as_number()
                .expect("number")
                .as_u64()
                .expect("fits u64")
        })
        .collect()
}

#[test]
fn array_elements_walk_in_order() {
    let encoded = encode(&json!([10, 20, 30]));
    let root = Jsonb::root(&encoded);
    assert_eq!(root.as_array().expect("array").len(), 3);
    assert_eq!(numbers_of(&root), [10, 20, 30]);
}

// The 32nd entry stores an end offset rather than a length, so the walk has to cross that boundary.
#[test]
fn array_crosses_the_offset_stride() {
    let expected: Vec<u64> = (0..100).collect();
    let value = Value::Array(expected.iter().map(|item| json!(item)).collect());
    let encoded = encode(&value);
    assert_eq!(numbers_of(&Jsonb::root(&encoded)), expected);
}

#[test]
fn nested_arrays_keep_their_own_bounds() {
    let encoded = encode(&json!([[1, 2], [], [3]]));
    let root = Jsonb::root(&encoded);
    let outer = root.as_array().expect("array");
    let inner: Vec<Vec<u64>> = outer.elements().map(|node| numbers_of(&node)).collect();
    assert_eq!(inner, [vec![1, 2], vec![], vec![3]]);
}

#[test]
fn empty_array_has_no_elements() {
    let encoded = encode(&json!([]));
    let array = Jsonb::root(&encoded).as_array().expect("array");
    assert_eq!(array.len(), 0);
    assert_eq!(array.elements().count(), 0);
}

fn member_names(node: &JsonbNode<'_>) -> Vec<String> {
    node.as_object()
        .expect("object")
        .members()
        .map(|(name, _)| name.into_owned())
        .collect()
}

#[test]
fn members_pair_keys_with_values() {
    let encoded = encode(&json!({"bb": 2, "a": 1, "ccc": 3}));
    let root = Jsonb::root(&encoded);
    let object = root.as_object().expect("object");
    assert_eq!(object.len(), 3);
    // Postgres orders keys by length first, then by bytes.
    assert_eq!(member_names(&root), ["a", "bb", "ccc"]);
    let values: Vec<u64> = object
        .members()
        .map(|(_, node)| node.as_number().expect("number").as_u64().expect("u64"))
        .collect();
    assert_eq!(values, [1, 2, 3]);
}

#[test_case("a", Some(1); "first key")]
#[test_case("bb", Some(2); "middle key")]
#[test_case("ccc", Some(3); "last key")]
#[test_case("b", None; "absent, same length as a stored key")]
#[test_case("zzzz", None; "absent, longer than every stored key")]
#[test_case("", None; "absent, empty")]
fn get_binary_searches_keys(key: &str, expected: Option<u64>) {
    let encoded = encode(&json!({"bb": 2, "a": 1, "ccc": 3}));
    let root = Jsonb::root(&encoded);
    let found = root
        .as_object()
        .expect("object")
        .get(&Jsonb::prepare_key(key))
        .map(|node| node.as_number().expect("number").as_u64().expect("u64"));
    assert_eq!(found, expected);
}

// "aa" < "b" lexically, but Postgres orders by length first. A search on raw bytes finds "b".
#[test]
fn get_orders_keys_by_length_before_lexical() {
    let encoded = encode(&json!({"b": 1, "aa": 2}));
    let root = Jsonb::root(&encoded);
    let object = root.as_object().expect("object");
    let get = |key: &str| {
        object
            .get(&Jsonb::prepare_key(key))
            .expect("present")
            .as_number()
            .expect("number")
            .as_u64()
    };
    assert_eq!(get("b"), Some(1));
    assert_eq!(get("aa"), Some(2));
    // Documents the storage order the binary search relies on.
    assert_eq!(member_names(&root), ["b", "aa"]);
}

// 80 members starts the values run mid-stride, so both `get` and `members` seed a cursor from a
// non-zero index.
#[test]
fn object_lookup_crosses_the_offset_stride() {
    let members: serde_json::Map<String, Value> = (0..80)
        .map(|index| (format!("k{index:03}"), json!(index)))
        .collect();
    let encoded = encode(&Value::Object(members));
    let root = Jsonb::root(&encoded);
    let object = root.as_object().expect("object");
    for index in 0..80_u64 {
        let key = Jsonb::prepare_key(&format!("k{index:03}"));
        let found = object
            .get(&key)
            .expect("present")
            .as_number()
            .expect("number")
            .as_u64();
        assert_eq!(found, Some(index));
    }
    let mut pairs: Vec<(String, u64)> = object
        .members()
        .map(|(name, node)| {
            (
                name.into_owned(),
                node.as_number().expect("number").as_u64().expect("u64"),
            )
        })
        .collect();
    pairs.sort_by_key(|(_, index)| *index);
    let expected: Vec<(String, u64)> = (0..80_u64)
        .map(|index| (format!("k{index:03}"), index))
        .collect();
    assert_eq!(pairs, expected);
}

#[test]
fn empty_object_has_no_members() {
    let encoded = encode(&json!({}));
    let root = Jsonb::root(&encoded);
    let object = root.as_object().expect("object");
    assert_eq!(object.len(), 0);
    assert!(object.is_empty());
    assert_eq!(object.members().count(), 0);
}

#[test_case("", 0; "empty")]
#[test_case("plain", 5; "ascii")]
#[test_case("héllo", 5; "multi byte")]
#[test_case("🦀🦀", 2; "astral plane")]
fn strings_read_back_with_code_point_length(text: &str, length: u64) {
    let encoded = encode(&json!(text));
    let node = Jsonb::root(&encoded);
    assert_eq!(node.as_string().as_deref(), Some(text));
    assert_eq!(node.string_length(), Some(length));
    assert!(node.is_string());
}

#[test_case(json!(true), Some(true); "bool_true")]
#[test_case(json!(false), Some(false); "bool_false")]
#[test_case(json!("x"), None; "string is not a boolean")]
#[test_case(json!(1), None; "number is not a boolean")]
#[allow(clippy::needless_pass_by_value)]
fn booleans_read_back(value: Value, expected: Option<bool>) {
    let encoded = encode(&value);
    assert_eq!(Jsonb::root(&encoded).as_boolean(), expected);
}

#[test_case(json!(null), true; "null")]
#[test_case(json!(false), false; "false is not null")]
#[allow(clippy::needless_pass_by_value)]
fn null_is_recognised(value: Value, expected: bool) {
    let encoded = encode(&value);
    assert_eq!(Jsonb::root(&encoded).is_null(), expected);
}

#[test]
fn string_nodes_reuse_the_buffer() {
    let mut buffer = Vec::new();
    for expected in ["héllo", "", "second"] {
        Jsonb::with_string_node(&mut buffer, expected, |node| {
            assert_eq!(node.json_type(), JsonType::String);
            assert_eq!(node.as_string().as_deref(), Some(expected));
        });
    }
}

#[test]
fn strings_inside_containers_keep_their_bounds() {
    let encoded = encode(&json!({"a": "one", "b": "", "c": "three"}));
    let root = Jsonb::root(&encoded);
    let object = root.as_object().expect("object");
    let read = |key: &str| {
        object
            .get(&Jsonb::prepare_key(key))
            .expect("present")
            .as_string()
            .expect("string")
            .into_owned()
    };
    assert_eq!(read("a"), "one");
    assert_eq!(read("b"), "");
    assert_eq!(read("c"), "three");
}

#[test_case(json!(true); "boolean")]
#[test_case(json!(1); "number")]
#[test_case(json!(null); "null")]
#[allow(clippy::needless_pass_by_value)]
fn non_strings_are_not_strings(value: Value) {
    let encoded = encode(&value);
    assert!(!Jsonb::root(&encoded).is_string());
}

#[test_case(json!(null); "null")]
#[test_case(json!(true); "boolean")]
#[test_case(json!("héllo"); "string")]
#[test_case(json!(42); "integer")]
#[test_case(json!(1.5); "float")]
#[test_case(json!([]); "empty array")]
#[test_case(json!({}); "empty object")]
#[test_case(json!({"a": [1, {"b": null}], "c": ""}); "nested")]
#[test_case(json!(["", "", ""]); "adjacent empty strings")]
#[allow(clippy::needless_pass_by_value)]
fn to_value_round_trips(value: Value) {
    let encoded = encode(&value);
    assert_eq!(Jsonb::root(&encoded).to_value().as_ref(), &value);
}

// The default `Node::lazy_value` materializes eagerly, which is the thing being avoided here.
#[test]
fn lazy_value_defers_on_jsonb() {
    let encoded = encode(&json!({"a": [1, null]}));
    let root = Jsonb::root(&encoded);
    assert!(matches!(root.lazy_value(), LazyInstance::Deferred { .. }));
}

#[test_case(json!(null); "null")]
#[test_case(json!(true); "boolean")]
#[test_case(json!("héllo"); "string")]
#[test_case(json!(42); "integer")]
#[test_case(json!(1.5); "float")]
#[test_case(json!([]); "empty array")]
#[test_case(json!({}); "empty object")]
#[test_case(json!({"a": [1, {"b": null}], "c": ""}); "nested")]
#[allow(clippy::needless_pass_by_value)]
fn lazy_value_round_trips(value: Value) {
    let encoded = encode(&value);
    assert_eq!(Jsonb::root(&encoded).lazy_value().get().as_ref(), &value);
}

// Deferring is only worth it if `get()` builds once and hands back the same storage after.
#[test]
fn lazy_value_memoizes_rather_than_rebuilding() {
    let encoded = encode(&json!({"a": [1, 2, 3], "b": "text"}));
    let lazy = Jsonb::root(&encoded).lazy_value();
    let first: *const Value = lazy.get().as_ref();
    let second: *const Value = lazy.get().as_ref();
    assert!(std::ptr::eq(first, second));
}

#[cfg(feature = "conformance")]
#[test]
fn conformance_contract_holds() {
    let encoded = encode(&conformance::document());
    conformance::assert_conformance::<Jsonb>(&Jsonb::root(&encoded));
}

#[test]
fn containers_have_distinct_identities() {
    let encoded = encode(&json!({"a": {"b": [{}, {}]}}));
    let root = Jsonb::root(&encoded);
    let a = root
        .as_object()
        .expect("object")
        .get(&Jsonb::prepare_key("a"))
        .expect("present");
    let b = a
        .as_object()
        .expect("object")
        .get(&Jsonb::prepare_key("b"))
        .expect("present");
    let inner: Vec<_> = b
        .as_array()
        .expect("array")
        .elements()
        .map(|node| node.container_identity())
        .collect();
    assert_eq!(root.identity(), root.container_identity());
    assert_ne!(root.identity(), a.identity());
    assert_ne!(a.identity(), b.identity());
    assert_ne!(inner[0], inner[1]);
    assert!(inner.iter().all(Option::is_some));
}

// A whole float above 2^53 is ambiguous: jsonb stores the exact decimal of its shortest text,
// which need not be the float's own value. Rewriting it to that value as `i64`/`u64` is exact and
// removes the ambiguity; past those bounds `float_roundtrip` keeps the decode correctly rounded.
#[cfg(not(feature = "arbitrary-precision"))]
fn unambiguous_numbers(value: &Value) -> Value {
    match value {
        Value::Number(number) => {
            if number.as_u64().is_some() || number.as_i64().is_some() {
                return value.clone();
            }
            let f = number
                .as_f64()
                .expect("a non-arbitrary-precision Number always has an f64");
            if f.fract() != 0.0 || f.abs() < 2f64.powi(53) {
                value.clone()
            } else if f >= 0.0 && f < 2f64.powi(64) {
                Value::Number((f as u64).into())
            } else if f < 0.0 && f >= -(2f64.powi(63)) {
                Value::Number((f as i64).into())
            } else {
                value.clone()
            }
        }
        Value::Array(items) => Value::Array(items.iter().map(unambiguous_numbers).collect()),
        Value::Object(members) => Value::Object(
            members
                .iter()
                .map(|(key, member)| (key.clone(), unambiguous_numbers(member)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

// An exact integer was never ambiguous, so the rewrite has to leave it alone.
#[cfg(not(feature = "arbitrary-precision"))]
#[test]
fn unambiguous_numbers_leaves_exact_integers_untouched() {
    let value = json!(9_007_199_254_740_993_u64);
    assert_eq!(unambiguous_numbers(&value), value);
}

// jsonb keeps a number's value, not how it was written, so this compares under JSON-Schema
// equality. `unambiguous_numbers` first removes the one float class whose f64 bits and jsonb's
// reading of its own text can differ; it discards no draw, and is a no-op under
// `arbitrary-precision`.
#[hegel::test(test_cases = 5_000)]
fn any_document_round_trips(tc: TestCase) {
    let value = tc.draw(json_gs::values());
    #[cfg(not(feature = "arbitrary-precision"))]
    let value = unambiguous_numbers(&value);
    let encoded = encode(&value);
    let decoded = Jsonb::root(&encoded).to_value();
    assert!(
        cmp::equal(&decoded, &value),
        "decoded {decoded:?} does not equal drawn {value:?}"
    );
}

// Postgres `numeric` has no int/float distinction, so a whole float comes back as the integer
// Postgres would print. Above 2^53 that is its shortest text read exactly, as the last case shows.
#[test_case(1e16, "10000000000000000"; "1e16 decodes as the integer it names")]
#[test_case(-1e16, "-10000000000000000"; "negative 1e16 decodes as the integer it names")]
#[test_case(1e17, "100000000000000000"; "1e17 decodes as the integer it names")]
#[test_case(1.801_439_850_948_199e16, "18014398509481990"; "a float above 2^53 decodes as its text's value, not its own")]
fn whole_number_float_loses_its_spelling(input: f64, expected_digits: &str) {
    let value = Value::Number(serde_json::Number::from_f64(input).expect("finite"));
    let encoded = encode(&value);
    let node = Jsonb::root(&encoded);
    let expected: Value = serde_json::from_str(expected_digits).expect("digits parse");
    assert_eq!(node.to_value().as_ref(), &expected);
    assert_eq!(node.as_number().expect("number").as_str(), expected_digits);
}

#[hegel::test(test_cases = 5_000)]
fn get_agrees_with_members(tc: TestCase) {
    let value = tc.draw(json_gs::values());
    let encoded = encode(&value);
    check_lookups(&Jsonb::root(&encoded));
}

fn check_lookups(node: &JsonbNode<'_>) {
    if let Some(object) = node.as_object() {
        let mut longest = 0;
        for (name, member) in object.members() {
            let found = object
                .get(&Jsonb::prepare_key(&name))
                .expect("a member name is found");
            assert_eq!(found.to_value(), member.to_value());
            longest = longest.max(name.len());
            check_lookups(&member);
        }
        // Longer than every stored key, so no stored key can equal it.
        let absent = "x".repeat(longest + 1);
        assert!(object.get(&Jsonb::prepare_key(&absent)).is_none());
    } else if let Some(array) = node.as_array() {
        for element in array.elements() {
            check_lookups(&element);
        }
    }
}

/// `(input, postgres_text, stored_bytes)` for every fixture, both sides of the reader-only marker.
fn reader_corpus() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    CORPUS.lines().filter_map(parse_corpus_line).collect()
}

// Against bytes Postgres produced, rather than bytes our own encoder produced.
#[test]
fn reader_decodes_postgres_bytes() {
    for (input, text, stored) in reader_corpus() {
        let expected: Value = serde_json::from_str(text).expect("postgres text parses");
        let container = strip_varlena(&stored);
        let actual = Jsonb::root(container).to_value();
        assert_eq!(
            actual.as_ref(),
            &expected,
            "reader diverges from postgres for input {input}"
        );
    }
}

// The reader-only rows hold numbers past f64, where `as_str` still carries every digit.
#[test]
fn reader_reads_wide_numbers() {
    let strict_count = strict_corpus().len();
    let rows = reader_corpus();
    let reader_only = &rows[strict_count..];
    assert_eq!(
        reader_only.len(),
        5,
        "reader-only rows moved; update this test alongside the marker"
    );
    for (input, text, stored) in reader_only {
        let container = strip_varlena(stored);
        let root = Jsonb::root(container);
        let (number, expected) = if let Some(number) = root.as_number() {
            (number, (*text).to_string())
        } else {
            let object = root.as_object().expect("object");
            assert_eq!(object.len(), 1, "single-member object");
            let (name, member) = object.members().next().expect("one member");
            let expected = text
                .strip_prefix(&format!("{{\"{name}\": "))
                .and_then(|rest| rest.strip_suffix('}'))
                .unwrap_or_else(|| panic!("unexpected rendering for {input}: {text}"))
                .to_string();
            (member.as_number().expect("number"), expected)
        };
        assert_eq!(number.as_str(), expected, "digits for {input}");
    }
}

#[test_case(&json!(["", ""]); "empty strings")]
#[test_case(&json!([null, null]); "nulls")]
#[test_case(&json!([true, true]); "booleans")]
fn adjacent_zero_length_scalars_have_distinct_identities(instance: &Value) {
    let encoded = encode(instance);
    let root = Jsonb::root(&encoded);
    let array = root.as_array().expect("array");
    let identities: Vec<_> = array.elements().map(|node| node.identity()).collect();
    assert!(identities.iter().all(Option::is_some));
    assert_ne!(identities[0], identities[1]);
}

#[test]
fn simultaneous_synthetic_empty_strings_have_distinct_stable_identities() {
    let mut first_buffer = Vec::new();
    let mut second_buffer = Vec::new();
    Jsonb::with_string_node(&mut first_buffer, "", |first| {
        Jsonb::with_string_node(&mut second_buffer, "", |second| {
            let first_identity = first.identity();
            let second_identity = second.identity();
            assert!(first_identity.is_some() && second_identity.is_some());
            assert_eq!(first_identity, first.identity());
            assert_eq!(second_identity, second.identity());
            assert_ne!(first_identity, second_identity);
        });
    });
}

// Beyond `f64` the digits cannot survive into a `serde_json::Number`, but the reported instance
// still has to carry the right sign and magnitude.
#[test_case("1e400", false; "positive")]
#[test_case("-1e400", true; "negative")]
fn wide_numbers_do_not_report_as_zero(text: &str, negative: bool) {
    let encoded = encode_numeric_scalar(text, NumericForm::Long);
    let node = Jsonb::root(&encoded);
    let number = node.as_number().expect("number");
    let reported = number.to_number();
    let reported = reported.to_string();
    assert_ne!(reported, "0", "reported zero for {text}");
    assert_eq!(reported.starts_with('-'), negative, "sign of {reported}");
}

// Postgres stores nesting far deeper than a recursive walk survives on a backend's 2MB stack.
const POSTGRES_NESTING: usize = 16_000;

#[test_case(false; "to_value")]
#[test_case(true; "lazy value")]
#[should_panic(expected = "JSONB value exceeds maximum materialization nesting depth (255)")]
fn materializing_beyond_the_nesting_limit_panics(lazy: bool) {
    let encoded = encode_nested_arrays(255);
    let node = Jsonb::root(&encoded);
    if lazy {
        let value = node.lazy_value();
        let _ = value.get();
    } else {
        let _ = node.to_value();
    }
}

#[test_case(false; "to_value")]
#[test_case(true; "lazy value")]
fn materializing_at_the_nesting_limit_succeeds(lazy: bool) {
    let encoded = encode_nested_arrays(254);
    let node = Jsonb::root(&encoded);
    if lazy {
        let value = node.lazy_value();
        let _ = value.get();
    } else {
        let _ = node.to_value();
    }
}

#[test]
fn deep_instances_navigate_without_overflowing() {
    let encoded = encode_nested_arrays(POSTGRES_NESTING);
    let mut node = Jsonb::root(&encoded);
    let mut depth = 0;
    while let Some(element) = node.as_array().expect("array").elements().next() {
        node = element;
        depth += 1;
    }
    assert_eq!(depth, POSTGRES_NESTING);
}

// `{"uniqueItems": true}` is shallow, so nothing about the schema bounds how deep the compared
// elements are.
#[test]
fn unique_items_over_deep_elements_does_not_overflow() {
    let deep = encode_nested_arrays(POSTGRES_NESTING);
    let encoded = encode_array_of(&[deep.clone(), deep]);
    let array = Jsonb::root(&encoded).as_array().expect("array");
    assert!(!array.is_unique());
}

// Nested empty containers differ only in shape, so a hash that ignores it drives `uniqueItems`
// into comparing every pair.
#[test]
fn nested_empty_containers_hash_apart() {
    let shapes = [
        encode(&json!([])),
        encode(&json!([[]])),
        encode(&json!([[[]]])),
        encode(&json!([[], []])),
        encode(&json!({})),
    ];
    let encoded = encode_array_of(&shapes);
    let array = Jsonb::root(&encoded).as_array().expect("array");
    assert!(array.is_unique());
}

// Two distinct byte sequences that a lossy decode would fold onto the same replacement character.
#[test]
fn strings_compare_by_their_stored_bytes() {
    let first: &[u8] = b"\xff";
    let second: &[u8] = b"\xfe";
    let entries = [JENTRY_HAS_OFF | first.len() as u32, second.len() as u32];
    let mut data = first.to_vec();
    data.extend_from_slice(second);
    let encoded = assemble(JB_FARRAY | 2, &entries, &data);
    let array = Jsonb::root(&encoded).as_array().expect("array");
    assert!(array.is_unique());
}

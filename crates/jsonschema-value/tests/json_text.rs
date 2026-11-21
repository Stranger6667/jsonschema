#![cfg(feature = "json-text")]
#[cfg(feature = "conformance")]
use jsonschema_value::conformance;

use std::fmt::Write as _;

use hegel::{extras::serde_json as json_gs, generators as gs, TestCase};
use jsonschema_value::{
    cmp, types::JsonType, unique, Array, Json, JsonText, JsonTextNode, Node, Object,
};
use serde_json::{json, Value};
use test_case::test_case;

// Reads `node` only through the trait, the way the validator does.
fn assert_reads_as(node: &JsonTextNode<'_>, expected: &Value) {
    match expected {
        Value::Null => assert_eq!(node.json_type(), JsonType::Null),
        Value::Bool(boolean) => assert_eq!(node.as_boolean(), Some(*boolean)),
        Value::Number(number) => {
            let got = node.as_number().expect("a number");
            assert_eq!(got.as_u64(), number.as_u64());
            assert_eq!(got.as_i64(), number.as_i64());
            assert_eq!(got.as_f64(), number.as_f64());
        }
        Value::String(string) => assert_eq!(node.as_string().as_deref(), Some(string.as_str())),
        Value::Array(items) => {
            let array = node.as_array().expect("an array");
            assert_eq!(array.len(), items.len());
            let elements: Vec<_> = array.elements().collect();
            assert_eq!(elements.len(), items.len());
            for (element, item) in elements.iter().zip(items) {
                assert_reads_as(element, item);
            }
        }
        Value::Object(map) => {
            let object = node.as_object().expect("an object");
            assert_eq!(object.len(), map.len());
            let mut seen = 0;
            for (name, member) in object.members() {
                assert_reads_as(&member, &map[name.as_ref()]);
                seen += 1;
            }
            assert_eq!(seen, map.len());
            for (name, value) in map {
                let member = object.get(&JsonText::prepare_key(name)).expect("present");
                assert_reads_as(&member, value);
            }
        }
    }
}

#[hegel::test(test_cases = 5_000)]
fn any_document_reads_as_serde_json_does(tc: TestCase) {
    let value = tc.draw(json_gs::values());
    for text in [
        serde_json::to_vec(&value).expect("serializable"),
        serde_json::to_vec_pretty(&value).expect("serializable"),
    ] {
        let document = JsonText::parse(&text).expect("valid JSON");
        assert_reads_as(&document.root(), &value);
        assert_eq!(document.root().to_value().as_ref(), &value);
    }
}

// One byte of a valid document replaced: both parsers must accept or reject it, and agree on
// the value when they accept.
#[hegel::test(test_cases = 20_000)]
fn damaged_documents_parse_as_serde_json_does(tc: TestCase) {
    let value = tc.draw(json_gs::values());
    let mut text = serde_json::to_vec(&value).expect("serializable");
    let at = tc.draw(gs::integers::<usize>().max_value(text.len() - 1));
    let replacement = tc.draw(gs::sampled_from(
        b"{}[]:,\"\\ \t\n\r\x0c\x00\x1f0123456789-+.eEtrufalsn/xu\xff\xc3".to_vec(),
    ));
    text[at] = replacement;
    assert_parses_as_serde_json(&text);
}

fn assert_parses_as_serde_json(input: &[u8]) {
    let expected = serde_json::from_slice::<Value>(input);
    match JsonText::parse(input) {
        Ok(document) => assert_eq!(
            document.root().to_value().into_owned(),
            expected.expect("serde_json accepts it too"),
            "{}",
            String::from_utf8_lossy(input)
        ),
        Err(_) => assert!(
            expected.is_err(),
            "serde_json accepts {}",
            String::from_utf8_lossy(input)
        ),
    }
}

#[test_case(b"" ; "empty")]
#[test_case(b" \t\n\r" ; "whitespace only")]
#[test_case(b" {} " ; "padded object")]
#[test_case(b"\x0c{}" ; "form feed is not whitespace")]
#[test_case(b"\xef\xbb\xbf{}" ; "byte order mark")]
#[test_case(b"01" ; "leading zero")]
#[test_case(b"-" ; "lone minus")]
#[test_case(b"-0" ; "negative zero")]
#[test_case(b"1." ; "dot without fraction")]
#[test_case(b".5" ; "fraction without integer")]
#[test_case(b"+1" ; "plus sign")]
#[test_case(b"1e" ; "exponent without digits")]
#[test_case(b"1E+5" ; "signed exponent")]
#[test_case(b"-1.5e-3" ; "negative fraction")]
#[test_case(b"1e400" ; "overflowing exponent")]
#[test_case(b"-1e400" ; "overflowing negative exponent")]
#[test_case(b"0.0000000001e309" ; "small mantissa large exponent")]
#[test_case(b"1e-400" ; "underflowing exponent")]
#[test_case(b"18446744073709551615" ; "u64 max")]
#[test_case(b"18446744073709551616" ; "past u64")]
#[test_case(b"-9223372036854775808" ; "i64 min")]
#[test_case(b"-9223372036854775809" ; "past i64")]
#[test_case(b"\"\\u00e9\"" ; "escaped code point")]
#[test_case(b"\"\\ud83d\\ude00\"" ; "surrogate pair")]
#[test_case(b"\"\\ud800\"" ; "lone leading surrogate")]
#[test_case(b"\"\\udc00\"" ; "lone trailing surrogate")]
#[test_case(b"\"\\ud800\\u0041\"" ; "leading surrogate before a letter")]
#[test_case(b"\"\\u12\"" ; "short unicode escape")]
#[test_case(b"\"\\x\"" ; "unknown escape")]
#[test_case(b"\"\\/\\b\\f\\n\\r\\t\\\"\\\\\"" ; "every short escape")]
#[test_case(b"\"a\nb\"" ; "raw newline in string")]
#[test_case(b"\"a\x7fb\"" ; "raw delete in string")]
#[test_case(b"\"\xff\"" ; "invalid UTF-8 in string")]
#[test_case(b"\"unterminated" ; "unterminated string")]
#[test_case(b"[1,]" ; "trailing comma in array")]
#[test_case(b"[,1]" ; "leading comma in array")]
#[test_case(b"{\"a\":1,}" ; "trailing comma in object")]
#[test_case(b"{\"a\" 1}" ; "missing colon")]
#[test_case(b"{1:1}" ; "number key")]
#[test_case(b"{\"a\":1 \"b\":2}" ; "missing comma")]
#[test_case(b"[1 2]" ; "missing comma in array")]
#[test_case(b"tr" ; "truncated literal")]
#[test_case(b"truex" ; "literal with trailing letter")]
#[test_case(b"null x" ; "trailing characters")]
#[test_case(b"[1]]" ; "extra closing bracket")]
#[test_case(b"{\"a\":[}" ; "mismatched brackets")]
#[test_case(br#"{"a":1,"a":2}"# ; "duplicate keys")]
#[test_case(br#"{"a\u0062":1,"ab":2}"# ; "duplicate keys written differently")]
fn parses_as_serde_json(input: &[u8]) {
    assert_parses_as_serde_json(input);
}

#[test_case(126)]
#[test_case(127)]
#[test_case(128)]
#[test_case(129)]
fn nesting_limit_matches_serde_json(depth: usize) {
    for (open, close) in [(b'[', b']'), (b'{', b'}')] {
        let mut text = Vec::new();
        for level in 0..depth {
            if open == b'{' && level > 0 {
                text.extend_from_slice(b"\"k\":");
            }
            text.push(open);
        }
        text.extend(std::iter::repeat_n(close, depth));
        assert_parses_as_serde_json(&text);
    }
}

#[test_case("0")]
#[test_case("-0")]
#[test_case("7")]
#[test_case("-7")]
#[test_case("9999999999999999999" ; "nineteen digits")]
#[test_case("18446744073709551615" ; "u64 max")]
#[test_case("18446744073709551616" ; "past u64")]
#[test_case("-9223372036854775808" ; "i64 min")]
#[test_case("-9223372036854775809" ; "past i64")]
#[test_case("123456789012345678901234567890" ; "past both")]
#[test_case("1.0")]
#[test_case("-0.0")]
#[test_case("1e2")]
#[test_case("1E+2")]
#[test_case("-1.5e-3")]
#[test_case("0.1")]
#[test_case("2.2250738585072011e-308" ; "subnormal boundary")]
#[test_case("1.7976931348623157e308" ; "f64 max")]
fn numbers_read_as_serde_json_does(literal: &str) {
    let expected: serde_json::Number = serde_json::from_str(literal).expect("a number");
    let document = JsonText::parse(literal.as_bytes()).expect("valid JSON");
    let got = document.root().as_number().expect("a number");
    assert_eq!(got, expected);
    assert_eq!(
        got.as_f64().map(f64::to_bits),
        expected.as_f64().map(f64::to_bits)
    );
}

#[test_case(br#"{"k0":0,"k1":1,"k2":2,"k3":3,"k4":4,"k5":5,"k6":6,"k7":7,"k8":8,"k3":"x"}"# ; "many keys, one repeated")]
#[test_case(br#"{"k0":0,"k1":1,"k2":2,"k3":3,"k4":4,"k5":5,"k6":6,"k7":7,"k8":8,"k9":9}"# ; "many distinct keys")]
#[test_case(br#"{"aaaaaaaaXbbbbbbbb":1,"aaaaaaaaYbbbbbbbb":2}"# ; "keys differing in the middle")]
#[test_case(br#"{"aaaaaaaaXbbbbbbbb":1,"aaaaaaaaYbbbbbbbb":2,"k2":2,"k3":3,"k4":4,"k5":5,"k6":6,"k7":7,"k8":8}"# ; "many keys differing in the middle")]
#[test_case(br#"{"abcdefgh":1,"abcdefgh":2}"# ; "eight byte keys repeated")]
#[test_case(br#"{"":1,"":2}"# ; "empty keys repeated")]
#[test_case(br#"{"b":1,"a":2}"# ; "short keys at the end")]
fn objects_read_as_serde_json_does(input: &[u8]) {
    let expected: Value = serde_json::from_slice(input).expect("valid JSON");
    let document = JsonText::parse(input).expect("valid JSON");
    assert_reads_as(&document.root(), &expected);
}

// Drawn from a small pool so duplicates are common, and long enough to reach hashing.
#[hegel::test(test_cases = 2_000)]
fn is_unique_agrees_with_serde_json(tc: TestCase) {
    let pool: Vec<Value> = tc.draw(gs::vecs(json_gs::values()).min_size(1).max_size(6));
    let picks: Vec<usize> =
        tc.draw(gs::vecs(gs::integers::<usize>().max_value(pool.len() - 1)).max_size(40));
    let items: Vec<Value> = picks.iter().map(|&index| pool[index].clone()).collect();
    let text = serde_json::to_vec(&Value::Array(items.clone())).expect("serializable");
    let document = JsonText::parse(&text).expect("valid JSON");
    let array = document.root().as_array().expect("an array");
    assert_eq!(array.is_unique(), unique::is_unique(&items), "{items:?}");
}

#[test_case(r#"{"a":1}"#, json!({"a": 2}), false ; "object with a different value")]
#[test_case(r#"{"a":1}"#, json!({"b": 1}), false ; "object with a different key")]
#[test_case(r#"{"a":1,"b":[1,2]}"#, json!({"b": [1, 2.0], "a": 1.0}), true ; "object with equal numbers")]
#[test_case(r#"[1,{"a":"x"}]"#, json!([1, {"a": "y"}]), false ; "array with a nested difference")]
#[test_case(r"[1,2]", json!([1, 2, 3]), false ; "array of another length")]
#[test_case(r#"{"a":1,"a":2}"#, json!({"a": 2}), true ; "object with a repeated key")]
#[test_case(r"[]", json!({}), false ; "array against object")]
#[allow(clippy::needless_pass_by_value)]
fn equals_value_compares_containers(text: &str, expected: Value, equal: bool) {
    let document = JsonText::parse(text.as_bytes()).expect("valid JSON");
    assert_eq!(document.root().equals_value(&expected), equal);
}

// `const` and `enum` compare the instance against schema values this way.
#[hegel::test(test_cases = 2_000)]
fn equals_value_agrees_with_serde_json(tc: TestCase) {
    let pool: Vec<Value> = tc.draw(gs::vecs(json_gs::values()).min_size(1).max_size(4));
    let left = &pool[tc.draw(gs::integers::<usize>().max_value(pool.len() - 1))];
    let right = &pool[tc.draw(gs::integers::<usize>().max_value(pool.len() - 1))];
    let text = serde_json::to_vec(left).expect("serializable");
    let document = JsonText::parse(&text).expect("valid JSON");
    assert_eq!(
        document.root().equals_value(right),
        cmp::equal(left, right),
        "{left} {right}"
    );
}

#[test_case(br#"["a","\u0061"]"#, false ; "string written with an escape")]
#[test_case(br#"["a","b","\""]"#, true ; "distinct strings")]
#[test_case(b"[1,1.0]", false ; "equal numbers written differently")]
#[test_case(br#"["x",true,null,"x"]"#, false ; "repeated string among literals")]
#[test_case(br#"[true,false,null,"true","null"]"#, true ; "literals and their names")]
#[test_case(b"[null,null]", false ; "repeated null")]
#[test_case(br#"["k0","k1","k2","k3","k4","k5","k6","k7","k8","k9","k10","k11","k12","k13","k14","k15","k16","k17","k18","k19","k20"]"#, true ; "many distinct strings")]
#[test_case(br#"["k0","k1","k2","k3","k4","k5","k6","k7","k8","k9","k10","k11","k12","k13","k14","k15","k16","k17","k18","k19","\u006b7"]"#, false ; "many strings, one repeated with an escape")]
#[test_case(br#"[{"a":1,"b":2},{"b":2,"a":1}]"#, false ; "objects with members in another order")]
#[test_case(br#"[{"a":1},{"a":1.0}]"#, false ; "objects with equal numbers written differently")]
#[test_case(br#"[{"a":1},{"a":2}]"#, true ; "objects with different values")]
#[test_case(br#"[{"a":1,"b":2},{"b":3,"a":1}]"#, true ; "objects in another order with a different value")]
#[test_case(br#"[{"a":1},{"b":1}]"#, true ; "objects with different keys")]
#[test_case(br#"[{"a":1},{"a":1,"b":2}]"#, true ; "objects of different sizes")]
#[test_case(br#"[{"a":1,"a":2},{"a":2}]"#, false ; "object with a repeated key")]
#[test_case(br"[[1,[2]],[1,[2.0]]]", false ; "nested arrays")]
#[test_case(br"[[1,2],[2,1]]", true ; "arrays in another order")]
#[test_case(br#"[{"\u0061":"x"},{"a":"\u0078"}]"#, false ; "objects written with escapes")]
fn is_unique_reads_values_not_text(input: &[u8], expected: bool) {
    let document = JsonText::parse(input).expect("valid JSON");
    let array = document.root().as_array().expect("an array");
    assert_eq!(array.is_unique(), expected);
}

#[test]
fn duplicate_keys_keep_the_last_value() {
    let document = JsonText::parse(br#"{"a": 1, "b": 2, "a": "x"}"#).expect("valid JSON");
    let object = document.root().as_object().expect("an object");
    assert_eq!(object.len(), 2);
    let a = object.get(&JsonText::prepare_key("a")).expect("present");
    assert_eq!(a.as_string().as_deref(), Some("x"));
    let members: Vec<(String, Value)> = object
        .members()
        .map(|(name, value)| (name.into_owned(), value.to_value().into_owned()))
        .collect();
    // `json.loads` and `JSON.parse` keep a repeated key where it first appears.
    assert_eq!(
        members,
        vec![("a".to_string(), json!("x")), ("b".to_string(), json!(2))]
    );
}

#[test]
fn repeated_keys_in_wide_objects_keep_their_first_position() {
    let mut text = String::from("{");
    for index in 0..40 {
        write!(text, "\"k{index}\":{index},").expect("write to String never fails");
    }
    text.push_str("\"k3\":\"x\",\"k3\":\"y\"}");
    let document = JsonText::parse(text.as_bytes()).expect("valid JSON");
    let object = document.root().as_object().expect("an object");
    assert_eq!(object.len(), 40);
    let members: Vec<(String, Value)> = object
        .members()
        .map(|(name, value)| (name.into_owned(), value.to_value().into_owned()))
        .collect();
    assert_eq!(members.len(), 40);
    assert_eq!(members[3], ("k3".to_string(), json!("y")));
    assert_eq!(members[39], ("k39".to_string(), json!(39)));
    let k3 = object.get(&JsonText::prepare_key("k3")).expect("present");
    assert_eq!(k3.as_string().as_deref(), Some("y"));
}

#[test]
fn escaped_keys_match_their_decoded_names() {
    let document = JsonText::parse(br#"{"a\u0062": 1, "\"": 2}"#).expect("valid JSON");
    let object = document.root().as_object().expect("an object");
    let ab = object.get(&JsonText::prepare_key("ab")).expect("present");
    assert_eq!(ab.as_number().expect("a number").as_u64(), Some(1));
    let quote = object.get(&JsonText::prepare_key("\"")).expect("present");
    assert_eq!(quote.as_number().expect("a number").as_u64(), Some(2));
    let names: Vec<String> = object
        .members()
        .map(|(name, _)| name.into_owned())
        .collect();
    assert_eq!(names, vec!["ab".to_string(), "\"".to_string()]);
}

#[test_case(r#""\ud83d\ude00 \uD83D\uDE00""# ; "surrogate pairs in either case")]
#[test_case(r#""\/\b\f\n\r\t\"\\""# ; "every short escape")]
#[test_case(r#""\u00e9t\u00e9 \u00E9 \u0000""# ; "code points")]
#[test_case(r#""plain é ✓""# ; "no escapes")]
fn strings_decode_as_serde_json_does(literal: &str) {
    let expected: String = serde_json::from_str(literal).expect("a string");
    let document = JsonText::parse(literal.as_bytes()).expect("valid JSON");
    let root = document.root();
    assert_eq!(root.as_string().as_deref(), Some(expected.as_str()));
    assert_eq!(root.string_length(), Some(expected.chars().count() as u64));
    let text = format!(
        "[{literal},{}]",
        serde_json::to_string(&expected).expect("serializable")
    );
    let pair = JsonText::parse(text.as_bytes()).expect("valid JSON");
    assert!(!pair.root().as_array().expect("an array").is_unique());
}

#[test_case("plain")]
#[test_case("with \"quotes\" and \\ backslash")]
#[test_case("control \u{1} char")]
#[test_case("")]
fn string_nodes_hold_the_name(name: &str) {
    let mut buffer = <JsonText as Json>::StringBuffer::default();
    JsonText::with_string_node(&mut buffer, name, |node| {
        assert_eq!(node.json_type(), JsonType::String);
        assert_eq!(node.as_string().as_deref(), Some(name));
    });
}

#[cfg(feature = "conformance")]
#[test]
fn conformance_contract_holds() {
    let text = serde_json::to_vec(&conformance::document()).expect("serializable");
    let document = JsonText::parse(&text).expect("valid JSON");
    conformance::assert_conformance::<JsonText>(&document.root());
}

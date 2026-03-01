use jsonschema::canonical::json::to_string;
use serde_json::Value;
use test_case::test_case;

fn assert_canonical(raw: &str, expected: &str) {
    let value: Value = serde_json::from_str(raw).unwrap();
    assert_eq!(to_string(&value).unwrap(), expected, "raw={raw}");
}

fn assert_roundtrip(raw: &str) {
    assert_canonical(raw, raw);
}

#[test]
fn canonical_string_is_stable_for_equivalent_schemas() {
    let left: Value =
        serde_json::from_str(r#"{"b":1,"a":{"z":3,"x":1,"y":2},"c":[{"d":4,"b":2}]}"#).unwrap();
    let right: Value =
        serde_json::from_str(r#"{"c":[{"b":2,"d":4}],"a":{"y":2,"x":1,"z":3},"b":1}"#).unwrap();

    assert_eq!(to_string(&left).unwrap(), to_string(&right).unwrap());
}

#[test_case("1.0", "1"; "positive_integral")]
#[test_case("-5.0", "-5"; "negative_integral")]
#[test_case("1.5", "1.5"; "fractional")]
fn float_values_are_serialized(raw: &str, expected: &str) {
    assert_canonical(raw, expected);
}

#[test_case("null"; "null")]
#[test_case("true"; "bool_true")]
#[test_case("false"; "bool_false")]
#[test_case(r#""hello""#; "simple")]
#[test_case(r#""line\nbreak""#; "escaped_newline")]
fn scalar_literals_roundtrip(raw: &str) {
    assert_roundtrip(raw);
}

#[test]
fn large_integer_valued_float_uses_integer_form() {
    let value: Value = serde_json::from_str("1e300").unwrap();

    #[cfg(feature = "arbitrary-precision")]
    let expected = {
        let mut output = String::with_capacity(301);
        output.push('1');
        output.push_str(&"0".repeat(300));
        output
    };
    #[cfg(not(feature = "arbitrary-precision"))]
    let expected = format!("{:.0}", 1e300_f64);

    assert_eq!(to_string(&value).unwrap(), expected);
}

#[cfg(feature = "arbitrary-precision")]
#[test_case("1.0", "1"; "integral_fraction")]
#[test_case("100E-2", "1"; "integral_exponent")]
#[test_case("-0E-1000", "0"; "negative_zero")]
#[test_case("1e+3", "1000"; "positive_exponent")]
#[test_case("0e+10", "0"; "zero_with_exponent")]
#[test_case("1e-2", "1e-2"; "fractional_exponent")]
#[test_case("1.25", "1.25"; "fractional_decimal")]
#[test_case("3.1400e-3", "3.1400e-3"; "fractional_significand")]
fn arbitrary_precision_number_forms(raw: &str, expected: &str) {
    assert_canonical(raw, expected);
}

#[test]
fn canonical_output_is_idempotent() {
    let value: Value =
        serde_json::from_str(r#"{"z":{"b":1,"a":2},"a":[3,2,1],"f":1.0,"v":1.5}"#).unwrap();

    let first = to_string(&value).unwrap();
    let parsed: Value = serde_json::from_str(&first).unwrap();

    assert_eq!(to_string(&parsed).unwrap(), first);
}

#[test]
fn recursion_limit_error_is_reported() {
    let mut value = Value::Null;
    for _ in 0..=255 {
        value = Value::Array(vec![value]);
    }

    let error = to_string(&value).unwrap_err();
    assert!(error.to_string().contains("Recursion limit reached"));
}

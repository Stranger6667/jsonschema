use std::borrow::Cow;

use jsonschema_value::{cmp, numeric, JsonNumber};
use serde_json::Number;
use test_case::test_case;

// A representation keeping the decimal literal, so a number past `f64` still has its digits.
struct Literal(&'static str);

impl JsonNumber for Literal {
    fn as_u64(&self) -> Option<u64> {
        self.0.parse().ok()
    }
    fn as_i64(&self) -> Option<i64> {
        self.0.parse().ok()
    }
    fn as_f64(&self) -> Option<f64> {
        self.0.parse().ok().filter(|value: &f64| value.is_finite())
    }
    fn as_str(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.0)
    }
    fn to_number(&self) -> Cow<'_, Number> {
        Cow::Owned(self.0.parse().expect("literal parses as a JSON number"))
    }
}

#[test_case("1e400", true; "positive past f64 sits above every limit")]
#[test_case("-1e400", false; "negative past f64 sits below every limit")]
fn a_number_past_f64_compares_as_an_infinity_of_its_sign(text: &'static str, above: bool) {
    let value = Literal(text);
    assert_eq!(numeric::ge(&value, 5), above);
    assert_eq!(numeric::gt(&value, 5), above);
    assert_eq!(numeric::le(&value, 5), !above);
    assert_eq!(numeric::lt(&value, 5), !above);
}

#[cfg(feature = "macros")]
#[test]
fn a_number_past_f64_equals_no_limit() {
    assert!(!numeric::eq(&Literal("1e400"), 5));
}

#[test]
fn a_number_past_f64_equals_no_literal() {
    assert!(!cmp::equal_numbers(&Literal("1e400"), &Number::from(1)));
}

#[test_case("1000000000000000000000000", 3.0, false; "past u64, the digits say no where f64 rounds to yes")]
#[test_case("1.5e21", 4.0, true; "exponent form with a point is left to f64")]
#[test_case("9007199254740993.0", 3.0, true; "zero fraction past 2^53 stays whole")]
#[test_case("9007199254740993.5", 4.0, false; "fraction past 2^53 is not whole")]
#[test_case("1e-400", 2.0, false; "below the smallest subnormal is not zero")]
fn multiple_of_integer_reads_the_digits(text: &'static str, divisor: f64, expected: bool) {
    assert_eq!(
        numeric::is_multiple_of_integer(&Literal(text), divisor),
        expected
    );
}

// An exponent form past `f64` is placed only with `arbitrary-precision`.
#[cfg(not(feature = "arbitrary-precision"))]
#[test]
fn multiple_of_integer_leaves_an_exponent_form_past_f64_unplaced() {
    assert!(!numeric::is_multiple_of_integer(&Literal("1e400"), 2.0));
}

#[test]
fn multiple_of_integer_divides_a_number_past_f64() {
    let text: &'static str = format!("1{}", "0".repeat(400)).leak();
    assert!(numeric::is_multiple_of_integer(&Literal(text), 4.0));
    assert!(!numeric::is_multiple_of_integer(&Literal(text), 3.0));
}

#[test_case("1e-400", false; "below the smallest subnormal is not zero")]
#[test_case("0e5", true; "zero mantissa is zero")]
#[test_case("-0.0", true; "negative zero is zero")]
fn multiple_of_float_reads_zero_from_the_digits(text: &'static str, expected: bool) {
    assert_eq!(numeric::is_multiple_of_float(&Literal(text), 0.5), expected);
}

// `serde_json` keeps the literal only with `arbitrary-precision`; without it `1e-400` is `0.0`
// before validation starts.
#[cfg(feature = "arbitrary-precision")]
#[test]
fn serde_json_tiny_literal_is_not_a_multiple() {
    let number: Number = serde_json::from_str("1e-400").expect("parses");
    assert!(!numeric::is_multiple_of_float(&number, 0.5));
}

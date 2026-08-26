use hegel::{generators as gs, TestCase};
use jsonschema::JsonType;
use serde_json::{json, Value};

pub(crate) fn small_int(tc: &TestCase) -> i32 {
    tc.draw(gs::integers::<i32>().min_value(-8).max_value(8))
}

// Integers on both sides of exact `f64` precision, where a rewritten divisor changes the verdict.
pub(crate) const WIDE_INTEGERS: &[&str] = &[
    "9007199254740992",
    "9007199254740993",
    "18014398509481986",
    "27021597764222976",
    "27021597764222977",
    "12345678900000001",
    "13510798882111488",
    "1e30",
];

pub(crate) fn wide_number(tc: &TestCase) -> Value {
    let text = tc.draw(gs::sampled_from(WIDE_INTEGERS.to_vec()));
    serde_json::from_str(text).expect("valid number literal")
}

pub(crate) fn finite_float(tc: &TestCase) -> f64 {
    tc.draw(gs::floats::<f64>().min_value(-8.0).max_value(8.0))
}

// One value per family, spelled several ways: normalization must equate them (and, under
// `arbitrary-precision` where serde keeps the raw token, integer-valued floats fold to integers).
pub(crate) const ALIAS_FAMILIES: &[&[&str]] = &[
    &["1.5", "1.50", "15e-1"],
    &["2", "2.0", "0.2e1", "2e0"],
    &["-0.5", "-5e-1", "-0.50"],
    &["0", "0.0", "-0", "0e0"],
];

pub(crate) fn aliased_number(tc: &TestCase) -> Value {
    let family = tc.draw(gs::sampled_from(ALIAS_FAMILIES.to_vec()));
    let index = tc.draw(
        gs::integers::<usize>()
            .min_value(0)
            .max_value(family.len() - 1),
    );
    serde_json::from_str(family[index]).expect("valid number literal")
}

// A bounded scalar for `const`/`enum`, across the primitive types.
#[hegel::composite]
pub(crate) fn arbitrary_scalar(tc: &TestCase) -> Value {
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(5)) {
        0 => Value::Null,
        1 => Value::Bool(tc.draw(gs::booleans())),
        2 => Value::String(tc.draw(gs::text().max_size(3))),
        3 => json!(tc.draw(gs::integers::<i32>().min_value(-8).max_value(8))),
        4 => json!(finite_float(tc)),
        _ => aliased_number(tc),
    }
}

#[hegel::composite]
pub(crate) fn arbitrary_instance(tc: &TestCase) -> Value {
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(10)) {
        0 => Value::Null,
        1 => Value::Bool(tc.draw(gs::booleans())),
        2 => json!(tc.draw(gs::integers::<i32>().min_value(-8).max_value(8))),
        3 => json!(finite_float(tc)),
        // An integer-valued float (`2.0`): Draft 4 treats it as a non-integer, later drafts as an integer.
        4 => json!(f64::from(
            tc.draw(gs::integers::<i32>().min_value(-4).max_value(4))
        )),
        5 => Value::String(tc.draw(gs::text().max_size(5))),
        6 => wide_number(tc),
        7 => json!([]),
        8 => {
            let mut object = serde_json::Map::new();
            for key in draw_keys(tc) {
                object.insert(key.to_string(), tc.draw(arbitrary_scalar()));
            }
            Value::Object(object)
        }
        9 => {
            let count = tc.draw(gs::integers::<usize>().min_value(0).max_value(2));
            Value::Array((0..count).map(|_| tc.draw(arbitrary_scalar())).collect())
        }
        _ => json!({}),
    }
}

// Keys drawn from a small pool so different leaves overlap often enough to exercise merging.
pub(crate) fn draw_keys(tc: &TestCase) -> Vec<&'static str> {
    let count = tc.draw(gs::integers::<usize>().min_value(0).max_value(2));
    let mut keys: Vec<&'static str> = (0..count)
        .map(|_| tc.draw(gs::sampled_from(vec!["a", "b", "c", "ab"])))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// An arbitrary value of one JSON type, constrained by nothing else.
pub(crate) fn draw_unconstrained(tc: &TestCase, ty: JsonType) -> Value {
    match ty {
        JsonType::Null => Value::Null,
        JsonType::Boolean => json!(tc.draw(gs::booleans())),
        JsonType::Integer => json!(small_int(tc)),
        JsonType::Number => json!(finite_float(tc)),
        JsonType::String => json!(tc.draw(gs::text().max_size(5))),
        JsonType::Array => {
            let count = tc.draw(gs::integers::<usize>().min_value(0).max_value(2));
            Value::Array((0..count).map(|_| tc.draw(arbitrary_scalar())).collect())
        }
        JsonType::Object => {
            let mut object = serde_json::Map::new();
            for key in draw_keys(tc) {
                object.insert(key.to_string(), tc.draw(arbitrary_scalar()));
            }
            Value::Object(object)
        }
    }
}

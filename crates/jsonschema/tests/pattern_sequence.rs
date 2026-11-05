#![cfg(not(target_arch = "wasm32"))]
use hegel::{generators as gs, TestCase};
use jsonschema::{FancyRegex, PatternOptions};
use serde_json::{json, Value};
use test_case::test_case;

fn validator(schema: &Value, fancy: bool) -> jsonschema::Validator {
    let options = jsonschema::options();
    if fancy {
        options
            .with_pattern_options(PatternOptions::fancy_regex())
            .build(schema)
    } else {
        options
            .with_pattern_options(PatternOptions::regex())
            .build(schema)
    }
    .expect("schema compiles")
}

#[test_case(r"^[a-zé]+$", "café", true; "non-ascii member on non-ascii subject")]
#[test_case(r"^[a-zé]+$", "Café", false; "non-ascii member rejects")]
#[test_case(r"^.$", "\r", false; "dot rejects carriage return")]
#[test_case(r"^.$", "\u{2028}", false; "dot rejects line separator")]
#[test_case(r"^.$", "\u{e9}", true; "dot accepts non-ascii")]
#[test_case(r"^\s$", "\u{a0}", true; "whitespace accepts no-break space")]
#[test_case(r"^\s$", "\r", true; "whitespace accepts carriage return")]
#[test_case(r"^abc$", "abc\n", false; "end is end of input")]
#[test_case(r"^[a-z]{2}", "ab\u{e9}", true; "match before non-ascii")]
#[test_case(r"^[a-z]{2}", "a\u{e9}", false; "non-ascii in the prefix")]
#[test_case(r"^[a-z]{1,100}$", "abcdefghij", true; "long single class")]
#[test_case(r"^.{1,1000}$", "\u{e9}", true; "single class at cap on non-ascii")]
#[test_case(r"^\w{1000,}$", "\u{e9}", false; "single class open at cap on non-ascii")]
#[test_case(r"^\d+[:-]\d+$", "8080:80", true; "port mapping")]
#[test_case(r"^\d+[:-]\d+$", "8080", false; "port mapping missing")]
fn pattern(pattern: &str, text: &str, expected: bool) {
    let schema = json!({"pattern": pattern});
    let instance = json!(text);
    assert_eq!(
        (
            validator(&schema, true).is_valid(&instance),
            validator(&schema, false).is_valid(&instance)
        ),
        (expected, expected)
    );
}

#[test_case(r"^(?=[a-z])[a-z0-9]{3}$", "a12", true; "lookahead parts accept")]
#[test_case(r"^(?=[a-z])[a-z0-9]{3}$", "1ab", false; "lookahead parts reject")]
#[test_case(r"^(?=[a-z])[a-z0-9]{3}$", "\u{e9}ab", false; "lookahead parts on non-ascii")]
#[test_case(r"^(?=\d)(?=.{2})[0-9a-f]+$", "1f", true; "two lookaheads")]
fn lookahead_parts(pattern: &str, text: &str, expected: bool) {
    let schema = json!({"pattern": pattern});
    assert_eq!(validator(&schema, true).is_valid(&json!(text)), expected);
}

// Each schema accepts `{key: 1}` exactly when `key` matches the pattern.
fn property_schemas(pattern: &str) -> [Value; 3] {
    [
        json!({"patternProperties": {pattern: true}, "additionalProperties": false}),
        json!({"patternProperties": {pattern: true}, "unevaluatedProperties": false}),
        json!({"not": {"patternProperties": {pattern: false}, "minProperties": 1}}),
    ]
}

fn property_answers(pattern: &str, key: &str) -> Vec<bool> {
    let instance = json!({key: 1});
    property_schemas(pattern)
        .iter()
        .flat_map(|schema| [true, false].map(|fancy| validator(schema, fancy).is_valid(&instance)))
        .collect()
}

#[test_case(r"^[a-z\u00e9]+$", "café", true; "non-ascii member on non-ascii key")]
#[test_case(r"^[a-z\u00e9]+$", "Café", false; "non-ascii member rejects key")]
#[test_case(r"^.$", "\r", false; "dot rejects carriage return")]
#[test_case(r"^.$", "\u{2028}", false; "dot rejects line separator")]
#[test_case(r"^abc$", "abc\n", false; "end is end of input")]
#[test_case(r"^[a-z]{2}", "ab\u{e9}", true; "match before non-ascii")]
#[test_case(r"^[a-z]{2}", "a\u{e9}", false; "non-ascii in the prefix")]
#[test_case(r"^\d+[:-]\d+$", "8080:80", true; "port mapping")]
#[test_case(r"^\d+[:-]\d+$", "8080", false; "port mapping missing")]
#[test_case(r"^[a-z]{1,100}$", "abcdefghij", true; "long single class")]
fn property_patterns(pattern: &str, key: &str, matches: bool) {
    assert_eq!(property_answers(pattern, key), vec![matches; 6]);
}

#[test_case("café", true; "sequence key on non-ascii")]
#[test_case("xb", true; "alternation key")]
#[test_case("Café", false; "neither")]
fn mixed_property_patterns(key: &str, expected: bool) {
    let patterns = json!({"^[a-z\\u00e9]+$": true, "^x(a|b)$": true});
    let instance = json!({key: 1});
    let answers: Vec<bool> = ["additionalProperties", "unevaluatedProperties"]
        .iter()
        .flat_map(|keyword| {
            let schema = json!({"patternProperties": patterns, *keyword: false});
            [true, false].map(|fancy| validator(&schema, fancy).is_valid(&instance))
        })
        .collect();
    assert_eq!(answers, vec![expected; 4]);
}

#[test_case(true; "fancy")]
#[test_case(false; "regex")]
fn error_names_the_schema_pattern(fancy: bool) {
    let validator = validator(&json!({"pattern": "^[a-z]+$"}), fancy);
    let messages: Vec<String> = validator
        .iter_errors(&json!("1"))
        .map(|error| error.to_string())
        .collect();
    assert_eq!(
        messages,
        vec![r#""1" does not match "^[a-z]+$""#.to_owned()]
    );
}

#[test_case(true; "fancy")]
#[test_case(false; "regex")]
fn size_limit_rejects_the_schema(fancy: bool) {
    let schema = json!({"pattern": "^[a-z]+$"});
    let options = jsonschema::options();
    let result = if fancy {
        options
            .with_pattern_options(PatternOptions::fancy_regex().size_limit(100))
            .build(&schema)
    } else {
        options
            .with_pattern_options(PatternOptions::regex().size_limit(100))
            .build(&schema)
    };
    assert!(result.is_err());
}

// Patterns the engine rejects at its default size limit build and count characters instead, so
// the sequence's fallback is never asked to build them, and non-ASCII letters are decided too.
#[test_case(json!({"pattern": r"^\p{L}{1,1000}$"}), json!("é"), json!("é1"); "pattern")]
#[test_case(
    json!({"pattern": r"^[\p{L}\p{N} ]{1,255}$"}), json!("é 1"), json!("é-");
    "pattern with a wide class"
)]
#[test_case(
    json!({"patternProperties": {r"^\p{L}{1,1000}$": false}}), json!({"1": 1}), json!({"é": 1});
    "pattern properties"
)]
#[test_case(
    json!({"patternProperties": {r"^\p{L}{1,1000}$": true}, "additionalProperties": false}),
    json!({"é": 1}),
    json!({"1": 1});
    "pattern properties with additional properties"
)]
#[allow(clippy::needless_pass_by_value)]
fn oversized_patterns_count_characters(schema: Value, accepted: Value, rejected: Value) {
    let fancy = jsonschema::options()
        .with_pattern_options(PatternOptions::fancy_regex())
        .build(&schema)
        .expect("builds within the default size limit");
    let regex = jsonschema::options()
        .with_pattern_options(PatternOptions::regex())
        .build(&schema)
        .expect("builds within the default size limit");
    for validator in [&fancy, &regex] {
        assert_eq!(
            (validator.is_valid(&accepted), validator.is_valid(&rejected)),
            (true, false)
        );
    }
}

// Only `fancy-regex` reads lookaheads; the part after them is oversized.
#[test]
fn oversized_lookahead_part_counts_characters() {
    let validator = jsonschema::options()
        .with_pattern_options(PatternOptions::fancy_regex())
        .build(&json!({"pattern": r"^(?=a)\p{L}{1,1000}$"}))
        .expect("builds within the default size limit");
    assert_eq!(
        (
            validator.is_valid(&json!("aé")),
            validator.is_valid(&json!("é"))
        ),
        (true, false)
    );
}

const ATOMS: &[&str] = &[
    "a", "b", "-", ".", r"\d", r"\w", r"\s", "[a-c]", r"[^\n]", "[x-z0-9]", r"\.",
];
const QUANTIFIERS: &[&str] = &["", "", "?", "*", "+", "{2}", "{1,3}", "{2,}"];
const ALPHABET: &[char] = &[
    'a', 'b', 'c', 'x', 'z', '0', '9', '-', '.', '_', ' ', '\t', '\r', '\n', '\u{e9}', '\u{a0}',
];

// With engine limits configured, a sequence pattern answers ASCII subjects itself and non-ASCII
// ones through a fallback built with those limits.
#[test_case(true, "cafe", true; "fancy ascii")]
#[test_case(true, "café", true; "fancy non-ascii")]
#[test_case(true, "Café", false; "fancy non-ascii rejected")]
#[test_case(false, "cafe", true; "regex ascii")]
#[test_case(false, "café", true; "regex non-ascii")]
#[test_case(false, "Café", false; "regex non-ascii rejected")]
fn sequence_under_engine_limits(fancy: bool, text: &str, expected: bool) {
    let schema = json!({"pattern": "^[a-zé]+$"});
    let options = jsonschema::options();
    let validator = if fancy {
        options
            .with_pattern_options(PatternOptions::fancy_regex().size_limit(1 << 20))
            .build(&schema)
    } else {
        options
            .with_pattern_options(PatternOptions::regex().size_limit(1 << 20))
            .build(&schema)
    }
    .expect("fits the limits");
    assert_eq!(validator.is_valid(&json!(text)), expected);
}

// A match the engine cannot finish makes validation run under each of its outcomes; a sequence
// pattern beside it still decides its own subject in those runs.
#[test_case(
    PatternOptions::fancy_regex(), r"^(a|a)+\1$", "aaaaaaaaaaaaaaaaaaaaaaaaaaaax", "/allOf/0/pattern",
    "Error executing regex: Max limit for backtracking count exceeded";
    "lazy fallback, sequence accepts"
)]
#[test_case(
    PatternOptions::fancy_regex(), r"^(a|a)+\1$", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaX", "/allOf/1/pattern",
    r#""aaaaaaaaaaaaaaaaaaaaaaaaaaaaX" does not match "^[a-z]+$""#;
    "lazy fallback, sequence rejects"
)]
#[test_case(
    PatternOptions::fancy_regex().backtrack_limit(1), r"(?<=ab)c", "abc", "/allOf/0/pattern",
    "Error executing regex: Max limit for backtracking count exceeded";
    "fallback built with the limits, sequence accepts"
)]
#[test_case(
    PatternOptions::fancy_regex().backtrack_limit(1), r"(?<=ab)c", "abC", "/allOf/1/pattern",
    r#""abC" does not match "^[a-z]+$""#;
    "fallback built with the limits, sequence rejects"
)]
fn sequence_beside_an_unfinished_match(
    options: PatternOptions<FancyRegex>,
    unfinished: &str,
    text: &str,
    schema_path: &str,
    message: &str,
) {
    let validator = jsonschema::options()
        .with_pattern_options(options)
        .build(&json!({"allOf": [{"pattern": unfinished}, {"pattern": "^[a-z]+$"}]}))
        .expect("schema compiles");
    let errors: Vec<_> = validator
        .iter_errors(&json!(text))
        .map(|error| (error.schema_path().as_str().to_owned(), error.to_string()))
        .collect();
    assert_eq!(errors, vec![(schema_path.to_owned(), message.to_owned())]);
}

// An invalid subschema under a `patternProperties` key fails the build at its own location,
// whichever matcher the key compiles to.
#[test_case(r"^x-"; "literal prefix")]
#[test_case(r"^[a-z]+$"; "sequence")]
#[test_case(r"[a-z]+x"; "regex")]
fn invalid_pattern_property_subschema(pattern: &str) {
    let error =
        jsonschema::validator_for(&json!({"patternProperties": {(pattern): {"pattern": "("}}}))
            .expect_err("invalid subschema");
    assert_eq!(
        error.schema_path().as_str(),
        format!("/patternProperties/{pattern}/pattern")
    );
}

fn draw_pattern(tc: &TestCase) -> String {
    let count = tc.draw(gs::integers::<usize>().min_value(0).max_value(6));
    let mut pattern = String::from("^");
    for _ in 0..count {
        pattern.push_str(tc.draw(gs::sampled_from(ATOMS.to_vec())));
        pattern.push_str(tc.draw(gs::sampled_from(QUANTIFIERS.to_vec())));
    }
    if tc.draw(gs::booleans()) {
        pattern.push('$');
    }
    pattern
}

fn draw_text(tc: &TestCase) -> String {
    let length = tc.draw(gs::integers::<usize>().min_value(0).max_value(12));
    (0..length)
        .map(|_| tc.draw(gs::sampled_from(ALPHABET.to_vec())))
        .collect()
}

#[hegel::test(test_cases = 2000)]
fn sequence_patterns_agree_with_the_engine(tc: TestCase) {
    let pattern = draw_pattern(&tc);
    let text = draw_text(&tc);
    let translated = jsonschema_regex::to_rust_regex(&pattern).expect("translates");
    let engine = regex::Regex::new(&translated).expect("compiles");
    let expected = engine.is_match(&text);
    let schema = json!({"pattern": pattern});
    let instance = json!(text);
    assert_eq!(
        (
            validator(&schema, true).is_valid(&instance),
            validator(&schema, false).is_valid(&instance)
        ),
        (expected, expected),
        "{pattern:?} on {text:?}"
    );
    assert_eq!(
        property_answers(&pattern, &text),
        vec![expected; 6],
        "{pattern:?} on key {text:?}"
    );
}

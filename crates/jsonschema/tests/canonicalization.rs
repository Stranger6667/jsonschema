#![allow(clippy::needless_pass_by_value)]

use jsonschema::{
    canonical::{options, CanonicalKind},
    canonicalize, CanonicalizationError, PatternOptions,
};
use referencing::{Draft, Registry};
use serde_json::{json, Value};
use test_case::test_case;

const DRAFT202012_SCHEMA_URI: &str = "https://json-schema.org/draft/2020-12/schema";

/// Canonicalize `schema`, then assert that both the raw and canonical validators produce the expected
/// verdict on each witness (an upfront sanity check that the raw schema validates as expected, plus a
/// parity check that canonicalization neither widened nor narrowed the schema).
fn assert_parity_against_expected(schema: Value, witnesses: &[(Value, bool)]) {
    let canonical = canonicalize(&schema).expect("canonicalize");
    let canon_value = canonical.to_json_schema();
    let draft = Draft::default().detect(&schema);
    let raw = jsonschema::options()
        .with_draft(draft)
        .build(&schema)
        .expect("raw compiles");
    let canon = jsonschema::options()
        .with_draft(draft)
        .build(&canon_value)
        .expect("canon compiles");
    for (witness, expected_raw) in witnesses {
        assert_eq!(
            raw.is_valid(witness),
            *expected_raw,
            "sanity (raw schema): schema={schema}, witness={witness}"
        );
        assert_eq!(
            canon.is_valid(witness),
            *expected_raw,
            "canonicalize widened/narrowed: schema={schema}, canon={canon_value}, witness={witness}"
        );
    }
}

#[cfg(feature = "arbitrary-precision")]
fn schema_from_str(text: &str) -> Value {
    serde_json::from_str(text).expect("valid schema JSON")
}

fn nested_array(depth: usize) -> Value {
    let mut value = Value::Null;
    for _ in 0..depth {
        value = Value::Array(vec![value]);
    }
    value
}

fn nested_all_of_schema(depth: usize) -> Value {
    let mut schema = json!({});
    for _ in 0..depth {
        schema = json!({"allOf": [schema]});
    }
    schema
}

#[cfg(feature = "arbitrary-precision")]
#[test_case(r#"{"type":"string","minLength":18446744073709551616}"#, "minLength" ; "min_length")]
#[test_case(r#"{"type":"string","maxLength":18446744073709551616}"#, "maxLength" ; "max_length")]
#[test_case(r#"{"type":"array","minItems":18446744073709551616}"#, "minItems" ; "min_items")]
#[test_case(r#"{"type":"array","maxItems":18446744073709551616}"#, "maxItems" ; "max_items")]
#[test_case(
    r#"{"type":"array","contains":{"type":"null"},"minContains":18446744073709551616,"$schema":"https://json-schema.org/draft/2020-12/schema"}"#,
    "minContains"
    ; "min_contains"
)]
#[test_case(
    r#"{"type":"array","contains":{"type":"null"},"maxContains":18446744073709551616,"$schema":"https://json-schema.org/draft/2020-12/schema"}"#,
    "maxContains"
    ; "max_contains"
)]
#[test_case(r#"{"type":"object","minProperties":18446744073709551616}"#, "minProperties" ; "min_properties")]
#[test_case(r#"{"type":"object","maxProperties":18446744073709551616}"#, "maxProperties" ; "max_properties")]
fn arbitrary_precision_cardinality_bounds_emit_exactly(schema: &str, keyword: &str) {
    let canonical = canonicalize(&schema_from_str(schema)).expect("canonicalize");
    let emitted = canonical.to_json_schema();
    let expected = schema_from_str("18446744073709551616");

    assert_eq!(
        emitted[keyword], expected,
        "canonicalization lost precision for {keyword}: {emitted}"
    );
}

// A `const` number past the expansion cap would emit in scientific normal form, which the
// runtime validator cannot compare exactly; the document stays raw so validation is unchanged.
#[cfg(feature = "arbitrary-precision")]
#[test]
fn past_cap_finite_values_stay_raw() {
    let one_then_zeroes = format!("1{}", "0".repeat(1 << 20));
    for text in [
        r#"{"const":1e999999999999999999999}"#.to_string(),
        format!(r#"{{"const":{one_then_zeroes}}}"#),
        format!(r#"{{"const":{}}}"#, "9".repeat((1 << 20) + 1)),
        format!(r#"{{"enum":[[{}]]}}"#, "9".repeat((1 << 20) + 1)),
        format!(
            r#"{{"type":"integer","const":1{}}}"#,
            "0".repeat((1 << 20) + 1)
        ),
    ] {
        let schema = schema_from_str(&text);
        let emitted = canonicalize(&schema)
            .expect("canonicalize")
            .to_json_schema();
        assert_eq!(emitted, schema);
    }
}

#[cfg(feature = "arbitrary-precision")]
#[test]
fn canonical_number_normalizes_negative_zero() {
    assert_eq!(
        jsonschema::canonical::json::canonical_number("-0").as_deref(),
        Some("0")
    );
}

// An integer bound between `i64::MAX` and `u64::MAX` emits exactly through the `u64` projection.
#[cfg(feature = "arbitrary-precision")]
#[test]
fn canonicalize_integer_bound_in_u64_range_emits_exactly() {
    let schema = schema_from_str(r#"{"type":"integer","minimum":18446744073709551615}"#);
    let emitted = canonicalize(&schema)
        .expect("canonicalize")
        .to_json_schema();
    assert_eq!(emitted["minimum"], schema_from_str("18446744073709551615"));
}

#[test_case("const" ; "const_value")]
#[test_case("enum" ; "enum_value")]
fn too_deep_literal_values_preserved_raw(keyword: &str) {
    let literal = nested_array(256);
    let schema = match keyword {
        "const" => json!({"const": literal}),
        "enum" => json!({"enum": [literal]}),
        _ => unreachable!("test cases are fixed"),
    };
    let canonical = canonicalize(&schema).expect("deep literal canonicalizes");

    assert_eq!(canonical.kind(), CanonicalKind::Raw);
    assert_eq!(canonical.to_json_schema(), schema);
}

#[test]
fn too_deep_content_schema_preserved_raw() {
    let content_schema = nested_all_of_schema(256);
    let schema = json!({
        "$schema": DRAFT202012_SCHEMA_URI,
        "type": "string",
        "contentSchema": content_schema,
    });
    let canonical = canonicalize(&schema).expect("deep contentSchema canonicalizes");

    assert_eq!(canonical.kind(), CanonicalKind::Raw);
    assert_eq!(canonical.to_json_schema(), schema);
}

#[test]
fn raw_preserved_schema_skips_literal_payload_depth_checks() {
    let literal = nested_array(256);
    let schema = json!({
        "$schema": "https://example.com/custom-meta-schema",
        "const": literal,
    });

    let canonical = canonicalize(&schema).expect("canonicalize");

    assert_eq!(canonical.kind(), CanonicalKind::Raw);
    assert_eq!(canonical.to_json_schema(), schema);
}

// An external `$ref` target deeper than the schema depth limit must reach the Raw-preservation
// gate; scanning it for dynamic-scope refs must not overflow the stack first.
#[test]
fn very_deep_external_ref_target_preserved_without_overflow() {
    let mut target = json!({"type": "integer"});
    for _ in 0..20_000 {
        target = Value::Object(
            [("allOf".to_owned(), Value::Array(vec![target]))]
                .into_iter()
                .collect(),
        );
    }
    let registry = referencing::Registry::new()
        .add("https://example.com/deep", target)
        .expect("valid resource")
        .prepare()
        .expect("registry prepares");
    let root = json!({"$ref": "https://example.com/deep"});
    let canonical = jsonschema::canonical::options()
        .with_registry(&registry)
        .canonicalize(&root)
        .expect("canonicalize");
    assert!(canonical.is_satisfiable());
    // The registry's recursive Drop over the deep resource would overflow independently; leak it.
    std::mem::forget(registry);
}

// A document far deeper than serde_json's recursive serializer handles must still be preserved as Raw
// without overflowing the stack while serializing it.
#[test]
fn very_deep_document_preserved_raw_without_overflow() {
    // Build by moving the growing value into each new node; `json!` here would re-serialize it and
    // overflow during construction instead.
    let mut schema = json!({});
    for _ in 0..100_000 {
        schema = Value::Object(
            [("allOf".to_owned(), Value::Array(vec![schema]))]
                .into_iter()
                .collect(),
        );
    }
    let canonical = canonicalize(&schema).expect("deep document canonicalizes");
    assert_eq!(canonical.kind(), CanonicalKind::Raw);
    // The input's own recursive Drop would overflow independently of what this covers; leak it.
    std::mem::forget(schema);
}

#[cfg(all(feature = "resolve-async", not(target_arch = "wasm32")))]
#[tokio::test]
async fn async_raw_preserved_schema_skips_literal_payload_depth_checks() {
    let literal = nested_array(256);
    let schema = json!({
        "$schema": "https://example.com/custom-meta-schema",
        "const": literal,
    });

    let canonical = jsonschema::canonical::async_options()
        .canonicalize(&schema)
        .await
        .expect("canonicalize");

    assert_eq!(canonical.kind(), CanonicalKind::Raw);
    assert_eq!(canonical.to_json_schema(), schema);
}

/// Canonicalize an `allOf` of two string `format`s with format assertions enabled for `draft`, returning whether
/// the result is satisfiable.
fn format_pair_satisfiable(draft: Draft, left: &str, right: &str) -> bool {
    options()
        .with_draft(draft)
        .should_validate_formats(true)
        .canonicalize(&json!({
            "allOf": [
                {"type": "string", "format": left},
                {"type": "string", "format": right},
            ],
        }))
        .expect("canonicalize")
        .is_satisfiable()
}

// Distinct "rigid" formats have disjoint value sets, so intersecting two of them under format assertions is empty.
#[test_case("date", "date-time")]
#[test_case("date", "time")]
#[test_case("date", "duration")]
#[test_case("date", "email")]
#[test_case("date", "idn-email")]
#[test_case("date", "uuid")]
#[test_case("date", "ipv4")]
#[test_case("date", "ipv6")]
#[test_case("date-time", "time")]
#[test_case("date-time", "duration")]
#[test_case("date-time", "email")]
#[test_case("date-time", "uuid")]
#[test_case("date-time", "ipv4")]
#[test_case("date-time", "ipv6")]
#[test_case("time", "duration")]
#[test_case("time", "email")]
#[test_case("time", "uuid")]
#[test_case("time", "ipv4")]
#[test_case("time", "ipv6")]
#[test_case("duration", "email")]
#[test_case("duration", "idn-email")]
#[test_case("duration", "uuid")]
#[test_case("duration", "ipv4")]
#[test_case("duration", "ipv6")]
#[test_case("email", "uuid")]
#[test_case("email", "ipv4")]
#[test_case("email", "ipv6")]
#[test_case("idn-email", "uuid")]
#[test_case("idn-email", "ipv4")]
#[test_case("idn-email", "ipv6")]
#[test_case("uuid", "ipv4")]
#[test_case("uuid", "ipv6")]
#[test_case("ipv4", "ipv6")]
fn rigid_format_pairs_are_disjoint_under_assertions(left: &str, right: &str) {
    assert!(
        !format_pair_satisfiable(Draft::Draft202012, left, right),
        "no string is both `{left}` and `{right}`, so their intersection must be empty",
    );
}

// `duration` and `uuid` are only defined from draft 2019-09. Under draft-07 they are unrecognized annotations that
// every string satisfies, so disjointness must not be claimed; under 2020-12 they are real assertions and disjoint.
#[test]
fn pre_2019_drafts_do_not_assert_duration_or_uuid() {
    assert!(format_pair_satisfiable(Draft::Draft7, "duration", "uuid"));
    assert!(format_pair_satisfiable(Draft::Draft7, "uuid", "email"));
    assert!(format_pair_satisfiable(Draft::Draft7, "duration", "date"));
    assert!(!format_pair_satisfiable(
        Draft::Draft202012,
        "duration",
        "uuid"
    ));
    assert!(!format_pair_satisfiable(
        Draft::Draft202012,
        "uuid",
        "email"
    ));
}

#[test]
fn mixed_draft_intersection_emits_draft_that_preserves_newer_keywords() {
    let draft4_array = options()
        .with_draft(Draft::Draft4)
        .canonicalize(&json!({"type": "array"}))
        .expect("canonicalize draft4");
    let contains_one = options()
        .with_draft(Draft::Draft202012)
        .canonicalize(&json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "array",
            "contains": {"const": 1},
        }))
        .expect("canonicalize draft2020");
    let intersection = draft4_array.intersect(&contains_one);
    let emitted = intersection.to_json_schema();
    let validator = jsonschema::validator_for(&emitted).expect("intersection compiles");

    assert_eq!(intersection.draft(), Draft::Draft202012);
    assert!(
        !validator.is_valid(&json!([])),
        "intersection emitted under a draft that ignores contains: {emitted}"
    );
    assert!(validator.is_valid(&json!([1])));
}

fn assert_intersection_matches_operand_format_semantics(
    left: &jsonschema::canonical::CanonicalSchema,
    right: &jsonschema::canonical::CanonicalSchema,
) {
    for (label, intersection) in [
        ("left then right", left.intersect(right)),
        ("right then left", right.intersect(left)),
    ] {
        assert!(
            intersection.is_satisfiable(),
            "{label} collapsed a satisfiable format intersection: {}",
            intersection.to_json_schema()
        );
        let emitted = intersection.to_json_schema();
        let validator = jsonschema::options()
            .with_draft(Draft::Draft202012)
            .should_validate_formats(true)
            .build(&emitted)
            .expect("intersection compiles");

        assert!(
            validator.is_valid(&json!("user@example.com")),
            "{label} reasserted an annotation-only format: {emitted}",
        );
        assert!(
            !validator.is_valid(&json!("550e8400-e29b-41d4-a716-446655440000")),
            "{label} dropped the asserted email format: {emitted}",
        );
    }
}

#[test]
fn mixed_draft_intersection_preserves_operand_format_semantics() {
    let draft7_uuid_annotation = options()
        .with_draft(Draft::Draft7)
        .canonicalize(&json!({"type": "string", "format": "uuid"}))
        .expect("canonicalize draft7");
    let asserted_email = options()
        .with_draft(Draft::Draft202012)
        .should_validate_formats(true)
        .canonicalize(&json!({"type": "string", "format": "email"}))
        .expect("canonicalize draft2020");

    assert_intersection_matches_operand_format_semantics(&draft7_uuid_annotation, &asserted_email);
}

#[test]
fn mixed_format_assertion_settings_preserve_operand_format_semantics() {
    let uuid_annotation = options()
        .with_draft(Draft::Draft202012)
        .should_validate_formats(false)
        .canonicalize(&json!({"type": "string", "format": "uuid"}))
        .expect("canonicalize annotation");
    let asserted_email = options()
        .with_draft(Draft::Draft202012)
        .should_validate_formats(true)
        .canonicalize(&json!({"type": "string", "format": "email"}))
        .expect("canonicalize assertion");

    assert_intersection_matches_operand_format_semantics(&uuid_annotation, &asserted_email);
}

#[test]
fn format_assertion_branch_is_a_resource_root() {
    let emitted = options()
        .with_draft(Draft::Draft202012)
        .should_validate_formats(true)
        .canonicalize(&json!({"type": "string", "format": "email"}))
        .expect("canonicalize")
        .to_json_schema();
    assert_eq!(
        emitted["allOf"][0],
        json!({
            "$id": "urn:jsonschema:format-assertion:email",
            "$schema": "http://json-schema.org/draft-07/schema#",
            "format": "email"
        })
    );
}

#[test]
fn asserted_email_self_asserts_under_plain_validator() {
    let emitted = options()
        .with_draft(Draft::Draft202012)
        .should_validate_formats(true)
        .canonicalize(&json!({"type": "string", "format": "email"}))
        .expect("canonicalize")
        .to_json_schema();
    let validator = jsonschema::validator_for(&emitted).expect("compiles");
    assert!(validator.is_valid(&json!("user@example.com")), "{emitted}");
    assert!(!validator.is_valid(&json!("not-an-email")), "{emitted}");
}

#[test]
fn repeated_asserted_format_still_compiles() {
    let emitted = options()
        .with_draft(Draft::Draft202012)
        .should_validate_formats(true)
        .canonicalize(&json!({
            "type": "object",
            "properties": {
                "a": {"type": "string", "format": "email"},
                "b": {"type": "string", "format": "email"}
            }
        }))
        .expect("canonicalize")
        .to_json_schema();
    jsonschema::validator_for(&emitted)
        .unwrap_or_else(|error| panic!("emitted rejected: {error}\n{emitted}"));
}

#[test]
fn intersect_disambiguates_transitive_symbolic_definitions() {
    let canonical = |schema: Value| {
        options()
            .with_inline_budget(0)
            .canonicalize(&schema)
            .expect("canonicalize")
    };
    let left = canonical(json!({
        "$ref": "#/$defs/A",
        "$defs": {
            "A": {"$ref": "#/$defs/B"},
            "B": {"type": "string"},
        },
    }));
    let right = canonical(json!({
        "$ref": "#/$defs/A",
        "$defs": {
            "A": {"$ref": "#/$defs/B"},
            "B": {"type": "integer"},
        },
    }));

    // Intersect both orders: swapping operands flips the source-map order the rename tiebreak maps each
    // side through, so the result must reject the same witnesses (disambiguation is order-independent).
    for (label, intersection) in [
        ("forward", left.intersect(&right).to_json_schema()),
        ("backward", right.intersect(&left).to_json_schema()),
    ] {
        let validator = jsonschema::validator_for(&intersection).expect("intersection compiles");
        assert!(
            !validator.is_valid(&json!("x")),
            "{label} intersection accepted the left-only string witness: {intersection}"
        );
        assert!(
            !validator.is_valid(&json!(1)),
            "{label} intersection accepted the right-only integer witness: {intersection}"
        );
    }
}

// Under arbitrary precision instances are exact decimals: a window between adjacent doubles
// still contains infinitely many numbers, so it must not collapse to the f64-projected `const`.
#[cfg(feature = "arbitrary-precision")]
#[test_case(
    r#"{"type":"number","minimum":0,"exclusiveMaximum":5e-324}"#,
    "2.5e-324"
    ; "subnormal_window"
)]
#[test_case(
    r#"{"type":"number","exclusiveMinimum":1.5,"maximum":1.5000000000000002}"#,
    "1.50000000000000001"
    ; "next_up_window"
)]
fn float_window_keeps_arbitrary_precision_witnesses(schema: &str, witness: &str) {
    assert_parity_against_expected(
        schema_from_str(schema),
        &[(serde_json::from_str(witness).expect("valid number"), true)],
    );
}

// Multiple indices past i64 must decline the boundary-exclusion fold: saturated indices made
// thousands of interior multiples count as boundary-only, deleting the exclusion.
#[test]
#[allow(
    clippy::excessive_precision,
    reason = "f64 spacing at 1e15 is exactly 0.125, so the fraction is exact, not excess digits"
)]
fn fractional_not_multiple_of_with_indices_past_i64_keeps_exclusion() {
    assert_parity_against_expected(
        json!({
            "type": "number",
            "minimum": 1_000_000_000_000_000_i64,
            "maximum": 1_000_000_000_000_001_i64,
            "not": {"multipleOf": 0.0001}
        }),
        &[
            (json!(1_000_000_000_000_000.5), false),
            (json!(1_000_000_000_000_000.125), false),
        ],
    );
}

// Grid factor indices past i64 must decline the finite respelling instead of enumerating an
// empty grid and collapsing a satisfiable leaf to `False`.
#[test]
fn fractional_grid_with_factor_indices_past_i64_stays_satisfiable() {
    assert_parity_against_expected(
        json!({
            "type": "number",
            "minimum": -6_000_000_000_000_000_000_i64,
            "maximum": -5_900_000_000_000_000_000_i64,
            "multipleOf": 0.5
        }),
        &[
            (json!(-5_950_000_000_000_000_000_i64), true),
            (json!(0.5), false),
        ],
    );
}

// A modulus-coverage ratio whose cross products overflow the default fraction carrier must
// decline coverage (keep the branch), not wrap into a bogus whole-number ratio.
#[test]
fn multiple_of_coverage_with_overflowing_ratio_keeps_branch() {
    assert_parity_against_expected(
        json!({"anyOf": [
            {"type": "number", "multipleOf": 0.3},
            {"type": "number", "multipleOf": 2_000_000_000_000_000_002_i64}
        ]}),
        &[
            (json!(2_000_000_000_000_000_002_i64), true),
            (json!(0.3), true),
            (json!(0.1), false),
        ],
    );
}

// A numeric bound whose plain expansion exceeds the digit cap has no exact in-cap spelling; the
// document stays raw instead of the bound emitting as a megabyte integer literal.
#[cfg(feature = "arbitrary-precision")]
#[test]
fn past_cap_numeric_bound_stays_raw() {
    let mantissa = "9".repeat(50_000);
    let text = format!(r#"{{"type":"number","minimum":{mantissa}e999999}}"#);
    let schema = schema_from_str(&text);
    let emitted = canonicalize(&schema)
        .expect("canonicalize")
        .to_json_schema();
    assert!(emitted == schema, "past-cap bound did not stay raw");
}

// A `uniqueItems` cap over an integer window spanning more than `i64` values must count the
// universe exactly in both feature configurations; an overflowing span computation drops the
// `maxItems` cap in one build but not the other.
#[test]
fn unique_items_cap_over_integer_window_past_i64_span() {
    let schema = json!({
        "type": "array",
        "uniqueItems": true,
        "items": {"type": "integer", "minimum": -1, "maximum": 9_223_372_036_854_775_807_i64}
    });
    let emitted = canonicalize(&schema)
        .expect("canonicalize")
        .to_json_schema();
    assert_eq!(
        emitted,
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "array",
            "uniqueItems": true,
            "items": {"type": "integer", "minimum": -1, "maximum": 9_223_372_036_854_775_807_i64},
            "maxItems": 9_223_372_036_854_775_809_u64
        })
    );
}

// A merged `multipleOf` that cannot be emitted exactly (needs more precision than `f64` carries)
// must decline the merge: the rounded emission enforces a different grid than the exact LCM.
#[test]
fn multiple_of_merge_past_f64_precision_stays_unmerged() {
    assert_parity_against_expected(
        json!({"allOf": [{"multipleOf": 200_000_001.1}, {"multipleOf": 300_000_001.9}]}),
        &[
            (json!(600_000_007_100_000_000_i64), false),
            (json!(200_000_001.1), false),
        ],
    );
}

// A type-set intersected with a fractional value-set body must keep the `integer` pin so the
// fractional member stays rejected under the narrowed kind.
#[test]
fn type_set_intersection_keeps_integer_pin_over_fractional_const() {
    assert_parity_against_expected(
        json!({"allOf": [{"type": ["integer", "string"]}, {"minimum": 1.5, "maximum": 1.5}]}),
        &[(json!(1.5), false), (json!("x"), true), (json!(2), false)],
    );
}

#[test]
fn type_set_intersection_keeps_integer_pin_over_fractional_enum() {
    assert_parity_against_expected(
        json!({"allOf": [{"type": ["integer", "string"]}, {"minimum": 1, "maximum": 2, "multipleOf": 0.5}]}),
        &[
            (json!(1.5), false),
            (json!(1), true),
            (json!(2), true),
            (json!("x"), true),
        ],
    );
}

// A subsumption check on an exclusive endpoint at the representable extreme must not overflow and
// delete a live sibling branch.
#[test]
fn exclusive_maximum_at_i64_min_does_not_swallow_siblings() {
    assert_parity_against_expected(
        json!({"anyOf": [
            {"type": "integer", "exclusiveMaximum": i64::MIN},
            {"type": "integer", "minimum": 0, "maximum": 5}
        ]}),
        &[(json!(3), true), (json!(-1), false)],
    );
}

#[test]
fn exclusive_minimum_at_i64_max_does_not_swallow_siblings() {
    assert_parity_against_expected(
        json!({"anyOf": [
            {"type": "integer", "exclusiveMinimum": i64::MAX},
            {"type": "integer", "minimum": 0, "maximum": 5}
        ]}),
        &[(json!(3), true), (json!(6), false)],
    );
}

// A `TypeGuard` branch is satisfied by any value outside its guarded type, so a recursive schema
// behind one is productive and must canonicalize rather than report infinite recursion.
#[test]
fn type_guard_branch_is_always_productive() {
    assert_parity_against_expected(
        json!({"allOf": [
            {"minProperties": 1, "additionalProperties": false},
            {"properties": {"c": {"$ref": "#"}}}
        ]}),
        &[(json!(42), true)],
    );
}

// A synthesized shared-definition name must not collide with a bare-anchor `$defs` key, or the
// emitted `{"$ref": "#name"}` dangles and the schema fails to compile.
#[test]
fn shared_definition_names_never_collide_with_anchor_definitions() {
    let mut props = serde_json::Map::new();
    for i in 0..300 {
        props.insert(
            format!("p{i}"),
            json!({"type": "string", "minLength": 1, "maxLength": 100, "pattern": format!("^p{i}")}),
        );
    }
    let big = json!({"type": "object", "properties": props, "required": ["p0"]});
    let schema = json!({
        "$anchor": "shared0",
        "type": "object",
        "properties": {
            "own": {"$ref": "#shared0"},
            "b": {"$ref": "#/$defs/big"},
            "c": {"$ref": "#/$defs/big"},
        },
        "$defs": {"big": big}
    });
    let canonical = canonicalize(&schema).expect("canonicalize");
    let emitted = canonical.negate().to_json_schema();
    jsonschema::validator_for(&emitted)
        .unwrap_or_else(|error| panic!("emitted schema must compile: {error}\n{emitted:#}"));
}

// A format with no pre-2019 pivot draft cannot carry an assertion via `$schema`, so it emits the plain
// keyword rather than a vacuous non-asserting `allOf` branch.
#[test]
fn format_without_pivot_draft_emits_plain_keyword() {
    let canonical = options()
        .with_draft(Draft::Draft202012)
        .should_validate_formats(true)
        .canonicalize(&json!({"type": "string", "format": "duration"}))
        .expect("canonicalize");
    assert_eq!(
        canonical.to_json_schema(),
        json!({"$schema": DRAFT202012_SCHEMA_URI, "type": "string", "format": "duration"})
    );
}

// Deep numeric partition-cover: the union-coverage remainder needs numeric simplification (`A ∧ const0 = {0}`).
#[test]
fn subtract_numeric_partition_cover_is_unsatisfiable() {
    let x = canonicalize(&json!({"not": {"allOf": [
        {"not": {"type": "integer", "minimum": 0, "maximum": 0}},
        {"anyOf": [
            {"type": "number", "multipleOf": 4},
            {"type": "integer", "minimum": 2, "maximum": 4}
        ]}
    ]}}))
    .expect("canonicalize");
    assert!(!x.subtract(&x).is_satisfiable());
}

// A `pattern` compiled under a non-default regex engine with explicit size limits still resolves membership: `^a`
// excludes `"abc"`, leaving `"xyz"`.
#[test]
fn regex_engine_size_limits_compile_pattern() {
    let schema = json!({
        "smuggled": {"type": "string", "pattern": "^a"},
        "allOf": [{"type": "string"}, {"not": {"$ref": "#/smuggled"}}, {"enum": ["abc", "xyz"]}]
    });
    let canonical = options()
        .with_pattern_options(
            &PatternOptions::regex()
                .size_limit(1 << 20)
                .dfa_size_limit(1 << 20),
        )
        .canonicalize(&schema)
        .expect("canonicalize");
    assert_eq!(canonical.to_json_schema()["const"], json!("xyz"));
}

#[test]
fn fancy_regex_engine_limits_compile_pattern() {
    let schema = json!({
        "smuggled": {"type": "string", "pattern": "^a"},
        "allOf": [{"type": "string"}, {"not": {"$ref": "#/smuggled"}}, {"enum": ["abc", "xyz"]}]
    });
    let canonical = options()
        .with_pattern_options(
            &PatternOptions::fancy_regex()
                .backtrack_limit(100_000)
                .size_limit(1 << 20)
                .dfa_size_limit(1 << 20),
        )
        .canonicalize(&schema)
        .expect("canonicalize");
    assert_eq!(canonical.to_json_schema()["const"], json!("xyz"));
}

/// Retriever serving exactly one external document; any other URI is an error.
struct SingleDocumentRetriever {
    uri: &'static str,
    document: Value,
}

impl jsonschema::Retrieve for SingleDocumentRetriever {
    fn retrieve(
        &self,
        uri: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        if uri.as_str() == self.uri {
            Ok(self.document.clone())
        } else {
            Err(format!("unknown uri: {uri}").into())
        }
    }
}

#[test]
fn relative_root_id_combines_with_base_for_sibling_relative_ref() {
    // The root `$id` is relative, so the initial base stays the synthetic fallback; the resource's `$id` is
    // combined with it during registry preparation, and a relative `$ref` must resolve against that combined
    // base (`file:///sub/`), not the bare fallback.
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$id": "sub/schema.json",
        "type": "object",
        "properties": {"x": {"$ref": "integer.json"}}
    });
    let canonical = options()
        .with_retriever(SingleDocumentRetriever {
            uri: "file:///sub/integer.json",
            document: json!({"type": "integer"}),
        })
        .canonicalize(&schema)
        .expect("canonicalize");
    let emitted = canonical.to_json_schema();
    let validator = jsonschema::validator_for(&emitted)
        .unwrap_or_else(|error| panic!("emitted schema rejected: {error}\n{emitted}"));
    assert!(validator.is_valid(&json!({"x": 1})), "{emitted}");
    assert!(!validator.is_valid(&json!({"x": "s"})), "{emitted}");
}

#[test]
fn relative_id_scope_applied_once_when_ref_target_carries_it() {
    // `$ref: "baseUriChangeFolder/"` resolves to a subresource whose own relative `$id` established
    // that base; entering the scope again would double it (`.../baseUriChangeFolder/baseUriChangeFolder/`)
    // and the inner ref would dangle.
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "http://localhost:1234/draft2020-12/scope_change_defs1.json",
        "type": "object",
        "properties": {"list": {"$ref": "baseUriChangeFolder/"}},
        "$defs": {
            "baz": {
                "$id": "baseUriChangeFolder/",
                "type": "array",
                "items": {"$ref": "folderInteger.json"}
            }
        }
    });
    let canonical = options()
        .with_retriever(SingleDocumentRetriever {
            uri: "http://localhost:1234/draft2020-12/baseUriChangeFolder/folderInteger.json",
            document: json!({"type": "integer"}),
        })
        .canonicalize(&schema)
        .expect("canonicalize");
    let emitted = canonical.to_json_schema();
    let validator = jsonschema::validator_for(&emitted)
        .unwrap_or_else(|error| panic!("emitted schema rejected: {error}\n{emitted}"));
    assert!(validator.is_valid(&json!({"list": [1, 2]})));
    assert!(!validator.is_valid(&json!({"list": ["x"]})), "{emitted}");
}

#[test]
fn local_pointer_through_relative_id_scope_uses_target_scope() {
    // The pointer walks through `baz`, whose relative `$id` changes the base; the target's inner
    // relative ref must resolve against that base, not the document root's.
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$id": "http://localhost:1234/scope_change_defs2.json",
        "type": "object",
        "properties": {
            "list": {"$ref": "#/definitions/baz/definitions/bar"}
        },
        "definitions": {
            "baz": {
                "$id": "baseUriChangeFolderInSubschema/",
                "definitions": {
                    "bar": {
                        "type": "array",
                        "items": {"$ref": "folderInteger.json"}
                    }
                }
            }
        }
    });
    let canonical = options()
        .with_retriever(SingleDocumentRetriever {
            uri: "http://localhost:1234/baseUriChangeFolderInSubschema/folderInteger.json",
            document: json!({"type": "integer"}),
        })
        .canonicalize(&schema)
        .expect("canonicalize");
    let emitted = canonical.to_json_schema();
    let validator = jsonschema::validator_for(&emitted)
        .unwrap_or_else(|error| panic!("emitted schema rejected: {error}\n{emitted}"));
    assert!(validator.is_valid(&json!({"list": [1, 2]})));
    assert!(!validator.is_valid(&json!({"list": ["x"]})), "{emitted}");
}

fn def_ref(node: Value) -> Value {
    json!({"$defs": {"node": node}, "$ref": "#/$defs/node"})
}

// A long linear `$ref` chain is valid and terminating, but each hop used to recurse one native frame in both
// the recursion check (now iterative) and the parser (now depth-bounded), overflowing the stack as an
// uncatchable abort. Canonicalization must complete; past the bound the schema is preserved verbatim.
#[test]
fn long_ref_chain_does_not_overflow_the_stack() {
    const N: usize = 5_000;
    let mut defs = serde_json::Map::new();
    for index in 0..N {
        let body = if index + 1 == N {
            json!({"type": "string"})
        } else {
            json!({"$ref": format!("#/$defs/n{}", index + 1)})
        };
        defs.insert(format!("n{index}"), body);
    }
    let schema = json!({
        "$schema": DRAFT202012_SCHEMA_URI,
        "$ref": "#/$defs/n0",
        "$defs": defs
    });
    let canonical = canonicalize(&schema).expect("long chain canonicalizes without overflow");
    // Beyond the ref-depth bound the whole schema is preserved verbatim rather than inlined.
    assert_eq!(canonical.kind(), CanonicalKind::Raw);
    assert_eq!(canonical.to_json_schema(), schema);
}

// The same composing keywords as the `InfiniteRecursion` suite cases, but with a finite base case, must canonicalize.
#[test_case(json!({"allOf": [
        {"type": "object", "properties": {"child": {"$ref": "#/$defs/node"}}},
        {"type": "object"}]}) ; "allOf where every member is finite")]
#[test_case(json!({"oneOf": [
        {"type": "string"},
        {"type": "object", "required": ["child"], "properties": {"child": {"$ref": "#/$defs/node"}}}]}) ; "oneOf with a finite branch")]
#[test_case(json!({
        "if": {"type": "object"},
        "then": {"type": "object", "required": ["child"], "properties": {"child": {"$ref": "#/$defs/node"}}}}) ; "if/then without an else arm")]
#[test_case(json!({"type": "array", "contains": {"$ref": "#/$defs/node"}, "minContains": 0}) ; "contains with minContains zero")]
#[test_case(json!({"anyOf": [
        {"type": "array", "minItems": 1, "prefixItems": [{"type": "integer"}], "items": {"$ref": "#/$defs/node"}},
        {"type": "string"}]}) ; "forced prefix position is finite")]
#[test_case(json!({"type": "object", "required": ["x", "y"],
        "properties": {"x": {"type": "string"}, "y": {"type": "integer"}, "child": {"$ref": "#/$defs/node"}}}) ; "required properties with finite constraints")]
#[test_case(json!({"type": "object", "minProperties": 1,
        "properties": {"child": {"$ref": "#/$defs/node"}}}) ; "non-required requirement with an optional recursive child")]
#[test_case(json!({"type": "object", "required": ["ghost"],
        "properties": {"child": {"$ref": "#/$defs/node"}}}) ; "required property with no matching constraint is unconstrained")]
#[test_case(json!({"type": "object", "required": ["a"],
        "properties": {"a": {"type": "string"}}, "additionalProperties": {"$ref": "#/$defs/node"}}) ; "named property shields required name from recursive additionalProperties")]
#[test_case(json!({"type": "object", "required": ["a"],
        "patternProperties": {"^a$": {"type": "string"}}}) ; "matching patternProperties gives required name a finite base case")]
#[test_case(json!({"type": "object", "required": ["a"],
        "patternProperties": {"^b": {"$ref": "#/$defs/node"}}, "additionalProperties": {"type": "string"}}) ; "non-matching recursive pattern does not block required name")]
fn canonicalize_accepts_recursion_with_base_case(node: Value) {
    canonicalize(&def_ref(node)).expect("finite recursion canonicalizes");
}

// Under 2019-09+ a `$ref` is evaluated alongside its composition siblings, so an `allOf` self-cycle beside a
// `$ref` is an unguarded cycle and must be reported as such - not deferred to the infinite-recursion backstop.
#[test]
fn unguarded_recursion_through_ref_sibling() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "#/$defs/a",
        "$defs": {
            "a": {"$ref": "#/$defs/terminal", "allOf": [{"$ref": "#/$defs/a"}]},
            "terminal": {"type": "integer"}
        }
    });
    assert!(
        matches!(
            canonicalize(&schema),
            Err(CanonicalizationError::UnguardedRecursion(_))
        ),
        "expected unguarded recursion, got {:?}",
        canonicalize(&schema)
    );
}

// Draft 7 treats `$ref` as exclusive, so the `allOf` sibling is ignored and no cycle exists.
#[test]
fn ref_sibling_cycle_ignored_under_exclusive_ref_draft() {
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$ref": "#/$defs/a",
        "$defs": {
            "a": {"$ref": "#/$defs/terminal", "allOf": [{"$ref": "#/$defs/a"}]},
            "terminal": {"type": "integer"}
        }
    });
    canonicalize(&schema).expect("exclusive `$ref` ignores the `allOf` sibling, so no cycle");
}

#[test]
fn exclusive_ref_suppresses_sibling_edge_on_guarded_node() {
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "properties": {"x": {"$ref": "#/definitions/a", "allOf": [{"$ref": "#/properties/x"}]}},
        "definitions": {"a": {"type": "string"}}
    });
    canonicalize(&schema)
        .expect("exclusive `$ref` ignores the guarded node's `allOf` sibling, so no cycle");
}

// A bare `$ref` node first reached through a guarded position (a `properties` child, so its own `$ref` is
// suppressed) and later reached as an unguarded `$ref` target still contributes that now-unguarded edge, so
// the `b <-> a/properties/p` cycle is detected rather than left to the infinite-recursion backstop.
#[test]
fn unguarded_recursion_through_guarded_then_targeted_ref() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "allOf": [{"$ref": "#/$defs/a"}, {"$ref": "#/$defs/b"}],
        "$defs": {
            "a": {"type": "object", "properties": {"p": {"$ref": "#/$defs/b"}}},
            "b": {"$ref": "#/$defs/a/properties/p"}
        }
    });
    assert!(
        matches!(
            canonicalize(&schema),
            Err(CanonicalizationError::UnguardedRecursion(_))
        ),
        "expected unguarded recursion, got {:?}",
        canonicalize(&schema)
    );
}

#[test]
fn repeated_items_ray_with_covering_sibling_converges() {
    let left = json!({"$defs": {"shared": {"anyOf": [{"oneOf": [{"allOf": [{"type": "string"}, {"pattern": "^a", "type": "string"}]}, {"anyOf": [{"maxItems": 2, "minItems": 1, "type": "array"}, {"enum": [null]}, {"pattern": "b$", "type": "string"}]}]}, {"allOf": [{"type": "null"}]}, {"not": {"uniqueItems": true}}]}}, "properties": {"a": {"$ref": "#/$defs/shared"}, "b": {"$ref": "#/$defs/shared"}}, "type": "object"});
    let right = json!({"$defs": {"shared": {"type": "null"}}, "properties": {"a": {"$ref": "#/$defs/shared"}, "b": {"$ref": "#/$defs/shared"}}, "type": "object"});
    let intersection = canonicalize(&left)
        .expect("left canonicalizes")
        .intersect(&canonicalize(&right).expect("right canonicalizes"));
    let _ = intersection.to_json_schema();
}

fn registry_with(resources: &[(&str, Value)]) -> Registry<'static> {
    let mut builder = Registry::new();
    for (uri, value) in resources {
        builder = builder.add(*uri, value.clone()).expect("valid resource");
    }
    builder.prepare().expect("registry prepares")
}

#[test]
fn external_ref_uses_resolved_document_draft() {
    // A 2020-12 schema refs a 2019-09 document. `prefixItems` is unknown in 2019-09 and must be ignored, not applied
    // as a 2020-12 keyword: an array whose first element is a non-string still validates.
    let registry = Registry::new()
        .draft(Draft::Draft201909)
        .add(
            "http://localhost:1234/doc",
            json!({"prefixItems": [{"type": "string"}]}),
        )
        .expect("valid resource")
        .prepare()
        .expect("registry prepares");
    let emitted = options()
        .with_draft(Draft::Draft202012)
        .with_registry(&registry)
        .with_base_uri("http://localhost:1234/main")
        .canonicalize(&json!({"$ref": "http://localhost:1234/doc"}))
        .expect("canonicalize")
        .to_json_schema();
    let validator = jsonschema::validator_for(&emitted)
        .unwrap_or_else(|error| panic!("emitted schema rejected: {error}\n{emitted}"));
    assert!(validator.is_valid(&json!([1, 2, 3])), "{emitted}");
}

// An external `$ref` whose target carries a `$dynamicRef` resolves the dynamic anchor in the target
// document's scope; canonicalize must keep the reference symbolic, not inline the fragment (which detaches
// the anchor and emits an unresolvable `$dynamicRef`).
#[test]
fn external_ref_to_dynamic_ref_fragment_stays_resolvable() {
    let registry = registry_with(&[(
        "http://localhost:1234/detached.json",
        json!({
            "$id": "http://localhost:1234/detached.json",
            "$defs": {
                "foo": {"$dynamicRef": "#detached"},
                "detached": {"$dynamicAnchor": "detached", "type": "integer"}
            }
        }),
    )]);
    let emitted = options()
        .with_registry(&registry)
        .with_base_uri("http://localhost:1234/main")
        .canonicalize(&json!({"$ref": "http://localhost:1234/detached.json#/$defs/foo"}))
        .expect("canonicalize")
        .to_json_schema();
    let validator = jsonschema::options()
        .with_registry(&registry)
        .build(&emitted)
        .unwrap_or_else(|error| panic!("emitted schema rejected: {error}\n{emitted}"));
    assert!(validator.is_valid(&json!(1)), "{emitted}");
    assert!(!validator.is_valid(&json!("x")), "{emitted}");
}

#[test]
fn relative_ref_cache_keyed_by_resolved_target_not_raw_string() {
    // Both docs use `$ref: "child"` under different bases. Caching by the raw string would collapse them to whichever
    // resolved first; the intersection of the two children (integer and string) must be unsatisfiable.
    let registry = registry_with(&[
        ("https://example.com/a/doc", json!({"$ref": "child"})),
        ("https://example.com/a/child", json!({"type": "integer"})),
        ("https://example.com/b/doc", json!({"$ref": "child"})),
        ("https://example.com/b/child", json!({"type": "string"})),
    ]);
    let emitted = options()
        .with_registry(&registry)
        .with_base_uri("https://example.com/main")
        .canonicalize(&json!({"allOf": [
            {"$ref": "https://example.com/a/doc"},
            {"$ref": "https://example.com/b/doc"}
        ]}))
        .expect("canonicalize")
        .to_json_schema();
    let validator = jsonschema::validator_for(&emitted)
        .unwrap_or_else(|error| panic!("emitted schema rejected: {error}\n{emitted}"));
    // integer AND string is empty: a raw-string cache collision would wrongly admit one of them.
    assert!(
        !validator.is_valid(&json!(1)),
        "integer wrongly admitted: {emitted}"
    );
    assert!(
        !validator.is_valid(&json!("x")),
        "string wrongly admitted: {emitted}"
    );
}

#[test]
fn external_ref_cycle_canonicalizes_and_validates_recursively() {
    // Cross-document A -> B -> A must terminate and emit a self-contained recursive schema.
    let registry = registry_with(&[
        (
            "https://example.com/a",
            json!({"type": "object", "properties": {"next": {"$ref": "https://example.com/b"}}}),
        ),
        (
            "https://example.com/b",
            json!({"type": "object", "properties": {"prev": {"$ref": "https://example.com/a"}}}),
        ),
    ]);
    let emitted = options()
        .with_registry(&registry)
        .with_base_uri("https://example.com/main")
        .canonicalize(&json!({"$ref": "https://example.com/a"}))
        .expect("cross-document cycle canonicalizes")
        .to_json_schema();
    let validator = jsonschema::validator_for(&emitted)
        .unwrap_or_else(|error| panic!("emitted schema rejected: {error}\n{emitted}"));
    assert!(validator.is_valid(&json!({})), "{emitted}");
    assert!(
        validator.is_valid(&json!({"next": {"prev": {"next": {}}}})),
        "{emitted}"
    );
    assert!(!validator.is_valid(&json!({"next": 5})), "{emitted}");
}

// A `$ref` to an external document inside a composition is inlined and merged with its sibling.
#[test]
fn external_ref_inside_composition_gets_inlined() {
    let registry = registry_with(&[("https://example.com/other", json!({"minimum": 0}))]);
    let emitted = options()
        .with_registry(&registry)
        .canonicalize(&json!({"allOf": [
            {"$ref": "https://example.com/other"},
            {"type": "integer"}
        ]}))
        .expect("canonicalize with registry")
        .to_json_schema();
    assert_eq!(
        emitted,
        json!({"$schema": DRAFT202012_SCHEMA_URI, "type": "integer", "minimum": 0})
    );
}

// A `$recursiveRef` pulled in through an external `$ref` resolves against the runtime dynamic scope, which
// canonicalization cannot reproduce, so the schema is preserved verbatim: canonicalize succeeds and is idempotent
// rather than wrongly reducing the dynamic reference away.
#[test]
fn dynamic_ref_through_external_ref_is_preserved() {
    let registry = registry_with(&[(
        "https://example.com/recursive",
        json!({"properties": {"self": {"$recursiveRef": "#"}}}),
    )]);
    let canonicalize_once = || {
        options()
            .with_draft(Draft::Draft201909)
            .with_registry(&registry)
            .canonicalize(&json!({"$ref": "https://example.com/recursive"}))
            .expect("canonicalize")
            .to_json_schema()
    };
    let once = canonicalize_once();
    let twice = options()
        .with_draft(Draft::Draft201909)
        .canonicalize(&once)
        .expect("re-canonicalize")
        .to_json_schema();
    assert_eq!(once, twice, "dynamic-ref preservation must be idempotent");
}

// Registering the root means `prepare` crawls its external refs; an unretrievable one cannot be resolved, so the
// schema is preserved verbatim instead of erroring.
#[test]
fn unresolvable_external_ref_is_preserved() {
    let registry = registry_with(&[("https://example.com/other", json!({"type": "integer"}))]);
    options()
        .with_registry(&registry)
        .canonicalize(&json!({"$ref": "https://example.com/unknown"}))
        .expect("unresolvable external ref is preserved, not an error");
}

// An external target whose numeric bound exceeds the supported range must be preserved verbatim, like the
// root path does, rather than saturated to `i64::MAX` (which would shift the accepted set).
#[test]
fn external_ref_out_of_range_numeric_bound_preserved_raw() {
    let registry = registry_with(&[(
        "https://example.com/ext",
        json!({"type": "integer", "maximum": 9_223_372_036_854_775_808u64}),
    )]);
    let emitted = options()
        .with_registry(&registry)
        .with_base_uri("https://example.com/main")
        .canonicalize(&json!({"$ref": "https://example.com/ext"}))
        .expect("canonicalize")
        .to_json_schema();
    let validator = jsonschema::validator_for(&emitted)
        .unwrap_or_else(|error| panic!("emitted schema rejected: {error}\n{emitted}"));
    // 2^63 satisfies `maximum: 2^63`; a bound saturated to 2^63 - 1 would wrongly reject it.
    assert!(
        validator.is_valid(&json!(9_223_372_036_854_775_808u64)),
        "{emitted}"
    );
}

// An external target carrying a `$dynamicRef` is kept as a symbolic external reference (never inlined or
// raw-preserved into the referrer, which would detach its dynamic anchor); it resolves at validation and
// canonicalize does not reach the `unreachable!()` in emit.
#[test]
fn external_ref_dynamic_ref_target_kept_symbolic() {
    let registry = registry_with(&[(
        "https://example.com/ext",
        json!({
            "$dynamicAnchor": "m",
            "type": "object",
            "properties": {"self": {"$dynamicRef": "#m"}}
        }),
    )]);
    let emitted = options()
        .with_draft(Draft::Draft202012)
        .with_registry(&registry)
        .with_base_uri("https://example.com/main")
        .with_inline_budget(0)
        .canonicalize(&json!({"$ref": "https://example.com/ext"}))
        .expect("canonicalize does not panic")
        .to_json_schema();
    let validator = jsonschema::options()
        .with_draft(Draft::Draft202012)
        .with_registry(&registry)
        .build(&emitted)
        .unwrap_or_else(|error| panic!("emitted schema rejected: {error}\n{emitted}"));
    assert!(validator.is_valid(&json!({"self": {}})), "{emitted}");
    assert!(!validator.is_valid(&json!({"self": 5})), "{emitted}");
}

// A configured fancy-regex engine drives canonicalize's pattern-vs-exact cross-matching, so a lookaround
// `patternProperties` key compiles and constrains the matching property.
#[test]
fn canonicalize_with_fancy_regex_compiles_lookaround_pattern() {
    let schema = json!({
        "type": "object",
        "properties": {"foo": {"type": "integer"}},
        "patternProperties": {"(?=foo)": {"type": "string"}}
    });
    let emitted = options()
        .with_pattern_options(&PatternOptions::fancy_regex())
        .canonicalize(&schema)
        .expect("schema parses")
        .to_json_schema();
    let validator = jsonschema::validator_for(&emitted).expect("compiles");
    assert!(validator.is_valid(&json!({})), "{emitted}");
    assert!(!validator.is_valid(&json!({"foo": 1})), "{emitted}");
}

// An open array with an unbounded `contains` is a decidably-inhabited shape: the disjointness residual
// proves non-containment against a string, while a `maxContains` bound makes the leaf undecidable and the
// subschema verdict inconclusive.
#[test]
fn open_contains_is_decidably_inhabited() {
    let string = canonicalize(&json!({"type": "string"})).expect("valid schema");
    let open = canonicalize(&json!({"type": "array", "contains": {"type": "integer"}}))
        .expect("valid schema");
    assert_eq!(open.is_subschema_of(&string), Some(false));

    let bounded =
        canonicalize(&json!({"type": "array", "contains": {"type": "integer"}, "maxContains": 3}))
            .expect("valid schema");
    assert_eq!(bounded.is_subschema_of(&string), None);
}

// The fast covers path runs under `self`'s context, so it must not fire when the operands disagree on
// `validate_formats`: an annotation-only `format` (admits every string) is not a subschema of the same
// schema with the format asserted (admits only date strings).
#[test]
fn is_subschema_of_respects_differing_format_settings() {
    let schema = json!({"type": "string", "format": "date"});
    let annotation = options()
        .should_validate_formats(false)
        .canonicalize(&schema)
        .expect("valid schema");
    let asserted = options()
        .should_validate_formats(true)
        .canonicalize(&schema)
        .expect("valid schema");
    // `annotation` admits "hello", which `asserted` rejects, so it is not contained.
    assert_ne!(annotation.is_subschema_of(&asserted), Some(true));
    // The reverse holds: every date string is a string.
    assert_eq!(asserted.is_subschema_of(&annotation), Some(true));
}

// A `union`/`intersect` with an operand from an unrecognized meta-schema (draft `Unknown`) must not leak
// that hidden sentinel through `.draft()`; the concrete operand's draft wins.
#[test]
fn algebra_draft_prefers_concrete_over_unknown_meta_schema() {
    let unknown =
        canonicalize(&json!({"$schema": "https://example.com/custom-meta", "type": "string"}))
            .expect("valid schema");
    let concrete = canonicalize(&json!({"type": "integer"})).expect("valid schema");
    assert_eq!(concrete.draft(), Draft::Draft202012);
    assert_eq!(unknown.union(&concrete).draft(), Draft::Draft202012);
    assert_eq!(concrete.union(&unknown).draft(), Draft::Draft202012);
}

// A `const` value nested past the document depth cap is preserved verbatim as `Raw` rather than
// silently truncated. The input is built programmatically (256 deep), so it stays in Rust.
#[test_case(false ; "nested_arrays")]
#[test_case(true ; "nested_objects")]
fn deeply_nested_const_value_preserved_raw(use_objects: bool) {
    let mut value = Value::Null;
    for _ in 0..=255 {
        value = if use_objects {
            let mut map = serde_json::Map::new();
            map.insert("a".to_owned(), value);
            Value::Object(map)
        } else {
            Value::Array(vec![value])
        };
    }
    let schema = json!({ "const": value });
    let canonical = canonicalize(&schema).expect("deep const canonicalizes");
    assert_eq!(canonical.kind(), CanonicalKind::Raw);
    assert_eq!(canonical.to_json_schema(), schema);
}

// `multipleOf: 0` is smuggled in through a non-schema-position `$ref` target to dodge meta-validation;
// the canonical form it produces is not itself re-canonicalizable, so this stays a Rust test (the JSON
// suite re-canonicalizes for its idempotency check).
#[test]
fn intersect_number_multiple_of_zero_and_nonzero_keeps_multiple_of_zero() {
    let canonical = canonicalize(&json!({
        "smuggled": {"type": "number", "multipleOf": 0},
        "allOf": [{"$ref": "#/smuggled"}, {"type": "number", "multipleOf": 2}]
    }))
    .expect("valid schema");
    assert_eq!(
        canonical.to_json_schema(),
        json!({"$schema": DRAFT202012_SCHEMA_URI, "type": "number", "multipleOf": 0})
    );
}

// Under the default (i64) build, two `multipleOf` values whose LCM overflows are unrepresentable, so
// `intersect` returns a residual and both number leaves stay in an `allOf`. The cfg can't live in JSON.
#[cfg(not(feature = "arbitrary-precision"))]
#[test]
fn intersect_multiple_of_lcm_overflow_keeps_allof() {
    let produced = canonicalize(&json!({
        "allOf": [
            {"type": "object", "properties": {"x": {"type": "number", "multipleOf": 4_611_686_018_427_387_904_i64}}},
            {"type": "object", "properties": {"x": {"type": "number", "multipleOf": 3}}}
        ]
    }))
    .expect("valid schema")
    .to_json_schema();
    assert_eq!(
        produced,
        json!({
            "$schema": DRAFT202012_SCHEMA_URI,
            "type": "object",
            "properties": {
                "x": {
                    "allOf": [
                        {"type": "integer", "multipleOf": 3},
                        {"type": "integer", "multipleOf": 4_611_686_018_427_387_904_i64}
                    ]
                }
            }
        })
    );
}

// A near-`i64::MAX` prime `multipleOf` used to drive `O(sqrt(P))` trial-division factorization (~3e9
// iterations, seconds of compute) during modular-exclusion reduction. The factorization input is now
// bounded, so this canonicalizes near-instantly. Timing-only: the reduction is a no-op here, so there is no
// form/witness difference to assert - hence not a JSON-suite case.
#[test]
fn large_prime_multiple_of_does_not_trigger_slow_factorization() {
    let start = std::time::Instant::now();
    let result = canonicalize(&json!({
        "type": "integer",
        "multipleOf": 9_223_372_036_854_775_783_i64,
        "not": {"multipleOf": 2}
    }));
    let elapsed = start.elapsed();
    assert!(result.is_ok(), "{result:?}");
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "modular-exclusion reduction factorized a large prime: {elapsed:?}"
    );
}

// A plain-decimal literal whose expansion exceeds the digit cap re-dispatches to the scientific normal form
// with no explicit exponent; that must still yield the one canonical text a value has, so it dedupes against
// the same value written in scientific form. Arbitrary-precision only (exact decimals), so not a JSON case.
#[cfg(feature = "arbitrary-precision")]
#[test]
fn oversized_plain_decimal_shares_one_canonical_text_with_scientific() {
    let plain = format!("0.{}10", "0".repeat(1 << 20)); // 10^-1048577, past MAX_EXPANDED_INTEGER_DIGITS
    let from_plain = jsonschema::canonical::json::canonical_number(&plain).expect("canonical text");
    let from_scientific =
        jsonschema::canonical::json::canonical_number("1e-1048577").expect("canonical text");
    assert_eq!(from_plain, from_scientific);
}

// An enum member in scientific normal form (past the digit-expansion cap) cannot be compared
// exactly by the runtime validator, so the whole document stays raw. Arbitrary-precision only,
// so the cfg can't live in JSON.
#[cfg(feature = "arbitrary-precision")]
#[test]
fn intersect_enum_with_unparsable_scientific_number() {
    let schema: Value = serde_json::from_str(
        r#"{"allOf": [{"type": "number", "minimum": 0}, {"enum": [1e2000000, 5]}]}"#,
    )
    .expect("valid json");
    let produced = canonicalize(&schema)
        .expect("valid schema")
        .to_json_schema();
    assert_eq!(produced, schema);
}

// `integer` is a subschema of `number`, but not the reverse — the asymmetry that keeps
// `allOf([number, not(integer)])` satisfiable. These are public `is_subschema_of` set-relations.
#[test]
fn integer_is_subschema_of_number_but_not_vice_versa() {
    let integer = canonicalize(&json!({"type": "integer"})).expect("valid");
    let number = canonicalize(&json!({"type": "number"})).expect("valid");
    assert_eq!(integer.is_subschema_of(&number), Some(true));
    assert_ne!(number.is_subschema_of(&integer), Some(true));
}

// --- emit: unknown-draft roots and external-ref bundling (public canonicalize + registry API) ---

// Under an unrecognized `$schema` (`Draft::Unknown`) the emitted root carries no `$schema` wrapper, so a
// bare boolean root reaches the non-object guards in emit. `subtract` of identical schemas yields `false`;
// negating it yields `true`.
#[test]
fn boolean_root_under_unknown_draft_emits_bare_bool() {
    let schema = canonicalize(&json!({
        "$schema": "http://unknown.example/schema#",
        "type": "string"
    }))
    .expect("valid schema");
    let empty = schema.subtract(&schema);
    assert_eq!(empty.to_json_schema(), json!(false));
    assert_eq!(empty.negate().to_json_schema(), json!(true));
}

// An unrecognised `$schema` is `Draft::Unknown`; `negate().to_json_schema()` emits a non-`Raw` root under
// it and must omit `$schema` rather than panic.
#[test]
fn negate_under_unknown_draft_emits_without_schema() {
    let input = json!({"$schema": "http://unknown.example/schema#", "type": "object"});
    let canonical = canonicalize(&input).expect("valid schema");
    let emitted = canonical.negate().to_json_schema();
    assert!(
        emitted.get("$schema").is_none(),
        "unknown draft must not emit a $schema URI: {emitted}"
    );
    canonicalize(&emitted).expect("emitted schema canonicalises");
}

// An external `$ref` on a cycle is symbolic (keyed by absolute uri). Emit must bundle its target into the
// output so the result is self-contained: it compiles with NO registry and validates like the original.
#[test]
fn emit_bundles_external_recursive_ref() {
    let registry = referencing::Registry::new()
        .add(
            "https://example.com/node",
            json!({"type": "object", "properties": {"next": {"$ref": "https://example.com/node"}}}),
        )
        .expect("valid resource")
        .prepare()
        .expect("registry prepares");
    let root = json!({"$ref": "https://example.com/node"});
    let emitted = options()
        .with_registry(&registry)
        .canonicalize(&root)
        .expect("canonicalize")
        .to_json_schema();
    let from_emit = jsonschema::validator_for(&emitted).expect("emitted is self-contained");
    let from_input = jsonschema::options()
        .with_registry(&registry)
        .build(&root)
        .expect("input compiles");
    for instance in [
        json!({}),
        json!({"next": {}}),
        json!({"next": {"next": {}}}),
        json!({"next": 5}),
        json!(5),
    ] {
        assert_eq!(
            from_input.is_valid(&instance),
            from_emit.is_valid(&instance),
            "external ref emit diverged on {instance}\n  emitted: {emitted}",
        );
    }
}

// Bundling the emitted schema must be self-contained (compiles with no registry) and validate like the
// registry-backed input.
fn assert_bundle_parity(
    registry: &referencing::Registry,
    root: &Value,
    instances: &[Value],
) -> Value {
    let emitted = options()
        .with_registry(registry)
        .canonicalize(root)
        .expect("canonicalize")
        .to_json_schema();
    let from_emit = jsonschema::validator_for(&emitted).expect("emitted is self-contained");
    let from_input = jsonschema::options()
        .with_registry(registry)
        .build(root)
        .expect("input compiles");
    for instance in instances {
        assert_eq!(
            from_input.is_valid(instance),
            from_emit.is_valid(instance),
            "external ref emit diverged on {instance}\n  emitted: {emitted}",
        );
    }
    emitted
}

fn recursive_node(self_uri: &str) -> Value {
    json!({"type": "object", "properties": {"next": {"$ref": self_uri}}})
}

enum BundleParityScenario {
    CollideLeafNames,
    UnnamedUri,
}

impl BundleParityScenario {
    fn registry(&self) -> referencing::Registry<'static> {
        match self {
            BundleParityScenario::CollideLeafNames => referencing::Registry::new()
                .add(
                    "https://a.example/node",
                    recursive_node("https://a.example/node"),
                )
                .expect("valid resource")
                .add(
                    "https://b.example/node",
                    recursive_node("https://b.example/node"),
                )
                .expect("valid resource")
                .prepare()
                .expect("registry prepares"),
            BundleParityScenario::UnnamedUri => referencing::Registry::new()
                .add(
                    "https://example.com/@",
                    recursive_node("https://example.com/@"),
                )
                .expect("valid resource")
                .prepare()
                .expect("registry prepares"),
        }
    }

    fn root(&self) -> Value {
        match self {
            BundleParityScenario::CollideLeafNames => json!({"type": "object", "properties": {
                "a": {"$ref": "https://a.example/node"},
                "b": {"$ref": "https://b.example/node"}
            }}),
            BundleParityScenario::UnnamedUri => json!({"$ref": "https://example.com/@"}),
        }
    }

    fn instances(&self) -> Vec<Value> {
        match self {
            BundleParityScenario::CollideLeafNames => vec![
                json!({}),
                json!({"a": {"next": {}}, "b": {}}),
                json!({"a": 5}),
            ],
            BundleParityScenario::UnnamedUri => vec![json!({}), json!({"next": {}}), json!(5)],
        }
    }

    fn verify_emitted(&self, emitted: &Value) {
        match self {
            BundleParityScenario::CollideLeafNames => {
                let defs = emitted["$defs"].as_object().expect("$defs object");
                assert!(
                    defs.contains_key("node") && defs.contains_key("node_1"),
                    "{emitted}"
                );
            }
            BundleParityScenario::UnnamedUri => {
                assert!(emitted["$defs"].get("external").is_some(), "{emitted}");
            }
        }
    }
}

#[test_case(BundleParityScenario::CollideLeafNames ; "collide_leaf_names")]
#[test_case(BundleParityScenario::UnnamedUri ; "unnamed_uri")]
fn emit_bundles_external_parity(scenario: BundleParityScenario) {
    let registry = scenario.registry();
    let root = scenario.root();
    let instances = scenario.instances();
    let emitted = assert_bundle_parity(&registry, &root, &instances);
    scenario.verify_emitted(&emitted);
}

#[test]
fn emit_bundles_external_ref_away_from_dangling_same_document_ref() {
    let registry = referencing::Registry::new()
        .add("https://example.com/foo", json!({"type": "integer"}))
        .expect("valid resource")
        .prepare()
        .expect("registry prepares");
    let root = json!({
        "type": "object",
        "properties": {
            "external": {"$ref": "https://example.com/foo"},
            "local": {"$ref": "#/$defs/foo"},
        },
    });
    let emitted = options()
        .with_registry(&registry)
        .with_inline_budget(0)
        .canonicalize(&root)
        .expect("canonicalize")
        .to_json_schema();
    let properties = emitted["properties"]
        .as_object()
        .expect("properties object");
    assert_eq!(properties["local"]["$ref"], json!("#/$defs/foo"));
    assert_ne!(properties["external"]["$ref"], json!("#/$defs/foo"));
    let defs = emitted["$defs"].as_object().expect("$defs object");
    assert!(
        !defs.contains_key("foo"),
        "external bundle rebound dangling local ref: {emitted}"
    );
}

// --- intersect cases that depend on pattern-engine choice or programmatic scale (not JSON-expressible) ---

// The configured regex dialect carries into intersection's pattern matching: the lookahead compiles under
// `fancy-regex` (so "dog" is filtered out) but the linear `regex` engine rejects it and defers, keeping both
// values — so the two engines must produce different emitted forms.
#[test]
fn intersect_honors_configured_regex_engine() {
    fn with_patterns<E>(
        opts: &PatternOptions<E>,
        value: &Value,
    ) -> jsonschema::canonical::CanonicalSchema {
        options()
            .with_pattern_options(opts)
            .canonicalize(value)
            .expect("canonicalize")
    }
    let schema = json!({"type": "string", "pattern": "(?=.*a)"});
    let values = json!({"enum": ["cat", "dog"]});

    let fancy = PatternOptions::fancy_regex();
    let fancy_result = with_patterns(&fancy, &schema).intersect(&with_patterns(&fancy, &values));

    let linear = PatternOptions::regex();
    let linear_result = with_patterns(&linear, &schema).intersect(&with_patterns(&linear, &values));

    assert_ne!(
        fancy_result.to_json_schema(),
        linear_result.to_json_schema(),
        "engine choice must change how the pattern filters the enum"
    );
}

// A pattern the linear `regex` engine compiles is used to filter the enum down to the matching member.
#[test]
fn regex_engine_filters_by_simple_pattern() {
    let opts = PatternOptions::regex();
    let with_patterns = |value: &Value| {
        options()
            .with_pattern_options(&opts)
            .canonicalize(value)
            .expect("canonicalize")
    };
    let result = with_patterns(&json!({"type": "string", "pattern": "^a"}))
        .intersect(&with_patterns(&json!({"enum": ["abc", "xyz"]})));
    assert_eq!(
        result.to_json_schema(),
        json!({"const": "abc", "$schema": DRAFT202012_SCHEMA_URI})
    );
}

// More than 64 distinct, mutually non-subsuming `contains` clauses are all retained: the emitted schema
// keeps one `allOf` member per clause. Built programmatically, so it stays in Rust.
#[test]
fn many_distinct_contains_clauses_all_retained() {
    let branches: Vec<Value> = (0..70)
        .map(|index| json!({"type": "array", "contains": {"const": index}}))
        .collect();
    let emitted = canonicalize(&json!({"allOf": branches}))
        .expect("canonicalize")
        .to_json_schema();
    assert_eq!(
        emitted["allOf"].as_array().map(Vec::len),
        Some(70),
        "all 70 distinct contains clauses must survive: {emitted}"
    );
}

// Regression tests for the unchecked-arithmetic / unbounded-loop paths in the cardinality and numeric code.
// Each input drives an overflow-prone or loop-bounded computation; canonicalization must terminate and stay
// sound.

// `IntegerBounds::below`/`above` stepped one past the bound with unchecked `i64` arithmetic; negating a leaf
// pinned to `i64::MIN`/`i64::MAX` overflowed.
#[test]
fn integer_negation_at_i64_bounds_does_not_overflow() {
    for schema in [
        json!({"type": "integer", "minimum": i64::MIN}),
        json!({"type": "integer", "maximum": i64::MAX}),
    ] {
        let canonical = canonicalize(&schema).expect("canonicalize");
        // `negate` reaches `below()`/`above()` at the `i64::MIN`/`i64::MAX` endpoints, where the `- 1` / `+ 1`
        // must not overflow.
        let _ = canonical.negate().to_json_schema();
    }
}

// A count keyword above 2^53 must not be silently rounded by the default build's `f64` parse path; the
// schema is preserved verbatim instead.
#[test]
fn cardinality_above_2_53_is_not_rounded() {
    let schema = json!({"type": "string", "minLength": 9_007_199_254_740_993u64});
    let emitted = canonicalize(&schema)
        .expect("canonicalize")
        .to_json_schema();
    assert_eq!(
        emitted.get("minLength"),
        Some(&json!(9_007_199_254_740_993u64)),
        "minLength above 2^53 must be preserved exactly, got {emitted}"
    );
}

// A `$defs` body referenced twice inlines to a single shared `Arc`; once its inlined cost would exceed
// `SHARED_EMIT_COST_LIMIT`, emit must re-extract it to one synthetic `$defs` entry that both uses `$ref`,
// rather than unfolding the subtree per occurrence.
#[test]
fn heavily_shared_subtree_emits_one_synthetic_definition() {
    let mut big_props = serde_json::Map::new();
    for i in 0..300 {
        big_props.insert(format!("p{i}"), json!({"type": "string"}));
    }
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "x": {"$ref": "#/$defs/big"},
            "y": {"$ref": "#/$defs/big"},
        },
        "$defs": {"big": {"type": "object", "properties": big_props}},
    });
    let emitted = canonicalize(&schema)
        .expect("canonicalize")
        .to_json_schema();
    let x = emitted["properties"]["x"]["$ref"]
        .as_str()
        .unwrap_or_else(|| panic!("x is not a $ref: {emitted}"));
    let y = emitted["properties"]["y"]["$ref"]
        .as_str()
        .unwrap_or_else(|| panic!("y is not a $ref: {emitted}"));
    assert_eq!(
        x, y,
        "both uses must point at the same synthetic def: {emitted}"
    );
    assert!(
        x.starts_with("#/$defs/"),
        "shared subtree must extract into $defs, got {x}"
    );
    // The extracted definition resolves and the emitted schema still enforces the string-typed property.
    let validator = jsonschema::validator_for(&emitted).expect("emitted schema compiles");
    assert!(validator.is_valid(&json!({"x": {"p0": "ok"}})));
    assert!(!validator.is_valid(&json!({"x": {"p0": 5}})));
}

// Negating a recursive schema whose recursion target is a plain-name `$id`/`$anchor`: the negated root is
// no longer the anchored body, so emit must bundle that body under a synthetic definition carrying the
// anchor keyword, keeping the `$ref` resolvable (an un-bundled body would dangle as `NoSuchAnchor`).
//
// Not a `canonical-suite` fixture: the suite only canonicalizes a single input, and this needs the
// `canonicalize(S).negate()` sequence with a *root-level* anchor. Expressing it as `{"not": S}` nests the
// `$id`, which canonicalization rewrites to a `#/$defs/...` pointer - so the workaround never reaches the
// root-plain-name-anchor branch this guards.
#[test_case(Draft::Draft7, json!({"$id":"#root","type":"object","properties":{"p":{"$ref":"#root"}}}) ; "draft7_id")]
#[test_case(Draft::Draft6, json!({"$id":"#root","type":"object","properties":{"p":{"$ref":"#root"}}}) ; "draft6_id")]
#[test_case(Draft::Draft4, json!({"id":"#root","type":"object","properties":{"p":{"$ref":"#root"}}}) ; "draft4_id")]
#[test_case(Draft::Draft201909, json!({"$anchor":"root","type":"object","properties":{"p":{"$ref":"#root"}}}) ; "draft2019_anchor")]
#[test_case(Draft::Draft202012, json!({"$anchor":"root","type":"object","properties":{"p":{"$ref":"#root"}}}) ; "draft2020_anchor")]
fn negating_recursive_plain_name_anchor_keeps_ref_resolvable(draft: Draft, schema: Value) {
    let negated = options()
        .with_draft(draft)
        .canonicalize(&schema)
        .expect("canonicalize")
        .negate()
        .to_json_schema();
    let positive = jsonschema::options()
        .with_draft(draft)
        .build(&schema)
        .expect("S compiles");
    let negative = jsonschema::options()
        .with_draft(draft)
        .build(&negated)
        .expect("negated compiles (no dangling $ref)");
    // Negation is the exact set complement: every witness lands in exactly one of S / ¬S.
    for witness in [
        json!({}),
        json!({"p": {}}),
        json!({"p": 5}),
        json!(5),
        json!("x"),
        json!(null),
        json!([]),
    ] {
        assert_ne!(
            positive.is_valid(&witness),
            negative.is_valid(&witness),
            "{draft:?}: S and its negation must disagree on {witness}\n  negated = {negated}"
        );
    }
}

// A constrained-body type guard intersected with a value set whose body membership the oracle cannot
// decide (a non-open array leaf) defers to a strict `allOf`, never widening or narrowing.
#[test]
fn intersect_constrained_array_guard_with_const_defers_to_allof() {
    let guard = canonicalize(&json!({"minItems": 1})).expect("canonicalize guard");
    let value = canonicalize(&json!({"const": [0]})).expect("canonicalize const");
    let intersection = guard.intersect(&value);
    let emitted = intersection.to_json_schema();
    assert!(
        emitted.get("allOf").is_some(),
        "undecidable guard/value membership should defer to allOf: {emitted}"
    );
    let validator = jsonschema::validator_for(&emitted).expect("intersection compiles");
    // The intersection admits only the const, which already meets `minItems`.
    for (witness, expected) in [
        (json!([0]), true),
        (json!([]), false),
        (json!([0, 0]), false),
        (json!([1]), false),
        (json!("x"), false),
    ] {
        assert_eq!(
            validator.is_valid(&witness),
            expected,
            "guard∩const parity: emitted={emitted}, witness={witness}"
        );
    }
}

// `is_satisfiable` exercises the finite-domain emptiness oracle (`canonical/oracle/membership.rs`).
// Each schema below drives a distinct branch of that oracle through the public API.

#[test]
fn deeply_nested_schema_preserved_raw() {
    let mut schema = serde_json::json!({"type": "integer"});
    for _ in 0..300 {
        schema = serde_json::json!({"allOf": [schema]});
    }
    let canonical = canonicalize(&schema).expect("deep schema canonicalizes");
    assert_eq!(canonical.kind(), CanonicalKind::Raw);
    assert_eq!(canonical.to_json_schema(), schema);
}

// External documents bypass the root depth gate, so the resolver-side gates must catch them.
// Depth stays below ~500, where `referencing::Registry::prepare` itself exhausts the stack.
#[test]
fn deeply_nested_external_ref_target_preserved_raw() {
    let mut target = json!({"type": "integer"});
    for _ in 0..300 {
        target = json!({"allOf": [target]});
    }
    let registry = registry_with(&[("https://example.com/deep", target)]);
    let canonical = options()
        .with_registry(&registry)
        .canonicalize(&json!({"$ref": "https://example.com/deep"}))
        .expect("deep external target canonicalizes");
    assert!(canonical.to_json_schema().is_object());
}

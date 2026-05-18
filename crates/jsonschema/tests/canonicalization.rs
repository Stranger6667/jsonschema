#![cfg(feature = "canonical")]
#![allow(clippy::needless_pass_by_value)]

use jsonschema::{
    canonical::{options, CanonicalKind},
    canonicalize, PatternOptions,
};
use referencing::{Draft, Registry};
use serde_json::{json, Value};
use test_case::test_case;

const DRAFT202012_SCHEMA_URI: &str = "https://json-schema.org/draft/2020-12/schema";

/// Canonicalize `schema`, then assert that both the raw and canonical validators produce the expected
/// verdict on each witness (an upfront sanity check that the raw schema validates as expected, plus a
/// parity check that canonicalization neither widened nor narrowed the schema).
#[cfg(feature = "arbitrary-precision")]
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

#[cfg(feature = "arbitrary-precision")]
#[test]
fn canonicalize_huge_integral_exponent_const_preserves_exponent_form() {
    let schema = schema_from_str(r#"{"const":1e999999999999999999999}"#);
    let canonical = canonicalize(&schema).expect("canonicalize");
    let emitted = canonical.to_json_schema();

    assert_eq!(
        emitted["const"],
        schema_from_str(r"1e+999999999999999999999"),
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

// An enum member in scientific normal form (past the digit-expansion cap) can't be parsed as a fraction,
// so it stays in a strict `allOf` instead of being matched against the number leaf. Arbitrary-precision
// only, so the cfg can't live in JSON.
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
    let expected: Value = serde_json::from_str(
        r#"{"$schema": "https://json-schema.org/draft/2020-12/schema",
            "allOf": [{"type": "number", "minimum": 0}, {"enum": [1e2000000, 5]}]}"#,
    )
    .expect("valid json");
    assert_eq!(produced, expected);
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

// Regression tests for unchecked-arithmetic / unbounded-loop fixes in the cardinality and numeric paths.
// Each input previously panicked under debug overflow checks, hung, or emitted an off-by-one bound; the
// assertion is that canonicalization terminates and stays sound.

// `IntegerBounds::below`/`above` stepped one past the bound with unchecked `i64` arithmetic; negating a leaf
// pinned to `i64::MIN`/`i64::MAX` overflowed.
#[test]
fn integer_negation_at_i64_bounds_does_not_overflow() {
    for schema in [
        json!({"type": "integer", "minimum": i64::MIN}),
        json!({"type": "integer", "maximum": i64::MAX}),
    ] {
        let canonical = canonicalize(&schema).expect("canonicalize");
        // `negate` reaches `below()`/`above()`, which previously computed `i64::MIN - 1` / `i64::MAX + 1`.
        let _ = canonical.negate().to_json_schema();
    }
}

// A count keyword above 2^53 must not be silently rounded by the default build's `f64` parse path; the
// schema is preserved verbatim instead. Before the fix it emitted `9_007_199_254_740_992`.
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
// anchor keyword, keeping the `$ref` resolvable. Before the fix the emitted `$ref` dangled (`NoSuchAnchor`).
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

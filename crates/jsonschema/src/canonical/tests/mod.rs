#![allow(clippy::needless_pass_by_value)]

mod proptest;

#[cfg(not(target_arch = "wasm32"))]
mod schemastore;

use std::{
    collections::{BTreeSet, HashSet},
    sync::Arc,
};

use referencing::Draft;
use serde_json::{json, Value};
use test_case::test_case;

use crate::{
    canonical::{
        context::CanonicalizationContext,
        coverage,
        emit::strip_synthetic_root,
        intern::shared,
        ir::{ObjectLeaf, ObjectRequirement, PropertyNameMatcher, Schema},
        options, reachable_definitions,
        tests_util::{canonicalize_symbolic, canonicalize_with},
        CanonicalKind, CanonicalSchema, CanonicalizationError, DefinitionMap,
    },
    canonicalize,
};

fn canonicalize_draft7(schema: &Value) -> CanonicalSchema {
    canonicalize_with(schema, Draft::Draft7)
}

fn canonicalize_2020(schema: &Value) -> CanonicalSchema {
    canonicalize_with(schema, Draft::Draft202012)
}

fn assert_raw(result: &CanonicalSchema) {
    assert!(
        matches!(result.as_schema(), Schema::Raw(_)),
        "expected opaque Raw, got {:?}",
        result.as_schema()
    );
}

fn assert_not_raw(result: &CanonicalSchema) {
    assert!(
        !matches!(result.as_schema(), Schema::Raw(_)),
        "schema forced opaque Raw, got {:?}",
        result.as_schema()
    );
}

// A Raw schema carrying a number whose shortest decimal does not round-trip under serde_json's
// default (imprecise) float parser must still canonicalize idempotently.
#[test]
fn raw_schema_with_imprecise_float_is_idempotent() {
    let schema: Value = serde_json::from_str(
            r#"{"$schema":"http://json-schema.org/rg/drafs8t/-0chema#","ty$e":33333333333333333333333333333333333333333333333333333}"#,
        )
        .expect("valid json");
    let once = canonicalize(&schema).expect("canonicalize");
    assert_eq!(once.kind(), CanonicalKind::Raw, "expected Raw");
    let twice = canonicalize(&once.to_json_schema()).expect("re-canonicalize");
    assert_eq!(once, twice, "Raw float canonicalisation must be idempotent");
}

#[test]
fn canonical_schema_equality_and_hash_ignore_nonsemantic_options() {
    let schema = json!({"type": "integer"});
    let draft7 = options()
        .with_draft(Draft::Draft7)
        .canonicalize(&schema)
        .expect("draft7 canonicalize");
    let draft2020_asserted = options()
        .with_draft(Draft::Draft202012)
        .should_validate_formats(true)
        .canonicalize(&schema)
        .expect("draft2020 canonicalize");

    assert_eq!(draft7, draft2020_asserted);

    let mut hash_set = HashSet::new();
    hash_set.insert(draft7.clone());
    hash_set.insert(draft2020_asserted.clone());
    assert_eq!(hash_set.len(), 1);

    let mut tree_set = BTreeSet::new();
    tree_set.insert(draft7);
    tree_set.insert(draft2020_asserted);
    assert_eq!(tree_set.len(), 1);
}

// A relative `$ref` resolves against the right base (document `$id`, `with_base_uri`, or an
// id-derived base), so the registered integer resource is found and inlined.
#[test_case(
    "https://example.com/other", None,
    json!({"$id": "https://example.com/root", "$ref": "other"})
    ; "against_document_id")]
#[test_case(
    "https://example.com/dir/other", Some("https://example.com/dir/schema"),
    json!({"$ref": "other"})
    ; "against_base_uri")]
#[test_case(
    "https://example.com/base/subdir/sibling.json", Some("https://example.com/base/root.json"),
    json!({"$id": "subdir/root.json", "$ref": "sibling.json"})
    ; "root_id_against_base_uri")]
fn relative_ref_resolves_to_integer(resource_uri: &str, base_uri: Option<&str>, schema: Value) {
    let registry = referencing::Registry::new()
        .add(resource_uri, json!({"type": "integer"}))
        .expect("add resource")
        .prepare()
        .expect("prepare registry");
    let mut opts = options()
        .with_draft(Draft::Draft202012)
        .with_registry(&registry);
    if let Some(base) = base_uri {
        opts = opts.with_base_uri(base);
    }
    let result = opts.canonicalize(&schema).expect("canonicalize");
    assert_eq!(
        result.to_json_schema(),
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "integer"})
    );
}

#[test]
fn external_root_self_ref_stays_in_external_scope() {
    let registry = referencing::Registry::new()
        .add(
            "https://example.com/node",
            json!({
                "$id": "https://example.com/node",
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "child": {"$ref": "#"}
                }
            }),
        )
        .expect("add resource")
        .prepare()
        .expect("prepare registry");
    let canonical = options()
        .with_draft(Draft::Draft202012)
        .with_registry(&registry)
        .canonicalize(&json!({
            "$id": "https://example.com/root",
            "type": "object",
            "properties": {
                "node": {"$ref": "https://example.com/node"}
            }
        }))
        .expect("canonicalize");
    let emitted = canonical.to_json_schema();
    let validator = crate::validator_for(&emitted).expect("emitted schema compiles");

    assert!(validator.is_valid(&json!({"node": {"child": {}}})));
    assert!(
        !validator.is_valid(&json!({"node": {"child": {"child": {"extra": true}}}})),
        "{emitted}"
    );
}

#[test]
fn emitted_asserted_modern_format_does_not_use_draft7_shim() {
    // Formats first known in 2019-09+ have no pre-2019 pivot draft, so the assertion cannot ride on a
    // `$schema` shim branch; the keyword emits plain under the canonicalizer's own assertion setting.
    for format in ["uuid", "duration"] {
        let canonical = options()
            .with_draft(Draft::Draft202012)
            .should_validate_formats(true)
            .canonicalize(&json!({"type": "string", "format": format}))
            .expect("canonicalize");
        assert_eq!(
            canonical.to_json_schema(),
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "string",
                "format": format
            })
        );
    }
}

#[test_case(json!({"type": "int"}) ; "unknown type name")]
#[test_case(json!({"type": 42}) ; "type as number")]
fn external_ref_malformed_schema_fails_like_root(target: Value) {
    let registry = referencing::Registry::new()
        .add("https://example.com/other", target)
        .expect("add resource")
        .prepare()
        .expect("prepare registry");
    let result = options()
        .with_draft(Draft::Draft202012)
        .with_registry(&registry)
        .canonicalize(&json!({"$id": "https://example.com/root", "$ref": "other"}));
    assert!(
        matches!(result, Err(CanonicalizationError::ValidationError(_))),
        "expected ValidationError, got {result:?}"
    );
}

// End-to-end against the real file retriever (mirroring the CLI): a `file://` base URI at the schema
// file lets a relative `$ref` to a sibling resolve and inline.
#[cfg(feature = "resolve-file")]
#[test]
fn relative_ref_retrieved_against_file_base_uri() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("sibling.json"), r#"{"type":"integer"}"#).expect("write");
    // `path_to_uri` builds a well-formed `file://` URI on every platform; hand-formatting breaks on
    // Windows (backslashes, `file:///C:/...` drive-letter form).
    let base_uri = crate::retriever::path_to_uri(&dir.path().join("root.json"));
    let result = options()
        .with_draft(Draft::Draft202012)
        .with_base_uri(base_uri)
        .canonicalize(&json!({"$ref": "sibling.json"}))
        .expect("canonicalize");
    assert_eq!(
        result.to_json_schema(),
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "integer"})
    );
}

// An annotation-contributing applicator spans the dynamic scope, so `unevaluated*` cannot reduce to
// its static equivalent and must stay opaque.
#[test_case(json!({
        "allOf": [{"properties": {"a": {"type": "integer"}}}],
        "unevaluatedProperties": false,
    }) ; "properties_with_applicator")]
#[test_case(json!({
        "type": "array",
        "contains": {"const": "x"},
        "unevaluatedItems": false,
    }) ; "items_with_contains")]
#[test_case(json!({
        "properties": {"foo": {"type": "string"}},
        "dependentSchemas": {"foo": {"required": ["bar"]}},
        "unevaluatedProperties": false,
    }) ; "properties_with_dependent_schemas")]
fn unevaluated_with_annotation_contributors_is_opaque(schema: Value) {
    assert_raw(&canonicalize_2020(&schema));
}

// Instance-data keywords carry arbitrary values, not subschemas: a `$schema`/`$dynamicRef` string in
// such data must not force whole-schema opaque preservation (canonicalisation drops the data anyway).
#[test_case(json!({"type": "string", "default": {"$schema": "https://example.com/x.json"}}) ; "default")]
#[test_case(json!({"const": {"$schema": "https://example.com/x.json"}}) ; "const_keyword")]
#[test_case(json!({"enum": [{"$schema": "https://example.com/x.json"}]}) ; "enum_keyword")]
#[test_case(json!({"type": "string", "examples": [{"$dynamicRef": "#x"}]}) ; "examples")]
fn schema_keyword_in_instance_data_does_not_force_raw(schema: Value) {
    assert_not_raw(&canonicalize_draft7(&schema));
}

// TS-generated `$defs` names use generic syntax like `TopLevel<FacetedUnitSpec>`, so `$ref`s carry
// URI-illegal `<`/`>`; a finite budget emits the ref symbolically and must resolve it, not bail opaque.
#[test]
fn ref_with_uri_unsafe_chars_resolves_symbolically() {
    let schema = json!({
        "$schema": "http://json-schema.org/draft-04/schema#",
        "$ref": "#/definitions/A<B>",
        "definitions": {"A<B>": {"type": "integer"}},
    });
    let result = options()
        .with_draft(Draft::Draft4)
        .with_inline_budget(0)
        .canonicalize(&schema)
        .expect("canonicalize");
    assert_not_raw(&result);
    assert_eq!(
        result.definitions().len(),
        1,
        "the `<>` def must resolve, not dangle"
    );
}

#[test_case(json!({"type":"number","minimum":0,"maximum":1}) ; "open_interval_many_doubles_no_collapse")]
#[test_case(json!({"type":"number","exclusiveMinimum":1.5,"maximum":2.0}) ; "wider_open_interval_no_collapse")]
#[test_case(json!({"type":"integer","minimum":5,"maximum":7}) ; "small_int_interval_no_collapse")]
#[cfg_attr(
    feature = "arbitrary-precision",
    test_case(json!({"type":"number","minimum":0,"maximum":5e-324}) ; "two_double_window_no_collapse")
)]
// Exact-decimal instances exist strictly between adjacent doubles, so f64-window folds are unsound
// under `arbitrary-precision` and must not fire.
#[cfg_attr(
    feature = "arbitrary-precision",
    test_case(json!({"type":"number","minimum":0,"exclusiveMaximum":5e-324}) ; "subnormal_window_no_collapse")
)]
#[cfg_attr(
    feature = "arbitrary-precision",
    test_case(json!({"type":"number","exclusiveMinimum":1.5,"maximum":1.500_000_000_000_000_2}) ; "next_up_window_no_collapse")
)]
fn tight_bound_no_false_collapse(schema: Value) {
    let result = canonicalize_draft7(&schema);
    assert!(
        !matches!(result.as_schema(), Schema::Const(_) | Schema::Enum(_)),
        "soundness bug: multi-value interval collapsed to const/enum\n  schema = {schema}\n  result = {result:?}",
    );
}

// The subnormal window `[0, 5e-324)` collapses nowhere: without `arbitrary-precision` the bound
// overflows the fraction (leaf stays `Raw`); with it the window still holds exact decimals.
#[cfg(not(feature = "arbitrary-precision"))]
#[test_case(json!({"type":"number","exclusiveMinimum":1.5,"maximum":1.500_000_000_000_000_2}) ; "next_up_window_collapses_next_double")]
fn tight_bound_collapses(schema: Value) {
    let result = canonicalize_draft7(&schema);
    assert!(
        matches!(result.as_schema(), Schema::Const(_) | Schema::Enum(_)),
        "expected canonicalize to collapse to const/enum\n  schema = {schema}\n  result = {result:?}",
    );
}

fn assert_canonicalizes_to(input: Value, expected: Value) {
    assert_eq!(
        canonicalize_draft7(&input),
        canonicalize_draft7(&expected),
        "canonicalize mismatch\n  input    = {input}\n  expected = {expected}",
    );
}

// Draft 4 `type: integer` rejects `1.0` while `enum`/`const` admit it, so neither spelling direction
// is sound there.
#[test_case(json!({"type": "integer", "minimum": 0, "maximum": 1}), CanonicalKind::Integer ; "window_keeps_leaf")]
#[test_case(json!({"enum": [0, 1, 2]}), CanonicalKind::Enum ; "enum_stays_enum")]
fn draft4_integer_spelling_unchanged(schema: Value, expected: CanonicalKind) {
    let result = canonicalize_with(&schema, Draft::Draft4);
    assert_eq!(
        CanonicalKind::from(result.as_schema()),
        expected,
        "draft 4 spelling changed: {result:?}",
    );
}

// Union-relative value extraction is gated the same way: the extracted `enum`/`const` admits `1.0`
// where the draft 4 integer leaf does not.
#[test_case(json!({"anyOf": [
        {"type": "integer", "minimum": 0, "maximum": 10, "multipleOf": 2},
        {"type": "integer", "minimum": 0, "maximum": 10, "multipleOf": 3}
    ]}) ; "covered_window_keeps_leaf")]
#[test_case(json!({"anyOf": [
        {"type": "integer", "minimum": 0, "maximum": 2},
        {"type": "number", "minimum": 1, "maximum": 2, "not": {"multipleOf": 1}}
    ]}) ; "fused_window_keeps_leaf")]
fn draft4_union_avoids_value_sets(schema: Value) {
    fn contains_value_set(schema: &Schema) -> bool {
        matches!(schema, Schema::Const(_) | Schema::Enum(_))
            || schema
                .children()
                .iter()
                .any(|child| contains_value_set(child.as_schema()))
    }
    let result = canonicalize_with(&schema, Draft::Draft4);
    assert!(
        !contains_value_set(result.as_shared().as_schema()),
        "draft 4 union extracted a value set: {}",
        result.to_json_schema(),
    );
}

// f64 instances only: with `arbitrary-precision` the window still holds exact decimals and must not
// collapse (see `tight_bound_no_false_collapse::next_up_window_no_collapse`).
#[cfg(not(feature = "arbitrary-precision"))]
#[test_case(json!({"type": "number", "exclusiveMinimum": 1.5, "maximum": 1.500_000_000_000_000_2}), json!({"const": 1.500_000_000_000_000_2}) ; "number_single_value_collapses_to_const")]
fn to_expected_bound_collapses_to_const(input: Value, expected: Value) {
    assert_canonicalizes_to(input, expected);
}

#[cfg(feature = "arbitrary-precision")]
#[test_case(json!({"type": "integer", "allOf": [{"multipleOf": 0.5}, {"multipleOf": 1e308}]}), json!({"type": "integer", "multipleOf": 1e308}) ; "all_of_multiple_of_picks_large_lcm")]
fn to_expected_all_of_merging(input: Value, expected: Value) {
    assert_canonicalizes_to(input, expected);
}

// A self-referential object must survive as object IR carrying a `Recursive` cycle leaf, not swept
// whole-schema to `Schema::Raw`.
#[test]
fn recursive_property_survives_as_object_ir() {
    let result = canonicalize_draft7(&json!({
        "type": "object",
        "properties": {"next": {"$ref": "#"}},
    }));
    assert!(
        matches!(result.as_schema(), Schema::Object(_)),
        "expected object IR, got {:?}",
        result.as_schema()
    );
    assert!(
        !result
            .as_shared()
            .mask
            .is_disjoint(CanonicalKind::Recursive),
        "cycle must survive as Recursive, got {:?}",
        result.as_schema()
    );
}

// A shared acyclic ref inlines at every use under the default (infinite) budget, but emits one symbolic
// `Reference` + one `definitions()` entry under a finite budget.
#[test]
fn inline_budget_emits_shared_ref_symbolic() {
    let schema = json!({
        "type": "object",
        "properties": {
            "x": {"$ref": "#/$defs/shared"},
            "y": {"$ref": "#/$defs/shared"},
        },
        "$defs": {"shared": {"type": "string", "minLength": 3}},
    });

    let prop = |canonical: &CanonicalSchema, name: &str| -> Schema {
        let Schema::Object(leaf) = canonical.as_schema() else {
            panic!("expected object, got {:?}", canonical.as_schema());
        };
        leaf.constraints
                .iter()
                .find(|c| matches!(&c.matcher, PropertyNameMatcher::NamedProperty(n) if n.as_ref() == name))
                .unwrap_or_else(|| panic!("`{name}` constraint present"))
                .schema
                .as_schema()
                .clone()
    };

    // Finite budget: both uses are the same symbolic reference; one def entry.
    let budgeted = canonicalize_symbolic(&schema);
    let budgeted_x = prop(&budgeted, "x");
    assert!(
        matches!(budgeted_x, Schema::Reference(_)),
        "x should be symbolic, got {budgeted_x:?}",
    );
    assert!(matches!(prop(&budgeted, "y"), Schema::Reference(_)));
    let defs: Vec<_> = budgeted.definitions().collect();
    assert_eq!(defs.len(), 1, "one definition entry, got {defs:?}");
    assert_eq!(defs[0].0, "#/$defs/shared");

    // Default (infinite) budget: fully inlined, no symbolic refs or defs.
    let inlined = options().canonicalize(&schema).expect("canonicalize");
    let inlined_x = prop(&inlined, "x");
    assert!(
        matches!(inlined_x, Schema::String(_)),
        "x should be inlined, got {inlined_x:?}",
    );
    assert_eq!(inlined.definitions().len(), 0);
}

// definitions() contract: transitive closure, uniform uri keys, dangling out.
#[test]
fn definitions_is_transitive_closure_with_uniform_keys() {
    // Mutual recursion A.b -> B, B.a -> A, plus a dangling ref.
    let schema = json!({
        "$ref": "#/$defs/A",
        "$defs": {
            "A": {"type": "object", "properties": {"b": {"$ref": "#/$defs/B"}, "x": {"$ref": "#/$defs/missing"}}},
            "B": {"type": "object", "properties": {"a": {"$ref": "#/$defs/A"}}},
        },
    });
    let canonical = canonicalize_symbolic(&schema);

    let defs: Vec<_> = canonical.definitions().collect();
    let keys: BTreeSet<&str> = defs.iter().map(|(k, _)| k.as_str()).collect();
    // Invariant 1+2: both reachable uris are keys, under uniform `#/...` form.
    assert!(keys.contains("#/$defs/A"), "A missing from {keys:?}");
    assert!(keys.contains("#/$defs/B"), "B missing from {keys:?}");
    // Invariant 3: the dangling uri is NOT a key.
    assert!(
        !keys.contains("#/$defs/missing"),
        "dangling leaked: {keys:?}"
    );

    // Every reference uri inside any definition body resolves to a key (closure is self-contained),
    // except the intentionally-dangling one.
    for (_, body) in &defs {
        let mut refs = Vec::new();
        collect_ref_uris(body.as_schema(), &mut refs);
        for uri in refs {
            assert!(
                keys.contains(uri.as_str()) || uri == "#/$defs/missing",
                "nested ref {uri:?} not in definitions {keys:?}"
            );
        }
    }
}

// A root self-reference (`$ref: "#"`) sits on a cycle, so it survives as `Recursive("#")`; its target
// is the document root, which must appear in definitions() under `#`.
#[test]
fn root_self_reference_registers_definition() {
    let canonical = canonicalize_symbolic(&json!({
        "type": "object",
        "properties": {"children": {"type": "array", "items": {"$ref": "#"}}},
    }));
    let keys: BTreeSet<String> = canonical.definitions().map(|(k, _)| k).collect();
    assert!(
        keys.contains("#"),
        "root `#` missing from definitions {keys:?}"
    );
    assert_self_contained(&canonical);
}

// Collect every reference uri reachable from a schema, normalized to `#/...` form.
fn collect_ref_uris(schema: &Schema, out: &mut Vec<String>) {
    match schema {
        Schema::Reference(uri) => out.push(strip_synthetic_root(uri.as_str()).into()),
        Schema::Recursive(uri) => out.push(strip_synthetic_root(uri).into()),
        _ => schema.for_each_child(|c| collect_ref_uris(c.as_schema(), out)),
    }
}

fn reachable_ref_uris(schema: &CanonicalSchema) -> Vec<String> {
    let mut out = Vec::new();
    collect_ref_uris(schema.as_schema(), &mut out);
    out
}

fn assert_self_contained(schema: &CanonicalSchema) {
    let keys: BTreeSet<String> = schema.definitions().map(|(k, _)| k).collect();
    for uri in reachable_ref_uris(schema) {
        assert!(
            keys.contains(&uri),
            "reachable ref {uri:?} missing from definitions {keys:?}"
        );
    }
    // Emitted schema must carry its own definitions (no dangling `$ref`).
    crate::validator_for(&schema.to_json_schema()).expect("emitted schema is self-contained");
}

// An operand binding `#/$defs/F` to `body`, with required `v` referencing it. Two with different
// bodies collide on the `F` key, exercising definition disambiguation across set ops.
fn colliding_def(body: Value) -> CanonicalSchema {
    canonicalize_symbolic(&json!({
        "type": "object",
        "required": ["v"],
        "properties": {"v": {"$ref": "#/$defs/F"}},
        "$defs": {"F": body},
    }))
}

// An object carrying a symbolic `Reference` plus its definition.
fn ref_carrying_operand() -> CanonicalSchema {
    canonicalize_symbolic(&json!({
        "type": "object",
        "properties": {"kids": {"type": "array", "items": {"$ref": "#/definitions/Root"}}},
        "definitions": {"Root": {"type": "integer"}},
    }))
}

// A surviving Reference in an intersect result must carry its definitions so `to_json_schema()` emits
// a self-contained schema.
#[test]
fn intersect_preserves_definitions() {
    let a = ref_carrying_operand();
    let b = options()
        .canonicalize(&json!({"type": "object"}))
        .expect("canonicalize b");

    let merged = a.intersect(&b);
    assert!(
        !reachable_ref_uris(&merged).is_empty(),
        "expected a surviving Reference in the intersect result"
    );
    assert_self_contained(&merged);
}

#[test]
fn intersect_numeric_definition_collision_converges() {
    let left = canonicalize_2020(&json!({
        "$defs": {"shared": {"type": "null"}},
        "type": "object",
        "properties": {
            "a": {"$ref": "#/$defs/shared"},
            "b": {"$ref": "#/$defs/shared"}
        }
    }));
    let right = canonicalize_2020(&json!({
        "$defs": {
            "shared": {
                "allOf": [
                    {
                        "anyOf": [
                            {"allOf": [{"type": "null"}]},
                            {"allOf": [{"multipleOf": 5}]},
                            {
                                "allOf": [
                                    {"type": "null"},
                                    {"type": "string", "pattern": "^a"},
                                    {"enum": [false, false, 1.5, 1.5]}
                                ]
                            }
                        ]
                    },
                    {
                        "allOf": [
                            {
                                "anyOf": [
                                    {"type": "integer", "multipleOf": 3},
                                    {"type": "number", "minimum": 0, "maximum": 5},
                                    {"multipleOf": 4}
                                ]
                            }
                        ]
                    }
                ]
            }
        },
        "type": "object",
        "properties": {
            "a": {"$ref": "#/$defs/shared"},
            "b": {"$ref": "#/$defs/shared"}
        }
    }));

    let merged = left.intersect(&right);

    crate::validator_for(&merged.to_json_schema()).expect("intersection compiles");
}

#[test]
fn intersect_prunes_definitions_when_reference_disappears() {
    let referenced = canonicalize_symbolic(&json!({
        "$ref": "#/$defs/number",
        "$defs": {"number": {"type": "number"}},
    }));
    let false_schema = canonicalize(&json!(false)).expect("canonicalize false");

    let merged = referenced.intersect(&false_schema);

    assert_eq!(merged, false_schema);
    assert!(
        merged.definitions().len() == 0,
        "unreachable definitions leaked from false intersection: {:?}",
        merged.definitions().collect::<Vec<_>>()
    );
}

// `anyOf[ref, true]` collapses to `True`; after `negate` the ref's definition is unreachable.
#[test]
fn negate_prunes_unreachable_definitions() {
    let schema = canonicalize_symbolic(&json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "anyOf": [{"$ref": "#/$defs/x"}, true],
        "$defs": {"x": {"type": "integer"}}
    }));
    assert!(
        matches!(schema.as_schema(), Schema::True),
        "anyOf with true should collapse to True"
    );
    let negated = schema.negate();
    assert!(
        negated.definitions().len() == 0,
        "negate should prune unreachable definitions, got: {:?}",
        negated.definitions().collect::<Vec<_>>()
    );
}

// DAG with 2^N paths to one leaf: reachable_definitions must memoize by identity, not walk every path.
#[test]
fn reachable_definitions_handles_shared_dag() {
    const DEPTH: usize = 60;
    let leaf_uri: Arc<str> = Arc::from("#/$defs/leaf");
    let mut node = shared(Schema::Recursive(Arc::clone(&leaf_uri)));
    for _ in 0..DEPTH {
        node = shared(Schema::AnyOf(vec![Arc::clone(&node), Arc::clone(&node)]));
    }
    let mut map = DefinitionMap::new();
    map.insert(Arc::clone(&leaf_uri), shared(Schema::True));
    let definitions = Arc::new(map);

    let reachable = reachable_definitions(&node, &definitions);

    assert_eq!(reachable.len(), 1);
    assert!(reachable.contains_key(&leaf_uri));
}

// `negate` carries a Reference too; its definitions must survive.
#[test]
fn negate_preserves_definitions() {
    let a = ref_carrying_operand();
    let negated = a.negate();
    assert!(
        !reachable_ref_uris(&negated).is_empty(),
        "expected a surviving Reference in the negate result"
    );
    assert_self_contained(&negated);
}

// Definitions under any same-document pointer (e.g. OpenAPI's `#/components/schemas/...`), not just `#/$defs` /
// `#/definitions`, must be reattached so emitted schemas are self-contained.
#[test]
fn custom_pointer_definitions_are_self_contained() {
    let schema = canonicalize_symbolic(&json!({
        "allOf": [
            {"$ref": "#/components/schemas/IntegerType"},
            {"$ref": "#/components/schemas/StringType"},
        ],
        "components": {
            "schemas": {
                "IntegerType": {"type": "integer"},
                "StringType": {"type": "string"},
            }
        },
    }));
    assert!(!reachable_ref_uris(&schema).is_empty());
    assert_self_contained(&schema);
}

// Two documents reuse `#/$defs/F` for different bodies. After intersect, `v` must be both string and
// integer, so neither validates. Before namespacing, the left body won and a string was wrongly accepted.
#[test]
fn intersect_namespaces_colliding_definitions() {
    let string_v = colliding_def(json!({"type": "string"}));
    let integer_v = colliding_def(json!({"type": "integer"}));
    let merged = string_v.intersect(&integer_v);
    let emitted = merged.to_json_schema();
    let validator = crate::validator_for(&emitted)
        .unwrap_or_else(|error| panic!("emitted must resolve, got {error}\n  {emitted}"));
    assert!(
        !validator.is_valid(&json!({"v": "hi"})),
        "string wrongly accepted at `v`; right integer body lost: {emitted}"
    );
    assert!(
        !validator.is_valid(&json!({"v": 1})),
        "integer wrongly accepted at `v`; left string body lost: {emitted}"
    );
    assert_self_contained(&merged);
}

// A recursive right-hand body relocates its `Recursive` self-edge too, and the renamed definition must
// still resolve against itself.
#[test]
fn intersect_relocates_recursive_collision() {
    let string_v = colliding_def(json!({"type": "string"}));
    let recursive_v =
        colliding_def(json!({"type": "object", "properties": {"child": {"$ref": "#/$defs/F"}}}));
    let merged = string_v.intersect(&recursive_v);
    let emitted = merged.to_json_schema();
    let validator = crate::validator_for(&emitted)
        .unwrap_or_else(|error| panic!("emitted must resolve, got {error}\n  {emitted}"));
    // `v` must be both a string (left) and a recursive object (right): no value qualifies.
    assert!(
        !validator.is_valid(&json!({"v": "hi"})),
        "string wrongly accepted: {emitted}"
    );
    assert!(
        !validator.is_valid(&json!({"v": {}})),
        "object wrongly accepted: {emitted}"
    );
    assert_self_contained(&merged);
}

// Same `#/$defs/F` key with different bodies per side: the union must relocate one so the emitted
// schema resolves and accepts members of either operand.
#[test]
fn union_resolves_colliding_definitions() {
    let string_v = colliding_def(json!({"type": "string"}));
    let integer_v = colliding_def(json!({"type": "integer"}));
    let union = string_v.union(&integer_v);
    let emitted = union.to_json_schema();
    let validator = crate::validator_for(&emitted)
        .unwrap_or_else(|error| panic!("emitted must resolve, got {error}\n  {emitted}"));
    assert!(
        validator.is_valid(&json!({"v": "hi"})),
        "string `v` is in the left operand: {emitted}"
    );
    assert!(
        validator.is_valid(&json!({"v": 1})),
        "integer `v` is in the right operand: {emitted}"
    );
    assert!(
        !validator.is_valid(&json!({"v": null})),
        "null `v` is in neither operand: {emitted}"
    );
}

#[test]
fn union_preserves_dangling_ref_when_other_operand_defines_same_uri() {
    let dangling = canonicalize_symbolic(&json!({"$ref": "#/$defs/F"}));
    let resolved = canonicalize_symbolic(&json!({
        "$ref": "#/$defs/F",
        "$defs": {"F": {"type": "integer"}},
    }));

    for union in [dangling.union(&resolved), resolved.union(&dangling)] {
        let keys: BTreeSet<String> = union.definitions().map(|(key, _)| key).collect();
        assert!(
            !keys.contains("#/$defs/F"),
            "dangling ref was rebound to the other operand's definition: {:?}",
            union.to_json_schema()
        );
        assert_eq!(
            keys,
            BTreeSet::from(["#/$defs/F__merge0".to_string()]),
            "definition must move away from the dangling ref URI"
        );
        let refs = reachable_ref_uris(&union);
        assert!(
            refs.contains(&"#/$defs/F".to_string()),
            "dangling ref must remain in the result: {refs:?}"
        );
        assert!(
            refs.contains(&"#/$defs/F__merge0".to_string()),
            "renamed resolved ref must remain in the result: {refs:?}"
        );
    }
}

#[test]
fn union_reserves_dangling_refs_when_generating_merge_names() {
    let dangling_ref = "#/$defs/F__merge0";
    let left = canonicalize_symbolic(&json!({
        "anyOf": [
            {"$ref": "#/$defs/F"},
            {"$ref": dangling_ref},
        ],
        "$defs": {"F": {"type": "string"}},
    }));
    let right = canonicalize_symbolic(&json!({
        "$ref": "#/$defs/F",
        "$defs": {"F": {"type": "integer"}},
    }));

    for union in [left.union(&right), right.union(&left)] {
        let keys: BTreeSet<String> = union.definitions().map(|(key, _)| key).collect();
        assert!(
            !keys.contains(dangling_ref),
            "merge-generated definition rebound a dangling ref: {:?}",
            union.to_json_schema()
        );
        let refs = reachable_ref_uris(&union);
        assert!(
            refs.contains(&dangling_ref.to_string()),
            "dangling ref must survive relocation: {refs:?}"
        );
    }
}

// `subtract` routes through `intersect`, so disambiguation must apply there too.
#[test]
fn subtract_disambiguates_colliding_definitions() {
    let string_v = colliding_def(json!({"type": "string"}));
    let integer_v = colliding_def(json!({"type": "integer"}));
    // A string `v` is in `string_v` but not `integer_v`, so it must survive the difference.
    let difference = string_v.subtract(&integer_v);
    let emitted = difference.to_json_schema();
    let validator = crate::validator_for(&emitted)
        .unwrap_or_else(|error| panic!("emitted must resolve, got {error}\n  {emitted}"));
    assert!(
        validator.is_valid(&json!({"v": "hi"})),
        "string `v` wrongly removed by the difference: {emitted}"
    );
}

// Fast path: a shared key with an identical body is left alone and still validates.
#[test]
fn intersect_shares_identical_definitions() {
    let make = || {
        canonicalize_symbolic(&json!({
            "type": "object",
            "properties": {"v": {"$ref": "#/$defs/F"}},
            "$defs": {"F": {"type": "string"}},
        }))
    };
    let merged = make().intersect(&make());
    let emitted = merged.to_json_schema();
    let validator = crate::validator_for(&emitted)
        .unwrap_or_else(|error| panic!("emitted must resolve, got {error}\n  {emitted}"));
    assert!(
        validator.is_valid(&json!({"v": "hi"})),
        "shared string body lost: {emitted}"
    );
    assert!(
        !validator.is_valid(&json!({"v": 1})),
        "string constraint lost: {emitted}"
    );
}

#[test]
fn subtract_is_set_difference() {
    let all_ints = canonicalize(&json!({"type": "integer"})).expect("all ints");
    let non_neg = canonicalize(&json!({"type": "integer", "minimum": 0})).expect("non-neg");
    // self \ self = empty
    assert!(!all_ints.subtract(&all_ints).is_satisfiable());
    // all_ints \ non_neg = negative integers (non-empty)
    assert!(all_ints.subtract(&non_neg).is_satisfiable());
    // non_neg \ all_ints = empty (non_neg is a subset)
    assert!(!non_neg.subtract(&all_ints).is_satisfiable());
}

// Consolidated `is_subschema_of` relation tests (all use `canonicalize`).
// Disjoint shapes: residual is concrete and inhabited, so non-containment is provable.
#[test_case(json!({"type": "string"}), json!({"type": "integer"}), Some(false) ; "string_not_in_integer")]
#[test_case(json!({"type": "integer"}), json!({"type": "string"}), Some(false) ; "integer_not_in_string")]
#[test_case(json!({"type": "object"}), json!({"type": "array"}), Some(false) ; "object_not_in_array")]
#[test_case(json!({"type": ["string", "integer"]}), json!({"type": "integer"}), Some(false) ; "multitype_not_in_integer")]
#[test_case(json!({"type": "string", "minLength": 5}), json!({"type": "integer"}), Some(false) ; "constrained_string_not_in_integer")]
#[test_case(json!({"type": "string"}), json!({"type": "string", "minLength": 5}), Some(false) ; "open_string_not_in_min_length")]
// Provable containment: every value of `left` also satisfies `right`.
#[test_case(json!({"const": "x"}), json!({"type": "string"}), Some(true) ; "const_in_type")]
#[test_case(json!({"enum": [1, 2]}), json!({"type": "integer"}), Some(true) ; "enum_in_type")]
#[test_case(json!({"type": "string", "minLength": 5}), json!({"type": "string"}), Some(true) ; "constrained_string_in_open_string")]
#[test_case(json!({"type": "integer"}), json!({"type": ["string", "integer"]}), Some(true) ; "integer_in_multitype")]
#[test_case(json!({"type": "integer", "minimum": 0, "maximum": 10}), json!({"type": "number"}), Some(true) ; "integer_interval_in_number")]
// Integer ray containment: the residual interval decides each direction; `false` is below everything,
// everything is below `true`.
#[test_case(json!({"type": "integer", "minimum": 5}), json!({"type": "integer", "minimum": 0}), Some(true) ; "tighter_min_in_looser_min")]
#[test_case(json!({"type": "integer", "minimum": 0}), json!({"type": "integer", "minimum": 5}), Some(false) ; "looser_min_not_in_tighter_min")]
#[test_case(json!(false), json!({"type": "integer", "minimum": 0}), Some(true) ; "false_in_anything")]
#[test_case(json!({"type": "integer", "minimum": 0}), json!(true), Some(true) ; "anything_in_true")]
// Pointwise-stronger recursive schemas.
#[test_case(
    json!({
        "$defs": {"strict": {"type": "object", "required": ["value"],
            "properties": {"value": {"type": "integer", "minimum": 5}, "next": {"$ref": "#/$defs/strict"}}}},
        "$ref": "#/$defs/strict"
    }),
    json!({
        "$defs": {"loose": {"type": "object", "required": ["value"],
            "properties": {"value": {"type": "integer", "minimum": 0}, "next": {"$ref": "#/$defs/loose"}}}},
        "$ref": "#/$defs/loose"
    }),
    Some(true)
    ; "self_recursive_pointwise")]
#[test_case(
    json!({
        "$defs": {
            "a": {"type": "object", "properties": {"next": {"$ref": "#/$defs/b"}}},
            "b": {"type": "object", "required": ["v"],
                "properties": {"v": {"type": "integer", "minimum": 5}, "back": {"$ref": "#/$defs/a"}}}
        },
        "$ref": "#/$defs/a"
    }),
    json!({
        "$defs": {
            "c": {"type": "object", "properties": {"next": {"$ref": "#/$defs/d"}}},
            "d": {"type": "object", "required": ["v"],
                "properties": {"v": {"type": "integer", "minimum": 0}, "back": {"$ref": "#/$defs/c"}}}
        },
        "$ref": "#/$defs/c"
    }),
    Some(true)
    ; "mutually_recursive_pointwise")]
// Containment proven when the extra matcher's schema sits within the big catch-all.
#[test_case(
    json!({
        "type": "object",
        "additionalProperties": {"type": "integer"},
        "properties": {"a": {"type": "integer", "minimum": 0}}
    }),
    json!({"type": "object", "additionalProperties": {"type": "integer"}}),
    Some(true)
    ; "object_extra_matcher_within_catch_all_is_contained")]
// A residual with a recursion binding has undecidable emptiness, so the prover stays inconclusive.
#[test_case(
    json!({
        "$ref": "#/$defs/node",
        "$defs": {
            "node": {"type": "object", "properties": {"next": {"$ref": "#/$defs/node"}}}
        }
    }),
    json!({"type": "integer"}),
    None
    ; "recursive_residual_is_inconclusive")]
// Residual shapes whose emptiness the pipeline cannot decide must stay inconclusive.
#[test_case(
    json!({"type": "string", "pattern": "^a"}),
    json!({"type": "string", "pattern": "^ab|^a[^b]|^a$"}),
    None
    ; "semantically_equivalent_patterns")]
#[test_case(
    json!({"type": "object", "required": ["a"], "propertyNames": {"pattern": "^b"}}),
    json!({"type": "integer"}),
    None
    ; "property_names_contradicts_required")]
#[test_case(
    json!({"type": "object", "required": ["a", "b"], "dependentSchemas": {"a": false}}),
    json!({"type": "integer"}),
    None
    ; "dependent_schema_contradicts_required")]
#[test_case(
    json!({"type": "array", "maxItems": 1, "prefixItems": [{"const": 1}], "contains": {"const": 2}}),
    json!({"type": "integer"}),
    None
    ; "contains_in_bounded_window")]
#[test_case(
    // ∃ key whose value matches the empty-language regex "a^": the witness schema's emptiness is undecided.
    json!({
        "type": "object",
        "not": {"additionalProperties": {"not": {"type": "string", "pattern": "a^"}}}
    }),
    json!({"type": "integer"}),
    None
    ; "existential_requirement_with_undecided_witness")]
fn is_subschema_of_relation(left: Value, right: Value, expected: Option<bool>) {
    let left = canonicalize(&left).expect("left");
    let right = canonicalize(&right).expect("right");
    assert_eq!(left.is_subschema_of(&right), expected);
}

#[test]
fn is_subschema_of_ref_with_one_sided_definition_is_inconclusive() {
    let dangling = canonicalize_symbolic(&json!({"$ref": "#/$defs/A"}));
    let resolved = canonicalize_symbolic(&json!({
        "$ref": "#/$defs/A",
        "$defs": {"A": {"type": "string"}},
    }));

    assert_eq!(dangling.is_subschema_of(&resolved), None);
    assert_eq!(resolved.is_subschema_of(&dangling), None);
}

// Containment must not be claimed (exact verdict `Some(false)` vs `None` is not pinned): an extra
// small-side matcher exempts its names from the small catch-all while the big one still governs them.
#[test_case(
        json!({"type": "object", "additionalProperties": {"type": "integer"}, "properties": {"a": {"const": "x"}}}),
        json!({"type": "object", "additionalProperties": {"type": "integer"}})
        ; "extra_named_property_escapes_catch_all")]
#[test_case(
        json!({"type": "object", "additionalProperties": {"type": "integer"}, "patternProperties": {"^a": {"const": "x"}}}),
        json!({"type": "object", "additionalProperties": {"type": "integer"}})
        ; "extra_pattern_property_escapes_catch_all")]
// Reverse of `is_subschema_of_relation::self_recursive_pointwise`: a loose list is not a strict one.
#[test_case(
        json!({"$defs": {"loose": {"type": "object", "required": ["value"],
            "properties": {"value": {"type": "integer", "minimum": 0}, "next": {"$ref": "#/$defs/loose"}}}},
            "$ref": "#/$defs/loose"}),
        json!({"$defs": {"strict": {"type": "object", "required": ["value"],
            "properties": {"value": {"type": "integer", "minimum": 5}, "next": {"$ref": "#/$defs/strict"}}}},
            "$ref": "#/$defs/strict"})
        ; "recursive_reverse_not_proven")]
fn is_subschema_of_not_proven(left: Value, right: Value) {
    let left = canonicalize(&left).expect("left");
    let right = canonicalize(&right).expect("right");
    assert_ne!(left.is_subschema_of(&right), Some(true));
}

// Operands binding the same definition key to different bodies must intersect commutatively: collision
// renames derive from the sorted union of (key, body) entries, not from sides.
#[test]
fn intersect_with_colliding_definitions_is_commutative() {
    let make = |value_type: &str| {
        canonicalize(&json!({
            "$defs": {"node": {"type": "object",
                "properties": {"next": {"$ref": "#/$defs/node"}, "value": {"type": value_type}}}},
            "$ref": "#/$defs/node"
        }))
        .expect("valid schema")
    };
    let left = make("null");
    let right = make("boolean");
    assert_eq!(left.intersect(&right), right.intersect(&left));
}

// Partitioning is for small residual shapes: each step spawns a full intersect+canonicalize, so
// oversized operands (e.g. SchemaStore-sized branches) must not enter it.
#[test]
fn oversized_covers_does_not_partition() {
    let properties: serde_json::Map<String, Value> = (0..70)
        .map(|index| {
            (
                format!("p{index}"),
                json!({"type": "integer", "minimum": index}),
            )
        })
        .collect();
    let big_object = json!({"type": "object", "properties": properties, "required": ["p0"]});
    let prefix: Vec<Value> = (0..70)
        .map(|index| json!({"type": "integer", "minimum": index}))
        .collect();
    let big_array = json!({"type": "array", "prefixItems": prefix, "minItems": 1});
    let union = canonicalize_draft7(&json!({"anyOf": [big_object, big_array]}));
    let small = canonicalize_draft7(&json!({"type": ["object", "array"]}));
    assert!(
        matches!(union.as_schema(), Schema::AnyOf(_)),
        "union must stay AnyOf"
    );
    let ctx = CanonicalizationContext::default();
    let fuel_before = ctx.partition_fuel_remaining();
    assert!(!coverage::covers(
        union.as_shared(),
        small.as_shared(),
        &ctx
    ));
    assert_eq!(ctx.partition_fuel_remaining(), fuel_before);
}

// An existential whose witness schema has undecidable emptiness (a recursion binding) must keep the
// leaf out of the decidably-inhabited set.
#[test]
fn object_existential_with_undecidable_witness_is_not_decidably_inhabited() {
    let leaf = ObjectLeaf {
        requirements: vec![ObjectRequirement::PatternPropertyRequirement {
            matcher: PropertyNameMatcher::AdditionalProperties,
            schema: shared(Schema::Recursive(Arc::from("node"))),
        }],
        constraints: Vec::new(),
        property_names: None,
    };
    assert!(!crate::canonical::schema::is_decidably_inhabited(
        &shared(Schema::Object(leaf)),
        false
    ));
}

// `small ∖ big` must keep the witness {"a": "x"}: "a" escapes `small`'s catch-all (exact-name exemption)
// but not `big`'s, so the negated existential applies to every key, including "a".
#[test]
fn subtract_object_keeps_catch_all_escaping_witness() {
    let small = canonicalize(&json!({
        "type": "object",
        "additionalProperties": {"type": "integer"},
        "properties": {"a": {"const": "x"}}
    }))
    .expect("small");
    let big = canonicalize(&json!({"type": "object", "additionalProperties": {"type": "integer"}}))
        .expect("big");
    let residual = small.subtract(&big);
    assert!(residual.is_satisfiable());
    let validator = crate::validator_for(&residual.to_json_schema()).expect("residual compiles");
    assert!(validator.is_valid(&json!({"a": "x"})));
}

// Every property value must be an integer while the existential `not` demands a string-valued one:
// the conjunction is empty, so containment in anything is proven.
#[test]
fn is_subschema_of_decides_contradicted_existential() {
    let left = canonicalize(&json!({
        "type": "object",
        "properties": {"a": {"type": "integer"}},
        "additionalProperties": {"type": "integer"},
        "not": {"additionalProperties": {"not": {"type": "string"}}}
    }))
    .expect("left");
    let right = canonicalize(&json!({"type": "integer"})).expect("right");
    assert!(!left.is_satisfiable());
    assert_eq!(left.is_subschema_of(&right), Some(true));
}

// `format` is an assertion here, so the (empty) uuid-with-maxLength-1 residual is beyond the pipeline.
#[test]
fn is_subschema_of_asserted_format_residual_is_inconclusive() {
    let left = options()
        .with_draft(Draft::Draft202012)
        .should_validate_formats(true)
        .canonicalize(&json!({"type": "string", "format": "uuid", "maxLength": 1}))
        .expect("left");
    let right = canonicalize(&json!({"type": "integer"})).expect("right");
    assert_eq!(left.is_subschema_of(&right), None);
}

#[test]
fn intersect_strips_unasserted_format_from_operand_definitions() {
    let annotated = options()
        .with_inline_budget(0)
        .should_validate_formats(false)
        .canonicalize(
            &json!({"$ref": "#/$defs/A", "$defs": {"A": {"type": "string", "format": "uuid"}}}),
        )
        .expect("canonicalize annotated");
    let asserted = options()
        .with_inline_budget(0)
        .should_validate_formats(true)
        .canonicalize(
            &json!({"$ref": "#/$defs/B", "$defs": {"B": {"type": "string", "format": "email"}}}),
        )
        .expect("canonicalize asserted");
    let merged = annotated.intersect(&asserted).to_json_schema();
    let validator = crate::validator_for(&merged).expect("merged compiles");
    assert!(!validator.is_valid(&json!("not-a-uuid-or-email")));
}

#[test]
fn union_keeps_asserted_format_definitions_unchanged() {
    let left = options()
        .with_inline_budget(0)
        .should_validate_formats(true)
        .canonicalize(
            &json!({"$ref": "#/$defs/A", "$defs": {"A": {"type": "string", "format": "email"}}}),
        )
        .expect("canonicalize left");
    let right = options()
        .with_inline_budget(0)
        .should_validate_formats(true)
        .canonicalize(
            &json!({"$ref": "#/$defs/B", "$defs": {"B": {"type": "string", "format": "uuid"}}}),
        )
        .expect("canonicalize right");
    let merged = left.union(&right).to_json_schema();
    let validator = crate::validator_for(&merged).expect("merged compiles");
    assert!(validator.is_valid(&json!("user@example.com")));
}

#[test]
fn intersect_walks_formatless_string_definitions() {
    let annotated = options()
        .with_inline_budget(0)
        .should_validate_formats(false)
        .canonicalize(
            &json!({"$ref": "#/$defs/A", "$defs": {"A": {"type": "string", "minLength": 1}}}),
        )
        .expect("canonicalize annotated");
    let asserted = options()
        .with_inline_budget(0)
        .should_validate_formats(true)
        .canonicalize(
            &json!({"$ref": "#/$defs/B", "$defs": {"B": {"type": "string", "format": "email"}}}),
        )
        .expect("canonicalize asserted");
    let merged = annotated.intersect(&asserted).to_json_schema();
    let validator = crate::validator_for(&merged).expect("merged compiles");
    assert!(!validator.is_valid(&json!("")));
}

// A `null`/`boolean` type set is a finite domain: conjoined with a negated value set that spans it,
// the whole conjunction is empty. The pipeline must reach that verdict from the `MultiType` form
// too, not only from the parsed `anyOf`-of-typed-groups form, or canonicalization is not idempotent.
#[test]
fn finite_type_domain_minus_covering_value_set_is_empty() {
    let schema = json!({
        "allOf": [
            {"oneOf": [
                {"anyOf": [{"type":"null"},{"type":"boolean"},{"maximum":0,"minimum":0,"type":"integer"}]},
                {"not": {"enum": [null,false]}},
                {"oneOf": [{"type":"null"},{"type":"null"},{"type":"null"}]}
            ]},
            {"type": ["null","boolean"]}
        ]
    });
    let once = canonicalize(&schema).expect("canonicalize");
    let twice = canonicalize(&once.to_json_schema()).expect("re-canonicalize");
    assert_eq!(once, twice, "canonicalize must be idempotent");
    assert_eq!(
        once.to_json_schema(),
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "enum": [false, null]})
    );
}

// `false` never matches, so it can never be the "exactly one" branch: an inert `false` inside
// `oneOf` must not change the canonical form, regardless of how many branches surround it.
#[test_case(2)]
#[test_case(6)]
#[test_case(7)]
fn one_of_inert_false_branch_converges(branch_count: usize) {
    let branches: Vec<Value> = (0..branch_count)
        .map(|index| json!({"contains": {"const": index}}))
        .collect();
    let plain = canonicalize(&json!({"oneOf": branches})).expect("canonicalize");
    let mut padded = branches.clone();
    padded.push(json!(false));
    let with_false = canonicalize(&json!({"oneOf": padded})).expect("canonicalize");
    assert_eq!(plain, with_false);
}

#[test]
fn external_target_unevaluated_folding_is_preserved_raw() {
    let registry = referencing::Registry::new()
        .add(
            "https://example.com/ext",
            json!({"allOf": [{"properties": {"a": {"type": "integer"}}}], "unevaluatedProperties": false}),
        )
        .expect("valid resource")
        .prepare()
        .expect("registry prepares");
    let emitted = options()
        .with_registry(&registry)
        .canonicalize(&json!({"$ref": "https://example.com/ext"}))
        .expect("canonicalize")
        .to_json_schema();
    let validator = crate::validator_for(&emitted).expect("compiles");
    assert!(validator.is_valid(&json!({"a": 1})));
    assert!(!validator.is_valid(&json!({"a": 1, "b": 2})));
}

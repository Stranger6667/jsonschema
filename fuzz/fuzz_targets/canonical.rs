#![no_main]
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

// Conservative markers for inputs where standalone idempotence and semantic checks are intentionally skipped.
// Reference and dynamic keywords can depend on registry context; `unevaluated*` can force raw preservation when scoped
// by applicators. We still canonicalize them and exercise derived operations for panics.
const STANDALONE_CHECK_MARKERS: &[&str] = &[
    "$ref",
    "$defs",
    "definitions",
    "$recursiveRef",
    "$recursiveAnchor",
    "$dynamicRef",
    "$dynamicAnchor",
    "unevaluatedProperties",
    "unevaluatedItems",
];

fn has_standalone_check_markers(schema: &Value) -> bool {
    match schema {
        Value::Object(map) => {
            map.keys()
                .any(|key| STANDALONE_CHECK_MARKERS.contains(&key.as_str()))
                || map.values().any(has_standalone_check_markers)
        }
        Value::Array(items) => items.iter().any(has_standalone_check_markers),
        _ => false,
    }
}

fuzz_target!(|data: (&[u8], &[u8], &[u8])| {
    let (left, right, instance) = data;
    let Ok(left) = serde_json::from_slice::<Value>(left) else {
        return;
    };

    let Ok(canonical_left) = jsonschema::canonicalize(&left) else {
        return;
    };

    // Infallible public queries must never panic.
    let _ = canonical_left.is_satisfiable();
    let _ = canonical_left.kind();
    let _ = canonical_left.view();
    let _ = canonical_left.definitions();

    let emitted = canonical_left.to_json_schema();
    // `negate`/`intersect` re-run the pipeline; drive them for panics.
    let negated = canonical_left.negate().to_json_schema();

    let canonical_right = serde_json::from_slice::<Value>(right)
        .ok()
        .and_then(|schema| {
            jsonschema::canonicalize(&schema)
                .ok()
                .map(|canonical| (schema, canonical))
        });
    let intersection = canonical_right
        .as_ref()
        .map(|(_, b)| canonical_left.intersect(b).to_json_schema());

    // Standalone-safe schemas must be idempotent after emitting and re-canonicalizing.
    let standalone_checks_safe =
        !has_standalone_check_markers(&left) && !has_standalone_check_markers(&emitted);
    if standalone_checks_safe {
        let round_trip =
            jsonschema::canonicalize(&emitted).expect("re-canonicalising emitted schema failed");
        assert_eq!(
            canonical_left, round_trip,
            "canonicalisation is not idempotent\n  schema    = {left}\n  canonical = {emitted}",
        );
    } else {
        let _ = jsonschema::canonicalize(&emitted);
    }

    let Ok(instance) = serde_json::from_slice::<Value>(instance) else {
        return;
    };

    let (Ok(raw_validator), Ok(canonical_validator)) = (
        jsonschema::validator_for(&left),
        jsonschema::validator_for(&emitted),
    ) else {
        return;
    };

    // Soundness of `is_satisfiable`: a schema it reports empty must reject every instance.
    // Validated against the source schema so a wrong collapse to `false` can't self-confirm.
    if !canonical_left.is_satisfiable() {
        assert!(
            !raw_validator.is_valid(&instance),
            "is_satisfiable()=false but the schema accepts an instance\n  schema   = {left}\n  instance = {instance}",
        );
    }

    if standalone_checks_safe {
        // Canonicalisation must preserve validation semantics.
        assert_eq!(
            raw_validator.is_valid(&instance),
            canonical_validator.is_valid(&instance),
            "canonicalisation changed validation result\n  schema    = {left}\n  canonical = {emitted}\n  instance  = {instance}",
        );

        // Negation must produce the exact complement.
        if let Ok(negated_validator) = jsonschema::validator_for(&negated) {
            assert_eq!(
                raw_validator.is_valid(&instance),
                !negated_validator.is_valid(&instance),
                "negation is not the exact complement\n  schema   = {left}\n  negated  = {negated}\n  instance = {instance}",
            );
        }
    }

    // Intersection soundness: anything the intersection accepts, both inputs accept.
    if let (Some((right_schema, _)), Some(intersection)) = (canonical_right, intersection) {
        if standalone_checks_safe
            && !has_standalone_check_markers(&right_schema)
            && !has_standalone_check_markers(&intersection)
        {
            if let (Ok(right_validator), Ok(inter_validator)) = (
                jsonschema::validator_for(&right_schema),
                jsonschema::validator_for(&intersection),
            ) {
                if inter_validator.is_valid(&instance) {
                    assert!(
                        raw_validator.is_valid(&instance) && right_validator.is_valid(&instance),
                        "intersection accepts an instance an input rejects\n  left  = {left}\n  right = {right_schema}\n  inter = {intersection}\n  instance = {instance}",
                    );
                }
            }
        }
    }
});

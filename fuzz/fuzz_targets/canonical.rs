#![no_main]
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

fuzz_target!(|data: (&[u8], &[u8], &[u8])| {
    let (left, right, instance) = data;
    let Ok(left) = serde_json::from_slice::<Value>(left) else {
        return;
    };
    let Ok(canonical) = jsonschema::canonicalize(&left) else {
        return;
    };

    // Infallible queries answer on every canonical form.
    let _ = canonical.view();
    let _ = canonical.definitions().count();
    let emitted = canonical.to_json_schema();

    // The emitted document carries `$schema`, so re-canonicalizing it detects the same draft.
    let again = jsonschema::canonicalize(&emitted).expect("a canonical form re-canonicalizes");
    assert_eq!(emitted, again.to_json_schema(), "source = {left}");

    let (Ok(source), Ok(canonical_validator)) = (
        jsonschema::validator_for(&left),
        jsonschema::validator_for(&emitted),
    ) else {
        return;
    };
    let Ok(instance) = serde_json::from_slice::<Value>(instance) else {
        return;
    };

    let admitted = source.is_valid(&instance);
    assert_eq!(
        admitted,
        canonical_validator.is_valid(&instance),
        "source = {left}\n  canonical = {emitted}\n  instance = {instance}"
    );

    if !canonical.is_satisfiable() {
        // Checked against the source so a wrong collapse cannot confirm itself.
        assert!(!admitted, "source = {left}\n  instance = {instance}");
    }

    if let Some(negated) = canonical.negate() {
        let complement = negated.to_json_schema();
        if let Ok(validator) = jsonschema::validator_for(&complement) {
            assert_eq!(
                admitted,
                !validator.is_valid(&instance),
                "source = {left}\n  complement = {complement}\n  instance = {instance}"
            );
        }
    }

    let Ok(right) = serde_json::from_slice::<Value>(right) else {
        return;
    };
    let (Ok(other), Ok(other_validator)) = (
        jsonschema::canonicalize(&right),
        jsonschema::validator_for(&right),
    ) else {
        return;
    };
    let Ok(intersection) = canonical.intersect(&other) else {
        return;
    };
    let intersected = intersection.to_json_schema();
    if let Ok(validator) = jsonschema::validator_for(&intersected) {
        assert_eq!(
            admitted && other_validator.is_valid(&instance),
            validator.is_valid(&instance),
            "left = {left}\n  right = {right}\n  intersection = {intersected}\n  instance = {instance}"
        );
    }
});

#![no_main]
use jsonschema::canonical::{Containment, Satisfiability};
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

    if canonical.satisfiability() == Satisfiability::No {
        // Checked against the source so a wrong collapse cannot confirm itself.
        assert!(!admitted, "source = {left}\n  instance = {instance}");
    }

    if let Ok(negated) = canonical.negate() {
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
    let accepted_by_other = other_validator.is_valid(&instance);

    // Every operation answers about the values its operands accept, so each is checked against the
    // two source validators on the same instance.
    for (name, result, expected) in [
        (
            "intersection",
            canonical.intersect(&other),
            admitted && accepted_by_other,
        ),
        (
            "union",
            canonical.union(&other),
            admitted || accepted_by_other,
        ),
        (
            "difference",
            canonical.subtract(&other),
            admitted && !accepted_by_other,
        ),
    ] {
        let Ok(result) = result else {
            continue;
        };
        let emitted = result.to_json_schema();
        let Ok(validator) = jsonschema::validator_for(&emitted) else {
            continue;
        };
        assert_eq!(
            expected,
            validator.is_valid(&instance),
            "{name}: left = {left}\n  right = {right}\n  result = {emitted}\n  instance = {instance}"
        );
        // A result the form proves empty accepts nothing, which the source validators confirm.
        if result.satisfiability() == Satisfiability::No {
            assert!(
                !expected,
                "{name} folded to nothing: left = {left}\n  right = {right}\n  instance = {instance}"
            );
        }
    }

    // The laws every operand obeys with itself, which hold whatever it is spelled like.
    for side in [&canonical, &other] {
        for (law, result, expected) in [
            ("a | a", side.union(side), Some(side)),
            ("a & a", side.intersect(side), Some(side)),
            ("a \\ a", side.subtract(side), None),
        ] {
            let Ok(result) = result else {
                continue;
            };
            match expected {
                Some(expected) => assert_eq!(
                    result.to_json_schema(),
                    expected.to_json_schema(),
                    "`{law}` does not hold: a = {}",
                    side.to_json_schema()
                ),
                None => assert_eq!(
                    result.satisfiability(),
                    Satisfiability::No,
                    "`{law}` left something over: a = {}",
                    side.to_json_schema()
                ),
            }
        }
        if let Ok(containment) = side.covers(side) {
            assert_eq!(
                containment,
                Containment::Yes,
                "a schema does not cover itself: {}",
                side.to_json_schema()
            );
        }
        // The same laws against the same values spelled as a pointer. Asked against the node
        // itself, the wrapper's identity shortcuts answer before the algebra runs.
        let Some(twin) = pointer_twin(&side.to_json_schema()) else {
            continue;
        };
        let Ok(twin) = jsonschema::canonicalize(&twin) else {
            continue;
        };
        for (law, difference) in [
            ("a \\ &a", side.subtract(&twin)),
            ("&a \\ a", twin.subtract(side)),
        ] {
            let Ok(difference) = difference else {
                continue;
            };
            assert_ne!(
                difference.satisfiability(),
                Satisfiability::Yes,
                "`{law}` claims a value: a = {}",
                side.to_json_schema()
            );
            if let Ok(validator) = jsonschema::validator_for(&difference.to_json_schema()) {
                assert!(
                    !validator.is_valid(&instance),
                    "`{law}` accepts {instance}: a = {}\n  difference = {}",
                    side.to_json_schema(),
                    difference.to_json_schema()
                );
            }
        }
        for (law, containment) in [
            ("a covers &a", side.covers(&twin)),
            ("&a covers a", twin.covers(side)),
        ] {
            if let Ok(containment) = containment {
                assert_ne!(
                    containment,
                    Containment::No,
                    "`{law}` was refused: a = {}",
                    side.to_json_schema()
                );
            }
        }
    }

    // Coverage is refuted by a value the argument admits and the receiver rejects, and proven by
    // there being none - both read against the sources rather than against the canonical forms.
    match canonical.covers(&other) {
        Ok(Containment::Yes) => assert!(
            !accepted_by_other || admitted,
            "covers yes: left = {left}\n  right = {right}\n  instance = {instance}"
        ),
        // `No` is a claim that some such value exists, which one instance cannot refute.
        Ok(Containment::No | Containment::Unknown) | Err(_) => {}
    }
});

/// The same document reached through a pointer: the same values under a form the algebra must read
/// through rather than recognise. `None` where the body names the root, which the wrapper rebinds.
fn pointer_twin(source: &Value) -> Option<Value> {
    const TWIN: &str = "canonical_twin";
    let object = source.as_object()?;
    if names_document_root(source) {
        return None;
    }
    let mut definitions = object
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut body = object.clone();
    body.remove("$defs");
    definitions.insert(TWIN.to_string(), Value::Object(body));
    let mut wrapper = serde_json::Map::new();
    wrapper.insert("$defs".to_string(), Value::Object(definitions));
    wrapper.insert("$ref".to_string(), Value::String(format!("#/$defs/{TWIN}")));
    Some(Value::Object(wrapper))
}

/// Whether `schema` names the document root anywhere inside it.
fn names_document_root(schema: &Value) -> bool {
    match schema {
        Value::Object(map) => {
            map.get("$ref").and_then(Value::as_str) == Some("#")
                || map.values().any(names_document_root)
        }
        Value::Array(items) => items.iter().any(names_document_root),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

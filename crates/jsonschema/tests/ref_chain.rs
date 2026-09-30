// Spawning a thread with a set stack size is unsupported on wasm targets.
#![cfg(not(target_arch = "wasm32"))]
use jsonschema::{ValidationError, Validator};
use serde_json::{json, Map, Value};
use test_case::test_case;

/// Definitions in each generated `$ref` chain.
const CHAIN_LENGTH: usize = 1000;
/// Stack of the thread that builds or drops the validator: far below what one frame per hop would need.
const SMALL_STACK_SIZE: usize = 256 * 1024;
/// Definitions in a `$ref` cycle that deferred targets enter again.
const CYCLE_LENGTH: usize = 64;

/// How each definition in the chain reaches the next one.
#[derive(Clone, Copy)]
enum Hop {
    Property,
    AllOf,
    Bare,
}

/// A schema whose definitions `C0` to `C{CHAIN_LENGTH - 1}` each refer to the next; the last is a string.
fn chain(hop: Hop) -> Value {
    let mut definitions = Map::new();
    for idx in 0..CHAIN_LENGTH {
        let next = if idx + 1 < CHAIN_LENGTH {
            json!({"$ref": format!("#/$defs/C{}", idx + 1)})
        } else {
            json!({"type": "string"})
        };
        let definition = match hop {
            Hop::Property => json!({"type": "object", "properties": {"next": next}}),
            Hop::AllOf => json!({"allOf": [next]}),
            Hop::Bare => next,
        };
        definitions.insert(format!("C{idx}"), definition);
    }
    json!({"$defs": definitions, "$ref": "#/$defs/C0"})
}

/// Definitions `C0` to `C{CYCLE_LENGTH - 1}` each refer to the next and to `Back`, which refers to `C1`.
fn cycle_entered_from_every_definition() -> Value {
    let back = json!({"$ref": "#/$defs/Back"});
    let mut definitions = Map::new();
    for idx in 0..CYCLE_LENGTH {
        let next = if idx + 1 < CYCLE_LENGTH {
            json!({"$ref": format!("#/$defs/C{}", idx + 1)})
        } else {
            back.clone()
        };
        definitions.insert(format!("C{idx}"), json!({"allOf": [next, back]}));
    }
    definitions.insert(
        "Back".to_string(),
        json!({"type": "integer", "allOf": [{"$ref": "#/$defs/C1"}]}),
    );
    json!({"$defs": definitions, "allOf": [back, {"$ref": "#/$defs/C0"}]})
}

fn on_small_stack<T: Send + 'static>(task: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(SMALL_STACK_SIZE)
        .spawn(task)
        .expect("the thread starts")
        .join()
        .expect("the thread finishes")
}

fn describe(error: &ValidationError<'_>) -> String {
    format!(
        "{} at {} ({})",
        error,
        error.instance_path().as_str(),
        error.schema_path().as_str()
    )
}

fn errors(validator: &Validator, instance: &Value) -> Vec<String> {
    validator
        .iter_errors(instance)
        .map(|error| describe(&error))
        .collect()
}

fn first_error(validator: &Validator, instance: &Value) -> Result<(), String> {
    validator
        .validate(instance)
        .map_err(|error| describe(&error))
}

#[test_case(Hop::Property, &json!({"next": {"next": {}}}), &json!({"next": {"next": 1}}), &["1 is not of type \"object\" at /next/next (/$defs/C2/type)"]; "property")]
#[test_case(Hop::AllOf, &json!("chain"), &json!(1), &[&format!("1 is not of type \"string\" at  (/$defs/C{}/allOf/0/type)", CHAIN_LENGTH - 1)]; "all of")]
#[test_case(Hop::Bare, &json!("chain"), &json!(1), &[&format!("1 is not of type \"string\" at  (/$defs/C{}/type)", CHAIN_LENGTH - 1)]; "bare")]
fn long_ref_chain_compiles_on_small_stack(
    hop: Hop,
    valid: &Value,
    invalid: &Value,
    expected: &[&str],
) {
    let schema = chain(hop);
    let validator =
        on_small_stack(move || jsonschema::validator_for(&schema).expect("the chain compiles"));
    assert_eq!(errors(&validator, valid), Vec::<String>::new());
    assert_eq!(errors(&validator, invalid), expected);
    assert_eq!(first_error(&validator, valid), Ok(()));
    assert_eq!(
        first_error(&validator, invalid),
        Err(expected[0].to_string())
    );
}

#[test_case(Hop::Property; "property")]
#[test_case(Hop::AllOf; "all of")]
#[test_case(Hop::Bare; "bare")]
fn long_ref_chain_drops_on_small_stack(hop: Hop) {
    let validator = jsonschema::validator_for(&chain(hop)).expect("the chain compiles");
    on_small_stack(move || drop(validator));
}

#[test]
fn ref_cycle_entered_from_deferred_targets_validates() {
    let validator = jsonschema::validator_for(&cycle_entered_from_every_definition())
        .expect("the cycle compiles");
    let instance = json!(1);
    assert!(validator.is_valid(&instance));
    assert_eq!(errors(&validator, &instance), Vec::<String>::new());
    assert!(validator.evaluate(&instance).flag().valid);
}

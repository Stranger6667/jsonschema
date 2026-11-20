// Spawning a thread with a set stack size is unsupported on wasm targets.
#![cfg(not(target_arch = "wasm32"))]
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use jsonschema::{Keyword, ValidationError, Validator};
use serde_json::{json, Map, Value};
use test_case::test_case;

/// Definitions in each generated `$ref` chain.
const CHAIN_LENGTH: usize = 1000;
/// Stack of the thread that builds or drops the validator: far below what one frame per hop would need.
const SMALL_STACK_SIZE: usize = 256 * 1024;
/// Definitions in a `$ref` cycle that deferred targets enter again.
const CYCLE_LENGTH: usize = 64;
/// Levels of the nested objects a `$ref` cycle walks.
const TREE_DEPTH: usize = 64;
/// Walks of the root schema allowed per call: a few per instance level.
const WALK_BUDGET: usize = 16 * TREE_DEPTH;

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

// Every level validates its child twice, so without memoization the work doubles per level.
#[test_case(&json!([]), true; "valid")]
#[test_case(&json!(1), false; "invalid")]
fn repeated_ref_to_the_same_target_stays_linear(innermost: &Value, expected: bool) {
    let validator = jsonschema::validator_for(&json!({
        "$defs": {"t": {"type": "array", "items": {"allOf": [{"$ref": "#/$defs/t"}, {"$ref": "#/$defs/t"}]}}},
        "$ref": "#/$defs/t"
    }))
    .expect("Invalid schema");
    let mut instance = innermost.clone();
    for _ in 0..64 {
        instance = json!([instance]);
    }
    assert_eq!(validator.is_valid(&instance), expected);
}

// Counts walks of the schema it sits in and panics past `budget`, so a walk that doubles per
// instance level fails fast.
struct CountedWalks {
    walks: Arc<AtomicUsize>,
    budget: usize,
}

impl CountedWalks {
    fn walk(&self) {
        let walks = self.walks.fetch_add(1, Ordering::Relaxed) + 1;
        assert!(walks <= self.budget, "more than {} walks", self.budget);
    }
}

impl<'i> Keyword<'i> for CountedWalks {
    fn validate(&self, _instance: &'i Value) -> Result<(), ValidationError<'i>> {
        self.walk();
        Ok(())
    }

    fn is_valid(&self, _instance: &'i Value) -> bool {
        self.walk();
        true
    }
}

#[derive(Clone, Copy)]
enum Reach {
    // `properties` and `patternProperties` both refer to the root.
    Keywords,
    // The same, beside an `allOf` that refers to the root.
    KeywordsAndAllOf,
    // `properties` and `patternProperties` reach the root through separate definitions.
    Definitions,
}

// An object whose `c...` members are objects of the same schema, each reached by two keywords.
fn tree(reach: Reach) -> Value {
    let (property, pattern) = match reach {
        Reach::Definitions => ("#/$defs/A", "#/$defs/B"),
        Reach::Keywords | Reach::KeywordsAndAllOf => ("#", "#"),
    };
    let mut schema = json!({
        "$id": "https://example.com/tree",
        "type": "object",
        "counted": true,
        "properties": {"c": {"$ref": property}},
        "patternProperties": {"^c": {"$ref": pattern}}
    });
    match reach {
        Reach::Keywords => {}
        Reach::KeywordsAndAllOf => schema["allOf"] = json!([{"$ref": "#"}]),
        Reach::Definitions => {
            schema["$defs"] = json!({
                "A": {"allOf": [{"$ref": "#"}]},
                "B": {"allOf": [{"$ref": "#"}]}
            });
        }
    }
    schema
}

fn nested(depth: usize, innermost: Value) -> Value {
    let mut instance = innermost;
    for _ in 0..depth {
        instance = json!({"c": instance});
    }
    instance
}

fn counted_validator(schema: &Value, budget: usize) -> Validator {
    let walks = Arc::new(AtomicUsize::new(0));
    jsonschema::options()
        .with_keyword("counted", move |_, _, _| {
            Ok(Box::new(CountedWalks {
                walks: Arc::clone(&walks),
                budget,
            }))
        })
        .build(schema)
        .expect("the tree compiles")
}

fn describe_with_locations(error: &ValidationError<'_>) -> String {
    format!(
        "{} at {} ({} | {} | {})",
        error,
        error.instance_path().as_str(),
        error.schema_path().as_str(),
        error.evaluation_path().as_str(),
        error
            .absolute_keyword_location()
            .map_or("-", |location| location.as_str())
    )
}

#[derive(Clone, Copy)]
enum Mode {
    IsValid,
    Validate,
    IterErrors,
}

// Every level reaches its child once per keyword, so without memoization the work doubles per level.
#[test_case(Reach::Keywords, Mode::IsValid; "two keywords is_valid")]
#[test_case(Reach::Keywords, Mode::Validate; "two keywords validate")]
#[test_case(Reach::Keywords, Mode::IterErrors; "two keywords iter_errors")]
#[test_case(Reach::KeywordsAndAllOf, Mode::IsValid; "two keywords and allOf is_valid")]
#[test_case(Reach::KeywordsAndAllOf, Mode::Validate; "two keywords and allOf validate")]
#[test_case(Reach::KeywordsAndAllOf, Mode::IterErrors; "two keywords and allOf iter_errors")]
#[test_case(Reach::Definitions, Mode::IsValid; "two definitions is_valid")]
#[test_case(Reach::Definitions, Mode::Validate; "two definitions validate")]
#[test_case(Reach::Definitions, Mode::IterErrors; "two definitions iter_errors")]
fn valid_tree_reached_by_two_keywords_stays_linear(reach: Reach, mode: Mode) {
    let validator = counted_validator(&tree(reach), WALK_BUDGET);
    let instance = nested(TREE_DEPTH, json!({}));
    match mode {
        Mode::IsValid => assert!(validator.is_valid(&instance)),
        Mode::Validate => assert_eq!(first_error(&validator, &instance), Ok(())),
        Mode::IterErrors => assert_eq!(errors(&validator, &instance), Vec::<String>::new()),
    }
}

#[test_case(
    Reach::Keywords,
    &nested(2, json!(1)),
    "/c/c",
    &[
        "/properties/c/$ref/properties/c/$ref/type",
        "/properties/c/$ref/patternProperties/^c/$ref/type",
        "/patternProperties/^c/$ref/properties/c/$ref/type",
        "/patternProperties/^c/$ref/patternProperties/^c/$ref/type",
    ];
    "two keywords at the innermost level"
)]
#[test_case(
    Reach::KeywordsAndAllOf,
    &nested(2, json!(1)),
    "/c/c",
    &[
        "/properties/c/$ref/properties/c/$ref/type",
        "/properties/c/$ref/patternProperties/^c/$ref/type",
        "/patternProperties/^c/$ref/properties/c/$ref/type",
        "/patternProperties/^c/$ref/patternProperties/^c/$ref/type",
        "/allOf/0/$ref/properties/c/$ref/properties/c/$ref/type",
        "/allOf/0/$ref/properties/c/$ref/patternProperties/^c/$ref/type",
        "/allOf/0/$ref/patternProperties/^c/$ref/properties/c/$ref/type",
        "/allOf/0/$ref/patternProperties/^c/$ref/patternProperties/^c/$ref/type",
    ];
    "two keywords and allOf at the innermost level"
)]
#[test_case(
    Reach::Definitions,
    &nested(2, json!(1)),
    "/c/c",
    &[
        "/properties/c/$ref/allOf/0/$ref/properties/c/$ref/allOf/0/$ref/type",
        "/properties/c/$ref/allOf/0/$ref/patternProperties/^c/$ref/allOf/0/$ref/type",
        "/patternProperties/^c/$ref/allOf/0/$ref/properties/c/$ref/allOf/0/$ref/type",
        "/patternProperties/^c/$ref/allOf/0/$ref/patternProperties/^c/$ref/allOf/0/$ref/type",
    ];
    "two definitions at the innermost level"
)]
#[test_case(
    Reach::Keywords,
    &mixed_tree(),
    "/cx/cy/cz",
    &["/patternProperties/^c/$ref/patternProperties/^c/$ref/patternProperties/^c/$ref/type"];
    "two keywords beside valid subtrees"
)]
#[test_case(
    Reach::KeywordsAndAllOf,
    &mixed_tree(),
    "/cx/cy/cz",
    &[
        "/patternProperties/^c/$ref/patternProperties/^c/$ref/patternProperties/^c/$ref/type",
        "/allOf/0/$ref/patternProperties/^c/$ref/patternProperties/^c/$ref/patternProperties/^c/$ref/type",
    ];
    "two keywords and allOf beside valid subtrees"
)]
#[test_case(
    Reach::Definitions,
    &mixed_tree(),
    "/cx/cy/cz",
    &["/patternProperties/^c/$ref/allOf/0/$ref/patternProperties/^c/$ref/allOf/0/$ref/patternProperties/^c/$ref/allOf/0/$ref/type"];
    "two definitions beside valid subtrees"
)]
fn invalid_tree_reached_by_two_keywords_keeps_every_error(
    reach: Reach,
    instance: &Value,
    instance_path: &str,
    evaluation_paths: &[&str],
) {
    let validator = counted_validator(&tree(reach), usize::MAX);
    let expected: Vec<String> = evaluation_paths
        .iter()
        .map(|path| {
            format!(
                "1 is not of type \"object\" at {instance_path} (/type | {path} | https://example.com/tree#/type)"
            )
        })
        .collect();
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|error| describe_with_locations(&error))
        .collect();
    assert_eq!(errors, expected);
    assert_eq!(
        validator
            .validate(instance)
            .map_err(|error| describe_with_locations(&error)),
        Err(expected[0].clone())
    );
}

// Invalid only at `/cx/cy/cz`, which only `patternProperties` reaches; every `c` member is a valid
// subtree deep enough to be walked again.
fn mixed_tree() -> Value {
    json!({
        "c": nested(8, json!({})),
        "cx": {"c": nested(8, json!({})), "cy": {"c": nested(8, json!({})), "cz": 1}}
    })
}

// `T` is first walked under `X`, where the cycle guard answers for `X`, so that walk says nothing
// about `T` entered without `X`, where `X` reports its error once more.
#[test]
fn ref_cycle_walked_under_another_target_keeps_every_error() {
    let validator = jsonschema::validator_for(&json!({
        "$defs": {
            "chain": {"properties": {"c": {"$ref": "#/$defs/chain"}}},
            "X": {"required": ["missing"], "allOf": [{"$ref": "#/$defs/T"}]},
            "T": {
                "allOf": [
                    {"$ref": "#/$defs/X"},
                    {"$ref": "#/$defs/T"},
                    {"properties": {"deep": {"$ref": "#/$defs/chain"}}}
                ]
            }
        },
        "$ref": "#/$defs/X"
    }))
    .expect("the cycle compiles");
    let instance = json!({"deep": nested(TREE_DEPTH, json!({}))});
    let expected: Vec<String> = [
        "/$ref/required",
        "/$ref/allOf/0/$ref/allOf/0/$ref/required",
        "/$ref/allOf/0/$ref/allOf/1/$ref/allOf/0/$ref/required",
    ]
    .iter()
    .map(|path| format!("\"missing\" is a required property at  (/$defs/X/required | {path} | -)"))
    .collect();
    let errors: Vec<String> = validator
        .iter_errors(&instance)
        .map(|error| describe_with_locations(&error))
        .collect();
    assert_eq!(errors, expected);
}

fn locate(error: &ValidationError<'_>) -> String {
    format!(
        "{} ({} | {})",
        error.instance_path().as_str(),
        error.schema_path().as_str(),
        error.evaluation_path().as_str()
    )
}

// The schema `R`, referring to itself by `target`.
fn walking_k_twice(target: &str) -> Value {
    let deep = json!({"properties": {"deep": {"$ref": "#/$defs/chain"}}});
    json!({
        "allOf": [
            {
                "if": {"required": ["never"]},
                "then": deep,
                "else": {"if": {"$ref": "#/$defs/Y"}, "then": deep, "else": false}
            },
            {"properties": {"k": {"$ref": target}}},
            {"properties": {"k": {"properties": {"p": {"$ref": "#/$defs/Y"}}}}},
            {"properties": {"k": {"$ref": target}}},
            {"not": {"not": {"$ref": target}}}
        ],
        "properties": {"p": {"not": {"$ref": target}}}
    })
}

// The root is a copy of `R`, so each `$ref` in it enters `R` without a guard entry. The first walk
// of `R` at `/k/k` asks `is_valid` of `R` at `/k/k/p` before `Y` is entered there: `R` rejects it
// before visiting enough targets to be cached. A later walk asks again with `Y` entered at
// `/k/k/p`, where the cycle guard stands in for `Y`, so `R` accepts it and the result is cached.
// Walking `R` at `/k/k` again reads that result and reports the `not` errors.
#[test_case(
    &json!({"never": 1, "k": {"never": 1, "k": {"never": 1, "p": {"deep": nested(20, json!({}))}, "deep": nested(20, json!({}))}}}),
    &[
        "/k/k/p (/$defs/Y/required | /allOf/1/properties/k/$ref/allOf/2/properties/k/properties/p/$ref/required)",
        "/k/k/p (/$defs/R/properties/p/not | /allOf/1/properties/k/$ref/allOf/3/properties/k/$ref/properties/p/not)",
        "/k (/$defs/R/allOf/4/not | /allOf/1/properties/k/$ref/allOf/4/not)",
        "/k/k/p (/$defs/R/properties/p/not | /allOf/3/properties/k/$ref/allOf/1/properties/k/$ref/properties/p/not)",
        "/k/k/p (/$defs/Y/required | /allOf/3/properties/k/$ref/allOf/2/properties/k/properties/p/$ref/required)",
        "/k/k/p (/$defs/R/properties/p/not | /allOf/3/properties/k/$ref/allOf/3/properties/k/$ref/properties/p/not)",
        "/k (/$defs/R/allOf/4/not | /allOf/3/properties/k/$ref/allOf/4/not)",
        " (/allOf/4/not | /allOf/4/not)",
    ];
    "is_valid result cached after the first walk"
)]
fn ref_cycle_reading_a_later_cached_result_keeps_every_error(instance: &Value, expected: &[&str]) {
    let mut schema = walking_k_twice("#/$defs/R");
    schema["$defs"] = json!({
        "chain": {"properties": {"c": {"$ref": "#/$defs/chain"}}},
        "Y": {"required": ["y"], "allOf": [{"$ref": "#/$defs/R"}]},
        "R": walking_k_twice("#/$defs/R")
    });
    let validator = jsonschema::validator_for(&schema).expect("the cycle compiles");
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|error| locate(&error))
        .collect();
    assert_eq!(errors, expected);
}

use jsonschema::{Keyword, ValidationError};
use serde_json::{json, Map, Value};
use std::sync::{Arc, Weak};
use test_case::test_case;

struct DropProbe;

struct ProbeKeyword {
    _probe: Arc<DropProbe>,
}

impl<'i> Keyword<'i> for ProbeKeyword {
    fn validate(&self, _instance: &'i Value) -> Result<(), ValidationError<'i>> {
        Ok(())
    }

    fn is_valid(&self, _instance: &'i Value) -> bool {
        true
    }
}

/// Definitions in a `$ref` cycle long enough that part of it compiles after its referrers.
const CYCLE_LENGTH: usize = 64;

fn tree_schema() -> Value {
    json!({
        "$defs": {
            "Tree": {
                "type": "object",
                "leak-probe": true,
                "properties": {
                    "value": {"type": "integer"},
                    "children": {
                        "type": "array",
                        "items": {"$ref": "#/$defs/Tree"}
                    }
                }
            }
        },
        "$ref": "#/$defs/Tree"
    })
}

/// Definitions `{name}0` to `{name}{CYCLE_LENGTH - 1}`, each referring to the next and the last
/// to `last`.
fn ref_chain(name: &str, last: &str) -> Map<String, Value> {
    let mut definitions = Map::new();
    for idx in 0..CYCLE_LENGTH {
        let next = if idx + 1 < CYCLE_LENGTH {
            format!("#/$defs/{name}{}", idx + 1)
        } else {
            last.to_string()
        };
        definitions.insert(
            format!("{name}{idx}"),
            json!({"leak-probe": true, "properties": {"next": {"$ref": next}}}),
        );
    }
    definitions
}

fn ref_cycle_schema() -> Value {
    json!({"$defs": ref_chain("C", "#/$defs/C0"), "$ref": "#/$defs/C0"})
}

/// The chain end refers back to its middle, compiled while the end waited.
fn back_into_chain_schema() -> Value {
    let middle = format!("#/$defs/C{}", CYCLE_LENGTH / 2);
    json!({"$defs": ref_chain("C", &middle), "$ref": "#/$defs/C0"})
}

/// The chain end refers back to its start through an anchor, a second name for a compiled node.
fn back_through_anchor_schema() -> Value {
    let mut definitions = ref_chain("C", "#start");
    definitions["C0"]["$anchor"] = json!("start");
    json!({"$defs": definitions, "$ref": "#/$defs/C0"})
}

/// The chain end refers to a definition compiled after the chain was cut, which refers to its start.
fn through_later_definition_schema() -> Value {
    let mut definitions = ref_chain("C", "#/$defs/Later");
    definitions.insert(
        "Later".to_string(),
        json!({"leak-probe": true, "properties": {"next": {"$ref": "#/$defs/C0"}}}),
    );
    json!({
        "$defs": definitions,
        "allOf": [{"$ref": "#/$defs/C0"}, {"$ref": "#/$defs/Later"}]
    })
}

/// Two chains, each cut, the end of one referring into the other and back.
fn across_chains_schema() -> Value {
    let mut definitions = ref_chain("A", &format!("#/$defs/B{}", CYCLE_LENGTH / 2));
    definitions.extend(ref_chain("B", "#/$defs/A0"));
    json!({
        "$defs": definitions,
        "allOf": [{"$ref": "#/$defs/A0"}, {"$ref": "#/$defs/B0"}]
    })
}

/// The schema enters the cycle at a subschema of the chain end, which the end compiles again.
fn into_subschema_schema() -> Value {
    let entry = format!("#/$defs/C{}/properties/next", CYCLE_LENGTH - 1);
    json!({"$defs": ref_chain("C", "#/$defs/C0"), "$ref": entry})
}

fn run_validator(probe: Arc<DropProbe>, schema: &Value, instance: &Value) {
    let validator = jsonschema::options()
        .with_keyword("leak-probe", move |_, _, _| {
            Ok(Box::new(ProbeKeyword {
                _probe: Arc::clone(&probe),
            }))
        })
        .build(schema)
        .expect("schema must compile");

    assert!(validator.is_valid(instance));
}

#[test]
fn recursive_validator_releases_tree_on_drop() {
    // See GH-1125
    let probe = Arc::new(DropProbe);
    let weak: Weak<DropProbe> = Arc::downgrade(&probe);

    let instance = json!({
        "value": 1,
        "children": [
            {"value": 2, "children": []},
            {"value": 3, "children": [{"value": 4, "children": []}]}
        ]
    });
    run_validator(probe, &tree_schema(), &instance);

    assert!(
        weak.upgrade().is_none(),
        "recursive validator tree leaked on drop",
    );
}

#[test]
fn long_ref_cycle_releases_tree_on_drop() {
    let probe = Arc::new(DropProbe);
    let weak: Weak<DropProbe> = Arc::downgrade(&probe);

    run_validator(probe, &ref_cycle_schema(), &json!({"next": {"next": {}}}));

    assert!(weak.upgrade().is_none(), "long `$ref` cycle leaked on drop");
}

#[test_case(&back_into_chain_schema(); "back into the chain")]
#[test_case(&back_through_anchor_schema(); "back through an anchor")]
#[test_case(&through_later_definition_schema(); "through a later definition")]
#[test_case(&across_chains_schema(); "across chains")]
#[test_case(&into_subschema_schema(); "into a subschema")]
fn ref_cycle_through_deferred_target_releases_tree_on_drop(schema: &Value) {
    let probe = Arc::new(DropProbe);
    let weak: Weak<DropProbe> = Arc::downgrade(&probe);

    run_validator(probe, schema, &json!({"next": {"next": {}}}));

    assert!(weak.upgrade().is_none(), "`$ref` cycle leaked on drop");
}

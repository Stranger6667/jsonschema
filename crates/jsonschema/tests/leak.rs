use jsonschema::{Keyword, ValidationError};
use serde_json::{json, Value};
use std::sync::{Arc, Weak};

struct DropProbe;

struct ProbeKeyword {
    _probe: Arc<DropProbe>,
}

impl Keyword for ProbeKeyword {
    fn validate<'i>(&self, _instance: &'i Value) -> Result<(), ValidationError<'i>> {
        Ok(())
    }

    fn is_valid(&self, _instance: &Value) -> bool {
        true
    }
}

fn run_validator(probe: Arc<DropProbe>) {
    let schema = json!({
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
    });
    let instance = json!({
        "value": 1,
        "children": [
            {"value": 2, "children": []},
            {"value": 3, "children": [{"value": 4, "children": []}]}
        ]
    });

    let validator = jsonschema::options()
        .with_keyword("leak-probe", move |_, _, _| {
            Ok(Box::new(ProbeKeyword {
                _probe: Arc::clone(&probe),
            }))
        })
        .build(&schema)
        .expect("schema must compile");

    assert!(validator.is_valid(&instance));
}

#[test]
fn recursive_validator_releases_tree_on_drop() {
    // See GH-1125
    let probe = Arc::new(DropProbe);
    let weak: Weak<DropProbe> = Arc::downgrade(&probe);

    run_validator(probe);

    assert!(
        weak.upgrade().is_none(),
        "recursive validator tree leaked on drop",
    );
}

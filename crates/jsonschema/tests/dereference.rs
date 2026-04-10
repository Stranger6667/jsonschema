use jsonschema::ReferencingError;
use serde_json::{json, Value};
use test_case::test_case;

const ADDR_URI: &str = "https://example.com/address.json";

fn address_schema() -> Value {
    json!({
        "$id": ADDR_URI,
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "street": {"type": "string"},
            "city": {"type": "string"}
        },
        "required": ["street", "city"]
    })
}

fn registry_with_address() -> jsonschema::Registry<'static> {
    jsonschema::Registry::new()
        .add(ADDR_URI, address_schema())
        .expect("resource should be accepted")
        .prepare()
        .expect("registry build failed")
}

#[test_case(
    json!({"type": "string", "minLength": 1}),
    json!({"type": "string", "minLength": 1});
    "no_refs_unchanged"
)]
#[test_case(
    json!({
        "$defs": {
            "address": {
                "type": "object",
                "properties": {
                    "street": {"type": "string"},
                    "city": {"type": "string"}
                }
            }
        },
        "properties": {
            "home": {"$ref": "#/$defs/address"}
        }
    }),
    json!({
        "$defs": {
            "address": {
                "type": "object",
                "properties": {
                    "street": {"type": "string"},
                    "city": {"type": "string"}
                }
            }
        },
        "properties": {
            "home": {
                "type": "object",
                "properties": {
                    "street": {"type": "string"},
                    "city": {"type": "string"}
                }
            }
        }
    });
    "simple_fragment_ref_inlined"
)]
#[test_case(
    json!({
        "$defs": {
            "name": {"type": "string"},
            "person": {
                "type": "object",
                "properties": {
                    "first": {"$ref": "#/$defs/name"}
                }
            }
        },
        "$ref": "#/$defs/person"
    }),
    json!({
        "$defs": {
            "name": {"type": "string"},
            "person": {
                "type": "object",
                "properties": {
                    "first": {"type": "string"}
                }
            }
        },
        "type": "object",
        "properties": {
            "first": {"type": "string"}
        }
    });
    "nested_refs_fully_inlined"
)]
#[test_case(
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://example.com/node.json",
        "type": "object",
        "properties": {
            "children": {
                "type": "array",
                "items": {"$ref": "https://example.com/node.json"}
            }
        }
    }),
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://example.com/node.json",
        "type": "object",
        "properties": {
            "children": {
                "type": "array",
                "items": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "$id": "https://example.com/node.json",
                    "type": "object",
                    "properties": {
                        "children": {
                            "type": "array",
                            "items": {"$ref": "https://example.com/node.json"}
                        }
                    }
                }
            }
        }
    });
    "circular_self_ref_left_in_place"
)]
#[test_case(
    json!({
        "$defs": {
            "item": {
                "type": "object",
                "properties": {
                    "next": {"$ref": "#/$defs/item"}
                }
            }
        },
        "$ref": "#/$defs/item"
    }),
    json!({
        "$defs": {
            "item": {
                "type": "object",
                "properties": {
                    "next": {
                        "type": "object",
                        "properties": {
                            "next": {"$ref": "#/$defs/item"}
                        }
                    }
                }
            }
        },
        "type": "object",
        "properties": {
            "next": {"$ref": "#/$defs/item"}
        }
    });
    "circular_fragment_ref_left_in_place"
)]
#[test_case(
    json!({
        "$defs": {
            "tag": {"type": "string"}
        },
        "properties": {
            "a": {"$ref": "#/$defs/tag"},
            "b": {"$ref": "#/$defs/tag"}
        }
    }),
    json!({
        "$defs": {
            "tag": {"type": "string"}
        },
        "properties": {
            "a": {"type": "string"},
            "b": {"type": "string"}
        }
    });
    "diamond_graph_both_paths_inlined"
)]
#[test_case(
    json!({
        "$defs": {
            "base": {"type": "integer"}
        },
        "properties": {
            "count": {
                "$ref": "#/$defs/base",
                "description": "how many"
            }
        }
    }),
    json!({
        "$defs": {
            "base": {"type": "integer"}
        },
        "properties": {
            "count": {"type": "integer", "description": "how many"}
        }
    });
    "sibling_keys_merged"
)]
#[test_case(
    json!({
        "$defs": {
            "tag": {"type": "string"}
        },
        "type": "array",
        "items": {"$ref": "#/$defs/tag"}
    }),
    json!({
        "$defs": {
            "tag": {"type": "string"}
        },
        "type": "array",
        "items": {"type": "string"}
    });
    "array_items_inlined"
)]
#[test_case(
    json!({
        "$schema": "http://json-schema.org/draft-04/schema#",
        "definitions": {
            "name": {"type": "string"}
        },
        "properties": {
            "first": {"$ref": "#/definitions/name"}
        }
    }),
    json!({
        "$schema": "http://json-schema.org/draft-04/schema#",
        "definitions": {
            "name": {"type": "string"}
        },
        "properties": {
            "first": {"type": "string"}
        }
    });
    "draft4_schema"
)]
#[allow(clippy::needless_pass_by_value)]
fn test_dereference(schema: Value, expected: Value) {
    assert_eq!(jsonschema::dereference(&schema).unwrap(), expected);
}

#[test]
fn test_dereference_with_registry() {
    // Exercises the registry fast-path in dereference_with_options
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": ADDR_URI
    });
    let registry = registry_with_address();
    let result = jsonschema::options()
        .with_registry(&registry)
        .dereference(&schema)
        .expect("dereference failed");
    assert_eq!(
        result,
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": ADDR_URI,
            "type": "object",
            "properties": {
                "street": {"type": "string"},
                "city": {"type": "string"}
            },
            "required": ["street", "city"]
        })
    );
}

#[test]
fn test_unknown_ref_returns_error() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/does-not-exist.json"
    });
    let err = jsonschema::dereference(&schema).unwrap_err();
    assert!(matches!(err, ReferencingError::Unretrievable { .. }));
}

#[test]
fn test_mutual_recursion_left_in_place() {
    // A references B, B references A — indirect cycle
    let schema = json!({
        "$defs": {
            "a": {
                "type": "object",
                "properties": {
                    "b": {"$ref": "#/$defs/b"}
                }
            },
            "b": {
                "type": "object",
                "properties": {
                    "a": {"$ref": "#/$defs/a"}
                }
            }
        },
        "$ref": "#/$defs/a"
    });
    let result = jsonschema::dereference(&schema).unwrap();
    // Top-level $ref to a is inlined
    assert_eq!(result["type"], "object");
    // b is inlined inside a
    assert_eq!(result["properties"]["b"]["type"], "object");
    // a inside b is circular — left as $ref
    assert_eq!(
        result["properties"]["b"]["properties"]["a"]["$ref"],
        "#/$defs/a"
    );
}

#[cfg(all(feature = "resolve-async", not(target_arch = "wasm32")))]
mod async_tests {
    use super::*;

    #[tokio::test]
    async fn test_async_dereference_no_external_refs() {
        let schema = json!({"type": "integer", "minimum": 0});
        let result = jsonschema::async_dereference(&schema)
            .await
            .expect("async dereference failed");
        assert_eq!(result, schema);
    }

    #[tokio::test]
    async fn test_async_dereference_with_registry() {
        // Exercises the registry fast-path in dereference_with_options_async
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$ref": ADDR_URI
        });
        let registry = registry_with_address();
        let result = jsonschema::async_options()
            .with_registry(&registry)
            .dereference(&schema)
            .await
            .expect("async dereference failed");
        assert_eq!(
            result,
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$id": ADDR_URI,
                "type": "object",
                "properties": {
                    "street": {"type": "string"},
                    "city": {"type": "string"}
                },
                "required": ["street", "city"]
            })
        );
    }

    #[tokio::test]
    async fn test_async_dereference_without_registry() {
        // Exercises the build_registry_async path in dereference_with_options_async
        let schema = json!({
            "$defs": {
                "tag": {"type": "string"}
            },
            "properties": {
                "name": {"$ref": "#/$defs/tag"}
            }
        });
        let result = jsonschema::async_dereference(&schema)
            .await
            .expect("async dereference failed");
        assert_eq!(
            result,
            json!({
                "$defs": {"tag": {"type": "string"}},
                "properties": {"name": {"type": "string"}}
            })
        );
    }

    #[tokio::test]
    async fn test_async_dereference_unresolvable_ref() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$ref": "https://example.com/does-not-exist.json"
        });
        let result = jsonschema::async_dereference(&schema).await;
        assert!(
            matches!(result, Err(ReferencingError::Unretrievable { .. })),
            "expected Unretrievable, got: {result:?}"
        );
    }
}

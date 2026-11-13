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
#[test_case(
    json!({
        "$defs": {"int": {"type": "integer"}},
        "properties": {"x": {"const": {"$ref": "#/$defs/int"}}}
    }),
    json!({
        "$defs": {"int": {"type": "integer"}},
        "properties": {"x": {"const": {"$ref": "#/$defs/int"}}}
    });
    "ref_in_const_is_data"
)]
#[test_case(
    json!({
        "$defs": {"int": {"type": "integer"}},
        "enum": [{"$ref": "#/$defs/int"}, 1]
    }),
    json!({
        "$defs": {"int": {"type": "integer"}},
        "enum": [{"$ref": "#/$defs/int"}, 1]
    });
    "ref_in_enum_is_data"
)]
#[test_case(
    json!({"default": {"$ref": "#/$defs/missing"}}),
    json!({"default": {"$ref": "#/$defs/missing"}});
    "ref_in_default_is_data"
)]
#[test_case(
    json!({"properties": {"x": {"examples": [{"$ref": "#/$defs/missing"}]}}}),
    json!({"properties": {"x": {"examples": [{"$ref": "#/$defs/missing"}]}}});
    "ref_in_examples_is_data"
)]
#[test_case(
    json!({
        "$defs": {"int": {"type": "integer"}},
        "properties": {"x": {"$ref": "#/$defs/int", "default": {"$ref": "#/$defs/missing"}}}
    }),
    json!({
        "$defs": {"int": {"type": "integer"}},
        "properties": {"x": {"type": "integer", "default": {"$ref": "#/$defs/missing"}}}
    });
    "ref_in_sibling_default_is_data"
)]
#[test_case(
    json!({
        "$defs": {"int": {"type": "integer"}},
        "properties": {"const": {"$ref": "#/$defs/int"}}
    }),
    json!({
        "$defs": {"int": {"type": "integer"}},
        "properties": {"const": {"type": "integer"}}
    });
    "property_named_const_inlined"
)]
#[test_case(
    json!({
        "$defs": {"money": {"type": "number", "minimum": -1000}},
        "properties": {"subtotal": {"$ref": "#/$defs/money", "minimum": 0}}
    }),
    json!({
        "$defs": {"money": {"type": "number", "minimum": -1000}},
        "properties": {"subtotal": {"allOf": [{"type": "number", "minimum": -1000}], "minimum": 0}}
    });
    "colliding_sibling_kept_next_to_target"
)]
#[test_case(
    json!({
        "$schema": "https://json-schema.org/draft/2019-09/schema",
        "$defs": {"money": {"minimum": -1000}},
        "properties": {"subtotal": {"$ref": "#/$defs/money", "minimum": 0}}
    }),
    json!({
        "$schema": "https://json-schema.org/draft/2019-09/schema",
        "$defs": {"money": {"minimum": -1000}},
        "properties": {"subtotal": {"allOf": [{"minimum": -1000}], "minimum": 0}}
    });
    "colliding_sibling_kept_next_to_target_draft2019_09"
)]
#[test_case(
    json!({
        "$defs": {"low": {"allOf": [{"minimum": -1000}]}},
        "properties": {"x": {"$ref": "#/$defs/low", "allOf": [{"minimum": 0}]}}
    }),
    json!({
        "$defs": {"low": {"allOf": [{"minimum": -1000}]}},
        "properties": {"x": {"allOf": [{"allOf": [{"minimum": -1000}]}, {"minimum": 0}]}}
    });
    "colliding_all_of_sibling_prepends_target"
)]
#[test_case(
    json!({
        "$defs": {"any": true},
        "properties": {"x": {"$ref": "#/$defs/any", "minimum": 0}}
    }),
    json!({
        "$defs": {"any": true},
        "properties": {"x": {"allOf": [true], "minimum": 0}}
    });
    "boolean_target_keeps_siblings"
)]
#[test_case(
    json!({
        "$defs": {"any": true},
        "properties": {"x": {"$ref": "#/$defs/any"}}
    }),
    json!({
        "$defs": {"any": true},
        "properties": {"x": true}
    });
    "boolean_target_without_siblings_inlined"
)]
// Draft 7 ignores `$ref` siblings
#[test_case(
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "definitions": {"money": {"minimum": -1000}},
        "properties": {"subtotal": {"$ref": "#/definitions/money", "minimum": 0}}
    }),
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "definitions": {"money": {"minimum": -1000}},
        "properties": {"subtotal": {"minimum": -1000}}
    });
    "draft7_colliding_sibling_ignored"
)]
#[test_case(
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "definitions": {"money": {"type": "number"}},
        "properties": {"subtotal": {"$ref": "#/definitions/money", "minimum": 0}}
    }),
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "definitions": {"money": {"type": "number"}},
        "properties": {"subtotal": {"type": "number"}}
    });
    "draft7_validating_sibling_ignored"
)]
#[test_case(
    json!({
        "$schema": "http://json-schema.org/draft-04/schema#",
        "definitions": {"money": {"type": "number"}},
        "properties": {"subtotal": {"$ref": "#/definitions/money", "minimum": 0}}
    }),
    json!({
        "$schema": "http://json-schema.org/draft-04/schema#",
        "definitions": {"money": {"type": "number"}},
        "properties": {"subtotal": {"type": "number"}}
    });
    "draft4_validating_sibling_ignored"
)]
// Draft 7 ignores `$id` beside `$ref`, so it must not become the base URI of the result
#[test_case(
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$id": "https://example.com/root.json",
        "definitions": {"int": {"type": "integer"}},
        "description": "d",
        "$ref": "#/definitions/int"
    }),
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "definitions": {"int": {"type": "integer"}},
        "description": "d",
        "type": "integer"
    });
    "draft7_id_sibling_ignored"
)]
#[test_case(
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://example.com/root.json",
        "$anchor": "root",
        "$comment": "c",
        "$defs": {"int": {"type": "integer"}},
        "definitions": {},
        "title": "t",
        "description": "d",
        "default": 1,
        "examples": [1],
        "deprecated": true,
        "readOnly": true,
        "writeOnly": false,
        "$ref": "#/$defs/int"
    }),
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://example.com/root.json",
        "$anchor": "root",
        "$comment": "c",
        "$defs": {"int": {"type": "integer"}},
        "definitions": {},
        "title": "t",
        "description": "d",
        "default": 1,
        "examples": [1],
        "deprecated": true,
        "readOnly": true,
        "writeOnly": false,
        "type": "integer"
    });
    "non_validating_siblings_merged"
)]
#[allow(clippy::needless_pass_by_value)]
fn test_dereference(schema: Value, expected: Value) {
    assert_eq!(jsonschema::dereference(&schema).unwrap(), expected);
}

// Merging these would let a sibling change what the target accepts, or the reverse
#[test_case(json!({"additionalProperties": false}), json!({"properties": {"b": {}}}); "additional_properties_beside_properties")]
#[test_case(json!({"unevaluatedProperties": false}), json!({"properties": {"b": {}}}); "unevaluated_properties_beside_properties")]
#[test_case(json!({"items": false}), json!({"prefixItems": [{}]}); "items_beside_prefix_items")]
#[test_case(json!({"then": {"minimum": 0}}), json!({"if": {"type": "number"}}); "then_beside_if")]
#[test_case(json!({"contains": {"type": "string"}}), json!({"minContains": 2}); "contains_beside_min_contains")]
#[allow(clippy::needless_pass_by_value)]
fn test_dereference_keeps_target_apart_from_validating_siblings(target: Value, siblings: Value) {
    let mut schema = json!({"$defs": {"target": target.clone()}, "$ref": "#/$defs/target"});
    let mut expected = json!({"$defs": {"target": target.clone()}, "allOf": [target]});
    for (key, value) in siblings.as_object().expect("siblings are an object") {
        schema[key] = value.clone();
        expected[key] = value.clone();
    }
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

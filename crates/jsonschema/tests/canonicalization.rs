use std::{
    cmp::Ordering,
    collections::{hash_map::DefaultHasher, HashSet},
    hash::{Hash, Hasher},
};

use jsonschema::{
    canonical::{
        options, CanonicalKind, CanonicalSchema, CanonicalView, Containment, Distinctness,
        ObjectViolationView, OperandMismatch, Satisfiability,
    },
    canonicalize, validator_for, CanonicalizationError, Draft, JsonType, PatternOptions, Registry,
    Retrieve, Uri,
};
use serde_json::{json, Map, Number, Value};
use test_case::test_case;

#[test_case(&json!({"if": {}, "unevaluatedProperties": false}); "unevaluated properties beside an applicator")]
fn unsupported_document_round_trips_verbatim(schema: &Value) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    assert_eq!(&canonical.to_json_schema(), schema);
    assert!(matches!(canonical.view(), CanonicalView::Raw(_)));
}

#[test]
fn reference_view_exposes_its_canonical_definition() {
    let canonical = canonicalize(&json!({
        "$ref": "#/$defs/user",
        "$defs": {
            "user": {
                "allOf": [
                    {"type": "integer"},
                    {"minimum": 0}
                ]
            }
        }
    }))
    .expect("canonicalizes");

    assert_eq!(
        canonical.view(),
        CanonicalView::Reference("#/$defs/user".to_string())
    );
    let definitions: Vec<_> = canonical.definitions().collect();
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].0, "#/$defs/user");
    assert!(matches!(definitions[0].1.view(), CanonicalView::Integer(_)));
}

#[test]
fn external_registry_reference_emits_a_self_contained_definition() {
    let external = json!({
        "$id": "https://example.com/external",
        "$anchor": "value",
        "type": "string"
    });
    let registry = Registry::new()
        .add("https://example.com/external", &external)
        .expect("resource URI is valid")
        .prepare()
        .expect("registry prepares");
    let canonical = options()
        .with_registry(&registry)
        .canonicalize(&json!({"$ref": "https://example.com/external#value"}))
        .expect("canonicalizes");

    let reference =
        "urn:jsonschema:canonical:https%3A%2F%2Fexample.com%2Fexternal%23value".to_string();
    assert_eq!(
        canonical.view(),
        CanonicalView::Reference(reference.clone())
    );
    assert_eq!(
        canonical
            .definitions()
            .map(|(uri, _)| uri)
            .collect::<Vec<_>>(),
        vec![reference]
    );

    let emitted = canonical.to_json_schema();
    let validator =
        jsonschema::validator_for(&emitted).expect("canonical output is self-contained");
    assert!(validator.is_valid(&json!("value")));
    assert!(!validator.is_valid(&json!(1)));
}

// The root spells no dynamic reference, so only the registered resource forces the dynamic scope.
#[test]
fn dynamic_reference_only_in_an_external_resource_is_modeled() {
    let external = json!({
        "$id": "https://example.com/list",
        "$dynamicRef": "#num",
        "$defs": {
            "n": {"$dynamicAnchor": "num", "type": "number"}
        }
    });
    let registry = Registry::new()
        .add("https://example.com/list", &external)
        .expect("resource URI is valid")
        .prepare()
        .expect("registry prepares");
    let canonical = options()
        .with_registry(&registry)
        .canonicalize(&json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$ref": "https://example.com/list"
        }))
        .expect("canonicalizes");
    assert_eq!(canonical.kind(), CanonicalKind::Reference);

    let emitted = canonical.to_json_schema();
    let validator =
        jsonschema::validator_for(&emitted).expect("canonical output is self-contained");
    assert!(validator.is_valid(&json!(1)));
    assert!(!validator.is_valid(&json!("x")));
}

struct StaticRetriever;

impl Retrieve for StaticRetriever {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        if uri.as_str() == "https://example.com/remote" {
            Ok(json!({"type": "string"}))
        } else {
            Err(format!("Unknown reference: {uri}").into())
        }
    }
}

#[test]
fn retriever_fetches_a_reference_absent_from_the_registry() {
    let canonical = options()
        .with_retriever(StaticRetriever)
        .canonicalize(&json!({"$ref": "https://example.com/remote"}))
        .expect("canonicalizes");

    let emitted = canonical.to_json_schema();
    let validator = validator_for(&emitted).expect("canonical output is self-contained");
    assert!(validator.is_valid(&json!("value")));
    assert!(!validator.is_valid(&json!(1)));
}

#[test]
fn base_uri_resolves_a_relative_reference_into_the_registry() {
    let external = json!({"type": "string"});
    let registry = Registry::new()
        .add("https://example.com/external", &external)
        .expect("resource URI is valid")
        .prepare()
        .expect("registry prepares");
    let canonical = options()
        .with_registry(&registry)
        .with_base_uri("https://example.com/root")
        .canonicalize(&json!({"$ref": "external"}))
        .expect("canonicalizes");

    let emitted = canonical.to_json_schema();
    let validator = validator_for(&emitted).expect("canonical output is self-contained");
    assert!(validator.is_valid(&json!("value")));
    assert!(!validator.is_valid(&json!(1)));
}

fn assert_validation_parity(schema: &Value, instances: &[Value]) {
    let canonical = canonicalize(schema)
        .expect("canonicalizes")
        .to_json_schema();
    let raw = jsonschema::validator_for(schema).expect("raw builds");
    let canon = jsonschema::validator_for(&canonical).expect("canonical builds");
    for instance in instances {
        assert_eq!(
            raw.is_valid(instance),
            canon.is_valid(instance),
            "parity disagrees on {instance}\n  canonical = {canonical}"
        );
    }
}

// The registry never indexes an anchor spelled inside a `const` value, so it must not bind, and
// re-canonicalizing must not grow the definition keys it would have minted.
#[test]
fn dynamic_anchor_in_a_const_value_does_not_bind() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://example.com/phantom/root",
        "properties": {
            "marker": {"const": {"$dynamicAnchor": "T"}},
            "a": {"$ref": "num"},
            "b": {"$ref": "str"}
        },
        "$defs": {
            "num": {
                "$id": "num",
                "$defs": {"bind": {"$dynamicAnchor": "T", "type": "number"}},
                "$ref": "shared"
            },
            "str": {
                "$id": "str",
                "$defs": {"bind": {"$dynamicAnchor": "T", "type": "string"}},
                "$ref": "shared"
            },
            "shared": {
                "$id": "shared",
                "$defs": {"fallback": {"$dynamicAnchor": "T", "type": "boolean"}},
                "items": {"$dynamicRef": "#T"}
            }
        }
    });
    assert_validation_parity(
        &schema,
        &[
            json!({"a": [1]}),
            json!({"a": ["s"]}),
            json!({"b": ["s"]}),
            json!({"b": [1]}),
        ],
    );
    let once = canonicalize(&schema)
        .expect("canonicalizes")
        .to_json_schema();
    let twice = canonicalize(&once)
        .expect("canonicalizes again")
        .to_json_schema();
    assert_eq!(once, twice);
}

// An anchor-less resource in the middle of the dynamic scope stops the `$recursiveRef` walk, so
// the two paths land differently and cannot share one definition.
#[test]
fn recursive_ref_chain_breaks_at_an_anchorless_resource() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2019-09/schema",
        "$id": "https://example.com/chain/root",
        "$recursiveAnchor": true,
        "properties": {
            "direct": {"$ref": "shared"},
            "via": {"$ref": "m"}
        },
        "$defs": {
            "m": {"$id": "m", "$ref": "a2"},
            "a2": {"$id": "a2", "$recursiveAnchor": true, "maxProperties": 1, "$ref": "shared"},
            "shared": {
                "$id": "shared",
                "$recursiveAnchor": true,
                "properties": {"deep": {"$recursiveRef": "#"}}
            }
        }
    });
    assert_validation_parity(
        &schema,
        &[
            json!({"direct": {"deep": {"q": 1, "r": 2}}}),
            json!({"via": {"deep": {"q": 1, "r": 2}}}),
            json!({"direct": {"deep": {}}}),
            json!({"via": {"deep": {}}}),
        ],
    );
}

// A `$recursiveRef` under a lexically entered anchored resource lands on that resource, not on
// the root, so the back-reference cannot take the `#` spelling.
#[test]
fn back_reference_through_an_anchored_resource_is_not_the_root() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2019-09/schema",
        "$id": "https://example.com/back/root",
        "$recursiveAnchor": true,
        "properties": {
            "wrap": {
                "$id": "x",
                "$recursiveAnchor": true,
                "maxProperties": 1,
                "properties": {"back": {"$ref": "https://example.com/back/root"}}
            },
            "k": {"$recursiveRef": "#"}
        }
    });
    assert_validation_parity(
        &schema,
        &[
            json!({"k": {"q": 1, "r": 2}}),
            json!({"wrap": {"back": {"k": {"q": 1, "r": 2}}}}),
            json!({"wrap": {"back": {"k": {}}}}),
        ],
    );
}

// A `$dynamicAnchor` re-declared by an embedded resource binds for paths through that resource,
// even though the root already bound the same name.
#[test]
fn redeclared_dynamic_anchor_in_an_embedded_resource_binds() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://example.com/redeclare/root",
        "$defs": {
            "rootbind": {"$dynamicAnchor": "X", "type": "object"},
            "shared": {
                "$id": "shared",
                "$defs": {"fallback": {"$dynamicAnchor": "X", "type": "boolean"}},
                "items": {"$dynamicRef": "#X"}
            }
        },
        "properties": {
            "a": {"$ref": "shared"},
            "b": {
                "$id": "emb",
                "$defs": {"embbind": {"$dynamicAnchor": "X", "required": ["c"], "type": "object"}},
                "properties": {"inner": {"$ref": "shared"}}
            }
        }
    });
    assert_validation_parity(
        &schema,
        &[
            json!({"a": [{}]}),
            json!({"b": {"inner": [{}]}}),
            json!({"b": {"inner": [{"c": 1}]}}),
        ],
    );
}

// Anchors the registry attributes to a resource bind for paths through it, whether the entry is a
// pointer fragment into its middle or the binder sits behind a bare `id` key (not a 2020-12
// identifier).
#[test_case(&json!({
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "$id": "https://example.com/pointer/root",
    "$defs": {
        "sub": {
            "$id": "sub",
            "$defs": {
                "bind": {"$dynamicAnchor": "T", "type": "number"},
                "middle": {"$ref": "https://example.com/pointer/shared"}
            }
        },
        "shared": {
            "$id": "shared",
            "$defs": {"fallback": {"$dynamicAnchor": "T", "type": "string"}},
            "items": {"$dynamicRef": "#T"}
        }
    },
    "properties": {
        "a": {"$ref": "sub#/$defs/middle"},
        "b": {"$ref": "shared"}
    }
}) ; "pointer entry")]
#[test_case(&json!({
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "$id": "https://example.com/stray/root",
    "$defs": {
        "holder": {
            "$id": "holder",
            "$defs": {
                "bind": {"id": "stray", "$dynamicAnchor": "T", "type": "number"},
                "use": {"$ref": "https://example.com/stray/shared"}
            }
        },
        "shared": {
            "$id": "shared",
            "$defs": {"fallback": {"$dynamicAnchor": "T", "type": "string"}},
            "items": {"$dynamicRef": "#T"}
        }
    },
    "properties": {
        "a": {"$ref": "holder#/$defs/use"},
        "b": {"$ref": "shared"}
    }
}) ; "stray id key")]
fn resource_anchors_bind_below_any_entry_point(schema: &Value) {
    assert_validation_parity(
        schema,
        &[
            json!({"a": [1]}),
            json!({"a": ["s"]}),
            json!({"b": ["s"]}),
            json!({"b": [1]}),
        ],
    );
}

// `$recursiveAnchor` counts only at a resource root; a nested spelling binds nothing.
#[test]
fn nested_recursive_anchor_does_not_bind_the_resource() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2019-09/schema",
        "$id": "https://example.com/nested/root",
        "$defs": {
            "phantom": {"properties": {"x": {"$recursiveAnchor": true}}},
            "a": {"$id": "a", "$recursiveAnchor": true, "maxProperties": 1, "properties": {"m": {"$ref": "shared"}}},
            "b": {"$id": "b", "$recursiveAnchor": true, "properties": {"m": {"$ref": "shared"}}},
            "shared": {
                "$id": "shared",
                "$recursiveAnchor": true,
                "properties": {"deep": {"$recursiveRef": "#"}}
            }
        },
        "properties": {
            "pa": {"$ref": "a"},
            "pb": {"$ref": "b"}
        }
    });
    assert_validation_parity(
        &schema,
        &[
            json!({"pa": {"m": {"deep": {"q": 1, "r": 2}}}}),
            json!({"pb": {"m": {"deep": {"q": 1, "r": 2}}}}),
            json!({"pa": {"m": {"deep": {}}}}),
        ],
    );
}

// A relative `$id` beside `$ref` shifts the base once; the sibling reparse must not apply it again.
#[test]
fn relative_id_beside_ref_with_assertion_siblings_canonicalizes() {
    let canonical = canonicalize(&json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://example.com/root.json",
        "properties": {
            "a": {
                "$id": "nested/inner.json",
                "$ref": "other.json",
                "type": "object",
                "properties": {
                    "b": {"$ref": "sibling.json"}
                }
            }
        },
        "$defs": {
            "o": {"$id": "nested/other.json", "minProperties": 1},
            "s": {"$id": "nested/sibling.json", "type": "integer"}
        }
    }))
    .expect("canonicalizes");

    let emitted = canonical.to_json_schema();
    let validator =
        jsonschema::validator_for(&emitted).expect("canonical output is self-contained");
    assert!(validator.is_valid(&json!({"a": {"b": 1}})));
    assert!(!validator.is_valid(&json!({"a": {"b": "s"}})));
    assert!(!validator.is_valid(&json!({"a": {}})));
}

#[test]
fn absolute_self_reference_collapses_to_the_root_pointer() {
    // A ref spelled as the root's own `$id` resolves to the whole document. Detection relies on the
    // registry borrowing the root `Value` (pointer identity), so a non-`#` spelling must still emit
    // the `#` self-pointer rather than a canonical URN definition.
    let canonical = canonicalize(&json!({
        "$id": "https://example.com/root",
        "type": "object",
        "properties": {
            "self": {"$ref": "https://example.com/root"}
        }
    }))
    .expect("canonicalizes");

    let emitted = canonical.to_json_schema();
    assert_eq!(emitted["properties"]["self"], json!({"$ref": "#"}));
    assert!(
        canonical.definitions().next().is_none(),
        "the root self-reference is not materialized as a definition"
    );
}

#[test]
fn registry_already_holding_the_root_uri_still_canonicalizes() {
    // A caller may pre-register the very schema they canonicalize. `build` re-adds the root under its
    // own base URI, so this exercises the add-under-an-existing-URI path.
    let schema = json!({
        "$id": "https://example.com/root",
        "$ref": "#/$defs/value",
        "$defs": {"value": {"type": "string"}}
    });
    let registry = Registry::new()
        .add("https://example.com/root", &schema)
        .expect("resource URI is valid")
        .prepare()
        .expect("registry prepares");

    let canonical = options()
        .with_registry(&registry)
        .canonicalize(&schema)
        .expect("canonicalizes despite the pre-registered root");

    assert_eq!(
        canonical.view(),
        CanonicalView::Reference("#/$defs/value".to_string())
    );
    let definitions: Vec<_> = canonical.definitions().collect();
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].0, "#/$defs/value");
    assert_eq!(
        canonical.to_json_schema()["$defs"]["value"],
        json!({"type": "string"})
    );
}

#[test]
fn a_definition_name_spelling_a_canonical_uri_does_not_alias_a_registry_resource() {
    // Both references mint the same key, so aliasing them canonicalized this to `false` while the
    // validator accepted `"x"`. Only a registry reaches this route; the suite covers the `$id` one.
    let remote = json!({"$id": "http://remote/a", "type": "string"});
    let registry = Registry::new()
        .add("http://remote/a", &remote)
        .expect("resource URI is valid")
        .prepare()
        .expect("registry prepares");
    let schema = json!({
        "anyOf": [
            {"$ref": "#/$defs/urn:jsonschema:canonical:http%253A%252F%252Fremote%252Fa"},
            {"$ref": "http://remote/a"}
        ],
        "$defs": {
            "urn:jsonschema:canonical:http%3A%2F%2Fremote%2Fa": {
                "type": "object",
                "required": ["p"],
                "properties": {
                    "p": {"$ref": "#/$defs/urn:jsonschema:canonical:http%253A%252F%252Fremote%252Fa"}
                }
            }
        }
    });

    let canonical = options()
        .with_registry(&registry)
        .canonicalize(&schema)
        .expect("canonicalizes");
    assert_eq!(canonical.kind(), CanonicalKind::Raw);
    assert_ne!(
        canonical.satisfiability(),
        Satisfiability::No,
        "the string branch admits a value, so the document is not empty"
    );

    let validator = jsonschema::options()
        .with_registry(&registry)
        .build(&schema)
        .expect("builds");
    assert!(validator.is_valid(&json!("x")));
}

#[test]
fn unsupported_reference_target_keeps_the_whole_document_raw() {
    let schema = json!({
        "$ref": "#/$defs/value",
        "$defs": {"value": {"if": {}, "unevaluatedProperties": false}}
    });
    let canonical = canonicalize(&schema).expect("canonicalizes");

    assert!(matches!(canonical.view(), CanonicalView::Raw(_)));
    assert_eq!(canonical.to_json_schema(), schema);
}

#[test]
fn symbolic_reference_operations_have_distinct_views() {
    // A cycle keeps the conjunction symbolic: such a document is not folded through its targets.
    let intersection = canonicalize(&json!({
        "allOf": [
            {"$ref": "#/$defs/left"},
            {"$ref": "#/$defs/right"}
        ],
        "$defs": {
            "left": {"type": "object", "properties": {"next": {"$ref": "#/$defs/left"}}},
            "right": {"minProperties": 1}
        }
    }))
    .expect("canonicalizes");

    let CanonicalView::AllOf(branches) = intersection.view() else {
        panic!("expected an AllOf view");
    };
    assert!(branches
        .iter()
        .all(|branch| matches!(branch.view(), CanonicalView::Reference(_))));

    let complement = canonicalize(&json!({
        "not": {"$ref": "#/$defs/other"},
        "$defs": {"other": {"type": "object", "properties": {"child": {"$ref": "#/$defs/other"}}}}
    }))
    .expect("canonicalizes");
    let CanonicalView::Not(inner) = complement.view() else {
        panic!("expected a Not view");
    };
    assert!(matches!(inner.view(), CanonicalView::Reference(_)));
}

#[test]
fn string_view_exposes_facets() {
    let CanonicalView::String(view) =
        canonicalize(&json!({"type": "string", "minLength": 2, "pattern": "^a"}))
            .unwrap()
            .view()
    else {
        panic!("expected a String view");
    };
    assert_eq!(view.min_length, Some(Number::from(2u64)));
    assert_eq!(view.max_length, None);
    assert_eq!(view.patterns, vec!["^a".to_string()]);
}

#[test]
fn integer_view_exposes_bounds() {
    let CanonicalView::Integer(view) =
        canonicalize(&json!({"type": "integer", "minimum": 2, "maximum": 9}))
            .unwrap()
            .view()
    else {
        panic!("expected an Integer view");
    };
    assert_eq!(view.minimum, Some(Number::from(2)));
    assert_eq!(view.maximum, Some(Number::from(9)));
}

#[test]
fn array_view_exposes_bounds() {
    let CanonicalView::Array(view) =
        canonicalize(&json!({"type": "array", "minItems": 1, "maxItems": 3, "uniqueItems": true}))
            .unwrap()
            .view()
    else {
        panic!("expected an Array view");
    };
    assert_eq!(view.min_items, Some(Number::from(1u64)));
    assert_eq!(view.max_items, Some(Number::from(3u64)));
    assert_eq!(view.distinctness, Distinctness::AllDistinct);
    assert!(view.prefix_items.is_empty());
}

#[test]
fn array_view_exposes_a_repeat_demand() {
    let CanonicalView::Array(view) = canonicalize(
        &json!({"type": "array", "allOf": [{"not": {"type": "array", "uniqueItems": true}}]}),
    )
    .unwrap()
    .view() else {
        panic!("expected an Array view");
    };
    assert_eq!(view.min_items, Some(Number::from(2u64)));
    assert_eq!(view.distinctness, Distinctness::SomeRepeated);
    assert_eq!(view.distinctness.as_str(), "some_repeated");
}

#[test]
fn array_view_exposes_prefix_items() {
    let CanonicalView::Array(view) = canonicalize(&json!({
        "type": "array",
        "prefixItems": [{"type": "integer"}, {"type": "string"}],
        "items": {"type": "boolean"}
    }))
    .unwrap()
    .view() else {
        panic!("expected an Array view");
    };
    let spellings: Vec<_> = view
        .prefix_items
        .iter()
        .map(CanonicalSchema::to_json_schema)
        .collect();
    assert_eq!(
        spellings,
        vec![
            json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "integer"}),
            json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "string"})
        ]
    );
    assert_eq!(
        view.items.unwrap().to_json_schema(),
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "boolean"})
    );
}

#[test]
fn object_view_exposes_bounds() {
    let CanonicalView::Object(view) = canonicalize(
        &json!({"type": "object", "minProperties": 1, "maxProperties": 3, "required": ["a"]}),
    )
    .unwrap()
    .view() else {
        panic!("expected an Object view");
    };
    // A required key already demands the one property `minProperties` asked for.
    assert_eq!(view.min_properties, None);
    assert_eq!(view.max_properties, Some(Number::from(3u64)));
    assert_eq!(view.required, vec!["a".to_string()]);
    assert!(view.property_names.is_none());
    assert!(view.properties.is_empty());
}

#[test]
fn object_view_exposes_properties() {
    let CanonicalView::Object(view) =
        canonicalize(&json!({"type": "object", "properties": {"a": {"type": "integer"}}}))
            .unwrap()
            .view()
    else {
        panic!("expected an Object view");
    };
    let schema = view.properties.get("a").expect("a property schema");
    assert_eq!(
        schema.to_json_schema(),
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "integer"})
    );
}

#[test]
fn object_view_exposes_property_names() {
    let CanonicalView::Object(view) =
        canonicalize(&json!({"type": "object", "propertyNames": {"maxLength": 4}}))
            .unwrap()
            .view()
    else {
        panic!("expected an Object view");
    };
    let names = view.property_names.expect("a key constraint");
    assert_eq!(
        names.to_json_schema(),
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "string",
            "maxLength": 4
        })
    );
}

#[test]
fn object_view_exposes_name_fails_violation() {
    let CanonicalView::Object(view) = canonicalize(&json!({
        "type": "object",
        "minProperties": 1,
        "properties": {"filter": {"type": "string"}},
        "not": {"additionalProperties": false, "properties": {"filter": {"type": "string"}}}
    }))
    .unwrap()
    .view() else {
        panic!("expected an Object view");
    };
    let [ObjectViolationView::NameFails(schema)] = view.violations.as_slice() else {
        panic!(
            "expected a single NameFails violation, got {:?}",
            view.violations
        );
    };
    assert_eq!(
        schema.to_json_schema(),
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "const": "filter"})
    );
}

#[test]
fn object_view_exposes_undeclared_value_fails_violation() {
    let CanonicalView::Object(view) = canonicalize(&json!({
        "type": "object",
        "properties": {"a": {}},
        "not": {"properties": {"a": {}}, "additionalProperties": {"type": "string"}}
    }))
    .unwrap()
    .view() else {
        panic!("expected an Object view");
    };
    let [ObjectViolationView::UndeclaredValueFails {
        names,
        patterns,
        additional,
    }] = view.violations.as_slice()
    else {
        panic!(
            "expected a single UndeclaredValueFails violation, got {:?}",
            view.violations
        );
    };
    assert_eq!(names, &vec!["a".to_string()]);
    assert!(patterns.is_empty());
    assert_eq!(
        additional.to_json_schema(),
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "string"})
    );
}

// A format the canonicalizer cannot check may still be checked at validation, so a union must not
// absorb a value into a leaf carrying one.
#[test_case(&json!({"type": "object", "propertyNames": {"format": "only-ok"}}), &json!({"nope": 1}); "key constraint")]
#[test_case(&json!({"type": "object", "properties": {"a": {"type": "string", "format": "only-ok"}}}), &json!({"a": "nope"}); "property schema")]
#[test_case(&json!({"type": "object", "properties": {"a": {"format": "only-ok"}}}), &json!({"a": "nope"}); "untyped property schema")]
#[test_case(&json!({"type": "object", "properties": {"a": {"type": "object", "propertyNames": {"format": "only-ok"}}}}), &json!({"a": {"nope": 1}}); "nested object")]
#[test_case(&json!({"type": "array", "items": {"type": "string", "format": "only-ok"}}), &json!(["nope"]); "item schema")]
#[test_case(&json!({"type": "array", "contains": {"type": "string", "format": "only-ok"}}), &json!(["nope"]); "contains schema")]
#[test_case(&json!({"type": "object", "properties": {"a": {"type": "array", "contains": {"type": "string", "format": "only-ok"}}}}), &json!({"a": ["nope"]}); "contains under a property")]
#[test_case(&json!({"type": "object", "properties": {"a": {"type": "object", "additionalProperties": {"format": "only-ok"}}}}), &json!({"a": {"b": "nope"}}); "additional properties shield")]
#[test_case(&json!({"type": "object", "properties": {"a": {"type": "array", "contains": {"not": {"format": "only-ok"}}, "minContains": 0, "maxContains": 1}}}), &json!({"a": ["nope", "other"]}); "barred format under a contains ceiling")]
fn uncheckable_format_keeps_the_value_beside_the_leaf(leaf: &Value, instance: &Value) {
    let schema = json!({"anyOf": [{"const": instance}, leaf]});
    let canonical = options()
        .should_validate_formats(true)
        .canonicalize(&schema)
        .expect("canonicalizes");
    let build = |value: &Value| {
        ::jsonschema::options()
            .with_format("only-ok", |text: &str| text == "ok")
            .should_validate_formats(true)
            .build(value)
            .expect("builds")
    };
    assert!(build(&schema).is_valid(instance));
    assert!(build(&canonical.to_json_schema()).is_valid(instance));
}

// A Draft 4 integer property schema is a typed group, which the format scan walks past to reach the
// key whose format it cannot check.
#[test]
fn uncheckable_format_scan_walks_a_typed_group() {
    let instance = json!({"b": "nope"});
    let schema = json!({"anyOf": [
        {"enum": [instance]},
        {"type": "object", "properties": {
            "a": {"type": "integer", "enum": [1, 2]},
            "b": {"type": "string", "format": "only-ok"}
        }}
    ]});
    let canonical = options()
        .with_draft(Draft::Draft4)
        .should_validate_formats(true)
        .canonicalize(&schema)
        .expect("canonicalizes");
    let build = |value: &Value| {
        ::jsonschema::options()
            .with_draft(Draft::Draft4)
            .with_format("only-ok", |text: &str| text == "ok")
            .should_validate_formats(true)
            .build(value)
            .expect("builds")
    };
    assert!(build(&schema).is_valid(&instance));
    assert!(build(&canonical.to_json_schema()).is_valid(&instance));
}

// An unsupported document keeps document identity, where `1` and `1.0` are distinct - unlike JSON
// value equality, which reads them as the same number.
#[test]
fn unsupported_documents_hash_by_document_identity() {
    let canonical = |text: &str| {
        canonicalize(&serde_json::from_str::<Value>(text).expect("valid schema JSON"))
            .expect("canonicalizes")
    };
    let integer = canonical(
        r#"{"if": {}, "unevaluatedProperties": {"enum": [1, null, true, "x", [2], {"b": 3}]}}"#,
    );
    let float = canonical(
        r#"{"if": {}, "unevaluatedProperties": {"enum": [1.0, null, true, "x", [2], {"b": 3}]}}"#,
    );
    assert_eq!(integer.kind(), CanonicalKind::Raw);
    let distinct: HashSet<CanonicalSchema> =
        [integer.clone(), float, integer].into_iter().collect();
    assert_eq!(distinct.len(), 2);
}

#[test]
fn number_view_exposes_bounds() {
    let CanonicalView::Number(view) = canonicalize(&json!({
        "type": "number",
        "exclusiveMinimum": 1.5,
        "maximum": 9.5
    }))
    .unwrap()
    .view() else {
        panic!("expected a Number view");
    };
    assert_eq!(view.minimum, Number::from_f64(1.5));
    assert!(view.exclusive_minimum);
    assert_eq!(view.maximum, Number::from_f64(9.5));
    assert!(!view.exclusive_maximum);
}

// Arbitrary precision models a bound past `u64`/`i64` - both signs, and the `(i64::MAX, u64::MAX]`
// range specifically - as a big integer and emits it back exactly; the default build keeps such a
// document raw.
#[cfg(feature = "arbitrary-precision")]
#[test_case(r#"{"type": "string", "minLength": 99999999999999999999999}"#, CanonicalKind::String, "minLength", "99999999999999999999999"; "length bound")]
#[test_case(r#"{"type": "integer", "minimum": 99999999999999999999999}"#, CanonicalKind::Integer, "minimum", "99999999999999999999999"; "integer bound")]
#[test_case(r#"{"type": "array", "minItems": 99999999999999999999999}"#, CanonicalKind::Array, "minItems", "99999999999999999999999"; "array length bound")]
#[test_case(r#"{"type": "object", "minProperties": 99999999999999999999999}"#, CanonicalKind::Object, "minProperties", "99999999999999999999999"; "object size bound")]
#[test_case(r#"{"type": "integer", "maximum": 18446744073709551615}"#, CanonicalKind::Integer, "maximum", "18446744073709551615"; "integer maximum at u64 max")]
#[test_case(r#"{"type": "number", "maximum": 18446744073709551615}"#, CanonicalKind::Number, "maximum", "18446744073709551615"; "number maximum at u64 max")]
#[test_case(r#"{"type": "integer", "minimum": -99999999999999999999999}"#, CanonicalKind::Integer, "minimum", "-99999999999999999999999"; "integer minimum below negative i64")]
fn past_range_bound_round_trips(text: &str, kind: CanonicalKind, keyword: &str, bound: &str) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    let canonical = canonicalize(&schema).expect("canonicalizes");
    assert_eq!(canonical.kind(), kind);
    assert_eq!(canonical.to_json_schema()[keyword].to_string(), bound);
}

// Without a `type`, arbitrary precision keeps the bound on the number branch of the type split rather
// than dropping to raw.
#[cfg(feature = "arbitrary-precision")]
#[test_case(r#"{"maximum": 18446744073709551615}"#, "maximum", "18446744073709551615"; "untyped maximum at u64 max")]
#[test_case(r#"{"minimum": -18446744073709551615}"#, "minimum", "-18446744073709551615"; "untyped minimum below negative i64")]
#[test_case(r#"{"maximum": -99999999999999999999999}"#, "maximum", "-99999999999999999999999"; "untyped maximum below negative i64")]
fn untyped_past_range_numeric_bound_round_trips(text: &str, keyword: &str, bound: &str) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    let canonical = canonicalize(&schema).expect("canonicalizes");
    assert_eq!(canonical.kind(), CanonicalKind::AnyOf);
    let emitted = canonical.to_json_schema();
    let branches = emitted["anyOf"].as_array().expect("anyOf branches");
    let number_branch = branches
        .iter()
        .find(|branch| branch["type"] == json!("number"))
        .expect("number branch");
    assert_eq!(number_branch[keyword].to_string(), bound);
}

// A bound past the `f64` range must not swallow its exclusive partner on the same side: the
// canonical form would accept values the document rejects.
#[cfg(feature = "arbitrary-precision")]
#[test_case(r#"{"type": "number", "maximum": 1e400, "exclusiveMaximum": 0.1}"#, "exclusiveMaximum", "0.1"; "maximum past f64 range")]
#[test_case(r#"{"type": "number", "minimum": -1e400, "exclusiveMinimum": 0.1}"#, "exclusiveMinimum", "0.1"; "minimum past f64 range")]
fn past_range_bound_keeps_its_tighter_partner(text: &str, keyword: &str, bound: &str) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    let emitted = canonicalize(&schema)
        .expect("canonicalizes")
        .to_json_schema();
    assert_eq!(emitted[keyword].to_string(), bound, "{emitted}");
}

#[cfg(not(feature = "arbitrary-precision"))]
#[test_case("string", "minLength")]
#[test_case("string", "maxLength")]
#[test_case("array", "minItems")]
#[test_case("array", "maxItems")]
#[test_case("object", "minProperties")]
#[test_case("object", "maxProperties")]
fn huge_count_bound_stays_raw(ty: &str, keyword: &str) {
    let schema: Value = serde_json::from_str(&format!(
        r#"{{"type": "{ty}", "{keyword}": 99999999999999999999999}}"#
    ))
    .unwrap();
    assert_eq!(canonicalize(&schema).unwrap().kind(), CanonicalKind::Raw);
}

// Default build: the integers past `i64` that such a bound admits have no modeled form. They still
// satisfy the schema, so the document stays raw rather than collapsing to "nothing matches". A
// `number` interval carries the same bound, and an `allOf` may put it against `integer` later.
#[cfg(not(feature = "arbitrary-precision"))]
#[test_case(r#"{"type": "integer", "minimum": 99999999999999999999999}"#; "integer minimum")]
#[test_case(r#"{"type": "integer", "maximum": 99999999999999999999999}"#; "integer maximum")]
#[test_case(r#"{"type": "number", "minimum": 99999999999999999999999}"#; "number minimum")]
#[test_case(r#"{"type": "number", "maximum": 99999999999999999999999}"#; "number maximum")]
#[test_case(r#"{"allOf": [{"type": "integer"}, {"minimum": 99999999999999999999999}]}"#; "interval meeting integer")]
// No type keyword: the bound alone still projects onto the integers through a later `allOf`.
#[test_case(r#"{"maximum": 18446744073709551615}"#; "untyped maximum at u64 max")]
#[test_case(r#"{"minimum": -18446744073709551615}"#; "untyped minimum below negative i64")]
#[test_case(r#"{"maximum": -99999999999999999999999}"#; "untyped maximum below negative i64")]
// The `(i64::MAX, u64::MAX]` positive range and the mirror negative range both leave `i64`.
#[test_case(r#"{"type": "integer", "maximum": 18446744073709551615}"#; "integer maximum at u64 max")]
#[test_case(r#"{"type": "number", "maximum": 18446744073709551615}"#; "number maximum at u64 max")]
#[test_case(r#"{"type": "integer", "minimum": -99999999999999999999999}"#; "integer minimum below negative i64")]
fn huge_numeric_bound_stays_raw(text: &str) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    assert_eq!(canonicalize(&schema).unwrap().kind(), CanonicalKind::Raw);
}

// The representable integer range is exactly `i64`. Past `i64::MAX` the document parses as `u64` and
// stays raw. Past `i64::MIN` it parses as `f64`, so one step past rounds back to exactly `i64::MIN`
// and is still modeled; raw starts at the first float below it.
#[cfg(not(feature = "arbitrary-precision"))]
#[test_case(r#"{"maximum": 9223372036854775807}"#, false; "maximum at i64 max is modeled")]
#[test_case(r#"{"maximum": 9223372036854775808}"#, true; "maximum one past i64 max stays raw")]
#[test_case(r#"{"minimum": -9223372036854775808}"#, false; "minimum at i64 min is modeled")]
#[test_case(r#"{"minimum": -9223372036854775809}"#, false; "minimum one past i64 min rounds to i64 min")]
#[test_case(r#"{"minimum": -9223372036854777856}"#, true; "minimum at first float below i64 min stays raw")]
fn representable_range_boundary(text: &str, stays_raw: bool) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    let is_raw = canonicalize(&schema).unwrap().kind() == CanonicalKind::Raw;
    assert_eq!(is_raw, stays_raw);
}

// The `regex` engine rejects a negative lookahead the fancy engine accepts.
#[test]
fn pattern_engine_selects_dialect() {
    let schema = json!({"pattern": "^(?!x)"});
    assert!(canonicalize(&schema).is_ok());
    let error = options()
        .with_pattern_options(PatternOptions::regex())
        .canonicalize(&schema)
        .unwrap_err();
    assert!(matches!(
        error,
        CanonicalizationError::InvalidPattern { .. }
    ));
}

// The suite checks only the error variant; the `Display` message is exercised here.
#[test_case(&json!(42), "schema must be a boolean or object, got: 42"; "invalid schema type")]
#[test_case(&json!({"pattern": "["}), "invalid regular expression: \"[\""; "invalid pattern")]
fn error_display(schema: &Value, message: &str) {
    assert_eq!(canonicalize(schema).unwrap_err().to_string(), message);
}

#[test]
fn unsupported_result_display() {
    let plain = canonicalize(&json!({"type": "object"})).expect("canonicalizes");
    let patterned =
        canonicalize(&json!({"type": "object", "patternProperties": {"^a": {"type": "string"}}}))
            .expect("canonicalizes");
    assert_eq!(
        plain.subtract(&patterned).unwrap_err().to_string(),
        "result is not supported in canonical form"
    );
}

#[test_case(&json!({"type": "string"}), CanonicalKind::MultiType, "multi_type"; "multi_type")]
#[test_case(&json!({"const": 1}), CanonicalKind::Const, "const"; "const_value")]
#[test_case(&json!({"enum": [1, 2, 3]}), CanonicalKind::Enum, "enum"; "enum_values")]
#[test_case(&json!({}), CanonicalKind::True, "true"; "empty object")]
#[test_case(&json!(false), CanonicalKind::False, "false"; "boolean false")]
#[test_case(&json!({"type": "integer", "minimum": 0}), CanonicalKind::Integer, "integer"; "integer_leaf")]
#[test_case(&json!({"type": "number", "minimum": 0}), CanonicalKind::Number, "number"; "number_leaf")]
#[test_case(&json!({"if": {}, "unevaluatedProperties": false}), CanonicalKind::Raw, "raw"; "raw")]
fn kind_reports_its_label(schema: &Value, kind: CanonicalKind, label: &str) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    assert_eq!(canonical.kind(), kind);
    assert_eq!(canonical.kind().as_str(), label);
}

// `view()` exposes each modeled node with its payload.
#[test_case(&json!({"type": ["string", "number"]}), &CanonicalView::MultiType(JsonType::String | JsonType::Number); "multi_type")]
#[test_case(&json!({"const": [1, "a"]}), &CanonicalView::Const(json!([1, "a"])); "const_value")]
#[test_case(&json!({"enum": [3, 1, 2]}), &CanonicalView::Enum(vec![json!(1), json!(2), json!(3)]); "enum_values")]
#[test_case(&json!(true), &CanonicalView::True; "boolean true")]
#[test_case(&json!({}), &CanonicalView::True; "empty object")]
#[test_case(&json!(false), &CanonicalView::False; "boolean false")]
fn view_matches_expected(schema: &Value, expected: &CanonicalView) {
    assert_eq!(&canonicalize(schema).unwrap().view(), expected);
}

// Default build: an integer past `i64` lies above the whole representable range, so it satisfies any
// minimum and violates any maximum - decided by overflow direction, without representing it.
#[cfg(not(feature = "arbitrary-precision"))]
#[test_case(r#"{"type":"integer","enum":[1,2,10000000000000000000],"minimum":2}"#, &json!({"enum":[2,10_000_000_000_000_000_000_u64]}); "minimum keeps oversized")]
#[test_case(r#"{"type":"integer","enum":[1,2,10000000000000000000],"maximum":5}"#, &json!({"enum":[1,2]}); "maximum drops oversized")]
#[test_case(r#"{"allOf":[{"type":"integer","minimum":2},{"enum":[1,2,10000000000000000000]}]}"#, &json!({"enum":[2,10_000_000_000_000_000_000_u64]}); "cross-branch minimum keeps oversized")]
#[test_case(r#"{"type":"integer","enum":[1,2,-10000000000000000000],"maximum":5}"#, &json!({"enum":[1,2,-1e19]}); "maximum keeps undersized")]
#[test_case(r#"{"type":"integer","enum":[1,2,-10000000000000000000],"minimum":0}"#, &json!({"enum":[1,2]}); "minimum drops undersized")]
fn oversized_integer_compares_by_overflow_direction(text: &str, expected: &Value) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    let canonical = canonicalize(&schema).expect("canonicalizes");
    let mut expected = expected.as_object().expect("object").clone();
    expected.insert(
        "$schema".into(),
        json!("https://json-schema.org/draft/2020-12/schema"),
    );
    assert_eq!(canonical.to_json_schema(), Value::Object(expected));
}

// Default build: a value past `i64` cannot lift into a window, so a covering interval absorbs it by
// overflow direction alone.
#[cfg(not(feature = "arbitrary-precision"))]
#[test_case(r#"{"anyOf":[{"type":"integer","minimum":2},{"const":1e30}]}"#, CanonicalKind::Integer; "absorbed above every maximum")]
#[test_case(r#"{"anyOf":[{"type":"integer","maximum":5},{"const":1e30}]}"#, CanonicalKind::AnyOf; "kept beyond the maximum")]
fn oversized_member_absorption(text: &str, kind: CanonicalKind) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    assert_eq!(canonicalize(&schema).expect("canonicalizes").kind(), kind);
}

// Draft 4 `integer` is a typed group an interval bound narrows; a bound excluding every member leaves
// nothing satisfiable, a mixed type set guards only its integer members, and the bound may sit on
// either side of the intersection.
#[test_case(&json!({"type": "integer", "enum": [1, 2, 3], "minimum": 2}), &json!({"type": "integer", "enum": [2, 3]}); "narrows to survivors")]
#[test_case(&json!({"type": "integer", "enum": [1, 2, 3], "minimum": 5}), &json!({"not": {}}); "bound excludes all")]
#[test_case(&json!({"allOf": [{"type": ["string", "integer"]}, {"enum": ["a", 1]}]}), &json!({"anyOf": [{"type": "integer", "enum": [1]}, {"enum": ["a"]}]}); "mixed type set guards only integers")]
#[test_case(&json!({"allOf": [{"type": "integer", "minimum": 2}, {"type": "integer", "enum": [1, 2, 3]}]}), &json!({"type": "integer", "enum": [2, 3]}); "bound before typed group")]
fn draft4_integer_typed_group_intersects_bound(schema: &Value, expected: &Value) {
    let canonical = options()
        .with_draft(Draft::Draft4)
        .canonicalize(schema)
        .expect("canonicalizes");
    let mut expected = expected.as_object().expect("object").clone();
    expected.insert(
        "$schema".into(),
        json!("http://json-schema.org/draft-04/schema#"),
    );
    assert_eq!(canonical.to_json_schema(), Value::Object(expected));
}

// Draft 4 keeps a type guard on `integer` values because value equality cannot tell `1` from `1.0`,
// whether the values come from the same object or meet a bound from another `allOf` branch.
#[test_case(&json!({"type": "integer", "enum": [1, 2, 3]}); "same object")]
#[test_case(&json!({"allOf": [{"enum": [1, 2, 3]}, {"type": "integer", "minimum": 2}]}); "value set meets a bound")]
fn draft4_integer_values_are_a_typed_group(schema: &Value) {
    let canonical = options()
        .with_draft(Draft::Draft4)
        .canonicalize(schema)
        .expect("canonicalizes");
    assert_eq!(canonical.kind(), CanonicalKind::TypedGroup);
    assert_eq!(canonical.kind().as_str(), "typed_group");
    let CanonicalView::TypedGroup(group) = canonical.view() else {
        panic!("expected a TypedGroup view");
    };
    assert_eq!(group.ty, JsonType::Integer);
    assert_eq!(group.body.kind(), CanonicalKind::Enum);
}

// Draft 4 gives `1` and `1.0` different types, so an element demand taking only the first leaves
// the part of the member it takes rather than the whole member or nothing.
#[test]
fn draft4_an_element_demand_keeps_the_member_spelling_it_takes() {
    let draft4 = || options().with_draft(Draft::Draft4);
    let member = draft4()
        .canonicalize(&json!({"enum": [[1]]}))
        .expect("canonicalizes");
    let elements = draft4()
        .canonicalize(&json!({
            "type": "array",
            "items": {"type": "number", "not": {"type": "integer"}}
        }))
        .expect("canonicalizes");
    let difference = member
        .subtract(&elements)
        .expect("subtracts")
        .to_json_schema();
    let validator = jsonschema::options()
        .with_draft(Draft::Draft4)
        .build(&difference)
        .expect("compiles");
    assert!(validator.is_valid(&json!([1])));
    assert!(!validator.is_valid(&json!([1.0])));
    // Only `Yes` claims containment, and the member holds a value the element demand refuses.
    assert_ne!(
        elements.covers(&member).expect("compares"),
        Containment::Yes
    );
}

// An `anyOf` whose branches stay disjoint surfaces as an AnyOf view exposing each branch.
#[test]
fn view_exposes_anyof_branches() {
    let canonical =
        canonicalize(&json!({"anyOf": [{"type": "string"}, {"const": 1}]})).expect("canonicalizes");
    assert_eq!(canonical.kind(), CanonicalKind::AnyOf);
    assert_eq!(canonical.kind().as_str(), "any_of");
    let CanonicalView::AnyOf(branches) = canonical.view() else {
        panic!("expected an AnyOf view");
    };
    assert_eq!(
        branches
            .iter()
            .map(CanonicalSchema::view)
            .collect::<Vec<_>>(),
        vec![
            CanonicalView::MultiType(JsonType::String.into()),
            CanonicalView::Const(json!(1)),
        ]
    );
}

#[test]
fn validation_error_display_and_source() {
    let error = canonicalize(&json!({"type": 123})).expect_err("invalid schema must error");
    assert!(error.to_string().starts_with("schema validation failed:"));
    assert!(std::error::Error::source(&error).is_some());
}

// `unevaluatedProperties` beside an instance-dependent applicator is unsupported, so the document
// goes raw at the root without descending into the nesting.
#[test]
fn deeply_nested_document_round_trips() {
    let mut schema = json!({"type": "string"});
    for _ in 0..300 {
        let mut map = Map::new();
        map.insert("if".to_string(), json!({}));
        map.insert("unevaluatedProperties".to_string(), schema);
        schema = Value::Object(map);
    }
    let canonical = canonicalize(&schema).expect("canonicalizes");
    assert_eq!(canonical.to_json_schema(), schema);
}

// The complement of a type set missing only `null` (or only `boolean`) is the same canonical node
// the direct spelling produces, not a sibling `MultiType` shape.
#[test]
fn negated_type_set_complement_converges_with_direct_spelling() {
    let negated =
        canonicalize(&json!({"not": {"type": ["boolean", "number", "string", "array", "object"]}}))
            .unwrap();
    assert_eq!(negated, canonicalize(&json!({"type": "null"})).unwrap());
    let negated =
        canonicalize(&json!({"not": {"type": ["null", "number", "string", "array", "object"]}}))
            .unwrap();
    assert_eq!(negated, canonicalize(&json!({"type": "boolean"})).unwrap());
}

// Numerals `ext::numeric::try_parse_bigint` refuses (huge exponents / digit counts) have no
// exact runtime comparison; documents carrying them in `const`/`enum` stay raw.
#[cfg(feature = "arbitrary-precision")]
#[test_case(r#"{"const":1e999999999999999999999}"#; "huge_exponent_const")]
#[test_case(r#"{"enum":[1e999999999999999999999]}"#; "huge_exponent_enum")]
#[test_case(r#"{"type":"number","minimum":1e999999999999999999999}"#; "huge_exponent_bound")]
#[test_case(r#"{"type":"number","multipleOf":1e999999999999999999999}"#; "huge_exponent_divisor")]
#[test_case(&format!(r#"{{"const":1{}}}"#, "0".repeat((1 << 20) + 1)); "huge_digit_count")]
#[test_case(&format!(r#"{{"const":0.{}1}}"#, "0".repeat(1 << 20)); "huge_fraction_digit_count")]
#[test_case(&format!(r#"{{"enum":[0.{}1]}}"#, "0".repeat(1 << 20)); "huge_fraction_digit_count_enum")]
fn numerals_without_exact_comparison_stay_raw(text: &str) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    let canonical = canonicalize(&schema).expect("canonicalizes");
    assert!(matches!(canonical.view(), CanonicalView::Raw(_)));
    assert_eq!(canonical.to_json_schema(), schema);
}

// A `contains` count bound with no modeled form keeps the document raw: past `u64` in the default
// build, a spelling without an exact integer reading under arbitrary precision.
#[cfg(not(feature = "arbitrary-precision"))]
#[test_case(&json!({"type": "array", "contains": {"type": "null"}, "minContains": 1e100}); "minimum past u64")]
#[test_case(&json!({"type": "array", "contains": {"type": "null"}, "maxContains": 1e100}); "maximum past u64")]
fn contains_counts_without_modeled_form_stay_raw(schema: &Value) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    assert!(matches!(canonical.view(), CanonicalView::Raw(_)));
    assert_eq!(&canonical.to_json_schema(), schema);
}

#[cfg(feature = "arbitrary-precision")]
#[test_case("string", "minLength"; "minimum string length past the expansion cap")]
#[test_case("string", "maxLength"; "maximum string length past the expansion cap")]
#[test_case("array", "minItems"; "minimum array length past the expansion cap")]
#[test_case("array", "maxItems"; "maximum array length past the expansion cap")]
#[test_case("object", "minProperties"; "minimum object size past the expansion cap")]
#[test_case("object", "maxProperties"; "maximum object size past the expansion cap")]
#[test_case("array", "minContains"; "minimum contains count past the expansion cap")]
#[test_case("array", "maxContains"; "maximum contains count past the expansion cap")]
fn count_bound_without_modeled_form_stays_raw(ty: &str, keyword: &str) {
    // More digits than the canonical expansion cap, yet within the validator's exponent limit:
    // the count is meta-valid, but its canonical spelling stays scientific.
    let count = format!("1{}e1000000", "0".repeat(48_577));
    let contains = keyword
        .ends_with("Contains")
        .then_some(r#","contains":{"type":"null"}"#);
    let text = format!(
        r#"{{"type":"{ty}"{},"{keyword}":{count}}}"#,
        contains.unwrap_or_default()
    );
    let schema: Value = serde_json::from_str(&text).expect("valid schema JSON");
    let canonical = canonicalize(&schema).expect("canonicalizes");
    assert!(matches!(canonical.view(), CanonicalView::Raw(_)));
    assert_eq!(canonical.to_json_schema(), schema);
}

// `const` compares by JSON value, so `1` and `1.0` share one canonical form; distinct values stay distinct.
#[test]
fn const_identity_is_value_identity() {
    let integer = canonicalize(&json!({"const": 1})).unwrap();
    let float = canonicalize(&json!({"const": 1.0})).unwrap();
    assert_eq!(integer, float);
    assert_ne!(integer, canonicalize(&json!({"const": "1"})).unwrap());
    assert_eq!(
        integer.to_json_schema(),
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "const": 1})
    );
}

// An integer-valued float folds to its integer form on both sides of zero.
#[test_case(&json!(5.0), &json!(5); "positive")]
#[test_case(&json!(-5.0), &json!(-5); "negative")]
fn integer_valued_float_const_folds_to_integer(float: &Value, integer: &Value) {
    let from_float = canonicalize(&json!({ "const": float })).unwrap();
    let from_integer = canonicalize(&json!({ "const": integer })).unwrap();
    assert_eq!(from_float, from_integer);
    assert_eq!(
        from_float.to_json_schema(),
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "const": integer})
    );
}

// A finite value set that fills a JSON type's whole domain collapses to a `type`; a partial set stays an `enum`.
#[test_case(&json!({"enum": [null, false, true]}), &json!({"type": ["null", "boolean"]}); "saturates null and boolean")]
#[test_case(&json!({"enum": [false, true]}), &json!({"type": "boolean"}); "saturates boolean")]
#[test_case(&json!({"enum": [null, false]}), &json!({"enum": [null, false]}); "partial set stays enum")]
fn finite_value_set_saturation(schema: &Value, expected: &Value) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    let mut expected = expected.as_object().expect("object").clone();
    expected.insert(
        "$schema".into(),
        json!("https://json-schema.org/draft/2020-12/schema"),
    );
    assert_eq!(canonical.to_json_schema(), Value::Object(expected));
}

// `const` and `enum` together admit only the values in both.
#[test]
fn const_intersects_enum() {
    let canonical = canonicalize(&json!({"enum": [1, 2, 3], "const": 2})).expect("canonicalizes");
    assert_eq!(
        canonical.to_json_schema(),
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "const": 2})
    );
}

// `CanonicalSchema` orders structurally: a schema equals itself and differs from a distinct one.
#[test]
fn canonical_schema_ordering() {
    let one = canonicalize(&json!({"const": 1})).unwrap();
    let two = canonicalize(&json!({"const": 2})).unwrap();
    assert_eq!(one.cmp(&one), Ordering::Equal);
    assert!(one < two);
    assert!(two > one);

    let raw = |text: &str| canonicalize(&serde_json::from_str(text).unwrap()).unwrap();
    let raw_one = raw(r#"{"if":{},"unevaluatedProperties":{"const":1}}"#);
    let raw_two = raw(r#"{"if":{},"unevaluatedProperties":{"const":2}}"#);
    assert_eq!(raw_one.partial_cmp(&raw_two), Some(Ordering::Less));
    assert!(raw_one < raw_two);

    #[cfg(feature = "arbitrary-precision")]
    assert!(
        raw(r#"{"if":{},"unevaluatedProperties":{"const":1e400}}"#)
            < raw(r#"{"if":{},"unevaluatedProperties":{"const":2e400}}"#)
    );
}

// Each draft stamps its own `$schema` URI onto the emitted document.
#[test_case(Draft::Draft6, "http://json-schema.org/draft-06/schema#"; "draft6")]
#[test_case(Draft::Draft201909, "https://json-schema.org/draft/2019-09/schema"; "draft2019-09")]
fn draft_stamps_its_schema_uri(draft: Draft, uri: &str) {
    let canonical = options()
        .with_draft(draft)
        .canonicalize(&json!({"type": "string"}))
        .expect("canonicalizes");
    assert_eq!(
        canonical.to_json_schema(),
        json!({"$schema": uri, "type": "string"})
    );
}

// Past `f64` precision a whole divisor keeps exact modulo only under arbitrary precision, so the
// forms below are default-build behaviour.
#[cfg(not(feature = "arbitrary-precision"))]
#[test_case(
    r#"{"type": "integer", "multipleOf": 9007199254740993}"#,
    &json!({"type": "integer", "multipleOf": 9_007_199_254_740_993_u64});
    "a divisor no decimal spells is kept as written"
)]
#[test_case(
    r#"{"type": "integer", "multipleOf": 4611686018427387903}"#,
    &json!({"type": "integer", "multipleOf": 4_611_686_018_427_387_903_u64});
    "a divisor past f64 precision is kept as written"
)]
#[test_case(
    r#"{"type": "integer", "multipleOf": 1e30}"#,
    &json!({"type": "integer", "multipleOf": 1e30});
    "a divisor past the integer range is kept as written"
)]
#[test_case(
    r#"{"allOf":[{"type":"integer","multipleOf":9007199254740992},{"type":"integer","multipleOf":9007199254740991}]}"#,
    &json!({"type": "integer", "allOf": [{"multipleOf": 9_007_199_254_740_991_u64}, {"multipleOf": 9_007_199_254_740_992_u64}]});
    "divisors with no exact common multiple stay apart"
)]
#[test_case(
    r#"{"allOf":[{"type":"integer","multipleOf":9007199254740992,"minimum":1},{"type":"integer","multipleOf":9007199254740991}]}"#,
    &json!({"type": "integer", "minimum": 9_007_199_254_740_992_u64, "allOf": [{"multipleOf": 9_007_199_254_740_991_u64}, {"multipleOf": 9_007_199_254_740_992_u64}]});
    "divisors with no exact common multiple keep a snapped bound"
)]
#[test_case(
    r#"{"allOf":[{"type":"number","multipleOf":3},{"type":"number","multipleOf":3002399751580331}]}"#,
    &json!({"type": "integer", "allOf": [{"multipleOf": 3}, {"multipleOf": 3_002_399_751_580_331_u64}]});
    "divisors whose common multiple no decimal spells stay apart"
)]
fn divisors_past_exact_precision(text: &str, expected: &Value) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    let mut form = canonicalize(&schema)
        .expect("canonicalizes")
        .to_json_schema();
    form.as_object_mut().expect("object").remove("$schema");
    assert_eq!(&form, expected);
}

// A member the divisor admits survives even where the integer type cannot hold it.
#[cfg(not(feature = "arbitrary-precision"))]
#[test]
fn divisor_keeps_member_past_representable_range() {
    let schema = json!({"allOf": [{"type": "integer", "multipleOf": 2}, {"const": 1e30}]});
    let mut form = canonicalize(&schema)
        .expect("canonicalizes")
        .to_json_schema();
    form.as_object_mut().expect("object").remove("$schema");
    assert_eq!(form, json!({"const": 1e30}));
}

// Membership for a divisor is decided by the validator's own arithmetic, so every rewrite the
// algebra makes rests on this agreeing with a compiled `multipleOf`.
#[test_case("2")]
#[test_case("3")]
#[test_case("1")]
#[test_case("0.5")]
#[test_case("0.75")]
#[test_case("1.5")]
#[test_case("0.123456789")]
#[test_case("9007199254740992")]
#[test_case("9007199254740993")]
#[test_case("4503599627370496")]
#[test_case("1e300")]
#[test_case("1e-7")]
fn divisor_oracle_matches_the_validator(divisor: &str) {
    const INSTANCES: &[&str] = &[
        "0",
        "1",
        "2",
        "3",
        "6",
        "-4",
        "1.5",
        "2.5",
        "0.25",
        "9007199254740993",
        "12345678900000001",
        "27021597764222977",
        "1e30",
        "-9007199254740993",
    ];
    let divisor: serde_json::Number = divisor.parse().expect("divisor");
    let validator = jsonschema::validator_for(&json!({"multipleOf": divisor})).expect("compiles");
    for instance in INSTANCES {
        let instance: serde_json::Number = instance.parse().expect("instance");
        assert_eq!(
            jsonschema_value::numeric_check::satisfies_multiple_of(&divisor, &instance),
            validator.is_valid(&Value::Number(instance.clone())),
            "multipleOf {divisor} on {instance}"
        );
    }
}

// A divisor no `f64` spells still constrains, so the leaf carries it instead of the document staying
// raw; only the arithmetic that would need its exact value is skipped.
#[cfg(not(feature = "arbitrary-precision"))]
#[test]
fn divisor_no_decimal_spells_is_modeled() {
    let schema = json!({"type": "number", "multipleOf": 9_007_199_254_740_993_u64});
    let canonical = canonicalize(&schema).expect("canonicalizes");
    assert_ne!(canonical.kind(), jsonschema::canonical::CanonicalKind::Raw);
    let mut form = canonical.to_json_schema();
    form.as_object_mut().expect("object").remove("$schema");
    // The validator reads the divisor as 2^53, whose multiples are all whole.
    assert_eq!(
        form,
        json!({"type": "integer", "multipleOf": 9_007_199_254_740_993_u64})
    );
}

// Bounds past `f64` precision: snapping must not move an end onto a value the validator reads
// differently, and a progression whose next multiple is unrepresentable is not empty.
#[cfg(not(feature = "arbitrary-precision"))]
#[test_case(
    r#"{"type":"number","minimum":9223372036854775807,"multipleOf":1}"#,
    &["9223372036854775807", "9223372036854775808"];
    "a bound with no representable multiple"
)]
#[test_case(
    r#"{"type":"number","minimum":-4,"maximum":9223372036854775807,"multipleOf":0.5}"#,
    &["9223372036854775808", "-4", "0.5"];
    "an upper end past exact precision"
)]
#[test_case(
    r#"{"type":"number","exclusiveMinimum":9007199254740992,"multipleOf":0.5}"#,
    &["9007199254740992", "9007199254740993"];
    "an excluded end past exact precision"
)]
#[test_case(
    r#"{"type":"integer","minimum":9223372036854775807,"multipleOf":2}"#,
    &["9223372036854775808", "1e30", "9223372036854775807"];
    "an integer bound with no representable multiple"
)]
fn wide_bounds_keep_validation(text: &str, instances: &[&str]) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    let emitted = canonicalize(&schema)
        .expect("canonicalizes")
        .to_json_schema();
    for instance in instances {
        let instance: Value = serde_json::from_str(instance).expect("instance");
        assert_eq!(
            jsonschema::is_valid(&schema, &instance),
            jsonschema::is_valid(&emitted, &instance),
            "{instance} against {emitted}"
        );
    }
}

// A divisor of one adds nothing beside a whole one, whose multiples are already whole. The wide
// divisor keeps its spelling only in the default build.
#[cfg(not(feature = "arbitrary-precision"))]
#[test]
fn identity_divisor_drops_beside_a_whole_one() {
    let schema = json!({"allOf": [
        {"type": "number", "multipleOf": 2},
        {"type": "number", "minimum": 0, "multipleOf": 1e30}
    ]});
    let mut form = canonicalize(&schema)
        .expect("canonicalizes")
        .to_json_schema();
    form.as_object_mut().expect("object").remove("$schema");
    assert_eq!(
        form,
        json!({"type": "integer", "minimum": 0, "multipleOf": 1e30})
    );
}

// Arbitrary precision decides every divisor exactly, so divisors the default build reads with
// different arithmetic still fold there.
#[cfg(feature = "arbitrary-precision")]
#[test_case(
    r#"{"allOf":[{"type":"number","multipleOf":3},{"type":"number","multipleOf":1.5}]}"#,
    &json!({"type": "integer", "multipleOf": 3});
    "a whole divisor stands for a fractional one it covers"
)]
#[test_case(
    r#"{"allOf":[{"type":"number","multipleOf":2},{"type":"number","multipleOf":2.5}]}"#,
    &json!({"type": "integer", "multipleOf": 10});
    "unlike divisors fold to their common multiple"
)]
fn unlike_divisors_fold_under_arbitrary_precision(text: &str, expected: &Value) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    let mut form = canonicalize(&schema)
        .expect("canonicalizes")
        .to_json_schema();
    form.as_object_mut().expect("object").remove("$schema");
    assert_eq!(&form, expected);
}

// Embedded rather than read off disk: `wasm32` has no filesystem, and canonicalization is exactly
// as much in use there as anywhere.
macro_rules! bundled_metaschemas {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_str!(concat!("../../jsonschema-referencing/metaschemas/", $path)))),*]
    };
}

const BUNDLED_METASCHEMAS: &[(&str, &str)] = bundled_metaschemas![
    "draft4.json",
    "draft6.json",
    "draft7.json",
    "draft2019-09/schema.json",
    "draft2019-09/meta/applicator.json",
    "draft2019-09/meta/content.json",
    "draft2019-09/meta/core.json",
    "draft2019-09/meta/format.json",
    "draft2019-09/meta/meta-data.json",
    "draft2019-09/meta/validation.json",
    "draft2020-12/schema.json",
    "draft2020-12/meta/applicator.json",
    "draft2020-12/meta/content.json",
    "draft2020-12/meta/core.json",
    "draft2020-12/meta/format-annotation.json",
    "draft2020-12/meta/format-assertion.json",
    "draft2020-12/meta/meta-data.json",
    "draft2020-12/meta/unevaluated.json",
    "draft2020-12/meta/validation.json",
];

// Every bundled metaschema must reach the structural IR.
#[test]
fn bundled_metaschemas_are_modeled() {
    let mut raw = Vec::new();
    for (name, text) in BUNDLED_METASCHEMAS {
        let schema: Value = serde_json::from_str(text).expect("a bundled metaschema is valid JSON");
        match options().canonicalize(&schema) {
            Ok(canonical) if canonical.kind() == CanonicalKind::Raw => raw.push(*name),
            Ok(_) => {}
            Err(error) => panic!("{name}: {error}"),
        }
    }
    assert!(raw.is_empty(), "these metaschemas stayed raw: {raw:#?}");
}

// A registry resource never passes metaschema validation - only the referring document does. A
// reference into a malformed one must degrade to `Raw` rather than error or panic.
#[test_case(&json!({"type": 5}); "type is not a name")]
#[test_case(&json!({"if": 5}); "subschema is not a schema")]
#[test_case(&json!({"dependencies": {"a": 5}}); "dependency is neither schema nor name list")]
#[test_case(&json!({"dependentRequired": {"a": ["x", 1]}}); "dependent requirement is not a name")]
#[test_case(&json!({"dependentSchemas": {"a": 5}}); "dependent schema is not a schema")]
#[test_case(&json!({"items": [true]}); "2020-12 items is not a tuple")]
#[test_case(
    &json!({"contains": 5, "unevaluatedItems": false});
    "contains beside unevaluated items is not a schema"
)]
#[test_case(
    &json!({"allOf": [5], "unevaluatedProperties": false});
    "property cover branch is not a schema"
)]
#[test_case(
    &json!({"allOf": [5], "unevaluatedItems": false});
    "item cover branch is not a schema"
)]
#[test_case(
    &json!({"allOf": [{"properties": {"a": true}}], "properties": 5, "unevaluatedProperties": false});
    "hoisted properties is not an object"
)]
#[test_case(
    &json!({"allOf": [{"prefixItems": [true]}], "prefixItems": 5, "unevaluatedItems": false});
    "padded tuple is not an array"
)]
#[test_case(
    &json!({"$schema": "http://json-schema.org/draft-07/schema#", "type": "integer"});
    "target declares another draft"
)]
fn unvalidated_registry_target_keeps_the_document_raw(target: &Value) {
    let registry = Registry::new()
        .add("https://example.com/target", target)
        .expect("resource URI is valid")
        .prepare()
        .expect("registry prepares");
    let document = json!({"$ref": "https://example.com/target"});
    let canonical = options()
        .with_draft(Draft::Draft202012)
        .with_registry(&registry)
        .canonicalize(&document)
        .expect("canonicalizes");

    assert_eq!(canonical.kind(), CanonicalKind::Raw);
    assert_eq!(canonical.to_json_schema(), document);
}

// The suite pins the error variant; these pin what a caller reads off it.
#[test]
fn a_resolution_error_carries_its_cause() {
    let error = canonicalize(&json!({"$ref": "#/$defs/%FF", "$defs": {"a": true}}))
        .expect_err("the pointer does not decode");

    assert!(error.to_string().contains("valid UTF-8"), "{error}");
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn an_invalid_schema_type_error_has_no_cause() {
    let error = canonicalize(&json!([])).expect_err("an array is not a schema");

    assert!(std::error::Error::source(&error).is_none());
}

// The version-less meta-schema URI names whichever draft is current, so a document spelling it
// models like one naming that draft outright.
#[test_case("http://json-schema.org/schema"; "http")]
#[test_case("http://json-schema.org/schema#"; "http with fragment")]
#[test_case("https://json-schema.org/schema"; "https")]
fn the_version_less_meta_schema_uri_models(uri: &str) {
    let canonical = canonicalize(&json!({
        "$schema": uri,
        "type": "object",
        "properties": {"a": {"type": "string", "minLength": 2}}
    }))
    .expect("canonicalizes");
    assert_eq!(canonical.draft(), Draft::Draft202012);
    assert_eq!(
        canonical.to_json_schema(),
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {"a": {"type": "string", "minLength": 2}}
        })
    );
}

#[test]
fn definition_looks_up_one_target() {
    let schema = canonicalize(&json!({
        "$defs": {"A": {"type": "string"}},
        "$ref": "#/$defs/A"
    }))
    .expect("canonicalizes");
    let (uri, expected) = schema.definitions().next().expect("one definition");
    assert_eq!(schema.definition(&uri), Some(expected));
    assert_eq!(schema.definition("#/$defs/absent"), None);
}

// A target naming `#` keeps the document it was written in, so the pointer resolves to that
// document rather than to the target standing in for it.
#[test_case(
    &json!({
        "$ref": "#/$defs/A",
        "$defs": {"A": {"type": "object", "properties": {"child": {"$ref": "#"}}}}
    }),
    "#/$defs/A";
    "direct"
)]
#[test_case(
    &json!({
        "$ref": "#/$defs/A",
        "$defs": {
            "A": {"type": "object", "properties": {"a": {"$ref": "#/$defs/B"}}},
            "B": {"type": "object", "properties": {"child": {"$ref": "#"}}}
        }
    }),
    "#/$defs/A";
    "through another target"
)]
fn definition_binds_a_target_naming_the_document_root(schema: &Value, uri: &str) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    let target = canonical.definition(uri).expect("target");
    assert!(canonical.definitions().any(|(name, _)| name == uri));
    let document = target.definition("#").expect("document");
    assert_eq!(document.to_json_schema(), canonical.to_json_schema());
}

// A target emitted on its own carries the document it named, so `#` still points at that document
// and not at the target standing in for it.
#[test]
fn emitting_a_target_carries_the_document_it_named() {
    let canonical = canonicalize(&json!({
        "type": "object",
        "properties": {"allOf": {"$ref": "#/$defs/schemaArray"}},
        "$defs": {"schemaArray": {"type": "array", "minItems": 1, "items": {"$ref": "#"}}}
    }))
    .expect("canonicalizes");
    assert_eq!(
        canonical
            .definition("#/$defs/schemaArray")
            .expect("target")
            .to_json_schema(),
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": {
                "schemaArray": {"type": "array", "minItems": 1, "items": {"$ref": "#/$defs/root"}},
                "root": {"type": "object", "properties": {"allOf": {"$ref": "#/$defs/schemaArray"}}}
            },
            "type": "array",
            "minItems": 1,
            "items": {"$ref": "#/$defs/root"}
        })
    );
}

// The document takes a name of its own, so an entry already holding that name keeps it.
#[test]
fn emitting_a_target_names_the_document_around_a_taken_name() {
    let canonical = canonicalize(&json!({
        "type": "object",
        "properties": {"allOf": {"$ref": "#/$defs/schemaArray"}, "root": {"$ref": "#/$defs/root"}},
        "$defs": {
            "schemaArray": {"type": "array", "minItems": 1, "items": {"$ref": "#"}},
            "root": {"type": "string"}
        }
    }))
    .expect("canonicalizes");
    assert_eq!(
        canonical
            .definition("#/$defs/schemaArray")
            .expect("target")
            .to_json_schema(),
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": {
                "schemaArray": {"type": "array", "minItems": 1, "items": {"$ref": "#/$defs/root0"}},
                "root": {"type": "string"},
                "root0": {"type": "object", "properties": {
                    "allOf": {"$ref": "#/$defs/schemaArray"}, "root": {"$ref": "#/$defs/root"}
                }}
            },
            "type": "array",
            "minItems": 1,
            "items": {"$ref": "#/$defs/root0"}
        })
    );
}

// A key naming the document root is a pointer only where a schema sits; under `properties` it is a
// property name, and the schema it holds carries pointers of its own.
#[test]
fn emitting_a_target_reaches_a_property_named_after_a_value_keyword() {
    let canonical = canonicalize(&json!({
        "type": "object",
        "properties": {"body": {"$ref": "#/$defs/body"}},
        "$defs": {"body": {"type": "object", "properties": {"const": {"$ref": "#"}}}}
    }))
    .expect("canonicalizes");
    assert_eq!(
        canonical
            .definition("#/$defs/body")
            .expect("target")
            .to_json_schema(),
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": {
                "body": {"type": "object", "properties": {"const": {"$ref": "#/$defs/root"}}},
                "root": {"type": "object", "properties": {"body": {"$ref": "#/$defs/body"}}}
            },
            "type": "object",
            "properties": {"const": {"$ref": "#/$defs/root"}}
        })
    );
}

// A `const` holds an instance, so a value spelled like a pointer stays the value it is.
#[test]
fn emitting_a_target_leaves_a_value_spelled_like_a_pointer() {
    let canonical = canonicalize(&json!({
        "type": "object",
        "properties": {"body": {"$ref": "#/$defs/body"}},
        "$defs": {"body": {"type": "object", "properties": {
            "kind": {"const": {"$ref": "#"}},
            "self": {"$ref": "#"}
        }}}
    }))
    .expect("canonicalizes");
    let emitted = canonical
        .definition("#/$defs/body")
        .expect("target")
        .to_json_schema();
    assert_eq!(
        emitted["properties"]["kind"],
        json!({"const": {"$ref": "#"}})
    );
    assert_eq!(
        emitted["properties"]["self"],
        json!({"$ref": "#/$defs/root"})
    );
}

// The emitted document admits what the target admits, which the pointer it carries decides.
#[test]
fn an_emitted_target_admits_what_the_target_admits() {
    let canonical = canonicalize(&json!({
        "type": "object",
        "properties": {"allOf": {"$ref": "#/$defs/schemaArray"}},
        "$defs": {"schemaArray": {"type": "array", "minItems": 1, "items": {"$ref": "#"}}}
    }))
    .expect("canonicalizes");
    let emitted = canonical
        .definition("#/$defs/schemaArray")
        .expect("target")
        .to_json_schema();
    let emitted = jsonschema::validator_for(&emitted).expect("emitted builds");
    for (instance, valid) in [
        (json!([{}]), true),
        (json!([{"allOf": [{}]}]), true),
        (json!([[]]), false),
        (json!([]), false),
    ] {
        assert_eq!(emitted.is_valid(&instance), valid, "{instance}");
    }
}

// A target naming no document root stands alone already, so it takes no copy of one.
#[test]
fn emitting_a_target_naming_no_document_root_adds_nothing() {
    let canonical = canonicalize(&json!({
        "type": "object",
        "properties": {"a": {"$ref": "#/$defs/A"}},
        "$defs": {"A": {"type": "string", "minLength": 2}}
    }))
    .expect("canonicalizes");
    assert_eq!(
        canonical
            .definition("#/$defs/A")
            .expect("target")
            .to_json_schema(),
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "string",
            "minLength": 2
        })
    );
}

// Same `Reference` root, different targets: unequal handles the hash no longer separates.
#[test]
fn hash_ignores_the_definition_map() {
    fn digest(schema: &jsonschema::canonical::CanonicalSchema) -> u64 {
        let mut hasher = DefaultHasher::new();
        schema.hash(&mut hasher);
        hasher.finish()
    }

    let string_target = canonicalize(&json!({
        "$id": "https://example.com/s",
        "$defs": {"A": {"type": "string"}},
        "$ref": "#/$defs/A"
    }))
    .expect("canonicalizes");
    let integer_target = canonicalize(&json!({
        "$id": "https://example.com/s",
        "$defs": {"A": {"type": "integer"}},
        "$ref": "#/$defs/A"
    }))
    .expect("canonicalizes");

    assert_ne!(string_target, integer_target);
    assert_eq!(digest(&string_target), digest(&integer_target));
}

#[test_case(&json!({"type": "string"}), &json!({"minLength": 4}), &json!({"type": "string", "minLength": 4}); "bound folds into the string leaf")]
#[test_case(&json!({"const": "A"}), &json!({"pattern": "^A$"}), &json!({"const": "A"}); "matching pattern keeps the constant")]
#[test_case(&json!({"const": "A"}), &json!({"const": "B"}), &json!({"not": {}}); "disjoint constants fold to false")]
fn intersect_folds(left: &Value, right: &Value, expected: &Value) {
    let left = canonicalize(left).expect("canonicalizes");
    let right = canonicalize(right).expect("canonicalizes");
    let mut expected = expected.as_object().expect("object").clone();
    expected.insert(
        "$schema".into(),
        json!("https://json-schema.org/draft/2020-12/schema"),
    );
    assert_eq!(
        left.intersect(&right).expect("intersects").to_json_schema(),
        Value::Object(expected)
    );
}

#[test]
fn intersect_is_commutative_and_idempotent() {
    let schemas = [
        json!({"type": "string", "minLength": 2}),
        json!({"anyOf": [{"type": "string"}, {"type": "integer"}]}),
        json!({"const": "a"}),
        json!({"type": "object", "properties": {"id": {"type": "integer"}}}),
    ]
    .map(|schema| canonicalize(&schema).expect("canonicalizes"));
    for left in &schemas {
        assert_eq!(left.intersect(left).expect("intersects"), *left);
        for right in &schemas {
            assert_eq!(
                left.intersect(right).expect("intersects"),
                right.intersect(left).expect("intersects")
            );
        }
    }
}

// A pattern the canonical form does not support keeps the whole document raw.
fn unsupported() -> Value {
    json!({"if": {}, "unevaluatedProperties": false})
}

#[test]
fn intersect_rejects_a_raw_root() {
    let raw = canonicalize(&unsupported()).expect("canonicalizes");
    let modeled = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    let error = raw.intersect(&modeled).expect_err("the left side is raw");
    assert!(matches!(error, CanonicalizationError::UnsupportedOperand));
    assert_eq!(
        error.to_string(),
        "operand is not supported in canonical form"
    );
    assert!(matches!(
        modeled.intersect(&raw),
        Err(CanonicalizationError::UnsupportedOperand)
    ));
}

// An unsupported `$ref` target takes the whole document raw rather than leaving a raw entry beside a
// modeled root, so the handle a consumer resolves from carries no definitions at all.
#[test]
fn intersect_rejects_a_raw_reference_target() {
    let document = json!({
        "properties": {"a": {"$ref": "#/$defs/A"}},
        "$defs": {"A": unsupported()}
    });
    let raw = canonicalize(&document).expect("canonicalizes");
    assert_eq!(raw.kind(), CanonicalKind::Raw);
    assert_eq!(raw.definitions().next(), None);
    let modeled = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    assert!(matches!(
        raw.intersect(&modeled),
        Err(CanonicalizationError::UnsupportedOperand)
    ));
}

// A result carries the targets it still names and no others, whether or not the operands shared a
// document: `definitions()` answers about the same schema `to_json_schema()` emits.
#[test]
fn a_result_carries_exactly_the_targets_it_still_names() {
    let root = canonicalize(&json!({
        "type": "object",
        "$defs": {"A": {"type": "string"}, "B": {"minLength": 2}},
        "properties": {"a": {"$ref": "#/$defs/A"}, "b": {"$ref": "#/$defs/B"}}
    }))
    .expect("canonicalizes");
    let CanonicalView::Object(view) = root.view() else {
        panic!("expected an Object view");
    };
    let left = view.properties.get("a").expect("property a").clone();
    let right = view.properties.get("b").expect("property b").clone();

    // Both pointers are read through and the meet folds into one leaf, naming neither.
    let merged = left.intersect(&right).expect("intersects");
    assert_eq!(merged.definitions().len(), 0);
    assert_eq!(merged.definition("#/$defs/A"), None);
    let emitted = merged.to_json_schema();
    assert!(emitted.get("$defs").is_none(), "emitted {emitted}");
    validator_for(&emitted).expect("the result resolves its own pointers");
}

#[test]
fn intersect_rejects_operands_from_different_drafts() {
    let draft7 = options()
        .with_draft(Draft::Draft7)
        .canonicalize(&json!({"type": "string"}))
        .expect("canonicalizes");
    let latest = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    let error = draft7.intersect(&latest).expect_err("the drafts differ");
    assert!(matches!(
        error,
        CanonicalizationError::IncompatibleOperands(OperandMismatch::Drafts {
            left: Draft::Draft7,
            right: Draft::Draft202012
        })
    ));
    assert_eq!(
        error.to_string(),
        "operands canonicalized under Draft7 and Draft202012"
    );
}

// 2020-12 annotates formats by default, so asserting them is the deliberate mismatch.
#[test]
fn intersect_rejects_operands_with_different_format_assertion_policy() {
    let asserting = options()
        .should_validate_formats(true)
        .canonicalize(&json!({"type": "string"}))
        .expect("canonicalizes");
    let annotating = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    let error = asserting
        .intersect(&annotating)
        .expect_err("the format policies differ");
    assert!(matches!(
        error,
        CanonicalizationError::IncompatibleOperands(OperandMismatch::FormatAssertions)
    ));
    assert_eq!(
        error.to_string(),
        "operands disagree on whether `format` asserts"
    );
}

// `regex` is the deliberate mismatch against the default `fancy-regex` engine.
#[test]
fn intersect_rejects_operands_with_different_pattern_engines() {
    let regex_engine = options()
        .with_pattern_options(PatternOptions::regex())
        .canonicalize(&json!({"type": "string"}))
        .expect("canonicalizes");
    let fancy_engine = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    let error = regex_engine
        .intersect(&fancy_engine)
        .expect_err("the pattern engines differ");
    assert!(matches!(
        error,
        CanonicalizationError::IncompatibleOperands(OperandMismatch::PatternEngine)
    ));
    assert_eq!(
        error.to_string(),
        "operands canonicalized with different pattern engines"
    );
}

// A side with no definitions of its own combines against the other's targets, whichever side it is.
// The intersection reads through the pointer, so the target lands in the result and its entry, now
// named by nothing, is dropped.
#[test_case(false; "empty map on the right")]
#[test_case(true; "empty map on the left")]
fn intersect_reads_through_the_only_definition_map(swap: bool) {
    let referencing = canonicalize(&json!({
        "$defs": {"A": {"type": "string"}},
        "$ref": "#/$defs/A"
    }))
    .expect("canonicalizes");
    let plain = canonicalize(&json!({"minLength": 4})).expect("canonicalizes");
    let merged = if swap {
        plain.intersect(&referencing)
    } else {
        referencing.intersect(&plain)
    }
    .expect("intersects");
    assert_eq!(
        merged.to_json_schema(),
        latest(
            json!({"type": "string", "minLength": 4})
                .as_object()
                .expect("object")
                .clone()
        )
    );
    assert_eq!(merged.definitions().count(), 0);
}

// A pointer and the schema it names accept the same values, so one written each way cancels.
#[test]
fn a_pointer_and_its_target_are_the_same_schema_to_the_algebra() {
    let referencing = canonicalize(&json!({
        "$defs": {"A": {"type": "string"}},
        "$ref": "#/$defs/A"
    }))
    .expect("canonicalizes");
    let inline = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    assert_eq!(
        referencing.covers(&inline).expect("compares"),
        Containment::Yes
    );
    assert_eq!(
        referencing
            .subtract(&inline)
            .expect("subtracts")
            .satisfiability(),
        Satisfiability::No
    );
    assert_eq!(
        inline
            .subtract(&referencing)
            .expect("subtracts")
            .satisfiability(),
        Satisfiability::No
    );
}

// A pointer nested under a property is read through as well, which is where a document that names
// a shared component meets the same component written out.
#[test]
fn a_nested_pointer_is_read_through() {
    let referencing = canonicalize(&json!({
        "$defs": {"User": {"type": "object", "properties": {"id": {"type": "integer"}}}},
        "type": "object",
        "properties": {"user": {"$ref": "#/$defs/User"}}
    }))
    .expect("canonicalizes");
    let inline = canonicalize(&json!({
        "type": "object",
        "properties": {"user": {"type": "object", "properties": {"id": {"type": "integer"}}}}
    }))
    .expect("canonicalizes");
    assert_eq!(
        referencing.covers(&inline).expect("compares"),
        Containment::Yes
    );
    assert_eq!(
        inline.covers(&referencing).expect("compares"),
        Containment::Yes
    );
    assert_eq!(
        referencing
            .subtract(&inline)
            .expect("subtracts")
            .satisfiability(),
        Satisfiability::No
    );
    assert_eq!(
        inline
            .subtract(&referencing)
            .expect("subtracts")
            .satisfiability(),
        Satisfiability::No
    );
}

// An intersection the form could only approximate stays approximated wherever it is read again, so
// a check that discards it cannot leave the run believing the pair was exact.
#[test]
fn an_approximated_intersection_read_twice_keeps_the_document_unsupported() {
    let schema = json!({
        "anyOf": [
            {
                "type": "object",
                "patternProperties": {"^b": {"type": "number"}},
                "additionalProperties": {"type": "integer"}
            },
            {
                "type": "object",
                "patternProperties": {"^a": {"type": "string"}, "^b": {"type": "integer"}},
                "additionalProperties": {"type": "integer"}
            }
        ]
    });
    assert_validation_parity(
        &schema,
        &[
            json!({"a": "s"}),
            json!({"a": "s", "b": 1}),
            json!({"b": 1}),
            json!({"b": 1.5}),
            json!({}),
        ],
    );
}

// A shield leaves the keys a pattern map matches to it, so closing a map around such a key keeps
// the pattern's demand rather than the shield's.
#[test]
fn a_closed_map_keeps_the_pattern_demand_of_an_admitted_key() {
    let closed = json!({"type": "object", "additionalProperties": false, "properties": {"a": {}}});
    let patterned = json!({
        "type": "object",
        "additionalProperties": {"type": "integer"},
        "patternProperties": {"^a": {"type": "string"}}
    });
    let merged = canonicalize(&closed)
        .expect("canonicalizes")
        .intersect(&canonicalize(&patterned).expect("canonicalizes"))
        .expect("intersects");
    let closed_validator = validator_for(&closed).expect("builds");
    let patterned_validator = validator_for(&patterned).expect("builds");
    let merged_validator = validator_for(&merged.to_json_schema()).expect("builds");
    for instance in [
        json!({}),
        json!({"a": "x"}),
        json!({"a": 1}),
        json!({"b": 1}),
    ] {
        assert_eq!(
            closed_validator.is_valid(&instance) && patterned_validator.is_valid(&instance),
            merged_validator.is_valid(&instance),
            "the intersection disagrees on {instance}\n  merged = {}",
            merged.to_json_schema()
        );
    }
}

// A meet of two pointers is a body no name denotes, so `intersect` writes it out; a join leaves
// every branch exactly a named body, so `union` keeps the names. Reading them through here would
// spell the union one way and the document holding that same union another.
#[test]
fn a_union_keeps_the_names_its_branches_arrived_under() {
    let referencing = canonicalize(&json!({
        "$defs": {"A": {"type": "string"}},
        "$ref": "#/$defs/A"
    }))
    .expect("canonicalizes");
    let inline = canonicalize(&json!({"type": "string"})).expect("canonicalizes");

    let united = referencing.union(&inline).expect("unions");
    assert_eq!(united.kind(), CanonicalKind::AnyOf);
    assert_eq!(united, inline.union(&referencing).expect("unions"));
    // The document holding that union reaches the same form.
    assert_eq!(
        united,
        canonicalize(&json!({
            "$defs": {"A": {"type": "string"}},
            "anyOf": [{"$ref": "#/$defs/A"}, {"type": "string"}]
        }))
        .expect("canonicalizes")
    );
    let validator = validator_for(&united.to_json_schema()).expect("builds");
    assert!(validator.is_valid(&json!("x")));
    assert!(!validator.is_valid(&json!(1)));
}

// The names travel with the branches, and the identities are answered on the operands the caller
// holds, so `a | a` is still `a`.
#[test]
fn a_union_of_pointers_is_one_form_whichever_way_round() {
    let pointer = |name: &str| {
        canonicalize(&json!({
            "$defs": {name: {"type": "string"}},
            "$ref": format!("#/$defs/{name}")
        }))
        .expect("canonicalizes")
    };
    let first = pointer("A");
    let second = pointer("B");

    assert_eq!(first.union(&first).expect("unions"), first);
    let united = first.union(&second).expect("unions");
    assert_eq!(united, second.union(&first).expect("unions"));
    // Both names survive, so neither operand's map is the one that gave way.
    assert_eq!(united.definitions().count(), 2);
    let validator = validator_for(&united.to_json_schema()).expect("builds");
    assert!(validator.is_valid(&json!("x")));
    assert!(!validator.is_valid(&json!(1)));
}

// A boolean target decides the pair, so the pointer standing for it must be read through before
// either side is dispatched.
#[test_case(&json!(true), &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "string"}); "a permissive target leaves the other side")]
#[test_case(&json!(false), &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "not": {}}); "a rejecting target empties the pair")]
fn intersect_reads_through_a_pointer_to_a_boolean(target: &Value, expected: &Value) {
    let referencing =
        canonicalize(&json!({"$defs": {"A": target}, "$ref": "#/$defs/A"})).expect("canonicalizes");
    let string = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    assert_eq!(
        referencing
            .intersect(&string)
            .expect("intersects")
            .to_json_schema(),
        *expected
    );
}

// Two definitions naming one target, which the walk over the map reaches through each of them.
#[test]
fn a_target_two_definitions_share_is_read_through() {
    let sharing = canonicalize(&json!({
        "$defs": {
            "A": {"type": "object", "properties": {"x": {"$ref": "#/$defs/C"}}},
            "B": {"type": "object", "properties": {"y": {"$ref": "#/$defs/C"}}},
            "C": {"type": "string"}
        },
        "anyOf": [{"$ref": "#/$defs/A"}, {"$ref": "#/$defs/B"}]
    }))
    .expect("canonicalizes");
    let object = canonicalize(&json!({"type": "object"})).expect("canonicalizes");
    let merged = sharing.intersect(&object).expect("intersects");
    let sharing_validator = validator_for(&sharing.to_json_schema()).expect("builds");
    let merged_validator = validator_for(&merged.to_json_schema()).expect("builds");
    for instance in [
        json!({"x": "a"}),
        json!({"y": "a"}),
        json!({"x": 1}),
        json!({}),
        json!("x"),
    ] {
        assert_eq!(
            sharing_validator.is_valid(&instance) && instance.is_object(),
            merged_validator.is_valid(&instance),
            "{instance}"
        );
    }
}

// A `#` inside a definition body names the document root, which is no entry of the map, so the walk
// finds no body to read through there.
#[test]
fn a_definition_naming_the_document_root_is_read_through() {
    let rooted = canonicalize(&json!({
        "$defs": {"A": {"type": "object", "properties": {"self": {"$ref": "#"}}}},
        "type": "object",
        "properties": {"a": {"$ref": "#/$defs/A"}}
    }))
    .expect("canonicalizes");
    let object = canonicalize(&json!({"type": "object"})).expect("canonicalizes");
    let merged = rooted.intersect(&object).expect("intersects");
    assert_eq!(merged.covers(&rooted).expect("compares"), Containment::Yes);
    let validator = validator_for(&merged.to_json_schema()).expect("builds");
    assert!(validator.is_valid(&json!({"a": {"self": {"a": {}}}})));
    assert!(!validator.is_valid(&json!("x")));
}

// A cycle stops the pointers on it, and no others: a recursive definition beside a plain one leaves
// the plain one read through.
#[test]
fn a_recursive_definition_leaves_the_other_pointers_readable() {
    let root = canonicalize(&json!({
        "$defs": {
            "Node": {"type": "object", "properties": {"next": {"$ref": "#/$defs/Node"}}},
            "Id": {"type": "string"}
        },
        "type": "object",
        "properties": {"id": {"$ref": "#/$defs/Id"}, "node": {"$ref": "#/$defs/Node"}}
    }))
    .expect("canonicalizes");
    let CanonicalView::Object(view) = root.view() else {
        panic!("expected an Object view");
    };
    let named = view.properties.get("id").expect("property id").clone();
    let inline = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    assert_eq!(named.covers(&inline).expect("compares"), Containment::Yes);
    assert_eq!(
        named.subtract(&inline).expect("subtracts").satisfiability(),
        Satisfiability::No
    );
}

// A pointer that leads back to a body already on the path never settles, so a cyclic map is left
// unread and the operands are combined as pointers.
#[test]
fn a_cyclic_map_is_not_read_through() {
    let recursive = canonicalize(&json!({
        "$defs": {"Node": {"type": "object", "properties": {"next": {"$ref": "#/$defs/Node"}}}},
        "$ref": "#/$defs/Node"
    }))
    .expect("canonicalizes");
    let object = canonicalize(&json!({"type": "object"})).expect("canonicalizes");
    let merged = recursive.intersect(&object).expect("intersects");
    assert!(merged.definition("#/$defs/Node").is_some());
    let emitted = merged.to_json_schema();
    let validator = validator_for(&emitted).expect("builds");
    assert!(validator.is_valid(&json!({"next": {}})));
    assert!(!validator.is_valid(&json!("x")));
}

#[test]
fn intersect_reads_through_two_definitions_of_one_name() {
    let left = canonicalize(&json!({
        "$defs": {"A": {"type": "string"}},
        "$ref": "#/$defs/A"
    }))
    .expect("canonicalizes");
    let right = canonicalize(&json!({
        "$defs": {"A": {"type": "string", "minLength": 4}},
        "$ref": "#/$defs/A"
    }))
    .expect("canonicalizes");
    // The two `A`s are renamed apart, so the meet is the narrower of the two - kept as the pointer
    // naming it, under the fresh name the merge gave it.
    assert_eq!(
        left.intersect(&right).expect("intersects").to_json_schema(),
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": {"A-2": {"type": "string", "minLength": 4}},
            "$ref": "#/$defs/A-2"
        })
    );
}

// The side that gives way renames its `$defs` keys, and the root it reads through `#` names those
// too - one left behind reads the other document's bodies under the old names.
#[test]
fn a_renamed_side_carries_its_own_document_root() {
    let recursive = canonicalize(&json!({
        "type": "object",
        "properties": {"k": {"$ref": "#/$defs/K"}, "self": {"$ref": "#"}},
        "$defs": {"K": {"type": "string"}}
    }))
    .expect("canonicalizes");
    let plain = canonicalize(&json!({
        "type": "object",
        "properties": {"k": {"$ref": "#/$defs/K"}},
        "$defs": {"K": {"type": "integer"}}
    }))
    .expect("canonicalizes");
    let meet = recursive.intersect(&plain).expect("intersects");
    assert_eq!(
        meet.to_json_schema(),
        plain
            .intersect(&recursive)
            .expect("intersects")
            .to_json_schema()
    );
    let validator = validator_for(&meet.to_json_schema()).expect("builds");
    // `self` is read against the recursive document, whose `K` is the string one.
    assert!(validator.is_valid(&json!({"self": {"k": "x"}})));
    assert!(!validator.is_valid(&json!({"self": {"k": 1}})));
}

// Renaming rebuilds the nodes that hold the renamed pointer, and a choice orders its branches by
// their bodies - a fresh name reorders them. Left unsorted the result no longer reads back as
// itself.
#[test]
fn a_renamed_choice_keeps_its_branches_sorted() {
    let document = |body: Value| {
        json!({
            "$defs": {"a": body, "a-1": {"type": "object", "required": ["y"]}},
            "type": "object",
            "properties": {"c": {"oneOf": [{"$ref": "#/$defs/a"}, {"$ref": "#/$defs/a-1"}]}}
        })
    };
    let left = canonicalize(&document(json!({"type": "object", "required": ["x"]})))
        .expect("canonicalizes");
    let right = canonicalize(&document(json!({"type": "object", "required": ["zzz"]})))
        .expect("canonicalizes");
    for (name, result) in [
        ("union", left.union(&right)),
        ("intersect", left.intersect(&right)),
    ] {
        let Ok(result) = result else { continue };
        let emitted = result.to_json_schema();
        assert_eq!(
            canonicalize(&emitted).expect("re-canonicalizes"),
            result,
            "{name} does not read back as itself: {emitted}"
        );
    }
}

// Two documents whose `#` reads a different body cannot merge: the entry they disagree on renames
// apart, but that renaming is exactly what leaves their roots naming different bodies. Left with
// the receiver's root un-renamed the two compare equal, the guard passes, and the operation answers
// with one document's pointers over the other's bodies - accepting a value both operands reject.
#[test]
fn two_recursive_documents_disagreeing_on_an_entry_decline() {
    let document = |body: Value| {
        json!({
            "$defs": {"K": body},
            "type": "object",
            "properties": {"k": {"$ref": "#/$defs/K"}, "up": {"$ref": "#"}}
        })
    };
    let left = document(json!({"type": "object", "additionalProperties": {"type": "integer"}}));
    let right = document(json!({"type": "integer", "minimum": 0}));
    let instance = json!({"k": {"k": 1}, "up": {"k": 1}});
    assert!(!validator_for(&left).expect("builds").is_valid(&instance));
    assert!(!validator_for(&right).expect("builds").is_valid(&instance));
    let left = canonicalize(&left).expect("canonicalizes");
    let right = canonicalize(&right).expect("canonicalizes");
    for answer in [
        left.union(&right),
        left.subtract(&right),
        left.intersect(&right),
    ] {
        assert!(matches!(
            answer,
            Err(CanonicalizationError::IncompatibleOperands(
                OperandMismatch::DocumentRoots
            ))
        ));
    }
}

// `#` names the document a node was read against, so the argument's root has to travel with it.
// Read against the receiver's document instead, a node naming `#` reads a body it never named.
#[test]
fn covers_reads_the_argument_against_its_own_document() {
    let target = canonicalize(&json!({
        "type": "object",
        "properties": {"a": {"$ref": "#/$defs/x"}},
        "$defs": {"x": {"$ref": "#"}}
    }))
    .expect("canonicalizes")
    .definition("#/$defs/x")
    .expect("names a definition");
    assert_eq!(target.satisfiability(), Satisfiability::Yes);
    // `false` admits no value at all, so it covers nothing that admits one.
    assert_ne!(
        canonicalize(&json!(false))
            .expect("canonicalizes")
            .covers(&target)
            .expect("answers"),
        Containment::Yes
    );
}

// A `$defs` entry under a nested `$id` is keyed by a minted URI rather than by a `#/$defs/` name,
// and is private to the document all the same: two versions of it rename apart like any other.
#[test]
fn a_definition_under_a_nested_id_is_private_to_its_document() {
    let document = |body: Value| {
        json!({
            "$id": "https://example.com/root",
            "$defs": {"sub": {
                "$id": "https://example.com/sub",
                "$defs": {"User": body},
                "$ref": "#/$defs/User"
            }},
            "$ref": "#/$defs/sub"
        })
    };
    let old = canonicalize(&document(json!({"type": "object"}))).expect("canonicalizes");
    let new = canonicalize(&document(json!({"type": "object", "minProperties": 1})))
        .expect("canonicalizes");
    // The empty object is what the edit took away.
    assert_eq!(
        old.subtract(&new).expect("subtracts").to_json_schema(),
        latest(json!({"const": {}}).as_object().expect("an object").clone())
    );
}

// The documented `DocumentRoots` mismatch has to be the answer the documented pair actually gets.
#[test]
fn two_documents_binding_the_root_differently_decline() {
    let plain = canonicalize(&json!({
        "type": "object", "properties": {"next": {"$ref": "#"}}
    }))
    .expect("canonicalizes");
    let bounded = canonicalize(&json!({
        "type": "object", "properties": {"next": {"$ref": "#"}}, "minProperties": 1
    }))
    .expect("canonicalizes");
    assert!(matches!(
        plain.union(&bounded),
        Err(CanonicalizationError::IncompatibleOperands(
            OperandMismatch::DocumentRoots
        ))
    ));
}

// A pointer stands for the schema it references, so a member that schema admits is admitted - comparing the
// pointer itself against the member instead reads as a rejection that never happened.
#[test]
fn a_member_admitted_through_a_pointer_survives_the_meet() {
    let left = canonicalize(&json!({
        "$defs": {"E": {"const": "x"}},
        "type": "array",
        "contains": {"$ref": "#/$defs/E"}
    }))
    .expect("canonicalizes");
    let right = canonicalize(&json!({"enum": [["x"], ["y"]]})).expect("canonicalizes");
    let meet = left.intersect(&right).expect("intersects").to_json_schema();
    // Both operands take `["x"]`, so their meet takes it too.
    assert!(validator_for(&meet)
        .expect("builds")
        .is_valid(&json!(["x"])));
}

// A schema wide at every level multiplies width by depth, so the walk that looks for a witness has
// to give up rather than build the number of values the keywords ask for.
#[test]
fn a_deep_and_wide_schema_gives_up_rather_than_building_every_witness() {
    let mut schema = json!({"type": "string"});
    for _ in 0..6 {
        schema = json!({"type": "array", "minItems": 64, "items": schema});
    }
    assert_eq!(
        canonicalize(&schema)
            .expect("canonicalizes")
            .satisfiability(),
        Satisfiability::Unknown
    );
}

// A choice between pointers naming targets that are not disjoint stays a choice: weighing the
// bodies would cost as much as canonicalizing them. A key renamed elsewhere in the document leaves
// its branches as they were, and the walk hands the node back untouched.
#[test]
fn intersect_renames_around_a_choice_it_leaves_alone() {
    // Held by a property entry, where a pointer stands rather than folding into the node.
    let document = |bound: Value| {
        json!({
            "$defs": {
                "K": bound,
                "P": {"type": "string"},
                "T": {"type": "string", "minLength": 1}
            },
            "type": "object",
            "properties": {
                "k": {"$ref": "#/$defs/K"},
                "choice": {"oneOf": [{"$ref": "#/$defs/P"}, {"$ref": "#/$defs/T"}]}
            }
        })
    };
    let left =
        canonicalize(&document(json!({"type": "integer", "minimum": 0}))).expect("canonicalizes");
    let right =
        canonicalize(&document(json!({"type": "integer", "minimum": 1}))).expect("canonicalizes");
    let meet = left.intersect(&right).expect("intersects");
    let emitted = meet.to_json_schema();
    // The disagreeing key is renamed apart; the choice keeps the two names it was spelled with.
    assert!(meet.definition("#/$defs/K-2").is_some());
    assert_eq!(
        emitted["properties"]["choice"],
        json!({"oneOf": [{"$ref": "#/$defs/P"}, {"$ref": "#/$defs/T"}]})
    );
    // Exactly one branch may match, so only the empty string is left for `choice`.
    let validator = validator_for(&emitted).expect("builds");
    assert!(validator.is_valid(&json!({"k": 1, "choice": ""})));
    for rejected in [
        json!({"k": 0, "choice": ""}),
        json!({"k": 1, "choice": "a"}),
        json!({"k": "x", "choice": ""}),
    ] {
        assert!(!validator.is_valid(&rejected), "accepted {rejected}");
    }
}

// Refuting a value scans the receiver for pointers this run cannot read, and a choice holds its
// branches where no other node does.
#[test]
fn covers_reads_a_choice_before_refuting_a_value() {
    let choice = canonicalize(&json!({
        "$defs": {"P": {"type": "string"}, "T": {"type": "string", "minLength": 1}},
        "oneOf": [{"$ref": "#/$defs/P"}, {"$ref": "#/$defs/T"}]
    }))
    .expect("canonicalizes");
    let member = canonicalize(&json!({"const": "a"})).expect("canonicalizes");
    // `"a"` matches both branches, so the choice turns it away.
    assert_eq!(choice.covers(&member).expect("compares"), Containment::No);
    let validator = validator_for(&choice.to_json_schema()).expect("builds");
    assert!(!validator.is_valid(&json!("a")));
    assert!(validator.is_valid(&json!("")));
}

// Draft 4 cannot tell `1` from `1.0` by value equality, so an integer value set keeps a type guard
// around it. A rename walks every body and hands the guard back as it found it: it names nothing.
#[test]
fn intersect_renames_past_a_draft4_type_guard() {
    let document = |bound: Value| {
        json!({
            "$defs": {
                "K": bound,
                "G": {"allOf": [{"type": "integer"}, {"enum": [1, 2]}]}
            },
            "type": "object",
            "properties": {"k": {"$ref": "#/$defs/K"}, "g": {"$ref": "#/$defs/G"}}
        })
    };
    let canonical = |schema: &Value| {
        options()
            .with_draft(Draft::Draft4)
            .canonicalize(schema)
            .expect("canonicalizes")
    };
    let left = canonical(&document(json!({"type": "integer", "minimum": 0})));
    let right = canonical(&document(json!({"type": "integer", "minimum": 2})));
    let meet = left.intersect(&right).expect("intersects");
    let emitted = meet.to_json_schema();
    assert!(meet.definition("#/$defs/K-2").is_some());
    assert_eq!(
        emitted["$defs"]["G"],
        json!({"type": "integer", "enum": [1, 2]})
    );
    let validator = validator_for(&emitted).expect("builds");
    assert!(validator.is_valid(&json!({"k": 2, "g": 1})));
    for rejected in [
        json!({"k": 1, "g": 1}),
        json!({"k": 2, "g": 3}),
        json!({"k": 2, "g": 1.0}),
    ] {
        assert!(!validator.is_valid(&rejected), "accepted {rejected}");
    }
}

// A format no checker covers is read as met by the meet, so nothing definite may rest on it.
#[test]
fn an_unchecked_format_decides_neither_coverage_nor_difference() {
    let canonical = |schema: &Value| {
        options()
            .should_validate_formats(true)
            .canonicalize(schema)
            .expect("canonicalizes")
    };
    let formatted = canonical(&json!({"type": "string", "format": "x-custom"}));
    let members = canonical(&json!({"enum": ["a", "b"]}));
    assert_ne!(
        formatted.covers(&members).expect("compares"),
        Containment::Yes
    );
    // Declining is sound; answering that the members are all kept is not.
    assert!(
        members
            .subtract(&formatted)
            .is_ok_and(|difference| difference.satisfiability() != Satisfiability::No)
            || members.subtract(&formatted).is_err()
    );
}

// The same unchecked facet on both sides: every value either side admits carries it already, so
// the fold rests on nothing undecided.
#[test]
fn one_unchecked_facet_on_both_sides_does_not_block_the_meet() {
    let shielded = canonicalize(&json!({
        "type": "object",
        "properties": {"a": {"type": "string", "minLength": 5, "contentMediaType": "application/x-nope"}},
        "additionalProperties": {"type": "string", "contentMediaType": "application/x-nope"}
    }))
    .expect("canonicalizes");
    let patterned = canonicalize(&json!({
        "type": "object",
        "patternProperties": {"^a": {"type": "string"}}
    }))
    .expect("canonicalizes");
    shielded.intersect(&patterned).expect("intersects");
    assert_eq!(
        canonicalize(&json!({"allOf": [
            {"type": "object", "properties": {"a": {"type": "string", "minLength": 5, "contentMediaType": "application/x-nope"}}, "additionalProperties": {"type": "string", "contentMediaType": "application/x-nope"}},
            {"type": "object", "patternProperties": {"^a": {"type": "string"}}}
        ]}))
        .expect("canonicalizes")
        .kind(),
        CanonicalKind::Object
    );
}

// An empty pattern matches every key, which parse spells by dropping it: one kept in a key
// constraint would not read back, and the shield behind it governs nothing.
#[test]
fn an_empty_pattern_leaves_neither_a_key_constraint_nor_a_shield() {
    let emitted = canonicalize(&json!({"allOf": [
        {"type": "object", "patternProperties": {"": {"type": "integer"}}, "additionalProperties": {"type": "array"}},
        {"type": "object", "additionalProperties": {"type": "string"}}
    ]}))
    .expect("canonicalizes")
    .to_json_schema();
    assert_eq!(
        canonicalize(&emitted)
            .expect("re-canonicalizes")
            .to_json_schema(),
        emitted
    );
    let shielded = canonicalize(&json!({
        "type": "object", "patternProperties": {"": {"type": "integer"}}, "additionalProperties": {"type": "string"}
    }))
    .expect("canonicalizes");
    let bare = canonicalize(&json!({
        "type": "object", "patternProperties": {"": {"type": "integer"}}
    }))
    .expect("canonicalizes");
    assert_eq!(shielded.to_json_schema(), bare.to_json_schema());
    assert_eq!(shielded.covers(&bare).expect("compares"), Containment::Yes);
    assert_eq!(bare.covers(&shielded).expect("compares"), Containment::Yes);
}

// One value set has one canonical form, whichever entry point builds it: without the fold a
// conjunction over a pointer would stay symbolic here while an operation folds it.
#[test]
fn a_document_and_an_operation_over_its_parts_reach_the_same_form() {
    // A result of an operation reads back as itself.
    let patterned = canonicalize(&json!({
        "$defs": {"k": {"enum": ["a", "b"]}},
        "type": "object",
        "patternProperties": {"^": {"$ref": "#/$defs/k"}}
    }))
    .expect("canonicalizes");
    let named = canonicalize(&json!({"type": "object", "properties": {"a": {"const": "a"}}}))
        .expect("canonicalizes");
    let meet = patterned.intersect(&named).expect("intersects");
    assert_eq!(
        canonicalize(&meet.to_json_schema()).expect("re-canonicalizes"),
        meet
    );

    // And a document folds the way an operation over the same values does.
    let together = canonicalize(&json!({
        "$defs": {"x": {"type": "string"}},
        "allOf": [{"$ref": "#/$defs/x"}, {"type": "string", "minLength": 4}]
    }))
    .expect("canonicalizes");
    let apart = canonicalize(&json!({"$defs": {"x": {"type": "string"}}, "$ref": "#/$defs/x"}))
        .expect("canonicalizes")
        .intersect(
            &canonicalize(&json!({"type": "string", "minLength": 4})).expect("canonicalizes"),
        )
        .expect("intersects");
    assert_eq!(together, apart);
    assert_eq!(
        together.to_json_schema(),
        latest(
            json!({"type": "string", "minLength": 4})
                .as_object()
                .expect("object")
                .clone()
        )
    );

    // The join keeps its branches named where the meet writes its result out, and a document
    // holding the same union reaches that named form too.
    let joined = canonicalize(&json!({
        "$defs": {"x": {"type": "integer", "minimum": 5}},
        "anyOf": [{"$ref": "#/$defs/x"}, {"type": "string"}]
    }))
    .expect("canonicalizes");
    assert_eq!(
        joined,
        canonicalize(
            &json!({"$defs": {"x": {"type": "integer", "minimum": 5}}, "$ref": "#/$defs/x"})
        )
        .expect("canonicalizes")
        .union(&canonicalize(&json!({"type": "string"})).expect("canonicalizes"))
        .expect("unions")
    );
    assert_eq!(joined.definitions().count(), 1);

    // A conjunction the bodies contradict is empty, rather than an opaque `allOf`.
    let contradiction = canonicalize(&json!({
        "$defs": {"left": {"type": "integer"}, "right": {"type": "string"}},
        "allOf": [{"$ref": "#/$defs/left"}, {"$ref": "#/$defs/right"}]
    }))
    .expect("canonicalizes");
    assert_eq!(contradiction.satisfiability(), Satisfiability::No);
}

fn latest(mut schema: Map<String, Value>) -> Value {
    schema.insert(
        "$schema".into(),
        json!("https://json-schema.org/draft/2020-12/schema"),
    );
    Value::Object(schema)
}

#[test_case(&json!({"type": "string"}), &json!({"type": "integer"}), &json!({"type": ["integer", "string"]}); "disjoint types join into one type set")]
#[test_case(&json!({"const": "a"}), &json!({"enum": ["a", "b"]}), &json!({"enum": ["a", "b"]}); "a value the other branch already holds adds nothing")]
#[test_case(&json!({"type": "string", "minLength": 4}), &json!({"type": "string"}), &json!({"type": "string"}); "a narrower branch is absorbed by the wider one")]
fn union_folds(left: &Value, right: &Value, expected: &Value) {
    let left = canonicalize(left).expect("canonicalizes");
    let right = canonicalize(right).expect("canonicalizes");
    assert_eq!(
        left.union(&right).expect("unions").to_json_schema(),
        latest(expected.as_object().expect("object").clone())
    );
}

#[test_case(&json!({"type": "integer", "minimum": 10}), &json!({"type": "integer", "minimum": 15}), &json!({"type": "integer", "minimum": 10, "maximum": 14}); "a tightened bound leaves the values it turned away")]
#[test_case(&json!({"enum": ["a", "b"]}), &json!({"enum": ["a"]}), &json!({"const": "b"}); "a shrunken value set leaves the value it dropped")]
#[test_case(&json!({"type": "object", "properties": {"a": {"type": "string"}}}), &json!({"type": "object", "required": ["a"], "properties": {"a": {"type": "string"}}}), &json!({"type": "object", "properties": {"a": false}}); "a new requirement leaves the objects that omit the key")]
#[test_case(&json!({"type": "integer"}), &json!({"type": ["integer", "string"]}), &json!({"not": {}}); "a widened type leaves nothing behind")]
fn subtract_folds(left: &Value, right: &Value, expected: &Value) {
    let left = canonicalize(left).expect("canonicalizes");
    let right = canonicalize(right).expect("canonicalizes");
    assert_eq!(
        left.subtract(&right).expect("subtracts").to_json_schema(),
        latest(expected.as_object().expect("object").clone())
    );
}

// The emitted result must accept exactly the values set arithmetic says it does. Each case carries
// the instances that tell its own three regions apart - left-only, right-only, and shared - since a
// list none of them reach passes whatever the operations return.
#[test_case(&json!({"type": "integer", "minimum": 10}), &json!({"type": "integer", "minimum": 15}), &[json!(12), json!(20), json!(5)]; "overlapping numeric windows")]
#[test_case(&json!({"type": "string"}), &json!({"type": "string", "maxLength": 2}), &[json!("abc"), json!("ab"), json!(7)]; "a length ceiling over an open string")]
#[test_case(&json!({"enum": ["a", "b", 1]}), &json!({"enum": ["a", 2]}), &[json!("b"), json!(1), json!(2), json!("a"), json!("z")]; "partly overlapping value sets")]
#[test_case(&json!({"type": "object", "properties": {"a": {"type": "string"}}}), &json!({"type": "object", "required": ["a"]}), &[json!({}), json!({"a": "x"}), json!({"b": 1}), json!("x")]; "an object gaining a requirement")]
#[test_case(&json!({"type": ["integer", "string"]}), &json!({"type": "boolean"}), &[json!(1), json!("a"), json!(true), json!(null)]; "disjoint type sets")]
fn union_and_difference_keep_validation_parity(
    left: &Value,
    right: &Value,
    discriminating: &[Value],
) {
    let mut instances = vec![
        json!(9),
        json!(10),
        json!(14),
        json!(15),
        json!("a"),
        json!("abc"),
        json!(true),
        json!(null),
        json!({}),
        json!({"a": "x"}),
        json!({"a": 1}),
        json!([]),
    ];
    instances.extend(discriminating.iter().cloned());
    let canonical_left = canonicalize(left).expect("canonicalizes");
    let canonical_right = canonicalize(right).expect("canonicalizes");
    let united = canonical_left.union(&canonical_right).expect("unions");
    let difference = canonical_left
        .subtract(&canonical_right)
        .expect("subtracts");
    let left_validator = validator_for(left).expect("builds");
    let right_validator = validator_for(right).expect("builds");
    let united_validator = validator_for(&united.to_json_schema()).expect("builds");
    let difference_validator = validator_for(&difference.to_json_schema()).expect("builds");
    let regions = |predicate: fn(bool, bool) -> bool| {
        instances.iter().any(|instance| {
            predicate(
                left_validator.is_valid(instance),
                right_validator.is_valid(instance),
            )
        })
    };
    assert!(
        // One value in the difference, so it is not vacuously empty; one `right` admits, so the
        // union and the difference part ways; one neither admits, so the union is not vacuously
        // true. A nested pair has no right-only region, which is why `right` alone is the second
        // requirement.
        regions(|left, right| left && !right)
            && regions(|_, right| right)
            && regions(|left, right| !left && !right),
        "case has no instance separating the difference, the union and the outside\n  left = {left}\n  right = {right}"
    );
    for instance in &instances {
        let in_left = left_validator.is_valid(instance);
        let in_right = right_validator.is_valid(instance);
        assert_eq!(
            in_left || in_right,
            united_validator.is_valid(instance),
            "union disagrees on {instance}\n  left = {left}\n  right = {right}\n  union = {}",
            united.to_json_schema()
        );
        assert_eq!(
            in_left && !in_right,
            difference_validator.is_valid(instance),
            "difference disagrees on {instance}\n  left = {left}\n  right = {right}\n  difference = {}",
            difference.to_json_schema()
        );
    }
}

#[test]
fn union_is_commutative_and_idempotent() {
    let schemas = [
        json!({"type": "string", "minLength": 2}),
        json!({"anyOf": [{"type": "string"}, {"type": "integer"}]}),
        json!({"const": "a"}),
        json!({"type": "object", "properties": {"id": {"type": "integer"}}}),
        json!({"$ref": "#/$defs/x", "$defs": {"x": {"type": "string"}}}),
        // A second pointer at the same value set: arrival order must not decide which stands in.
        json!({"$ref": "#/$defs/y", "$defs": {"y": {"type": "string"}}}),
    ]
    .map(|schema| canonicalize(&schema).expect("canonicalizes"));
    for left in &schemas {
        assert_eq!(left.union(left).expect("unions"), *left);
        for right in &schemas {
            assert_eq!(
                left.union(right).expect("unions"),
                right.union(left).expect("unions")
            );
        }
    }
}

// Nothing is left over once a schema is taken away from itself, and taking away nothing changes it.
#[test_case(&json!({"type": "string", "minLength": 2}); "a string window")]
#[test_case(&json!({"anyOf": [{"type": "string"}, {"type": "integer"}]}); "a union of types")]
#[test_case(&json!({"type": "object", "properties": {"id": {"type": "integer"}}}); "an object")]
fn subtract_is_empty_against_itself(schema: &Value) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    let empty = canonicalize(&json!(false)).expect("canonicalizes");
    assert_eq!(
        canonical
            .subtract(&canonical)
            .expect("subtracts")
            .satisfiability(),
        Satisfiability::No
    );
    assert_eq!(canonical.subtract(&empty).expect("subtracts"), canonical);
}

// Both operations run the same operand frame as `intersect`, and reject what it rejects.
#[test]
fn union_and_difference_reject_uncombinable_operands() {
    let raw = canonicalize(&unsupported()).expect("canonicalizes");
    let modeled = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    assert!(matches!(
        modeled.union(&raw),
        Err(CanonicalizationError::UnsupportedOperand)
    ));
    assert!(matches!(
        modeled.subtract(&raw),
        Err(CanonicalizationError::UnsupportedOperand)
    ));

    let draft7 = options()
        .with_draft(Draft::Draft7)
        .canonicalize(&json!({"type": "string"}))
        .expect("canonicalizes");
    assert!(matches!(
        modeled.union(&draft7),
        Err(CanonicalizationError::IncompatibleOperands(
            OperandMismatch::Drafts { .. }
        ))
    ));
    assert!(matches!(
        modeled.subtract(&draft7),
        Err(CanonicalizationError::IncompatibleOperands(
            OperandMismatch::Drafts { .. }
        ))
    ));
}

// Pruning leaves the result naming fewer targets than its source, and the two still resolve every
// shared pointer the same way, so they stay combinable.
#[test]
fn a_pruned_result_still_combines_with_its_source() {
    let source = canonicalize(&json!({
        "$defs": {"A": {"type": "string"}, "B": {"type": "integer"}},
        "type": "object",
        "properties": {"a": {"$ref": "#/$defs/A"}, "b": {"$ref": "#/$defs/B"}}
    }))
    .expect("canonicalizes");
    let narrowing = canonicalize(&json!({"type": "object", "properties": {"b": false}}))
        .expect("canonicalizes");
    let narrowed = source.intersect(&narrowing).expect("intersects");
    assert_eq!(narrowed.definitions().count(), 1);
    assert_eq!(
        narrowed.intersect(&source).expect("intersects"),
        narrowed.intersect(&narrowed).expect("intersects")
    );
}

// The result reads both maps, so a target only one operand named still resolves in it.
#[test_case(false; "the root reader on the left")]
#[test_case(true; "the root reader on the right")]
fn a_combination_keeps_the_targets_both_operands_named(swap: bool) {
    let rooted = canonicalize(&json!({"type": "object", "properties": {"a": {"$ref": "#"}}}))
        .expect("canonicalizes");
    let referencing = canonicalize(&json!({
        "type": "object",
        "properties": {"b": {"$ref": "#/$defs/Y"}},
        "$defs": {"Y": {"type": "string"}}
    }))
    .expect("canonicalizes");
    let merged = if swap {
        referencing.intersect(&rooted)
    } else {
        rooted.intersect(&referencing)
    }
    .expect("intersects");
    assert!(merged.definition("#/$defs/Y").is_some());
    let validator = validator_for(&merged.to_json_schema()).expect("builds");
    assert!(validator.is_valid(&json!({"b": "x"})));
    assert!(!validator.is_valid(&json!({"b": 1})));
}

// The root a result keeps reads targets of its own, which stay with it even where the result no
// longer names them itself.
#[test]
fn a_combination_keeps_the_targets_its_root_names() {
    let rooted = canonicalize(&json!({
        "$defs": {"Z": {"type": "integer"}},
        "type": "object",
        "properties": {"a": {"$ref": "#"}, "z": {"$ref": "#/$defs/Z"}}
    }))
    .expect("canonicalizes");
    let narrowing = canonicalize(&json!({"type": "object", "properties": {"z": false}}))
        .expect("canonicalizes");
    let merged = rooted.intersect(&narrowing).expect("intersects");
    let validator = validator_for(&merged.to_json_schema()).expect("builds");
    assert!(validator.is_valid(&json!({"a": {"z": 1}})));
    assert!(!validator.is_valid(&json!({"z": 1})));
}

// A `#` inside a definition the result keeps names the document that definition was written in,
// which the combination is not.
#[test]
fn a_combination_keeps_the_root_a_kept_definition_names() {
    let referencing = canonicalize(&json!({
        "type": "object",
        "properties": {"child": {"$ref": "#/$defs/node"}},
        "$defs": {"node": {"type": "object", "properties": {"parent": {"$ref": "#"}}}}
    }))
    .expect("canonicalizes");
    let required =
        canonicalize(&json!({"type": "object", "required": ["z"]})).expect("canonicalizes");
    let merged = referencing.intersect(&required).expect("intersects");
    let validator = validator_for(&merged.to_json_schema()).expect("builds");
    let referencing_validator = validator_for(&referencing.to_json_schema()).expect("builds");
    let instance = json!({"z": 1, "child": {"parent": {}}});
    assert!(referencing_validator.is_valid(&instance));
    assert!(
        validator.is_valid(&instance),
        "the kept definition's `#` was rebound\n  merged = {}",
        merged.to_json_schema()
    );
}

// `#` names the document a node was read against, so two nodes written the same way in different
// documents are not the same set.
#[test]
fn operands_naming_different_roots_do_not_cancel() {
    let outer = canonicalize(&json!({
        "type": "object",
        "properties": {"a": {"type": "object", "properties": {"self": {"$ref": "#"}}}}
    }))
    .expect("canonicalizes");
    let CanonicalView::Object(view) = outer.view() else {
        panic!("expected an Object view");
    };
    let nested = view.properties.get("a").expect("property a").clone();
    let standalone = canonicalize(&json!({
        "type": "object",
        "properties": {"self": {"$ref": "#"}}
    }))
    .expect("canonicalizes");
    assert!(matches!(
        nested.subtract(&standalone),
        Err(CanonicalizationError::IncompatibleOperands(
            OperandMismatch::DocumentRoots
        ))
    ));
    assert!(matches!(
        nested.covers(&standalone),
        Err(CanonicalizationError::IncompatibleOperands(
            OperandMismatch::DocumentRoots
        ))
    ));
}

// The pattern engine decides what a schema accepts, so two read under different engines are two
// schemas - the same answer the operations give when they refuse to combine them.
#[test]
fn schemas_read_under_different_pattern_engines_are_not_equal() {
    let schema = json!({"type": "string", "pattern": "^a"});
    let fancy = options()
        .with_pattern_options(PatternOptions::fancy_regex())
        .canonicalize(&schema)
        .expect("canonicalizes");
    let standard = options()
        .with_pattern_options(PatternOptions::regex())
        .canonicalize(&schema)
        .expect("canonicalizes");
    assert_ne!(fancy, standard);
    assert!(matches!(
        fancy.intersect(&standard),
        Err(CanonicalizationError::IncompatibleOperands(
            OperandMismatch::PatternEngine
        ))
    ));
}

// A node naming `#` reads whatever the root reads, so two of them written the same way in
// documents whose roots read different targets are different schemas.
#[test]
fn nodes_naming_roots_that_read_different_targets_are_not_equal() {
    let document = |target: Value| {
        canonicalize(&json!({
            "$defs": {"Y": target},
            "type": "object",
            "properties": {"self": {"$ref": "#"}, "y": {"$ref": "#/$defs/Y"}}
        }))
        .expect("canonicalizes")
    };
    let child = |root: &CanonicalSchema| {
        let CanonicalView::Object(view) = root.view() else {
            panic!("expected an Object view");
        };
        view.properties.get("self").expect("property self").clone()
    };
    let strings = document(json!({"type": "string"}));
    let integers = document(json!({"type": "integer"}));
    assert_ne!(child(&strings), child(&integers));
    let mut seen = HashSet::new();
    seen.insert(child(&strings));
    seen.insert(child(&integers));
    assert_eq!(seen.len(), 2);
}

// A result naming no root is the same schema whichever document it was combined in.
#[test]
fn a_result_naming_no_root_ignores_the_document_it_came_from() {
    let root = canonicalize(&json!({
        "type": "object",
        "properties": {"a": {"type": "string"}, "b": {"type": "string", "maxLength": 4}}
    }))
    .expect("canonicalizes");
    let CanonicalView::Object(view) = root.view() else {
        panic!("expected an Object view");
    };
    let narrow = view.properties.get("b").expect("property b").clone();
    let wide = view.properties.get("a").expect("property a").clone();
    let difference = narrow.subtract(&wide).expect("subtracts");
    assert_eq!(
        difference,
        canonicalize(&json!(false)).expect("canonicalizes")
    );
}

// A complement leaves `#` naming the document it was written in, never the complement itself.
#[test]
fn a_complement_keeps_the_root_its_pointer_names() {
    let recursive = canonicalize(&json!({
        "type": "object",
        "properties": {"a": {"$ref": "#"}}
    }))
    .expect("canonicalizes");
    // A complement reaching `#` would name the wrong document once it took the root's place, so it
    // is declined rather than spelled - which is the behaviour this pins.
    assert!(matches!(
        recursive.negate(),
        Err(CanonicalizationError::UnsupportedResult)
    ));

    // The pointer still resolves through every operation that does answer, and the result keeps the
    // root it names rather than repointing `#` at itself.
    let objects = canonicalize(&json!({"type": "object"})).expect("canonicalizes");
    let met = recursive.intersect(&objects).expect("intersects");
    let complement_validator = validator_for(&met.to_json_schema()).expect("builds");
    let recursive_validator = validator_for(&recursive.to_json_schema()).expect("builds");
    for instance in [json!({"a": {}}), json!({"a": 1}), json!("x"), json!({})] {
        assert_eq!(
            recursive_validator.is_valid(&instance),
            complement_validator.is_valid(&instance),
            "the result disagrees on {instance}\n  result = {}",
            met.to_json_schema()
        );
    }
}

// Two children of one root keep that root's map, so the references in the result still resolve -
// checked through the emitted document, where a pointer the map lost would fail to compile.
#[test]
fn union_and_difference_keep_references_resolvable_within_one_document() {
    let root = canonicalize(&json!({
        "type": "object",
        "$defs": {"A": {"type": "string"}, "B": {"minLength": 2}},
        "properties": {"a": {"$ref": "#/$defs/A"}, "b": {"$ref": "#/$defs/B"}}
    }))
    .expect("canonicalizes");
    let CanonicalView::Object(view) = root.view() else {
        panic!("expected an Object view");
    };
    let left = view.properties.get("a").expect("property a").clone();
    let right = view.properties.get("b").expect("property b").clone();
    let left_validator = validator_for(&left.to_json_schema()).expect("builds");
    let right_validator = validator_for(&right.to_json_schema()).expect("builds");
    for (combined, admits) in [
        (
            left.union(&right).expect("unions"),
            (|left: bool, right: bool| left || right) as fn(bool, bool) -> bool,
        ),
        (
            left.subtract(&right).expect("subtracts"),
            (|left, right| left && !right) as fn(bool, bool) -> bool,
        ),
    ] {
        let emitted = combined.to_json_schema();
        let validator = validator_for(&emitted).expect("the emitted references resolve");
        for instance in [json!("a"), json!("abc"), json!(1), json!(null)] {
            assert_eq!(
                admits(
                    left_validator.is_valid(&instance),
                    right_validator.is_valid(&instance)
                ),
                validator.is_valid(&instance),
                "disagrees on {instance}\n  combined = {emitted}"
            );
        }
    }
}

// `#` names the document a node was read against. A combination of nodes from two documents becomes
// a root of its own, which is neither, so a result still naming a root would rebind the pointer to
// itself and admit values neither operand does. Either it declines, or the pointer is gone.
#[test_case(&json!({"type": "object", "properties": {"a": {"$ref": "#"}}}), &json!({"type": "string"}); "a root self-reference")]
#[test_case(&json!({"type": "object", "properties": {"a": {"$ref": "#"}}}), &json!({"type": "object", "minProperties": 1}); "a root self-reference beside an object")]
#[test_case(&json!({"type": "object", "properties": {"a": {"$ref": "#"}}}), &json!({"type": "object", "required": ["a"]}); "a root self-reference beside a requirement")]
fn combining_across_documents_never_rebinds_a_root_reference(left: &Value, right: &Value) {
    // One operation: its name, what it returned, and the verdict it owes on an instance.
    struct Combination {
        name: &'static str,
        result: Result<CanonicalSchema, CanonicalizationError>,
        admits: fn(bool, bool) -> bool,
    }
    let instances = [
        json!({"a": "x"}),
        json!({"a": {"a": "x"}}),
        json!({"a": {}}),
        json!({"next": {"a": "x"}}),
        json!({}),
        json!("x"),
    ];
    let left_validator = validator_for(left).expect("builds");
    let right_validator = validator_for(right).expect("builds");
    let canonical_left = canonicalize(left).expect("canonicalizes");
    let canonical_right = canonicalize(right).expect("canonicalizes");
    let combinations = [
        Combination {
            name: "union",
            result: canonical_left.union(&canonical_right),
            admits: |left, right| left || right,
        },
        Combination {
            name: "intersect",
            result: canonical_left.intersect(&canonical_right),
            admits: |left, right| left && right,
        },
        Combination {
            name: "subtract",
            result: canonical_left.subtract(&canonical_right),
            admits: |left, right| left && !right,
        },
    ];
    for Combination {
        name,
        result,
        admits,
    } in combinations
    {
        let combined = match result {
            Ok(combined) => combined,
            // Declining is the other sound answer.
            Err(error) => {
                assert!(matches!(error, CanonicalizationError::UnsupportedResult));
                continue;
            }
        };
        let emitted = combined.to_json_schema();
        let validator = validator_for(&emitted).expect("builds");
        for instance in &instances {
            assert_eq!(
                admits(
                    left_validator.is_valid(instance),
                    right_validator.is_valid(instance)
                ),
                validator.is_valid(instance),
                "{name} disagrees on {instance}\n  left = {left}\n  right = {right}\n  combined = {emitted}"
            );
        }
    }
}

// Canonicalizing one document twice builds two maps that resolve every pointer the same way, so the
// handles stay combinable - `==` already reports them equal.
#[test]
fn operands_from_one_document_canonicalized_twice_combine() {
    let source = json!({
        "$defs": {"Id": {"type": "string"}},
        "type": "object",
        "properties": {"id": {"$ref": "#/$defs/Id"}}
    });
    let left = canonicalize(&source).expect("canonicalizes");
    let right = canonicalize(&source).expect("canonicalizes");
    assert_eq!(left, right);
    assert_eq!(left.intersect(&right).expect("intersects"), left);
    assert_eq!(left.union(&right).expect("unions"), left);
    assert_eq!(
        left.subtract(&right).expect("subtracts").satisfiability(),
        Satisfiability::No
    );
    assert_eq!(left.covers(&right).expect("compares"), Containment::Yes);
}

// A degenerate difference is one of the operands or empty, so it needs no complement - and these
// are schemas whose complement is not modeled, where asking for one would decline.
#[test_case(&json!({"type": "array", "contains": {"type": "string"}, "minContains": 2}); "a counted contains")]
#[test_case(&json!({"type": "object", "patternProperties": {"^a": {"type": "string"}}}); "pattern properties")]
#[test_case(&json!({"type": "object", "properties": {"a": {"$ref": "#"}}}); "root self-reference")]
fn subtracting_a_schema_from_itself_needs_no_complement(schema: &Value) {
    let left = canonicalize(schema).expect("canonicalizes");
    let right = canonicalize(schema).expect("canonicalizes");
    assert!(
        left.negate().is_err(),
        "the complement must be the part that declines"
    );
    assert_eq!(
        left.subtract(&right).expect("subtracts").satisfiability(),
        Satisfiability::No
    );
}

// What `subtract` declines is not what `negate` declines: it skips the degenerate cases, so a
// caller cannot predict one from the other.
#[test_case(&json!({"type": "object"}), &json!({"type": "object", "patternProperties": {"^a": {"type": "string"}}}); "a pattern map complement")]
fn subtract_declines_an_unsupported_complement(left: &Value, right: &Value) {
    let left = canonicalize(left).expect("canonicalizes");
    let right = canonicalize(right).expect("canonicalizes");
    assert!(matches!(
        left.subtract(&right),
        Err(CanonicalizationError::UnsupportedResult)
    ));
}

#[test]
fn subtracting_the_trivial_bounds_needs_no_complement() {
    let hard =
        canonicalize(&json!({"type": "object", "patternProperties": {"^a": {"type": "string"}}}))
            .expect("canonicalizes");
    let empty = canonicalize(&json!(false)).expect("canonicalizes");
    let everything = canonicalize(&json!(true)).expect("canonicalizes");
    let string = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    assert_eq!(
        empty.subtract(&hard).expect("subtracts").satisfiability(),
        Satisfiability::No
    );
    assert_eq!(
        string
            .subtract(&everything)
            .expect("subtracts")
            .satisfiability(),
        Satisfiability::No
    );
    assert_eq!(string.subtract(&empty).expect("subtracts"), string);
}

// A combination can narrow a target out of the result. Carrying the dead entry would emit an
// unreferenced definition and leave the result uncombinable with a third document.
#[test]
fn combining_prunes_definitions_the_result_no_longer_names() {
    let plain = canonicalize(&json!({"type": "object"})).expect("canonicalizes");
    let referencing = canonicalize(&json!({
        "$defs": {"s": {"type": "string"}},
        "type": "object",
        "properties": {"a": {"$ref": "#/$defs/s"}}
    }))
    .expect("canonicalizes");
    let third =
        canonicalize(&json!({"type": "object", "maxProperties": 9})).expect("canonicalizes");
    for combined in [
        plain.subtract(&referencing).expect("subtracts"),
        plain.union(&referencing).expect("unions"),
    ] {
        let emitted = combined.to_json_schema();
        assert!(
            emitted.get("$defs").is_none(),
            "the complement inlined the target, so its entry is dead: {emitted}"
        );
        assert!(
            combined.intersect(&third).is_ok(),
            "chaining must stay open"
        );
    }
}

// A union keeps both branches unless one is proven redundant, and a check that had to approximate
// proves nothing - so the union stays exact even where the intersection of the same operands is not
// modeled.
#[test]
fn a_union_survives_an_unsupported_intersection() {
    let left = canonicalize(&json!({
        "type": "object",
        "properties": {"ab": {"minLength": 1}},
        "additionalProperties": {"type": "string"}
    }))
    .expect("canonicalizes");
    let right = canonicalize(&json!({
        "type": "object",
        "patternProperties": {"^a": {"maxLength": 9}}
    }))
    .expect("canonicalizes");
    assert!(matches!(
        left.intersect(&right),
        Err(CanonicalizationError::UnsupportedResult)
    ));
    let united = left.union(&right).expect("`anyOf` expresses the union");
    let validator = validator_for(&united.to_json_schema()).expect("builds");
    let left_validator = validator_for(&left.to_json_schema()).expect("builds");
    let right_validator = validator_for(&right.to_json_schema()).expect("builds");
    for instance in [
        json!({"ab": "x"}),
        json!({"a": "y"}),
        json!({"b": 1}),
        json!("x"),
    ] {
        assert_eq!(
            left_validator.is_valid(&instance) || right_validator.is_valid(&instance),
            validator.is_valid(&instance),
            "the union disagrees on {instance}"
        );
    }
}

// The bindings expose these labels verbatim.
#[test]
fn three_valued_answers_carry_stable_labels() {
    assert_eq!(
        [
            Containment::Yes.as_str(),
            Containment::No.as_str(),
            Containment::Unknown.as_str()
        ],
        ["yes", "no", "unknown"]
    );
    assert_eq!(
        [
            Satisfiability::Yes.as_str(),
            Satisfiability::No.as_str(),
            Satisfiability::Unknown.as_str()
        ],
        ["yes", "no", "unknown"]
    );
}

// Every mismatch says which part of a schema's identity the operands disagreed on.
#[test_case(
    OperandMismatch::FormatAssertions,
    "operands disagree on whether `format` asserts"
)]
#[test_case(
    OperandMismatch::PatternEngine,
    "operands canonicalized with different pattern engines"
)]
#[test_case(
    OperandMismatch::Definitions,
    "operands resolve one external resource to different schemas"
)]
#[test_case(
    OperandMismatch::DocumentRoots,
    "operands read `#` in different documents"
)]
fn an_operand_mismatch_says_what_disagreed(mismatch: OperandMismatch, expected: &str) {
    assert_eq!(mismatch.to_string(), expected);
}

#[test_case(&json!(true), Satisfiability::Yes; "everything names its own members")]
#[test_case(&json!({"const": 5}), Satisfiability::Yes; "a constant carries the value that proves it")]
#[test_case(&json!({"enum": [1, 2]}), Satisfiability::Yes; "a value set names its members")]
#[test_case(&json!(false), Satisfiability::No; "nothing admits no value")]
#[test_case(&json!({"type": "string"}), Satisfiability::Yes; "a type has values of its own")]
#[test_case(&json!({"type": ["integer", "object"]}), Satisfiability::Yes; "a type list has values of its own")]
#[test_case(
    &json!({"$schema": "http://json-schema.org/draft-04/schema#", "type": "integer", "enum": [1, 2]}),
    Satisfiability::Yes;
    "a typed group holds a value of its type"
)]
#[test_case(&json!({"type": "integer", "minimum": 0}), Satisfiability::Yes; "an integer window holds integers")]
#[test_case(&json!({"type": "number", "exclusiveMaximum": 5}), Satisfiability::Yes; "a number interval holds numbers")]
#[test_case(&json!({"type": "string", "minLength": 2}), Satisfiability::Yes; "a length window holds strings")]
#[test_case(&json!({"type": "array", "items": {"type": "string"}}), Satisfiability::Yes; "an array window holds the empty array")]
#[test_case(&json!({"type": "object", "properties": {"a": false}}), Satisfiability::Yes; "an object window holds the empty object")]
#[test_case(&json!({"type": "string", "pattern": "^a"}), Satisfiability::Yes; "a value is built from a pattern anchored at the start")]
#[test_case(&json!({"type": "string", "pattern": "a.b"}), Satisfiability::Yes; "a value is built from a pattern with a wildcard")]
#[test_case(&json!({"type": "string", "pattern": "a{100}"}), Satisfiability::Unknown; "a pattern with more repetitions than are written out is left undecided")]
#[test_case(&json!({"type": "integer", "multipleOf": 3}), Satisfiability::Yes; "zero is a multiple of every divisor")]
#[test_case(&json!({"type": "object", "required": ["a"]}), Satisfiability::Yes; "an object carrying the key it requires")]
#[test_case(&json!({"type": "array", "minItems": 2, "items": {"type": "integer"}}), Satisfiability::Yes; "an array as long as it must be")]
#[test_case(
    &json!({"type": "object", "required": ["a"], "properties": {"a": {"type": ["boolean", "array"]}}}),
    Satisfiability::Yes;
    "a required key over a type list"
)]
#[test_case(
    &json!({"type": "integer", "multipleOf": 2, "minimum": 3, "maximum": 3}),
    Satisfiability::No;
    "a window narrower than its own step"
)]
fn satisfiability_answers(schema: &Value, expected: Satisfiability) {
    assert_eq!(
        canonicalize(schema)
            .expect("canonicalizes")
            .satisfiability(),
        expected
    );
}

// A shield governs every key the other side's patterns match, so the meet has to reach into those
// pattern entries; leaving them alone admits values the conjunction rejects.
#[test_case(
    &json!({"patternProperties": {"^a": {"type": "integer"}}, "additionalProperties": false}),
    &json!({"additionalProperties": {"type": "string"}});
    "closed pattern map meets a shield"
)]
#[test_case(
    &json!({"additionalProperties": {"type": "string"}}),
    &json!({"patternProperties": {"^a": {"type": "integer"}}, "additionalProperties": false});
    "shield meets a closed pattern map"
)]
#[test_case(
    &json!({"patternProperties": {"^a": {"type": "integer"}}}),
    &json!({"additionalProperties": {"type": "string"}});
    "open pattern map meets a shield"
)]
#[test_case(
    &json!({"additionalProperties": {"type": "string"}}),
    &json!({"patternProperties": {"^a": {"type": "integer"}}});
    "shield meets an open pattern map"
)]
#[test_case(
    &json!({"patternProperties": {"^a": {"type": "integer"}}, "additionalProperties": false}),
    &json!({"properties": {"a1": {"type": "string"}}, "additionalProperties": {"type": "string"}});
    "closed pattern map meets a shield naming a matched key"
)]
#[test_case(
    &json!({"patternProperties": {"^a": {"type": "integer"}}, "additionalProperties": false}),
    &json!({"patternProperties": {"^a": {"minimum": 3}}, "additionalProperties": false});
    "two closed pattern maps"
)]
fn intersect_object_shield_and_patterns_keeps_validation_parity(left: &Value, right: &Value) {
    let merged = canonicalize(left)
        .expect("canonicalizes")
        .intersect(&canonicalize(right).expect("canonicalizes"))
        .expect("intersects")
        .to_json_schema();
    let document = canonicalize(&json!({"allOf": [left, right]}))
        .expect("canonicalizes")
        .to_json_schema();

    let left_validator = validator_for(left).expect("compiles");
    let right_validator = validator_for(right).expect("compiles");
    let merged_validator = validator_for(&merged).expect("compiles");
    let document_validator = validator_for(&document).expect("compiles");
    for instance in [
        json!({"a1": 5}),
        json!({"a1": "s"}),
        json!({"b": "s"}),
        json!({"b": 5}),
        json!({}),
        json!(1),
    ] {
        let conjunction = left_validator.is_valid(&instance) && right_validator.is_valid(&instance);
        assert_eq!(
            merged_validator.is_valid(&instance),
            conjunction,
            "{instance}"
        );
        assert_eq!(
            document_validator.is_valid(&instance),
            conjunction,
            "{instance}"
        );
    }
}

#[test]
fn intersect_object_shield_meets_the_pattern_entries_it_governs() {
    let patterns = canonicalize(&json!({"patternProperties": {"^a": {"type": "integer"}}}))
        .expect("canonicalizes");
    let shield =
        canonicalize(&json!({"additionalProperties": {"type": "string"}})).expect("canonicalizes");

    assert_eq!(
        patterns
            .intersect(&shield)
            .expect("intersects")
            .to_json_schema(),
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "anyOf": [
                {"type": ["null", "boolean", "number", "string", "array"]},
                {
                    "type": "object",
                    "patternProperties": {"^a": false},
                    "additionalProperties": {"type": "string"}
                }
            ]
        })
    );
}

// A key only one pattern map matches answers to the other map's shield, and a key both match
// answers to neither, so placing the shields needs to know which keys the two patterns share.
#[test]
fn intersect_declines_pattern_maps_on_both_sides_of_a_shield() {
    let shield =
        canonicalize(&json!({"additionalProperties": {"type": "string"}})).expect("canonicalizes");
    let left =
        canonicalize(&json!({"patternProperties": {"^a": {"type": "string", "minLength": 2}}}))
            .expect("canonicalizes")
            .intersect(&shield)
            .expect("intersects");
    let right =
        canonicalize(&json!({"patternProperties": {"^b": {"type": "string", "maxLength": 5}}}))
            .expect("canonicalizes")
            .intersect(&shield)
            .expect("intersects");

    assert!(matches!(
        left.intersect(&right),
        Err(CanonicalizationError::UnsupportedResult)
    ));
}

// `a1` is outside the shield that names it, so meeting that shield into the `^a` entry would
// demand of `a1` something neither side does.
#[test]
fn intersect_declines_a_shield_naming_a_key_its_own_entry_leaves_the_pattern() {
    let patterns = canonicalize(&json!({"patternProperties": {"^a": {"type": "integer"}}}))
        .expect("canonicalizes");
    let shield = canonicalize(
        &json!({"properties": {"a1": {"type": "number"}}, "additionalProperties": {"type": "string"}}),
    )
    .expect("canonicalizes");

    assert!(matches!(
        patterns.intersect(&shield),
        Err(CanonicalizationError::UnsupportedResult)
    ));
}

// Containment is proved by intersecting the two sides, so an intersection `intersect` itself
// declines cannot carry a verdict either: it may be wider than the real one.
#[test]
fn covers_declines_what_only_an_unsupported_intersection_would_prove() {
    let shield =
        canonicalize(&json!({"additionalProperties": {"type": "string"}})).expect("canonicalizes");
    let wide = canonicalize(
        &json!({"patternProperties": {"^a": {"type": "string"}, "^b": {"type": "string"}}}),
    )
    .expect("canonicalizes")
    .intersect(&shield)
    .expect("intersects");
    let narrow = canonicalize(&json!({"patternProperties": {"^a": {"type": "string"}}}))
        .expect("canonicalizes")
        .intersect(&shield)
        .expect("intersects");

    assert!(matches!(
        wide.intersect(&narrow),
        Err(CanonicalizationError::UnsupportedResult)
    ));
    assert_eq!(
        wide.covers(&narrow).expect("compares"),
        Containment::Unknown
    );
}

// Matching pattern maps leave no key one side matches and the other does not, so neither shield
// can reach past the entries and the meet needs no pattern-overlap reasoning.
#[test]
fn intersect_meets_pattern_maps_naming_the_same_patterns_beside_shields() {
    let shield =
        canonicalize(&json!({"additionalProperties": {"minLength": 2}})).expect("canonicalizes");
    let left = canonicalize(&json!({"patternProperties": {"^a": {"type": "string"}}}))
        .expect("canonicalizes")
        .intersect(&shield)
        .expect("intersects");
    let right = canonicalize(&json!({"patternProperties": {"^a": {"maxLength": 4}}}))
        .expect("canonicalizes")
        .intersect(&shield)
        .expect("intersects");
    let merged = left.intersect(&right).expect("intersects").to_json_schema();

    let left_validator = validator_for(&left.to_json_schema()).expect("compiles");
    let right_validator = validator_for(&right.to_json_schema()).expect("compiles");
    let merged_validator = validator_for(&merged).expect("compiles");
    for instance in [
        json!({"a1": "abc"}),
        json!({"a1": "abcde"}),
        json!({"a1": "a"}),
        json!({"a1": 5}),
        json!({"b": "abc"}),
        json!({"b": "a"}),
        json!({}),
    ] {
        assert_eq!(
            merged_validator.is_valid(&instance),
            left_validator.is_valid(&instance) && right_validator.is_valid(&instance),
            "{instance}"
        );
    }
}

fn draft4(body: &Value) -> Value {
    let mut map = body.as_object().expect("object").clone();
    map.insert(
        "$schema".into(),
        json!("http://json-schema.org/draft-04/schema#"),
    );
    Value::Object(map)
}

fn draft4_closed_pattern_map(pattern: &str) -> Value {
    draft4(&json!({
        "patternProperties": {pattern: {"type": "integer"}},
        "additionalProperties": false
    }))
}

// Draft 4 has no `propertyNames`, so meeting two closed pattern maps must keep a spelling a Draft 4
// validator reads - emitting one it ignores would admit every key the meet forbids.
#[test_case(&json!({"c": 1}); "key outside both patterns")]
#[test_case(&json!({"a1": 5}); "key inside one pattern only")]
#[test_case(&json!({}); "no key at all")]
fn intersect_draft4_closed_pattern_maps_keeps_validation_parity(instance: &Value) {
    let left = draft4_closed_pattern_map("^a");
    let right = draft4_closed_pattern_map("^b");
    let merged = canonicalize(&left)
        .expect("canonicalizes")
        .intersect(&canonicalize(&right).expect("canonicalizes"))
        .expect("intersects")
        .to_json_schema();

    let both = validator_for(&left).expect("compiles").is_valid(instance)
        && validator_for(&right).expect("compiles").is_valid(instance);
    assert_eq!(
        validator_for(&merged).expect("compiles").is_valid(instance),
        both
    );
}

#[test]
fn intersect_draft4_closed_pattern_maps_emits_a_closed_map_per_pattern() {
    let merged = canonicalize(&draft4_closed_pattern_map("^a"))
        .expect("canonicalizes")
        .intersect(&canonicalize(&draft4_closed_pattern_map("^b")).expect("canonicalizes"))
        .expect("intersects");
    assert_eq!(
        merged.to_json_schema(),
        json!({
            "$schema": "http://json-schema.org/draft-04/schema#",
            "anyOf": [
                {"type": ["null", "boolean", "number", "string", "array"]},
                {
                    "type": "object",
                    "patternProperties": {"^a": {"type": "integer"}, "^b": {"type": "integer"}},
                    "allOf": [
                        {"patternProperties": {"^a": {}}, "additionalProperties": false},
                        {"patternProperties": {"^b": {}}, "additionalProperties": false}
                    ]
                }
            ]
        })
    );
}

fn draft4_closed_key_pattern(pattern: &str) -> Value {
    draft4(&json!({
        "patternProperties": {pattern: {}},
        "additionalProperties": false
    }))
}

// The meet's key constraint takes a closed map per pattern to spell, so the demand for a key
// breaking it carries them all - the alternative is a `propertyNames` a Draft 4 validator ignores,
// leaving a demand no object can break.
#[test]
fn negate_intersected_draft4_closed_pattern_maps_bars_every_closed_map() {
    let complement = canonicalize(&draft4_closed_key_pattern("^a"))
        .expect("canonicalizes")
        .intersect(&canonicalize(&draft4_closed_key_pattern("b$")).expect("canonicalizes"))
        .expect("intersects")
        .negate()
        .expect("negates");
    assert_eq!(
        complement.to_json_schema(),
        json!({
            "$schema": "http://json-schema.org/draft-04/schema#",
            "type": "object",
            "not": {"allOf": [
                {"patternProperties": {"^a": {}}, "additionalProperties": false},
                {"patternProperties": {"b$": {}}, "additionalProperties": false}
            ]}
        })
    );
}

#[test_case(&json!({"ab": 1}); "key inside both patterns")]
#[test_case(&json!({"a1": 1}); "key inside one pattern only")]
#[test_case(&json!({"c": 1}); "key outside both patterns")]
#[test_case(&json!({}); "no key at all")]
#[test_case(&json!(5); "not an object")]
fn negate_intersected_draft4_closed_pattern_maps_keeps_validation_parity(instance: &Value) {
    let left = draft4_closed_key_pattern("^a");
    let right = draft4_closed_key_pattern("b$");
    let complement = canonicalize(&left)
        .expect("canonicalizes")
        .intersect(&canonicalize(&right).expect("canonicalizes"))
        .expect("intersects")
        .negate()
        .expect("negates")
        .to_json_schema();

    let both = validator_for(&left).expect("compiles").is_valid(instance)
        && validator_for(&right).expect("compiles").is_valid(instance);
    assert_eq!(
        validator_for(&complement)
            .expect("compiles")
            .is_valid(instance),
        !both
    );
}

// Meeting two documents reshapes a key constraint past the closed map either was parsed from.
#[test_case(
    &json!({"patternProperties": {"^a": {"type": "integer"}}, "additionalProperties": false}),
    &json!({"patternProperties": {"^c": {"type": "string"}}});
    "pattern entry outside the closed map"
)]
#[test_case(
    &json!({"patternProperties": {"^a": {"type": "integer"}}, "additionalProperties": false}),
    &json!({"properties": {"a1": {"type": "string"}}});
    "named entry a pattern admits"
)]
#[test_case(
    &json!({"properties": {"x": {"type": "integer"}}, "patternProperties": {"^a": {"type": "integer"}}, "additionalProperties": false}),
    &json!({"properties": {"x": {"type": "integer"}}, "patternProperties": {"^b": {"type": "integer"}}, "additionalProperties": false});
    "shared key beside disjoint patterns"
)]
#[test_case(
    &json!({"patternProperties": {"^a": {"type": "integer"}}, "additionalProperties": false}),
    &json!({"additionalProperties": {"type": "string"}});
    "closed pattern map meets a shield"
)]
fn intersect_draft4_object_leaves_keeps_validation_parity(left: &Value, right: &Value) {
    let left = draft4(left);
    let right = draft4(right);
    let merged = canonicalize(&left)
        .expect("canonicalizes")
        .intersect(&canonicalize(&right).expect("canonicalizes"))
        .expect("intersects")
        .to_json_schema();

    let left_validator = validator_for(&left).expect("compiles");
    let right_validator = validator_for(&right).expect("compiles");
    let merged_validator = validator_for(&merged).expect("compiles");
    for instance in [
        json!({}),
        json!({"a1": 5}),
        json!({"a1": "s"}),
        json!({"b1": 5}),
        json!({"c": 1}),
        json!({"x": 1}),
    ] {
        assert_eq!(
            merged_validator.is_valid(&instance),
            left_validator.is_valid(&instance) && right_validator.is_valid(&instance),
            "{instance}"
        );
    }
}

#[test_case(&json!({"type": "integer"}), &json!({"type": "integer"}), Containment::Yes; "identical forms")]
#[test_case(&json!({"type": "integer"}), &json!({"const": 1}), Containment::Yes; "a type over a constant")]
#[test_case(&json!({"type": "integer"}), &json!({"enum": [1, 2]}), Containment::Yes; "a type over an enum")]
#[test_case(&json!({"type": "integer"}), &json!({"type": "integer", "minimum": 5}), Containment::Yes; "unbounded over bounded")]
#[test_case(&json!({"type": "integer"}), &json!({"const": "x"}), Containment::No; "a constant outside the type refutes")]
#[test_case(&json!({"type": "integer"}), &json!({"enum": [1, "x"]}), Containment::No; "an enum member outside the type refutes")]
#[test_case(&json!({"type": "integer"}), &json!({"type": "string"}), Containment::No; "a disjoint type leaves values behind")]
#[test_case(&json!({"type": "integer", "minimum": 5}), &json!({"type": "integer"}), Containment::No; "the values below the bound refute it")]
#[test_case(&json!({"type": "integer", "minimum": 0}), &json!({"type": "integer", "maximum": 100}), Containment::No; "the values outside the window refute it")]
// A string the pattern turns away is a value the argument admits and the receiver rejects.
#[test_case(&json!({"type": "string", "pattern": "^a"}), &json!({"type": "string"}), Containment::No; "a pattern the argument does not carry refutes")]
fn covers_decides(outer: &Value, inner: &Value, expected: Containment) {
    let outer = canonicalize(outer).expect("canonicalizes");
    let inner = canonicalize(inner).expect("canonicalizes");
    assert_eq!(outer.covers(&inner).expect("compares"), expected);
}

#[test_case(&json!({
    "anyOf": [
        {"type": "array", "contains": {"type": "null"}},
        {"type": "array", "items": {"$ref": "#/$defs/a"}}
    ],
    "$defs": {"a": {"type": "integer"}}
}); "array union of a contains branch and a referencing items branch")]
#[test_case(&json!({
    "anyOf": [
        {"type": "array", "contains": {"type": "null"}},
        {"type": "array", "items": {"type": "integer"}}
    ]
}); "array union of a contains branch and an items branch")]
#[test_case(&json!({
    "anyOf": [
        {"type": "array", "contains": {"type": "string"}},
        {"type": "array", "prefixItems": [{"type": "integer"}]}
    ]
}); "array union of a contains branch and a prefix branch")]
#[test_case(&json!({
    "anyOf": [
        {"type": "array", "contains": {"type": "null"}, "minItems": 2},
        {"type": "array", "items": {"type": "string"}, "maxItems": 3}
    ]
}); "array union of sized contains and items branches")]
#[test_case(&json!({
    "anyOf": [
        {"type": "array", "contains": {"type": "null"}},
        {"type": "array", "contains": {"type": "boolean"}}
    ]
}); "array union of two contains branches")]
#[test_case(&json!({"type": "array", "contains": {"type": "null"}}); "single array leaf with contains")]
#[test_case(&json!({"type": "array", "items": {"$ref": "#/$defs/a"}, "$defs": {"a": {"type": "integer"}}}); "array items behind a reference")]
#[test_case(&json!({"$ref": "#/$defs/a", "$defs": {"a": {"type": "integer", "minimum": 0}}}); "a pointer standing for the whole schema")]
#[test_case(&json!({"type": "array", "prefixItems": [{"type": "integer"}], "items": {"type": "string"}}); "array with a prefix and a tail")]
#[test_case(&json!({
    "anyOf": [
        {"type": "object", "properties": {"a": {"type": "integer"}}, "required": ["a"]},
        {"type": "object", "properties": {"b": {"type": "string"}}, "required": ["b"]}
    ]
}); "object union of two required-key branches")]
#[test_case(&json!({
    "anyOf": [
        {"type": "object", "additionalProperties": {"$ref": "#/$defs/a"}},
        {"type": "object", "required": ["b"]}
    ],
    "$defs": {"a": {"type": "integer"}}
}); "object union of a referencing value branch and a required-key branch")]
#[test_case(&json!({"anyOf": [{"type": "string", "minLength": 2}, {"type": "string", "pattern": "^a"}]}); "string union of a length branch and a pattern branch")]
#[test_case(&json!({"anyOf": [{"type": "string", "format": "date"}, {"type": "string", "maxLength": 4}]}); "string union of a format branch and a length branch")]
#[test_case(&json!({"anyOf": [{"type": "array", "contains": {"type": "null"}}, {"type": "string", "pattern": "^a"}, {"type": "integer", "minimum": 3}]}); "union across three types")]
fn covers_proves_a_schema_covers_itself(schema: &Value) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    assert_eq!(
        canonical.covers(&canonical).expect("compares"),
        Containment::Yes
    );
}

// Two pointers are compared through the bodies they name, so which of them is written as a pointer
// decides nothing.
#[test]
fn covers_reads_distinct_references_through_their_targets() {
    let root = canonicalize(&json!({
        "type": "object",
        "$defs": {"A": {"type": "string"}, "B": {"type": "string"}},
        "properties": {"a": {"$ref": "#/$defs/A"}, "b": {"$ref": "#/$defs/B"}}
    }))
    .expect("canonicalizes");
    let CanonicalView::Object(view) = root.view() else {
        panic!("expected an Object view");
    };
    let left = view.properties.get("a").expect("property a").clone();
    let right = view.properties.get("b").expect("property b").clone();
    assert_eq!(left.covers(&right).expect("compares"), Containment::Yes);
}

// A cyclic map leaves the pointer unread, so no value the argument names decides the question.
#[test]
fn covers_declines_over_an_unread_pointer() {
    let recursive = canonicalize(&json!({
        "$defs": {"Node": {"type": "object", "properties": {"next": {"$ref": "#/$defs/Node"}}}},
        "$ref": "#/$defs/Node"
    }))
    .expect("canonicalizes");
    let value = canonicalize(&json!({"const": {"next": {}}})).expect("canonicalizes");
    assert_eq!(
        recursive.covers(&value).expect("compares"),
        Containment::Unknown
    );
}

#[test_case(false; "raw on the left")]
#[test_case(true; "raw on the right")]
fn covers_rejects_a_raw_operand(swap: bool) {
    let raw = canonicalize(&unsupported()).expect("canonicalizes");
    let modeled = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    let (left, right) = if swap {
        (&modeled, &raw)
    } else {
        (&raw, &modeled)
    };
    assert!(matches!(
        left.covers(right),
        Err(CanonicalizationError::UnsupportedOperand)
    ));
}

#[test]
fn covers_rejects_operands_from_different_drafts() {
    let draft7 = options()
        .with_draft(Draft::Draft7)
        .canonicalize(&json!({"type": "string"}))
        .expect("canonicalizes");
    let latest = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    assert!(matches!(
        draft7.covers(&latest),
        Err(CanonicalizationError::IncompatibleOperands(
            OperandMismatch::Drafts {
                left: Draft::Draft7,
                right: Draft::Draft202012
            }
        ))
    ));
}

// Two documents binding one `$defs` key differently are compared all the same: `A` on the right is
// every string of length four or more, which the strings on the left cover.
#[test]
fn covers_reads_through_two_definitions_of_one_name() {
    let left = canonicalize(&json!({
        "$defs": {"A": {"type": "string"}},
        "$ref": "#/$defs/A"
    }))
    .expect("canonicalizes");
    let right = canonicalize(&json!({
        "$defs": {"A": {"type": "string", "minLength": 4}},
        "$ref": "#/$defs/A"
    }))
    .expect("canonicalizes");

    assert_eq!(left.covers(&right).expect("covers"), Containment::Yes);
    assert_eq!(right.covers(&left).expect("covers"), Containment::No);
}

#[test_case(
    &json!({"type": "string", "minLength": 5}),
    &json!({"anyOf": [
        {"type": ["null", "boolean", "number", "array", "object"]},
        {"type": "string", "maxLength": 4}
    ]});
    "string leaf"
)]
#[test_case(
    &json!({"type": "number", "minimum": 5}),
    &json!({"anyOf": [
        {"type": ["null", "boolean", "string", "array", "object"]},
        {"type": "number", "exclusiveMaximum": 5}
    ]});
    "number leaf"
)]
#[test_case(
    &json!({"type": "object", "required": ["a"]}),
    &json!({"anyOf": [
        {"type": ["null", "boolean", "number", "string", "array"]},
        {"type": "object", "properties": {"a": false}}
    ]});
    "object leaf"
)]
#[test_case(
    &json!({"$defs": {"a": {"type": "string"}}, "oneOf": [{"$ref": "#/$defs/a"}, {"type": "integer"}]}),
    &json!({
        "anyOf": [
            {"type": ["null", "boolean", "array", "object"]},
            {"type": "number", "not": {"multipleOf": 1}}
        ]
    });
    "choice between disjoint branches"
)]
#[test_case(
    &json!({
        "$defs": {"a": {"type": "string", "minLength": 3}},
        "oneOf": [{"$ref": "#/$defs/a"}, {"type": "string", "maxLength": 5}]
    }),
    // Exactly one of "at least 3" and "at most 5" holds for no string, so the complement is every
    // non-string beside the strings both take - folded into one window, leaving the definition dead.
    &json!({
        "anyOf": [
            {"type": ["null", "boolean", "number", "array", "object"]},
            {"type": "string", "minLength": 3, "maxLength": 5}
        ]
    });
    "choice between overlapping branches"
)]
// A pointer at a choice resolves like any other, so the complement takes the pointer's place and
// the target it named drops out of the definitions.
#[test_case(
    &json!({
        "$defs": {
            "a": {"type": "string"},
            "node": {"oneOf": [{"$ref": "#/$defs/a"}, {"type": "integer"}]}
        },
        "$ref": "#/$defs/node"
    }),
    &json!({
        "anyOf": [
            {"type": ["null", "boolean", "array", "object"]},
            {"type": "number", "not": {"multipleOf": 1}}
        ]
    });
    "pointer at a choice"
)]
// A pointer back to a target already being negated keeps its complement symbolic, so the walk
// spells one level and stops instead of unrolling a cycle that has no end.
#[test_case(
    &json!({
        "$defs": {"A": {"type": "object", "properties": {"a": {"$ref": "#/$defs/A"}}}},
        "$ref": "#/$defs/A"
    }),
    &json!({
        "$defs": {"A": {"type": "object", "properties": {"a": {"$ref": "#/$defs/A"}}}},
        "anyOf": [
            {"type": ["null", "boolean", "number", "string", "array"]},
            {"type": "object", "required": ["a"],
             "properties": {"a": {"not": {"$ref": "#/$defs/A"}}}}
        ]
    });
    "self-recursive reference"
)]
#[test_case(
    &json!({
        "$defs": {
            "A": {"type": "object", "properties": {"b": {"$ref": "#/$defs/B"}}},
            "B": {"type": "object", "properties": {"a": {"$ref": "#/$defs/A"}}}
        },
        "$ref": "#/$defs/A"
    }),
    &json!({
        "$defs": {
            "A": {"type": "object", "properties": {"b": {"$ref": "#/$defs/B"}}},
            "B": {"type": "object", "properties": {"a": {"$ref": "#/$defs/A"}}}
        },
        "anyOf": [
            {"type": ["null", "boolean", "number", "string", "array"]},
            {"type": "object", "required": ["b"], "properties": {"b": {"anyOf": [
                {"type": ["null", "boolean", "number", "string", "array"]},
                {"type": "object", "required": ["a"],
                 "properties": {"a": {"not": {"$ref": "#/$defs/A"}}}}
            ]}}}
        ]
    });
    "mutually recursive references"
)]
#[test_case(
    &json!({
        "$defs": {"A": {"type": "array", "items": {"$ref": "#/$defs/A"}}},
        "$ref": "#/$defs/A"
    }),
    &json!({
        "$defs": {"A": {"type": "array", "items": {"$ref": "#/$defs/A"}}},
        "anyOf": [
            {"type": ["null", "boolean", "number", "string", "object"]},
            {"type": "array", "contains": {"not": {"$ref": "#/$defs/A"}}}
        ]
    });
    "recursive array reference"
)]
#[test_case(
    &json!({
        "$defs": {"node": {"oneOf": [
            {"type": "string"},
            {"type": "object", "properties": {"next": {"$ref": "#/$defs/node"}}, "required": ["next"]}
        ]}},
        "$ref": "#/$defs/node"
    }),
    &json!({
        "$defs": {"node": {"anyOf": [
            {"type": "string"},
            {"type": "object", "required": ["next"],
             "properties": {"next": {"$ref": "#/$defs/node"}}}
        ]}},
        "anyOf": [
            {"type": ["null", "boolean", "number", "array"]},
            {"type": "object", "properties": {"next": {"not": {"$ref": "#/$defs/node"}}}}
        ]
    });
    "recursive choice reference"
)]
#[test_case(
    &json!({
        "$defs": {
            "left": {"oneOf": [
                {"type": "string"},
                {"type": "object", "properties": {"next": {"$ref": "#/$defs/right"}}, "required": ["next"]}
            ]},
            "right": {"oneOf": [
                {"type": "integer"},
                {"type": "object", "properties": {"back": {"$ref": "#/$defs/left"}}, "required": ["back"]}
            ]}
        },
        "$ref": "#/$defs/left"
    }),
    &json!({
        "$defs": {
            "left": {"anyOf": [
                {"type": "string"},
                {"type": "object", "required": ["next"],
                 "properties": {"next": {"$ref": "#/$defs/right"}}}
            ]},
            "right": {"anyOf": [
                {"type": "integer"},
                {"type": "object", "required": ["back"],
                 "properties": {"back": {"$ref": "#/$defs/left"}}}
            ]}
        },
        "anyOf": [
            {"type": ["null", "boolean", "number", "array"]},
            {"type": "object", "properties": {"next": {"anyOf": [
                {"type": ["null", "boolean", "string", "array"]},
                {"type": "number", "not": {"multipleOf": 1}},
                {"type": "object", "properties": {"back": {"not": {"$ref": "#/$defs/left"}}}}
            ]}}}
        ]
    });
    "mutually recursive choice references"
)]
fn negate_spells_the_complement(schema: &Value, expected: &Value) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    let mut expected = expected.as_object().expect("object").clone();
    expected.insert(
        "$schema".into(),
        json!("https://json-schema.org/draft/2020-12/schema"),
    );
    assert_eq!(
        canonical.negate().expect("negates").to_json_schema(),
        Value::Object(expected)
    );
}

#[test_case(
    &json!({"type": "object", "additionalProperties": {"type": "string"}}),
    &json!({"anyOf": [
        {"type": ["null", "boolean", "number", "string", "array"]},
        {"type": "object", "not": {"additionalProperties": {"type": "string"}}}
    ]});
    "value shield"
)]
#[test_case(
    &json!({"type": "object", "properties": {"a": {"type": "string"}}, "additionalProperties": false}),
    &json!({"anyOf": [
        {"type": ["null", "boolean", "number", "string", "array"]},
        {"type": "object", "not": {"properties": {"a": {}}, "additionalProperties": false}},
        {"type": "object", "required": ["a"],
         "properties": {"a": {"type": ["null", "boolean", "number", "array", "object"]}}}
    ]});
    "closed object"
)]
#[test_case(
    &json!({"type": "array", "items": {"type": "string"}}),
    &json!({"anyOf": [
        {"type": ["null", "boolean", "number", "string", "object"]},
        {"type": "array",
         "not": {"items": {"not": {"type": ["null", "boolean", "number", "array", "object"]}}}}
    ]});
    "element schema"
)]
#[test_case(
    &json!({"type": "array", "items": {"type": "string"}, "maxItems": 2}),
    &json!({"anyOf": [
        {"type": ["null", "boolean", "number", "string", "object"]},
        {"type": "array",
         "not": {"items": {"not": {"type": ["null", "boolean", "number", "array", "object"]}}}},
        {"type": "array", "minItems": 3}
    ]});
    "element schema beside a size bound"
)]
fn negate_spells_the_draft_4_complement(schema: &Value, expected: &Value) {
    let canonical = options()
        .with_draft(Draft::Draft4)
        .canonicalize(schema)
        .expect("canonicalizes");
    let mut expected = expected.as_object().expect("object").clone();
    expected.insert(
        "$schema".into(),
        json!("http://json-schema.org/draft-04/schema#"),
    );
    assert_eq!(
        canonical.negate().expect("negates").to_json_schema(),
        Value::Object(expected)
    );
}

#[test_case(&json!({"type": "string", "minLength": 5}); "string leaf")]
#[test_case(&json!({"type": "number", "minimum": 5}); "number leaf")]
#[test_case(&json!({"type": "object", "required": ["a"]}); "object leaf")]
#[test_case(&json!({"const": 1.5}); "numeric constant")]
#[test_case(&json!({"type": "array", "items": {"type": "string"}}); "array element schema")]
#[test_case(&json!({"type": "array", "maxItems": 2, "items": {"type": "string"}}); "array element schema beside a size bound")]
#[test_case(&json!({"const": "a"}); "string constant")]
#[test_case(&json!({"enum": ["a", "b"]}); "string value set")]
#[test_case(&json!({"type": "string", "minLength": 2}); "excluded value under a window")]
#[test_case(&json!({"type": "array", "contains": {"type": "string"}}); "array existential demand")]
#[test_case(&json!({"type": "array", "contains": {"const": "a"}}); "array existential demand on a value")]
#[test_case(
    &json!({"type": "array", "minItems": 1, "contains": {"type": "string"}});
    "array existential demand beside a size bound"
)]
#[test_case(
    &json!({"type": "array", "items": {"type": "string"}, "contains": {"const": "a"}});
    "array existential demand beside an element schema"
)]
#[test_case(
    &json!({"$schema": "http://json-schema.org/draft-04/schema#", "type": "array", "items": {"type": "string"}});
    "draft 4 array element schema"
)]
#[test_case(
    &json!({"$schema": "http://json-schema.org/draft-04/schema#", "items": {"type": "string"}});
    "draft 4 untyped array element schema"
)]
#[test_case(
    &json!({"$schema": "http://json-schema.org/draft-04/schema#", "type": "array",
        "items": {"type": "string"}, "minItems": 1, "maxItems": 4});
    "draft 4 array element schema in a length window"
)]
#[test_case(&json!({"type": "array", "uniqueItems": true}); "array distinctness demand")]
#[test_case(
    &json!({"type": "array", "uniqueItems": true, "minItems": 3});
    "array distinctness demand above a size floor"
)]
#[test_case(
    &json!({"type": "array", "allOf": [{"not": {"type": "array", "uniqueItems": true}}]});
    "array repeat demand"
)]
#[test_case(&json!({"type": "array", "prefixItems": [{"type": "string"}]}); "array tuple")]
#[test_case(
    &json!({"type": "array", "prefixItems": [{"type": "string"}, {"type": "integer"}]});
    "two-position array tuple"
)]
#[test_case(
    &json!({"type": "array", "prefixItems": [{"type": "string"}], "items": false});
    "array tuple with a closed tail"
)]
#[test_case(
    &json!({"type": "array", "prefixItems": [{"type": "string"}], "minItems": 1, "maxItems": 2});
    "array tuple within a size window"
)]
#[test_case(&json!({"type": "integer"}); "integer leaf")]
#[test_case(&json!({"type": "integer", "minimum": 0}); "bounded integer leaf")]
#[test_case(
    &json!({"$schema": "http://json-schema.org/draft-04/schema#", "type": "integer"});
    "draft 4 integer leaf"
)]
#[test_case(&json!({"type": "integer", "multipleOf": 3}); "integer leaf with a divisor")]
#[test_case(&json!({"type": "number", "multipleOf": 0.5}); "number leaf with a divisor")]
#[test_case(
    &json!({"$schema": "http://json-schema.org/draft-04/schema#", "type": "integer", "enum": [1, 2]});
    "draft 4 typed group"
)]
#[test_case(&reference_chain_schema(); "reference chain")]
#[test_case(
    &json!({"$defs": {"a": {"type": "string"}}, "oneOf": [{"$ref": "#/$defs/a"}, {"type": "integer"}]});
    "choice between disjoint branches"
)]
#[test_case(
    &json!({
        "$defs": {"a": {"type": "string", "minLength": 5}},
        "oneOf": [{"$ref": "#/$defs/a"}, {"type": "string"}]
    });
    "choice between overlapping branches"
)]
#[test_case(&json!({"type": "object", "propertyNames": {"enum": ["a", "b"]}}); "key constraint")]
#[test_case(&json!({"type": "object", "propertyNames": {"pattern": "^a"}}); "key pattern constraint")]
#[test_case(
    &json!({"type": "object", "properties": {"a": {"type": "string"}}, "additionalProperties": false});
    "closed object with a declared property"
)]
#[test_case(&json!({"type": "object", "additionalProperties": {"type": "string"}}); "value shield")]
#[test_case(
    &json!({"type": "object", "properties": {"a": {"type": "integer"}}, "additionalProperties": {"type": "string"}});
    "value shield beside a declared property"
)]
#[test_case(
    &json!({
        "$defs": {"A": {"type": "object", "properties": {"a": {"$ref": "#/$defs/A"}}}},
        "$ref": "#/$defs/A"
    });
    "self-recursive reference"
)]
#[test_case(
    &json!({
        "$defs": {
            "A": {"type": "object", "properties": {"b": {"$ref": "#/$defs/B"}}},
            "B": {"type": "object", "properties": {"a": {"$ref": "#/$defs/A"}}}
        },
        "$ref": "#/$defs/A"
    });
    "mutually recursive references"
)]
#[test_case(
    &json!({
        "$defs": {"A": {"type": "array", "items": {"$ref": "#/$defs/A"}}},
        "$ref": "#/$defs/A"
    });
    "recursive array reference"
)]
#[test_case(
    &json!({
        "$defs": {"node": {"oneOf": [
            {"type": "string"},
            {"type": "object", "properties": {"next": {"$ref": "#/$defs/node"}}, "required": ["next"]}
        ]}},
        "$ref": "#/$defs/node"
    });
    "recursive choice reference"
)]
#[test_case(
    &json!({"type": "array", "minItems": 2, "maxItems": 2,
        "prefixItems": [{"type": "string"}, {"type": "integer"}], "items": {"type": "boolean"}});
    "array tuple with a tail beyond its ceiling"
)]
fn negate_admits_exactly_what_the_source_rejects(schema: &Value) {
    let complement = canonicalize(schema)
        .expect("canonicalizes")
        .negate()
        .expect("negates")
        .to_json_schema();
    let source = jsonschema::validator_for(schema).expect("source builds");
    let complement = jsonschema::validator_for(&complement).expect("complement builds");
    for instance in [
        json!(null),
        json!(true),
        json!(1),
        json!(2),
        json!(4),
        json!(5),
        json!(1.0),
        json!(2.0),
        json!(1.5),
        json!("abcd"),
        json!("abcde"),
        json!([]),
        json!(["a"]),
        json!(["a", "b", "c"]),
        json!(["a", "a"]),
        json!(["a", 1]),
        json!([1]),
        json!([1, 1]),
        json!({}),
        json!({"a": 1}),
        json!({"inner": 3}),
        json!({"inner": {}}),
        json!({"inner": {"x": "s"}}),
        json!({"inner": {"x": 1}}),
        json!({"a": {"a": 1}}),
        json!({"a": {"a": {}}}),
        json!({"next": "a"}),
        json!({"next": {"next": 1}}),
        json!([[1]]),
        json!([["a"]]),
    ] {
        assert_ne!(
            source.is_valid(&instance),
            complement.is_valid(&instance),
            "{instance} lands on the same side of both"
        );
    }
}

// A `Raw` operand raises `UnsupportedOperand`, not `UnsupportedResult`.
#[test]
fn negate_rejects_an_unsupported_schema() {
    assert!(matches!(
        canonicalize(&unsupported())
            .expect("canonicalizes")
            .negate(),
        Err(CanonicalizationError::UnsupportedOperand)
    ));
}

// What `negate` declines is contract: a caller sizes its fallback on it, so widening it is a
// visible change.
#[test_case(
    &json!({"type": "array", "contains": {"type": "string"}, "minContains": 2});
    "a counted contains"
)]
#[test_case(
    &json!({"type": "object", "properties": {"a": {"$ref": "#"}}});
    "root self-reference"
)]
#[test_case(
    &json!({"type": "object", "properties": {"a": {"not": {"$ref": "#"}}}});
    "barred root self-reference"
)]
#[test_case(
    &json!({
        "$defs": {"A": {"not": {"$ref": "#/$defs/A"}}},
        "$ref": "#/$defs/A"
    });
    "reference through its own complement"
)]
#[test_case(
    &json!({"type": "object", "patternProperties": {"^a": {"type": "string"}}});
    "pattern properties"
)]
#[test_case(
    &json!({"type": "array", "prefixItems": [{"type": "string"}], "items": {"type": "integer"}});
    "array tuple with an open tail"
)]
#[test_case(
    &json!({"type": "array", "prefixItems": [{"enum": [[1]]}]});
    "array tuple over an array value"
)]
fn negate_declines(schema: &Value) {
    assert!(matches!(
        canonicalize(schema).expect("canonicalizes").negate(),
        Err(CanonicalizationError::UnsupportedResult)
    ));
}

// A definition whose `if` reads it back is the validator's answer, not the algebra's, so negation
// bars the pointer whole - as it does for a choice reading its own target.
#[test]
fn a_condition_reading_its_own_definition_is_barred_whole() {
    let ill_founded = canonicalize(&json!({
        "$defs": {"t": {
            "if": {"$ref": "#/$defs/t"},
            "then": {"type": "array"},
            "else": {"type": "null"}
        }},
        "$ref": "#/$defs/t"
    }))
    .expect("canonicalizes");
    let barred = ill_founded.negate().expect("negates");
    let (whole, complement) = (
        validator_for(&ill_founded.to_json_schema()).expect("builds"),
        validator_for(&barred.to_json_schema()).expect("builds"),
    );
    for instance in [json!(null), json!([1]), json!({}), json!("x"), json!(1)] {
        assert_eq!(
            !whole.is_valid(&instance),
            complement.is_valid(&instance),
            "the complement disagrees on {instance}\n  complement = {}",
            barred.to_json_schema()
        );
    }
}

// Each definition resolves twice per level, so the complement's size doubles with depth; the walk
// declines rather than spelling a complement exponentially larger than the source.
#[test]
fn negate_declines_past_the_resolution_budget() {
    let mut definitions = Map::new();
    definitions.insert("d0".into(), json!({"type": "string"}));
    for level in 1..13 {
        definitions.insert(
            format!("d{level}"),
            json!({"type": "object", "properties": {
                "left": {"$ref": format!("#/$defs/d{}", level - 1)},
                "right": {"$ref": format!("#/$defs/d{}", level - 1)}
            }}),
        );
    }
    let schema = json!({"$defs": definitions, "$ref": "#/$defs/d12"});
    assert!(matches!(
        canonicalize(&schema).expect("canonicalizes").negate(),
        Err(CanonicalizationError::UnsupportedResult)
    ));
}

fn choice_over_pointers(count: usize) -> Value {
    let mut definitions = Map::new();
    let mut branches = Vec::new();
    for index in 0..count {
        definitions.insert(
            format!("d{index}"),
            json!({"type": "object", "required": [format!("k{index}")]}),
        );
        branches.push(json!({"$ref": format!("#/$defs/d{index}")}));
    }
    json!({"$defs": definitions, "oneOf": branches})
}

// Every pair of branches an intersection cannot rule out becomes a branch of the complement, so the
// spelling grows with the square of the branch count and the walk declines once it outgrows use.
#[test]
fn negate_declines_past_the_overlap_budget() {
    assert!(canonicalize(&choice_over_pointers(11))
        .expect("canonicalizes")
        .negate()
        .is_ok());
    assert!(matches!(
        canonicalize(&choice_over_pointers(12))
            .expect("canonicalizes")
            .negate(),
        Err(CanonicalizationError::UnsupportedResult)
    ));
}

// A fully resolved complement names no definitions, so it carries none - and a handle with an
// empty map stays combinable with documents holding their own.
#[test]
fn negated_complement_drops_dead_definitions_and_intersects() {
    let complement = canonicalize(&json!({
        "$defs": {"A": {"type": "string"}},
        "$ref": "#/$defs/A"
    }))
    .expect("canonicalizes")
    .negate()
    .expect("negates");
    assert_eq!(complement.definitions().len(), 0);
    let other = canonicalize(&json!({
        "$defs": {"B": {"type": "integer"}},
        "type": "object",
        "properties": {"b": {"$ref": "#/$defs/B"}}
    }))
    .expect("canonicalizes");
    let met = complement.intersect(&other).expect("intersects");
    assert_eq!(
        met.to_json_schema(),
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": {"B": {"type": "integer"}},
            "type": "object",
            "properties": {"b": {"$ref": "#/$defs/B"}}
        })
    );
}

#[test]
fn negate_resolves_a_reference() {
    let schema = canonicalize(&json!({
        "$defs": {"A": {"type": "string"}},
        "$ref": "#/$defs/A"
    }))
    .expect("canonicalizes");
    let complement = schema.negate().expect("negates");
    assert_eq!(
        complement.to_json_schema(),
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": ["null", "boolean", "number", "array", "object"]
        })
    );
}

// A pointer whose target is still being parsed stays symbolic, and the surrounding cycle leaves
// its complement inexpressible at every later attempt too.
#[test]
fn a_barred_pointer_reaching_a_cycle_stays_symbolic() {
    let schema = canonicalize(&json!({
        "$defs": {
            "K": {"type": "object", "properties": {"m": {"$ref": "#/$defs/M"}}},
            "M": {"not": {"$ref": "#/$defs/K"}}
        },
        "$ref": "#/$defs/M"
    }))
    .expect("canonicalizes");
    let named = schema.definition("#/$defs/M").expect("named");
    let CanonicalView::Not(barred) = named.view() else {
        panic!("the barred pointer stays symbolic, got {:?}", named.kind());
    };
    assert_eq!(barred.kind(), CanonicalKind::Reference);
    assert!(matches!(
        barred.negate(),
        Err(CanonicalizationError::UnsupportedResult)
    ));
}

// The corpus spelling puts the barred pointer inside a property, so the fold runs wherever `not`
// is parsed and not only at the document root.
#[test]
fn negated_pointer_inside_a_property_resolves() {
    let canonical = canonicalize(&json!({
        "$defs": {"a": {"type": "string", "minLength": 2}},
        "type": "object",
        "properties": {"p": {"not": {"allOf": [{"$ref": "#/$defs/a"}, {"description": "x"}]}}}
    }))
    .expect("canonicalizes");
    assert_eq!(
        canonical.to_json_schema(),
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {"p": {"anyOf": [
                {"type": ["null", "boolean", "number", "array", "object"]},
                {"type": "string", "maxLength": 1}
            ]}}
        })
    );
}

fn reference_chain_schema() -> Value {
    json!({
        "$defs": {
            "outer": {
                "type": "object",
                "properties": {"inner": {"$ref": "#/$defs/inner"}},
                "required": ["inner"]
            },
            "inner": {
                "type": "object",
                "properties": {"x": {"type": "string"}},
                "required": ["x"]
            }
        },
        "$ref": "#/$defs/outer"
    })
}

// Negating a pointer chain resolves every hop, so both definitions' complements inline.
#[test]
fn negate_resolves_a_reference_chain() {
    let schema = canonicalize(&reference_chain_schema()).expect("canonicalizes");
    let complement = schema.negate().expect("negates").to_json_schema();
    assert_eq!(
        complement,
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "anyOf": [
                {"type": ["null", "boolean", "number", "string", "array"]},
                {
                    "type": "object",
                    "properties": {"inner": {"anyOf": [
                        {"type": ["null", "boolean", "number", "string", "array"]},
                        {
                            "type": "object",
                            "properties": {"x": {"type": ["null", "boolean", "number", "array", "object"]}}
                        }
                    ]}}
                }
            ]
        })
    );
}

#[test]
fn negated_key_constraint_keeps_the_definition() {
    // The reference reaches "#/$defs/K" only through the raw `not`, so parsing must walk the
    // demand the `not` produces to keep the definition from being pruned as unreachable.
    let canonical = canonicalize(&json!({
        "$defs": {"K": {"enum": ["a", "b"]}},
        "not": {"type": "object", "propertyNames": {"$ref": "#/$defs/K"}}
    }))
    .expect("canonicalizes");
    assert!(canonical.definition("#/$defs/K").is_some());
    jsonschema::validator_for(&canonical.to_json_schema()).expect("emitted schema builds");
}

#[test]
fn negated_value_shield_keeps_the_definition() {
    // The reference reaches "#/$defs/V" only through the raw `not`, so parsing must walk the
    // demand the `not` produces to keep the definition from being pruned as unreachable.
    let canonical = canonicalize(&json!({
        "$defs": {"V": {"type": "string"}},
        "not": {"type": "object", "additionalProperties": {"$ref": "#/$defs/V"}}
    }))
    .expect("canonicalizes");
    assert!(canonical.definition("#/$defs/V").is_some());
    jsonschema::validator_for(&canonical.to_json_schema()).expect("emitted schema builds");
}

#[test_case(&json!({"const": "a"}); "string constant")]
#[test_case(&json!({"enum": ["a", "b"]}); "string value set")]
#[test_case(&json!({"type": "string", "minLength": 2}); "string window")]
#[test_case(&json!({"type": "object", "propertyNames": {"enum": ["a", "b"]}}); "key constraint")]
#[test_case(
    &json!({"type": "object", "properties": {"a": {"type": "string"}}, "additionalProperties": false});
    "closed object with a declared property"
)]
#[test_case(
    &json!({"type": "object", "propertyNames": {"pattern": "^a"}});
    "key pattern constraint"
)]
#[test_case(&json!({"type": "object", "additionalProperties": {"type": "string"}}); "value shield")]
#[test_case(
    &json!({"type": "object", "properties": {"a": {"type": "integer"}}, "additionalProperties": {"type": "string"}});
    "value shield beside a declared property"
)]
#[test_case(&json!({"type": "array", "prefixItems": [{"type": "string"}]}); "array tuple")]
fn negate_is_an_involution(schema: &Value) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    let doubled = canonical
        .negate()
        .expect("negates")
        .negate()
        .expect("negates back");
    assert_eq!(doubled, canonical);
}

// "abc" has length 3, so it always fails `maxLength: 2`: the demand is implied by the required
// key and folding it away leaves the same value set the direct spelling admits.
#[test]
fn required_key_violation_folds_when_it_always_fails_property_names() {
    let composed = canonicalize(&json!({"allOf": [
        {"not": {"type": "object", "propertyNames": {"maxLength": 2}}},
        {"type": "object", "required": ["abc"]}
    ]}))
    .expect("canonicalizes");
    let direct =
        canonicalize(&json!({"type": "object", "required": ["abc"]})).expect("canonicalizes");
    assert_eq!(composed.to_json_schema(), direct.to_json_schema());
}

// "ab" has length 2, which `maxLength: 2` admits, so the required key alone cannot satisfy the
// demand: it must survive.
#[test]
fn required_key_violation_stays_when_it_can_satisfy_property_names() {
    let composed = canonicalize(&json!({"allOf": [
        {"not": {"type": "object", "propertyNames": {"maxLength": 2}}},
        {"type": "object", "required": ["ab"]}
    ]}))
    .expect("canonicalizes");
    let CanonicalView::Object(view) = composed.view() else {
        panic!("expected an Object view");
    };
    assert_eq!(view.violations.len(), 1);
    assert!(matches!(
        view.violations[0],
        ObjectViolationView::NameFails(_)
    ));
}

// A required key the demand admits cannot itself carry the violation, and the size ceiling
// gives it no room for a second key to carry it instead: no object can validate.
#[test_case(
    &json!({"type": "object", "maxProperties": 1, "minProperties": 1, "required": ["a"],
            "properties": {"a": {"type": "string"}},
            "not": {"type": "object", "propertyNames": {"enum": ["a", "b"]}}}),
    Satisfiability::No;
    "no room for the name-fails demand beyond the required key"
)]
#[test_case(
    &json!({"type": "object", "maxProperties": 2, "minProperties": 1, "required": ["a"],
            "properties": {"a": {"type": "string"}},
            "not": {"type": "object", "propertyNames": {"enum": ["a", "b"]}}}),
    Satisfiability::Unknown;
    "a second slot admits the violating key"
)]
#[test_case(
    // `{"c": null}`: one property, the required key present, and outside `{a, b}` as demanded.
    &json!({"type": "object", "maxProperties": 1, "minProperties": 1, "required": ["c"],
            "properties": {"a": {"type": "string"}},
            "not": {"type": "object", "propertyNames": {"enum": ["a", "b"]}}}),
    Satisfiability::Yes;
    "the required key itself already carries the violation"
)]
#[test_case(
    &json!({"type": "object", "maxProperties": 1, "required": ["a"],
            "properties": {"a": {"type": "integer"}},
            "not": {"type": "object", "properties": {"a": {"type": "integer"}},
                    "additionalProperties": {"type": "string"}}}),
    Satisfiability::No;
    "no room for the undeclared-value-fails demand beyond the required key"
)]
fn a_required_key_the_demand_admits_needs_room_for_another(
    schema: &Value,
    expected: Satisfiability,
) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    assert_eq!(canonical.satisfiability(), expected);
}

// The extra slot in the second case above is not just theoretical: an object using it validates.
#[test]
fn the_extra_slot_admits_a_validating_instance() {
    let schema = json!({"type": "object", "maxProperties": 2, "minProperties": 1, "required": ["a"],
        "properties": {"a": {"type": "string"}},
        "not": {"type": "object", "propertyNames": {"enum": ["a", "b"]}}});
    let validator = validator_for(&schema).expect("builds");
    assert!(validator.is_valid(&json!({"a": "x", "c": 1})));
}

// The required key "c" already breaks the schema the demand names on its own, so the fold above
// removes the demand entirely and the leaf is satisfiable with just that one key.
#[test]
fn the_folded_required_key_admits_a_validating_instance() {
    let schema = json!({"type": "object", "maxProperties": 1, "minProperties": 1, "required": ["c"],
        "properties": {"a": {"type": "string"}},
        "not": {"type": "object", "propertyNames": {"enum": ["a", "b"]}}});
    let validator = validator_for(&schema).expect("builds");
    assert!(validator.is_valid(&json!({"c": 1})));
}

// "a" definitely satisfies the demand, but "z" only might: whether the custom format rejects it
// is unknown to the checker, so the demand still needs a candidate and must survive. A caller
// registering a checker that fails "z" makes it the violator, so the leaf stays satisfiable.
#[test]
fn a_required_key_left_undecided_keeps_the_demand_from_folding() {
    let schema = json!({"type": "object", "maxProperties": 2, "required": ["a", "z"],
    "properties": {"a": {}, "z": {}},
    "not": {"type": "object", "propertyNames": {"anyOf": [
        {"const": "a"}, {"format": "custom-uncheckable"}
    ]}}});
    let canonical = options()
        .should_validate_formats(true)
        .canonicalize(&schema)
        .expect("canonicalizes");
    assert_ne!(canonical.satisfiability(), Satisfiability::No);
    let validator = ::jsonschema::options()
        .with_format("custom-uncheckable", |text: &str| text != "z")
        .should_validate_formats(true)
        .build(&schema)
        .expect("builds");
    assert!(validator.is_valid(&json!({"a": 1, "z": 2})));
}

// "a" is name-covered by the demand's scope, so it cannot carry the value-side violation, but "z"
// is outside that scope and matches no pattern - it can carry it, so the demand must survive.
#[test]
fn a_required_key_outside_the_demand_names_keeps_it_from_folding() {
    let schema = json!({"type": "object", "maxProperties": 2, "required": ["a", "z"],
        "properties": {"a": {}, "z": {"type": "integer"}},
        "not": {"type": "object", "properties": {"a": {}},
                "additionalProperties": {"type": "string"}}});
    let canonical = canonicalize(&schema).expect("canonicalizes");
    assert_ne!(canonical.satisfiability(), Satisfiability::No);
    let validator = validator_for(&schema).expect("builds");
    assert!(validator.is_valid(&json!({"a": 1, "z": 5})));
}

#[test_case(&json!({"type": "object", "propertyNames": {"enum": ["a", "b"]}}); "key constraint")]
#[test_case(
    &json!({"type": "object", "properties": {"a": {"type": "string"}}, "additionalProperties": false});
    "closed object with a declared property"
)]
#[test_case(
    &json!({"type": "object", "propertyNames": {"pattern": "^a"}});
    "key pattern constraint"
)]
#[test_case(&json!({"type": "object", "additionalProperties": {"type": "string"}}); "value shield")]
#[test_case(
    &json!({"type": "object", "properties": {"a": {"type": "integer"}}, "additionalProperties": {"type": "string"}});
    "value shield beside a declared property"
)]
fn complement_intersects_to_nothing(schema: &Value) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    let complement = canonical.negate().expect("negates");
    let meet = canonical.intersect(&complement).expect("intersects");
    assert_eq!(meet.satisfiability(), Satisfiability::No);
}

// A fanning-out $ref graph must complete quickly (bailing to Raw via the fold budget), not hang.
#[test]
fn unevaluated_properties_beside_a_fanout_reference_graph_does_not_blow_up() {
    let depth = 24;
    let mut defs = serde_json::Map::new();
    defs.insert("d24".to_string(), json!({"type": "integer"}));
    for level in (0..depth).rev() {
        defs.insert(
            format!("d{level}"),
            json!({"allOf": [
                {"$ref": format!("#/$defs/d{}", level + 1)},
                {"$ref": format!("#/$defs/d{}", level + 1)}
            ]}),
        );
    }
    let schema = json!({
        "$defs": defs,
        "$ref": "#/$defs/d0",
        "unevaluatedProperties": false
    });
    let canonical = canonicalize(&schema).expect("canonicalizes without erroring");
    assert_eq!(canonical.kind(), CanonicalKind::Raw);
}

// Item-cover twin of the property-cover fanout test above.
#[test]
fn unevaluated_items_beside_a_fanout_reference_graph_does_not_blow_up() {
    let depth = 24;
    let mut defs = serde_json::Map::new();
    defs.insert("d24".to_string(), json!({"type": "integer"}));
    for level in (0..depth).rev() {
        defs.insert(
            format!("d{level}"),
            json!({"allOf": [
                {"$ref": format!("#/$defs/d{}", level + 1)},
                {"$ref": format!("#/$defs/d{}", level + 1)}
            ]}),
        );
    }
    let schema = json!({
        "$defs": defs,
        "$ref": "#/$defs/d0",
        "unevaluatedItems": false
    });
    let canonical = canonicalize(&schema).expect("canonicalizes without erroring");
    assert_eq!(canonical.kind(), CanonicalKind::Raw);
}

// A $ref target the cover computation cannot fully evaluate - either it fetches raw JSON that isn't
// a schema (only reachable via an external registry resource, which bypasses the inline metaschema
// check `$defs` gets), or it declares a different draft - both leave the document Raw.
#[test_case(&json!(5), "unevaluatedProperties"; "non-schema property target")]
#[test_case(&json!(5), "unevaluatedItems"; "non-schema item target")]
#[test_case(
    &json!({"$schema": "http://json-schema.org/draft-07/schema#", "type": "object"}),
    "unevaluatedProperties";
    "cross-draft property target"
)]
#[test_case(
    &json!({"$schema": "http://json-schema.org/draft-07/schema#", "type": "array"}),
    "unevaluatedItems";
    "cross-draft item target"
)]
fn unevaluated_cover_over_an_unresolvable_registry_target_stays_raw(target: &Value, keyword: &str) {
    let registry = Registry::new()
        .add("https://example.com/target", target)
        .expect("resource URI is valid")
        .prepare()
        .expect("registry prepares");
    let mut document = Map::new();
    document.insert("$ref".to_string(), json!("https://example.com/target"));
    document.insert(keyword.to_string(), json!(false));
    let document = Value::Object(document);
    let canonical = options()
        .with_draft(Draft::Draft202012)
        .with_registry(&registry)
        .canonicalize(&document)
        .expect("canonicalizes");

    assert_eq!(canonical.kind(), CanonicalKind::Raw);
    assert_eq!(canonical.to_json_schema(), document);
}

/// The names a document emits under `$defs`, sorted.
fn emitted_definition_names(schema: &CanonicalSchema) -> Vec<String> {
    let document = schema.to_json_schema();
    let mut names: Vec<String> = document
        .get("$defs")
        .and_then(Value::as_object)
        .map(|entries| entries.keys().cloned().collect())
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn a_definition_emits_only_what_it_names() {
    let canonical = canonicalize(&json!({
        "type": "object",
        "properties": {"a": {"$ref": "#/$defs/a"}, "z": {"$ref": "#/$defs/z"}},
        "$defs": {
            "a": {"type": "object", "properties": {"inner": {"$ref": "#/$defs/a_inner"}}},
            "a_inner": {"type": "string"},
            "z": {"type": "object", "properties": {"inner": {"$ref": "#/$defs/z_inner"}}},
            "z_inner": {"type": "integer"}
        }
    }))
    .expect("canonicalizes");

    assert_eq!(
        emitted_definition_names(&canonical),
        vec!["a", "a_inner", "z", "z_inner"],
        "the document names every one of them"
    );
    let branch = canonical
        .definition("#/$defs/a")
        .expect("definition is here");
    assert_eq!(emitted_definition_names(&branch), vec!["a_inner"]);
}

#[test]
fn a_definition_emits_the_whole_chain_it_names() {
    let canonical = canonicalize(&json!({
        "$ref": "#/$defs/a",
        "$defs": {
            "a": {"type": "object", "properties": {"b": {"$ref": "#/$defs/b"}}},
            "b": {"$ref": "#/$defs/c"},
            "c": {"type": "string"},
            "other": {"type": "integer"},
            "names_other": {"$ref": "#/$defs/other"}
        }
    }))
    .expect("canonicalizes");

    let branch = canonical
        .definition("#/$defs/a")
        .expect("definition is here");
    assert_eq!(emitted_definition_names(&branch), vec!["b", "c"]);
}

#[test]
fn a_definition_in_a_cycle_emits_the_cycle() {
    let canonical = canonicalize(&json!({
        "$ref": "#/$defs/a",
        "$defs": {
            "a": {"type": "object", "properties": {"next": {"$ref": "#/$defs/b"}}},
            "b": {"type": "object", "properties": {"next": {"$ref": "#/$defs/a"}}},
            "unrelated": {"type": "integer"},
            "names_unrelated": {"$ref": "#/$defs/unrelated"}
        }
    }))
    .expect("canonicalizes");

    let branch = canonical
        .definition("#/$defs/a")
        .expect("definition is here");
    assert_eq!(emitted_definition_names(&branch), vec!["a", "b"]);
}

#[test]
fn a_definition_naming_the_document_keeps_what_the_document_names() {
    let canonical = canonicalize(&json!({
        "type": "object",
        "properties": {"self": {"$ref": "#/$defs/points_at_root"}, "kept": {"$ref": "#/$defs/kept"}},
        "$defs": {
            "points_at_root": {"type": "object", "properties": {"up": {"$ref": "#"}}},
            "kept": {"type": "string"},
            "aside": {"type": "integer"},
            "names_aside": {"$ref": "#/$defs/aside"}
        }
    }))
    .expect("canonicalizes");

    // Emitting this one re-homes the document as a definition of its own, and what the document
    // names has to travel with it.
    let branch = canonical
        .definition("#/$defs/points_at_root")
        .expect("definition is here");
    let names = emitted_definition_names(&branch);
    assert!(
        names.contains(&"kept".to_string()) && names.contains(&"points_at_root".to_string()),
        "the re-homed document and what it names are missing: {names:?}"
    );
    assert!(
        !names.contains(&"aside".to_string()),
        "nothing reaches `aside`: {names:?}"
    );
}

#[test_case("#/$defs/a"; "a plain definition")]
#[test_case("#/$defs/points_at_root"; "a definition naming the document")]
fn an_emitted_definition_stands_on_its_own(pointer: &str) {
    let canonical = canonicalize(&json!({
        "type": "object",
        "properties": {"a": {"$ref": "#/$defs/a"}, "self": {"$ref": "#/$defs/points_at_root"}},
        "$defs": {
            "a": {"type": "object", "properties": {"inner": {"$ref": "#/$defs/a_inner"}}},
            "a_inner": {"type": "string"},
            "points_at_root": {"type": "object", "properties": {"up": {"$ref": "#"}}}
        }
    }))
    .expect("canonicalizes");

    // Every pointer the emitted document still spells has to resolve inside it.
    let emitted = canonical
        .definition(pointer)
        .expect("definition is here")
        .to_json_schema();
    canonicalize(&emitted).expect("the emitted definition canonicalizes on its own");
    validator_for(&emitted).expect("the emitted definition compiles on its own");
}

// A member the receiver leaves out of its own equality class is no refutation: under Draft 4 the
// intersection pins a whole number to its integer spelling, which the member outlives by matching
// the decimal one too - and the member itself is admitted all the same.
#[test_case(&json!({"not": {"type": "integer", "minimum": -2}}), &json!({"enum": [-3]}), Containment::Yes; "a member below a negated floor")]
#[test_case(&json!({"not": {"type": "integer", "maximum": 2}}), &json!({"enum": [5]}), Containment::Yes; "a member above a negated ceiling")]
#[test_case(&json!({"not": {"type": "integer", "minimum": 3}}), &json!({"enum": [5]}), Containment::No; "a member the receiver turns away")]
fn draft_4_covers_a_member_of_a_negated_integer_window(
    left: &Value,
    right: &Value,
    expected: Containment,
) {
    let canonicalize = |schema: &Value| {
        options()
            .with_draft(Draft::Draft4)
            .canonicalize(schema)
            .expect("canonicalizes")
    };
    let left = canonicalize(left);
    let right = canonicalize(right);
    assert_eq!(left.covers(&right).expect("covers"), expected);
}

// The root a result keeps names the targets that root reads, so pruning against the result alone
// would leave it pointing at a definition the document no longer holds.
#[test]
fn a_kept_root_keeps_the_definitions_it_names() {
    let left = canonicalize(&json!({
        "$defs": {"D": {"type": "string"}},
        "type": "object",
        "properties": {"q": {"$ref": "#/$defs/D"}, "r": {"$ref": "#"}}
    }))
    .expect("canonicalizes");
    let right = canonicalize(&json!({"type": "integer"})).expect("canonicalizes");

    let result = left.intersect(&right).expect("intersects");
    let root = result.definition("#").expect("the root is a definition");
    validator_for(&root.to_json_schema()).expect("the emitted root resolves its own pointers");
}

// `#` names a root, not a whole document: two handles sharing one root bind it to the same node
// however their maps differ, which is what chaining a set operation with its own operand does.
#[test]
fn a_result_combines_with_its_own_operand() {
    let recursive = json!({
        "type": "object",
        "properties": {"r": {"$ref": "#"}, "a": {"$ref": "#/$defs/A"}},
        "$defs": {"A": {"type": "string"}}
    });
    let left = canonicalize(&recursive).expect("canonicalizes");
    let right = canonicalize(&recursive).expect("canonicalizes");

    let united = left.union(&right).expect("unions");
    assert_eq!(united.union(&left).expect("unions"), united);
    assert_eq!(united.covers(&left).expect("covers"), Containment::Yes);
}

// Two documents binding `#` to different roots stay incomparable: the pointer cannot name both.
#[test]
fn operands_reading_different_document_roots_are_refused() {
    let left = canonicalize(&json!({
        "type": "object", "properties": {"r": {"$ref": "#"}}, "maxProperties": 1
    }))
    .expect("canonicalizes");
    let right = canonicalize(&json!({
        "type": "object", "properties": {"r": {"$ref": "#"}}, "minProperties": 1
    }))
    .expect("canonicalizes");

    assert!(matches!(
        left.union(&right),
        Err(CanonicalizationError::IncompatibleOperands(
            OperandMismatch::DocumentRoots
        ))
    ));
}

// Parsing drops what a document never names, so an entry neither version keeps cannot collide.
#[test]
fn a_definition_the_document_never_names_is_not_carried() {
    let with_spare = |spare: Value| {
        canonicalize(&json!({
            "$defs": {"Used": {"type": "string"}, "Spare": spare},
            "$ref": "#/$defs/Used"
        }))
        .expect("canonicalizes")
    };
    let old = with_spare(json!({"type": "integer"}));
    let new = with_spare(json!({"type": "boolean"}));
    assert_eq!(old.definitions().len(), 1, "the spare entry is unreachable");

    assert_eq!(
        old.subtract(&new).expect("subtracts").satisfiability(),
        Satisfiability::No
    );
}

// A key renamed apart takes every key that refers to it along, or those would collide in turn -
// here neither node names the edited entry, and both reach it through the root.
#[test]
fn a_rename_carries_the_keys_that_refer_to_it() {
    let version = |ty: &str| {
        canonicalize(&json!({
            "$defs": {"alias": {"$ref": "#/$defs/x"}, "x": {"type": ty}},
            "$ref": "#/$defs/alias"
        }))
        .expect("canonicalizes")
    };
    let old = version("integer");
    let new = version("string");

    // The two entries accept different objects, so neither covers the other.
    assert_eq!(old.covers(&new).expect("covers"), Containment::No);
    assert_eq!(new.covers(&old).expect("covers"), Containment::No);
    let difference = old.subtract(&new).expect("subtracts");
    let (taken, removed, left_over) = (
        validator_for(&old.to_json_schema()).expect("builds"),
        validator_for(&new.to_json_schema()).expect("builds"),
        validator_for(&difference.to_json_schema()).expect("builds"),
    );
    for instance in [json!(1), json!("a"), json!(null), json!({})] {
        assert_eq!(
            taken.is_valid(&instance) && !removed.is_valid(&instance),
            left_over.is_valid(&instance),
            "the difference disagrees on {instance}"
        );
    }
}

// A `$defs` key is a name private to its document, so two versions may bind it differently: the
// clashing key is renamed apart and the operation answers, which is what editing a shared component
// looks like.
#[test]
fn an_edited_definition_both_versions_carry_is_renamed_apart() {
    let old = json!({"$defs": {"Name": {"type": "string"}}, "$ref": "#/$defs/Name"});
    let new = json!({
        "$defs": {"Name": {"type": "string", "maxLength": 50}}, "$ref": "#/$defs/Name"
    });
    let (before, after) = (
        canonicalize(&old).expect("canonicalizes"),
        canonicalize(&new).expect("canonicalizes"),
    );

    // The strings the edit turned away, and nothing gained the other way round.
    assert_eq!(
        before.subtract(&after).expect("subtracts").to_json_schema(),
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "string",
            "minLength": 51
        })
    );
    assert_eq!(
        after.subtract(&before).expect("subtracts").satisfiability(),
        Satisfiability::No
    );
    assert_eq!(before.covers(&after).expect("covers"), Containment::Yes);
    assert_eq!(after.covers(&before).expect("covers"), Containment::No);
    // Renaming is decided by the two bodies, so neither operand's position changes the answer.
    assert_eq!(
        before.union(&after).expect("unions"),
        after.union(&before).expect("unions")
    );
}

// Operands sharing no value, and operands one of which holds the other, have a difference the form
// spells outright - whatever asking for the other side's complement would have cost.
#[test_case(
    &json!({"type": "boolean"}),
    &json!({"type": "object", "patternProperties": {"^a": {"type": "string"}}}),
    &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "boolean"});
    "operands sharing no value"
)]
#[test_case(
    &json!({"type": "object", "patternProperties": {"^a": {"type": "string"}}}),
    &json!({"type": "object"}),
    &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "not": {}});
    "an operand the other one holds"
)]
fn a_difference_needing_no_complement_is_spelled_outright(
    left: &Value,
    right: &Value,
    expected: &Value,
) {
    let left = canonicalize(left).expect("canonicalizes");
    let right = canonicalize(right).expect("canonicalizes");
    assert_eq!(
        &left.subtract(&right).expect("subtracts").to_json_schema(),
        expected
    );
}

// A pointer and the schema it references accept the same values, so both give the same answer.
#[test]
fn satisfiability_reads_through_a_reference() {
    let pointer = canonicalize(&json!({
        "$defs": {"s": {"type": "string"}}, "$ref": "#/$defs/s"
    }))
    .expect("canonicalizes");
    assert_eq!(pointer.satisfiability(), Satisfiability::Yes);

    // A target on a cycle is never read through: the walk would not terminate.
    let recursive = canonicalize(&json!({
        "$defs": {"r": {"anyOf": [{"$ref": "#/$defs/r"}, {"type": "string"}]}},
        "$ref": "#/$defs/r"
    }))
    .expect("canonicalizes");
    assert_eq!(recursive.satisfiability(), Satisfiability::Unknown);
}

// A `true`/`false` answer is returned directly: hiding it behind a `$ref`
// reads as undecided to every emptiness check above it.
#[test]
fn an_intersection_a_pointer_decides_keeps_its_answer_readable() {
    let everything = canonicalize(&json!(true)).expect("canonicalizes");
    let pointer =
        canonicalize(&json!({"$defs": {"X": true}, "$ref": "#/$defs/X"})).expect("canonicalizes");

    // `true and A` is `A`, so the pointer is returned unchanged - and reads as decided all the
    // same, since every query reads through the schema it names.
    let met = everything.intersect(&pointer).expect("intersects");
    assert_eq!(met.kind(), CanonicalKind::Reference);
    assert_eq!(met.satisfiability(), Satisfiability::Yes);
    assert_eq!(met, pointer.intersect(&everything).expect("intersects"));
}

// Cycle membership is a property of the graph, so renaming the keys cannot change which targets a
// walk reads through - and with it, what the canonical form of the document is.
#[test]
fn a_cycle_carrying_a_chord_is_found_whatever_its_keys_are_called() {
    let document = |a: &str, b: &str, c: &str| {
        json!({
            "$defs": {
                a: {"anyOf": [{"$ref": format!("#/$defs/{b}")}, {"$ref": format!("#/$defs/{c}")}]},
                b: {"type": "object", "properties": {"p": {"$ref": format!("#/$defs/{c}")}}},
                c: {"type": "object", "properties": {"q": {"$ref": format!("#/$defs/{a}")}}}
            },
            "$ref": format!("#/$defs/{b}")
        })
    };
    let objects = canonicalize(&json!({"type": "object"})).expect("canonicalizes");
    let named = canonicalize(&document("a", "b", "c")).expect("canonicalizes");
    let renamed = canonicalize(&document("m", "n", "l")).expect("canonicalizes");

    assert_eq!(
        objects.covers(&named).expect("covers"),
        objects.covers(&renamed).expect("covers")
    );
}

// A walk that could only approximate an intersection carries no verdict: `covers` reads each of its
// three answers only where that answer's own walk finished exactly.
#[test]
fn coverage_over_an_intersection_the_form_cannot_spell_rests_on_a_value_or_declines() {
    let shielded = canonicalize(&json!({
        "type": "object",
        "properties": {"ab": {"minLength": 1}},
        "additionalProperties": {"type": "string"}
    }))
    .expect("canonicalizes");
    let patterns = canonicalize(&json!({
        "type": "object",
        "patternProperties": {"^a": {"maxLength": 9}}
    }))
    .expect("canonicalizes");

    // Their meet is what the form cannot spell.
    assert!(matches!(
        shielded.intersect(&patterns),
        Err(CanonicalizationError::UnsupportedResult)
    ));
    // Refused on a value the argument takes and the receiver does not: `{"b": 1}`, which the shield
    // turns away for not being a string. The other way round rests on the meet, which is what the
    // form cannot spell, so it stays undecided.
    assert_eq!(shielded.covers(&patterns).expect("covers"), Containment::No);
    assert_eq!(
        patterns.covers(&shielded).expect("covers"),
        Containment::Unknown
    );
    // The complement is built on a context of its own, and one that approximated is no complement
    // to subtract with.
    assert!(matches!(
        shielded.subtract(&patterns),
        Err(CanonicalizationError::UnsupportedResult)
    ));
    assert!(matches!(
        patterns.negate(),
        Err(CanonicalizationError::UnsupportedResult)
    ));
}

// `#` names the document root, which is no entry of the map: reading through the pointer is reading
// the root itself.
#[test]
fn satisfiability_reads_through_a_pointer_at_the_document_root() {
    let canonical = canonicalize(&json!({
        "type": "object",
        "properties": {"p": {"$ref": "#/$defs/x"}},
        "$defs": {"x": {"$ref": "#"}}
    }))
    .expect("canonicalizes");

    let alias = canonical
        .definition("#/$defs/x")
        .expect("the alias is a definition");
    assert_eq!(alias.kind(), CanonicalKind::Reference);
    assert_eq!(alias.satisfiability(), Satisfiability::Yes);
}

// De Morgan leaves `#` naming the document the complement was written in, so it stays there rather
// than binding the pointer to itself.
#[test]
fn a_complement_reading_the_document_root_stays_in_its_document() {
    let canonical = canonicalize(&json!({
        "type": "object",
        "properties": {"p": {"$ref": "#"}},
        "required": ["p"],
        "maxProperties": 1
    }))
    .expect("canonicalizes");

    let complement = canonical.negate().expect("negates");
    let emitted = complement.to_json_schema();
    validator_for(&emitted).expect("the complement resolves its own pointers");
    assert_eq!(
        complement.negate().expect("negates back"),
        canonical,
        "double negation restores the schema"
    );
}

// Two nodes written the same way and reading the same bodies are one schema whichever documents
// they came out of - and two reading different bodies are not.
#[test]
fn nodes_are_ordered_by_the_part_of_their_document_they_read() {
    let pointer = |target: Value| {
        canonicalize(&json!({"$ref": "#/$defs/x", "$defs": {"x": target}})).expect("canonicalizes")
    };
    let strings = pointer(json!({"type": "string"}));
    let integers = pointer(json!({"type": "integer"}));

    assert_eq!(strings.cmp(&strings.clone()), Ordering::Equal);
    assert_ne!(strings.cmp(&integers), Ordering::Equal);
    assert_ne!(strings, integers);
}

// Everything is the whole union, whatever it is united with.
#[test]
fn a_branch_admitting_everything_is_the_whole_union() {
    let everything = canonicalize(&json!(true)).expect("canonicalizes");
    let strings = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    assert_eq!(everything.union(&strings).expect("unions"), everything);
}

// A shield governs only the keys nothing else declares: where the key constraint admits nothing the
// pattern map does not match, no key is left for it.
#[test]
fn a_shield_no_key_can_reach_says_nothing() {
    let patterned = json!({
        "type": "object",
        "propertyNames": {"pattern": "^a"},
        "patternProperties": {"^a": {"type": "integer"}},
        "additionalProperties": {"type": "string"},
        "maxProperties": 1
    });
    let canonical = canonicalize(&patterned).expect("canonicalizes");
    assert!(
        canonical
            .to_json_schema()
            .get("additionalProperties")
            .is_none(),
        "the shield is dropped: {}",
        canonical.to_json_schema()
    );

    // A pattern map missing some admitted key leaves the shield something to govern, so it stays.
    let partly_patterned = canonicalize(&json!({
        "type": "object",
        "propertyNames": {"pattern": "^a"},
        "patternProperties": {"^ab": {"type": "integer"}},
        "additionalProperties": {"type": "string"}
    }))
    .expect("canonicalizes");
    assert!(
        partly_patterned
            .to_json_schema()
            .get("additionalProperties")
            .is_some(),
        "the shield still governs a key: {}",
        partly_patterned.to_json_schema()
    );

    // The meet with a shield of its own answers about the values both sides take.
    let integers = canonicalize(&json!({
        "type": "object", "additionalProperties": {"type": "integer"}, "minProperties": 1
    }))
    .expect("canonicalizes");
    let met = integers.intersect(&canonical).expect("intersects");
    let (left, right, both) = (
        validator_for(&patterned).expect("builds"),
        validator_for(&integers.to_json_schema()).expect("builds"),
        validator_for(&met.to_json_schema()).expect("builds"),
    );
    for instance in [
        json!({"a": 1}),
        json!({"ab": 2}),
        json!({"a": "s"}),
        json!({}),
        json!({"b": 1}),
    ] {
        assert_eq!(
            left.is_valid(&instance) && right.is_valid(&instance),
            both.is_valid(&instance),
            "the meet disagrees on {instance}\n  meet = {}",
            met.to_json_schema()
        );
    }
}

// What a purely in-place reference cycle accepts is left to the validator, not the algebra, so
// the choice is barred whole rather than taken apart - the way its `anyOf` and `allOf` siblings are.
#[test]
fn a_choice_reading_its_own_target_is_barred_whole() {
    let ill_founded = canonicalize(&json!({
        "$defs": {"x": {"oneOf": [{"$ref": "#/$defs/x"}, {"type": "object", "required": ["a"]}]}},
        "$ref": "#/$defs/x"
    }))
    .expect("canonicalizes");
    let barred = ill_founded.negate().expect("negates");
    let (whole, complement) = (
        validator_for(&ill_founded.to_json_schema()).expect("builds"),
        validator_for(&barred.to_json_schema()).expect("builds"),
    );
    for instance in [
        json!({"a": 1}),
        json!({"b": 2}),
        json!("x"),
        json!(5),
        json!(null),
    ] {
        assert_eq!(
            !whole.is_valid(&instance),
            complement.is_valid(&instance),
            "the complement disagrees on {instance}\n  complement = {}",
            barred.to_json_schema()
        );
    }
    // A difference does not replace the root, so its complement may stay symbolic under `not` -
    // which is exact, and leaves what the cycle accepts to the validator that walks it.
    let anything = canonicalize(&json!(true)).expect("canonicalizes");
    let difference = anything.subtract(&ill_founded).expect("subtracts");
    let (source, complement) = (
        validator_for(&ill_founded.to_json_schema()).expect("builds"),
        validator_for(&difference.to_json_schema()).expect("builds"),
    );
    for instance in [
        json!({"a": 1}),
        json!({"b": 2}),
        json!("x"),
        json!(5),
        json!(null),
    ] {
        assert_eq!(
            !source.is_valid(&instance),
            complement.is_valid(&instance),
            "the difference disagrees on {instance}"
        );
    }

    // A choice naming a target off the negation path still has one.
    let guarded = canonicalize(&json!({
        "$defs": {"t": {"type": "string"}},
        "oneOf": [{"$ref": "#/$defs/t"}, {"type": "integer"}]
    }))
    .expect("canonicalizes");
    guarded.negate().expect("negates");
}

// A side the other leaves whole is returned as the pointer, whichever exit reached it.
#[test]
fn an_intersection_hands_a_pointer_back_from_every_exit() {
    let document = json!({
        "$defs": {"s": {"type": "string", "minLength": 2}},
        "type": "object",
        "properties": {"a": {"$ref": "#/$defs/s"}, "b": {"$ref": "#/$defs/s"}}
    });
    let canonical = canonicalize(&document).expect("canonicalizes");
    let CanonicalView::Object(view) = canonical.view() else {
        panic!("expected an Object view");
    };
    let pointer = view.properties.get("a").expect("property a").clone();
    let strings = canonicalize(&json!({"type": "string"})).expect("canonicalizes");

    // The pointer decides the pair against a wider schema, and the target stays shared.
    let met = strings.intersect(&pointer).expect("intersects");
    assert_eq!(met.kind(), CanonicalKind::Reference);
    assert_eq!(met, pointer.intersect(&strings).expect("intersects"));
}

// `#` names a root, so two nodes written the same way in documents that bind it differently are not
// the same schema.
#[test]
fn nodes_binding_different_roots_are_told_apart() {
    let version = |extra: Value| {
        let mut root = json!({
            "$defs": {"e": {"$ref": "#"}},
            "type": "object",
            "properties": {"e": {"$ref": "#/$defs/e"}}
        });
        root.as_object_mut()
            .expect("object")
            .extend(extra.as_object().expect("object").clone());
        canonicalize(&root)
            .expect("canonicalizes")
            .definition("#/$defs/e")
            .expect("the entry is a definition")
    };
    let narrow = version(json!({"maxProperties": 1}));
    let wide = version(json!({"minProperties": 1}));

    assert_eq!(narrow.kind(), CanonicalKind::Reference);
    assert_ne!(narrow, wide);
    assert_ne!(narrow.cmp(&wide), Ordering::Equal);
    assert_eq!(narrow, narrow.clone());
}

// Extracting a repeated subschema into `$defs` changes no value, and the comparison says so - which
// is what makes the difference usable as a review of the refactoring.
#[test_case(
    &json!({
        "$defs": {"User": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}},
        "type": "object",
        "properties": {"user": {"$ref": "#/$defs/User"}, "admin": {"$ref": "#/$defs/User"}}
    });
    "both sites extracted"
)]
#[test_case(
    &json!({
        "$defs": {"User": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}},
        "type": "object",
        "properties": {
            "user": {"$ref": "#/$defs/User"},
            "admin": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}
        }
    });
    "one site extracted"
)]
#[test_case(
    &json!({
        "$defs": {
            "User": {"$ref": "#/$defs/Base"},
            "Base": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}
        },
        "type": "object",
        "properties": {"user": {"$ref": "#/$defs/User"}, "admin": {"$ref": "#/$defs/User"}}
    });
    "extracted behind a pointer chain"
)]
fn extracting_a_subschema_changes_no_value(extracted: &Value) {
    let inline = canonicalize(&json!({
        "type": "object",
        "properties": {
            "user": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]},
            "admin": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}
        }
    }))
    .expect("canonicalizes");
    let extracted = canonicalize(extracted).expect("canonicalizes");

    assert_eq!(
        inline.covers(&extracted).expect("compares"),
        Containment::Yes
    );
    assert_eq!(
        extracted.covers(&inline).expect("compares"),
        Containment::Yes
    );
    for (name, difference) in [
        ("lost", inline.subtract(&extracted)),
        ("gained", extracted.subtract(&inline)),
    ] {
        assert_eq!(
            difference.expect("subtracts").satisfiability(),
            Satisfiability::No,
            "the refactoring {name} values"
        );
    }
}

// An extraction that quietly edited the schema on the way is never reported as no change. The
// engine may decline to decide - a complement it cannot spell leaves the difference `Unknown` - but
// it must not answer that nothing moved.
#[test_case(
    &json!({"type": "object", "properties": {"id": {"type": "integer"}}}),
    &[json!({"user": {}, "admin": {}})];
    "the required key was dropped"
)]
#[test_case(
    &json!({"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]}),
    &[json!({"user": {"id": 1}, "admin": {"id": 1}}), json!({"user": {"id": "a"}, "admin": {"id": "a"}})];
    "the property type was changed"
)]
fn an_extraction_that_edited_the_schema_is_never_reported_as_no_change(
    body: &Value,
    moved: &[Value],
) {
    let source = json!({
        "type": "object",
        "properties": {
            "user": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]},
            "admin": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}
        }
    });
    let edited = json!({
        "$defs": {"User": body},
        "type": "object",
        "properties": {"user": {"$ref": "#/$defs/User"}, "admin": {"$ref": "#/$defs/User"}}
    });
    // The instances prove the two really do accept different values.
    let (before, after) = (
        validator_for(&source).expect("builds"),
        validator_for(&edited).expect("builds"),
    );
    assert!(
        moved
            .iter()
            .any(|instance| before.is_valid(instance) != after.is_valid(instance)),
        "the case moves no value"
    );

    let inline = canonicalize(&source).expect("canonicalizes");
    let extracted = canonicalize(&edited).expect("canonicalizes");
    // Undecided is allowed here; agreeing that nothing moved is not.
    assert_ne!(
        (
            inline.covers(&extracted).expect("compares"),
            extracted.covers(&inline).expect("compares")
        ),
        (Containment::Yes, Containment::Yes)
    );
    // An empty difference is a claim that the change turned nothing away, so every moved value has
    // to be one the difference itself keeps.
    for difference in [inline.subtract(&extracted), extracted.subtract(&inline)]
        .into_iter()
        .flatten()
        .filter(|difference| difference.satisfiability() == Satisfiability::No)
    {
        let empty = validator_for(&difference.to_json_schema()).expect("builds");
        for instance in moved {
            assert!(
                !before.is_valid(instance) || after.is_valid(instance) || empty.is_valid(instance),
                "a value the change turns away fell out of the difference: {instance}"
            );
        }
    }
}

// The same bound reached through a difference the caller never wrote out.
#[test]
fn a_bound_a_difference_computed_decides_nothing() {
    let wide =
        canonicalize(&json!({"type": "string", "maxLength": 18_446_744_073_709_551_614_u64}))
            .expect("canonicalizes");
    let patterned =
        canonicalize(&json!({"type": "string", "pattern": "^a"})).expect("canonicalizes");
    assert_eq!(
        wide.covers(&patterned).expect("covers"),
        Containment::Unknown
    );
}

// `A op A` is `A` whatever `A` is spelled like, and the constants leave their operand alone. The
// identities are answered on the operands the caller holds, so reading a pointer through cannot
// fold the answer further than the node handed in.
#[test_case(
    &json!({"type": "object", "properties": {"q": {"anyOf": [{"type": "object", "required": ["a"]},
                                                             {"$ref": "#/$defs/t"}]}},
            "$defs": {"t": {"type": "integer", "multipleOf": 2}}});
    "a pointer under a union under a property"
)]
#[test_case(
    &json!({"$defs": {"x": {"anyOf": [{"type": "string"}, {"type": "integer"}]}}, "$ref": "#/$defs/x"});
    "a pointer at a union"
)]
#[test_case(&json!({"type": "string", "minLength": 2}); "a plain leaf")]
fn the_set_identities_keep_the_form_the_caller_holds(schema: &Value) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    let nothing = canonicalize(&json!(false)).expect("canonicalizes");
    let everything = canonicalize(&json!(true)).expect("canonicalizes");

    for (law, result) in [
        ("a | a", canonical.union(&canonical)),
        ("a & a", canonical.intersect(&canonical)),
        ("a | nothing", canonical.union(&nothing)),
        ("a & everything", canonical.intersect(&everything)),
        ("a \\ nothing", canonical.subtract(&nothing)),
    ] {
        assert_eq!(result.expect("holds"), canonical, "`{law}` does not hold");
    }
    assert_eq!(
        canonical
            .subtract(&canonical)
            .expect("subtracts")
            .satisfiability(),
        Satisfiability::No
    );
    // The definitions travel with the form, so an identity never sheds the targets it still names.
    assert_eq!(
        canonical
            .intersect(&canonical)
            .expect("holds")
            .definitions()
            .len(),
        canonical.definitions().len()
    );
}

// Two pointers at one body are one value set, so the difference answers the same way round either
// way - the meet keeps whichever spelling is canonical, which need not be the operand's own.
#[test]
fn a_difference_between_two_names_for_one_body_is_empty_either_way() {
    let named = |uri: &str| {
        canonicalize(&json!({
            "$defs": {"a": {"type": "string", "minLength": 2}, "b": {"type": "string", "minLength": 2}},
            "$ref": uri
        }))
        .expect("canonicalizes")
    };
    let (first, second) = (named("#/$defs/a"), named("#/$defs/b"));

    for difference in [first.subtract(&second), second.subtract(&first)] {
        assert_eq!(
            difference.expect("subtracts").satisfiability(),
            Satisfiability::No
        );
    }
}

// A pattern entry declares the keys it matches, so a barring shield does not close the map down to
// the named keys alone - those matches would be excluded with it.
#[test]
fn a_barring_shield_beside_a_pattern_map_keeps_the_keys_the_patterns_match() {
    let shielded = json!({"type": "object", "additionalProperties": {"type": "integer"}});
    let patterned = json!({
        "type": "object",
        "additionalProperties": {"type": "string"},
        "patternProperties": {"^a": {"type": "integer"}}
    });
    let met = canonicalize(&shielded)
        .expect("canonicalizes")
        .intersect(&canonicalize(&patterned).expect("canonicalizes"))
        .expect("intersects");

    let (left, right, both) = (
        validator_for(&shielded).expect("builds"),
        validator_for(&patterned).expect("builds"),
        validator_for(&met.to_json_schema()).expect("builds"),
    );
    for instance in [
        json!({"a": 2}),
        json!({"b": 2}),
        json!({"a": "s"}),
        json!({}),
    ] {
        assert_eq!(
            left.is_valid(&instance) && right.is_valid(&instance),
            both.is_valid(&instance),
            "the meet disagrees on {instance}\n  meet = {}",
            met.to_json_schema()
        );
    }
}

// A demand needs a key present to break its rule, and a leaf whose every admitted key is barred
// leaves no slot for one - so it admits nothing rather than the empty object. The two only meet
// where the ceiling drops below what the slot check saw when it read it, which is what this pair
// reaches: the difference carries a demand from the complement onto a leaf a pattern entry closed.
#[test]
fn a_difference_carrying_a_demand_onto_a_closed_leaf_agrees_with_the_validators() {
    let named_b = json!({"type": "object", "propertyNames": {"enum": ["b"]}});
    let closed = json!({
        "anyOf": [
            {"type": "object", "properties": {"a": {"type": "null"}}, "required": ["a"],
             "additionalProperties": false},
            {"type": "object", "patternProperties": {"^a": false}}
        ]
    });
    let difference = canonicalize(&closed)
        .expect("canonicalizes")
        .subtract(&canonicalize(&named_b).expect("canonicalizes"))
        .expect("subtracts");

    let (taken, removed, left_over) = (
        validator_for(&closed).expect("builds"),
        validator_for(&named_b).expect("builds"),
        validator_for(&difference.to_json_schema()).expect("builds"),
    );
    for instance in [
        json!({"a": null}),
        json!({"a": 1}),
        json!({"b": 1}),
        json!({"b": null}),
        json!({"c": 1}),
        json!({}),
    ] {
        assert_eq!(
            taken.is_valid(&instance) && !removed.is_valid(&instance),
            left_over.is_valid(&instance),
            "the difference disagrees on {instance}\n  difference = {}",
            difference.to_json_schema()
        );
    }
}

// Under Draft 4 a member shares its equality class with a spelling the `integer` type turns away:
// `{"a": 1}` and `{"a": 1.0}` are one member, and a schema requiring an integer takes only the
// first. A demand that the value fail such a schema is met by half the class, so the member is
// admitted in part - reading it as met outright would hand back a difference holding a value both
// operands accept.
#[test]
fn a_draft_4_member_half_a_demand_admits_is_not_admitted_whole() {
    let taken = json!({"$defs": {"t": {"minimum": -2, "type": "integer"}}, "enum": [{"a": 1}]});
    let removed = json!({
        "$defs": {"t": {"minimum": -2, "type": "integer"}},
        "type": "object",
        "additionalProperties": {"$ref": "#/$defs/t"}
    });
    let canonicalize = |schema: &Value| {
        options()
            .with_draft(Draft::Draft4)
            .canonicalize(schema)
            .expect("canonicalizes")
    };
    let build = |schema: &Value| {
        jsonschema::options()
            .with_draft(Draft::Draft4)
            .build(schema)
            .expect("builds")
    };
    let difference = canonicalize(&taken)
        .subtract(&canonicalize(&removed))
        .expect("subtracts");

    let (taken, removed, left_over) = (
        build(&taken),
        build(&removed),
        build(&difference.to_json_schema()),
    );
    for instance in [
        json!({"a": 1}),
        json!({"a": 1.0}),
        json!({"a": "x"}),
        json!({}),
    ] {
        assert_eq!(
            taken.is_valid(&instance) && !removed.is_valid(&instance),
            left_over.is_valid(&instance),
            "the difference disagrees on {instance}\n  difference = {}",
            difference.to_json_schema()
        );
    }
}

// Renaming rewrites whatever holds the reference, so the answer is the same wherever in a document
// the edited entry is reached from.
#[test_case(
    &json!({"$defs": {"held": {"not": {"$ref": "#/$defs/x"}}, "x": {"type": "TYPE"}},
            "$ref": "#/$defs/held"});
    "reached through a complement"
)]
#[test_case(
    &json!({"$defs": {"held": {"type": "object", "properties": {"p": {"$ref": "#/$defs/x"}}},
                      "x": {"type": "TYPE"}},
            "$ref": "#/$defs/held"});
    "reached through a property"
)]
#[test_case(
    &json!({"$defs": {"held": {"type": "object", "additionalProperties": {"$ref": "#/$defs/x"}},
                      "x": {"type": "TYPE"}},
            "$ref": "#/$defs/held"});
    "reached through a shield"
)]
#[test_case(
    &json!({"$defs": {"held": {"not": {"type": "object",
                                       "additionalProperties": {"$ref": "#/$defs/x"}}},
                      "x": {"type": "TYPE"}},
            "$ref": "#/$defs/held"});
    "reached through a demand a complement left"
)]
#[test_case(
    &json!({"$defs": {"held": {"type": "array", "contains": {"$ref": "#/$defs/x"}},
                      "x": {"type": "TYPE"}},
            "$ref": "#/$defs/held"});
    "reached through an array demand"
)]
#[test_case(
    &json!({"$defs": {"held": {"anyOf": [{"$ref": "#/$defs/x"}, {"type": "null"}]},
                      "x": {"type": "TYPE"}},
            "$ref": "#/$defs/held"});
    "reached through a union branch"
)]
fn an_edited_entry_is_renamed_apart_wherever_it_is_reached_from(shape: &Value) {
    let version = |ty: &str| {
        let source: Value =
            serde_json::from_str(&shape.to_string().replace("TYPE", ty)).expect("a schema");
        (
            canonicalize(&source).expect("canonicalizes"),
            validator_for(&source).expect("builds"),
        )
    };
    let (old, taken) = version("integer");
    let (new, removed) = version("string");

    // Both maps bind `#/$defs/x`, and `#/$defs/held` reaches it: renaming one carries the other.
    let difference = old.subtract(&new).expect("subtracts");
    let left_over = validator_for(&difference.to_json_schema()).expect("builds");
    for instance in [
        json!(1),
        json!("a"),
        json!(null),
        json!({"p": 1}),
        json!([1]),
        json!({}),
    ] {
        assert_eq!(
            taken.is_valid(&instance) && !removed.is_valid(&instance),
            left_over.is_valid(&instance),
            "the difference disagrees on {instance}\n  difference = {}",
            difference.to_json_schema()
        );
    }
    // Which side gives way is read off the two bodies, so operand order changes nothing.
    assert_eq!(
        old.union(&new).expect("unions"),
        new.union(&old).expect("unions")
    );
}

// A resource is named by its URI rather than by a key private to one document, so two documents
// resolving one to different schemas disagree about the resource itself and cannot be combined.
#[test]
fn operands_resolving_one_resource_two_ways_are_refused() {
    fn shared(ty: &str) -> Value {
        json!({"$id": "https://example.com/shared", "$anchor": "v", "type": ty})
    }
    fn prepared(source: &Value) -> Registry<'_> {
        Registry::new()
            .add("https://example.com/shared", source)
            .expect("the resource URI is valid")
            .prepare()
            .expect("the registry prepares")
    }

    let (first, second) = (shared("string"), shared("integer"));
    let (left_registry, right_registry) = (prepared(&first), prepared(&second));
    let canonicalize = |registry: &Registry<'_>| {
        options()
            .with_registry(registry)
            .canonicalize(&json!({"$ref": "https://example.com/shared#v"}))
            .expect("canonicalizes")
    };
    let left = canonicalize(&left_registry);
    let right = canonicalize(&right_registry);

    for outcome in [
        left.intersect(&right).map(|_| ()),
        left.union(&right).map(|_| ()),
        left.subtract(&right).map(|_| ()),
        left.covers(&right).map(|_| ()),
    ] {
        assert!(matches!(
            outcome,
            Err(CanonicalizationError::IncompatibleOperands(
                OperandMismatch::Definitions
            ))
        ));
    }
}

// A private name marks a body the document wrote itself, which is what makes renaming it apart
// safe. Pruning drops the body; a marker left behind would let the next operation adopt a
// retrieved resource under that name, and two documents reading that resource differently would
// then rename apart instead of refusing.
#[test]
fn a_pruned_private_name_does_not_launder_a_retrieved_resource() {
    fn shared(ty: &str) -> Value {
        json!({"$id": "https://example.com/shared", "type": "object",
               "properties": {"self": {"$ref": "https://example.com/shared"}, "v": {"type": ty}}})
    }
    fn retrieved(source: &Value) -> CanonicalSchema {
        let registry = Registry::new()
            .add("https://example.com/shared", source)
            .expect("the resource URI is valid")
            .prepare()
            .expect("the registry prepares");
        options()
            .with_registry(&registry)
            .canonicalize(&json!({"$ref": "https://example.com/shared"}))
            .expect("canonicalizes")
    }

    let (first, second) = (shared("string"), shared("integer"));
    assert!(matches!(
        retrieved(&first).union(&retrieved(&second)),
        Err(CanonicalizationError::IncompatibleOperands(
            OperandMismatch::Definitions
        ))
    ));

    // A document writing that URI itself, met down to a form naming it no longer.
    let written = canonicalize(&json!({
        "$defs": {"x": {"$id": "https://example.com/shared", "type": "boolean"}},
        "anyOf": [{"$ref": "https://example.com/shared"}, {"type": "integer"}]
    }))
    .expect("canonicalizes");
    let pruned = written
        .intersect(&canonicalize(&json!({"type": "integer"})).expect("canonicalizes"))
        .expect("intersects");
    assert_eq!(pruned.definitions().count(), 0);

    let adopted = |source: &Value| pruned.union(&retrieved(source)).expect("unions");
    assert!(matches!(
        adopted(&first).union(&adopted(&second)),
        Err(CanonicalizationError::IncompatibleOperands(
            OperandMismatch::Definitions
        ))
    ));
}

// The rename follows the references back, so a key reached from two places is renamed once, and a
// node holding no renamed reference is handed back as it was.
#[test]
fn a_rename_reaches_every_holder_once() {
    let version = |ty: &str| {
        let source = json!({
            "$defs": {
                "x": {"type": ty},
                "left": {"type": "object", "properties": {"p": {"$ref": "#/$defs/x"}}},
                "right": {"type": "array", "items": {"$ref": "#/$defs/x"}},
                "both": {"anyOf": [{"$ref": "#/$defs/left"}, {"$ref": "#/$defs/right"}]},
                // Reached from nothing that is renamed, so it is left exactly as it was.
                "apart": {"not": {"type": "null"}}
            },
            "anyOf": [{"$ref": "#/$defs/both"}, {"$ref": "#/$defs/apart"}]
        });
        (
            canonicalize(&source).expect("canonicalizes"),
            validator_for(&source).expect("builds"),
        )
    };
    let (old, taken) = version("integer");
    let (new, removed) = version("string");

    let difference = old.subtract(&new).expect("subtracts");
    let left_over = validator_for(&difference.to_json_schema()).expect("builds");
    for instance in [
        json!({"p": 1}),
        json!({"p": "a"}),
        json!([1]),
        json!(["a"]),
        json!(1),
        json!(null),
    ] {
        assert_eq!(
            taken.is_valid(&instance) && !removed.is_valid(&instance),
            left_over.is_valid(&instance),
            "the difference disagrees on {instance}\n  difference = {}",
            difference.to_json_schema()
        );
    }
}

// A demand a complement leaves behind carries a reference of its own, which the rename rewrites
// like any other.
#[test]
fn a_rename_rewrites_the_demands_a_complement_left() {
    let version = |ty: &str| {
        let source = json!({
            "$defs": {"keys": {"type": "string", "pattern": ty},
                      "held": {"not": {"type": "object", "propertyNames": {"$ref": "#/$defs/keys"}}}},
            "$ref": "#/$defs/held"
        });
        (
            canonicalize(&source).expect("canonicalizes"),
            validator_for(&source).expect("builds"),
        )
    };
    let (old, taken) = version("^a");
    let (new, removed) = version("^b");

    let difference = old.subtract(&new).expect("subtracts");
    let left_over = validator_for(&difference.to_json_schema()).expect("builds");
    for instance in [json!({"a": 1}), json!({"b": 1}), json!({}), json!(1)] {
        assert_eq!(
            taken.is_valid(&instance) && !removed.is_valid(&instance),
            left_over.is_valid(&instance),
            "the difference disagrees on {instance}\n  difference = {}",
            difference.to_json_schema()
        );
    }
}

// A node holding no renamed reference comes back exactly as it was, whatever shape holds it.
#[test]
fn a_rename_leaves_the_entries_it_does_not_reach_alone() {
    let version = |ty: &str| {
        let source = json!({
            "$defs": {
                "x": {"type": ty},
                "held": {"type": "object", "properties": {"p": {"$ref": "#/$defs/x"}}},
                "other": {"type": "object", "required": ["z"]},
                // Neither reaches `x`, so neither is renamed - one symbolic, one a choice.
                "barred": {"not": {"$ref": "#/$defs/other"}},
                "chosen": {"oneOf": [{"$ref": "#/$defs/other"}, {"type": "integer"}]}
            },
            "anyOf": [{"$ref": "#/$defs/held"}, {"$ref": "#/$defs/barred"}, {"$ref": "#/$defs/chosen"}]
        });
        (
            canonicalize(&source).expect("canonicalizes"),
            validator_for(&source).expect("builds"),
        )
    };
    let (old, taken) = version("integer");
    let (new, removed) = version("string");

    let difference = old.subtract(&new).expect("subtracts");
    let left_over = validator_for(&difference.to_json_schema()).expect("builds");
    for instance in [
        json!({"p": 1}),
        json!({"p": "a"}),
        json!({"z": 1}),
        json!(1),
        json!(null),
    ] {
        assert_eq!(
            taken.is_valid(&instance) && !removed.is_valid(&instance),
            left_over.is_valid(&instance),
            "the difference disagrees on {instance}\n  difference = {}",
            difference.to_json_schema()
        );
    }
}

// A node holding no `$ref` sees nothing of its document, so two written the same way are one schema
// whichever documents they came out of - and comparing them walks neither document.
#[test]
fn nodes_holding_no_reference_compare_across_documents() {
    let entry = |uri: &str, extra: Value| {
        let mut source = json!({"$defs": {uri: {"type": "string", "minLength": 2}}});
        source
            .as_object_mut()
            .expect("object")
            .extend(extra.as_object().expect("object").clone());
        canonicalize(&source)
            .expect("canonicalizes")
            .definition(&format!("#/$defs/{uri}"))
            .expect("the entry is a definition")
    };
    let here = entry("a", json!({"$ref": "#/$defs/a"}));
    let there = entry("b", json!({"$ref": "#/$defs/b"}));

    assert_eq!(here, there);
    assert_eq!(here.cmp(&there), Ordering::Equal);
}

// A complement the form keeps symbolic still holds a reference of its own, which the rename
// rewrites along with everything else the edited entry is reached from.
#[test]
fn a_rename_rewrites_a_symbolic_complement() {
    let version = |ty: &str| {
        let source = json!({
            "$defs": {
                "r": {"type": "object", "properties": {"a": {"$ref": "#/$defs/r"}}},
                "x": {"type": "object", "patternProperties": {"^a": {"type": ty}}},
                // One complement over an entry the rename leaves alone, one over the edited entry.
                "held": {"allOf": [{"not": {"$ref": "#/$defs/r"}}, {"not": {"$ref": "#/$defs/x"}}]}
            },
            "$ref": "#/$defs/held"
        });
        (
            canonicalize(&source).expect("canonicalizes"),
            validator_for(&source).expect("builds"),
        )
    };
    let (old, taken) = version("integer");
    let (new, removed) = version("string");

    let difference = old.subtract(&new).expect("subtracts");
    let left_over = validator_for(&difference.to_json_schema()).expect("builds");
    for instance in [
        json!({"a": 1}),
        json!({"a": "s"}),
        json!({"b": 1}),
        json!(1),
        json!(null),
    ] {
        assert_eq!(
            taken.is_valid(&instance) && !removed.is_valid(&instance),
            left_over.is_valid(&instance),
            "the difference disagrees on {instance}\n  difference = {}",
            difference.to_json_schema()
        );
    }
}

#[test]
fn offline_refuses_remote_references() {
    let schema = json!({"$ref": "https://example.com/schema.json"});
    let error = options()
        .offline()
        .canonicalize(&schema)
        .expect_err("must refuse to fetch");
    assert_eq!(
        error.to_string(),
        "Resource 'https://example.com/schema.json' is not present in a registry \
         and retrieving it failed: Retrieval is disabled, cannot fetch \
         https://example.com/schema.json"
    );
}

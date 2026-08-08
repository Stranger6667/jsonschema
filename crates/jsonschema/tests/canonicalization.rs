use std::{
    cmp::Ordering,
    collections::{hash_map::DefaultHasher, HashSet},
    hash::{Hash, Hasher},
};

use jsonschema::{
    canonical::{
        options, CanonicalKind, CanonicalSchema, CanonicalView, Distinctness, ObjectViolationView,
        OperandMismatch,
    },
    canonicalize, validator_for, CanonicalizationError, Draft, JsonType, PatternOptions, Registry,
    Retrieve, Uri,
};
use serde_json::{json, Map, Number, Value};
use test_case::test_case;

#[test_case(&json!({"if": {}, "unevaluatedProperties": false}); "unevaluated properties beside an applicator")]
fn unmodeled_document_round_trips_verbatim(schema: &Value) {
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
    assert!(
        canonical.is_satisfiable(),
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
    let intersection = canonicalize(&json!({
        "allOf": [
            {"$ref": "#/$defs/left"},
            {"$ref": "#/$defs/right"}
        ],
        "$defs": {
            "left": {"type": "integer"},
            "right": {"type": "string"}
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

// An unmodeled document keeps document identity, where `1` and `1.0` are distinct - unlike JSON
// value equality, which reads them as the same number.
#[test]
fn unmodeled_documents_hash_by_document_identity() {
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

// `unevaluatedProperties` beside an instance-dependent applicator is unmodeled, so the document
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
            "$defs": {"A": {"type": "string", "minLength": 2}},
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

// A pattern the canonical form does not model keeps the whole document raw.
fn unmodeled() -> Value {
    json!({"if": {}, "unevaluatedProperties": false})
}

#[test]
fn intersect_rejects_a_raw_root() {
    let raw = canonicalize(&unmodeled()).expect("canonicalizes");
    let modeled = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    let error = raw.intersect(&modeled).expect_err("the left side is raw");
    assert!(matches!(error, CanonicalizationError::UnmodeledOperand));
    assert_eq!(
        error.to_string(),
        "operand is not modeled in canonical form"
    );
    assert!(matches!(
        modeled.intersect(&raw),
        Err(CanonicalizationError::UnmodeledOperand)
    ));
}

// An unmodeled `$ref` target takes the whole document raw rather than leaving a raw entry beside a
// modeled root, so the handle a consumer resolves from carries no definitions at all.
#[test]
fn intersect_rejects_a_raw_reference_target() {
    let document = json!({
        "properties": {"a": {"$ref": "#/$defs/A"}},
        "$defs": {"A": unmodeled()}
    });
    let raw = canonicalize(&document).expect("canonicalizes");
    assert_eq!(raw.kind(), CanonicalKind::Raw);
    assert_eq!(raw.definitions().next(), None);
    let modeled = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    assert!(matches!(
        raw.intersect(&modeled),
        Err(CanonicalizationError::UnmodeledOperand)
    ));
}

// Two children of one root share the map, so both references resolve through the result.
#[test]
fn intersect_keeps_references_resolvable_within_one_document() {
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
    let merged = left.intersect(&right).expect("intersects");
    assert!(merged.definition("#/$defs/A").is_some());
    assert!(merged.definition("#/$defs/B").is_some());
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

// A side with no definitions of its own adopts the other's map, whichever side it is.
#[test_case(false; "empty map on the right")]
#[test_case(true; "empty map on the left")]
fn intersect_adopts_the_only_definition_map(swap: bool) {
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
    assert!(merged.definition("#/$defs/A").is_some());
}

#[test]
fn intersect_rejects_operands_with_distinct_definition_maps() {
    let left = canonicalize(&json!({
        "$defs": {"A": {"type": "string"}},
        "$ref": "#/$defs/A"
    }))
    .expect("canonicalizes");
    let right = canonicalize(&json!({
        "$defs": {"B": {"minLength": 4}},
        "$ref": "#/$defs/B"
    }))
    .expect("canonicalizes");
    let error = left.intersect(&right).expect_err("the maps differ");
    assert!(matches!(
        error,
        CanonicalizationError::IncompatibleOperands(OperandMismatch::Definitions)
    ));
    assert_eq!(
        error.to_string(),
        "operands carry different definition maps"
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
        Err(CanonicalizationError::UnmodeledOperand)
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
        Err(CanonicalizationError::UnmodeledOperand)
    ));
}

// Containment is proved by meeting the two sides, so a meet `intersect` will not hand out cannot
// carry a verdict either: it stands in for the real one and may be wider than it.
#[test]
fn is_subset_of_declines_what_only_an_unspellable_meet_would_prove() {
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
        Err(CanonicalizationError::UnmodeledOperand)
    ));
    assert_eq!(wide.is_subset_of(&narrow).expect("compares"), None);
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

#[test_case(&json!({"type": "integer"}), &json!({"type": "integer"}), Some(true); "identical forms")]
#[test_case(&json!({"const": 1}), &json!({"type": "integer"}), Some(true); "constant inside a type")]
#[test_case(&json!({"enum": [1, 2]}), &json!({"type": "integer"}), Some(true); "enum inside a type")]
#[test_case(&json!({"type": "integer", "minimum": 5}), &json!({"type": "integer"}), Some(true); "bounded inside unbounded")]
#[test_case(&json!({"const": "x"}), &json!({"type": "integer"}), Some(false); "a constant outside the type refutes")]
#[test_case(&json!({"enum": [1, "x"]}), &json!({"type": "integer"}), Some(false); "an enum member outside the type refutes")]
#[test_case(&json!({"type": "string"}), &json!({"type": "integer"}), None; "disjoint types without a decisive counterexample")]
#[test_case(&json!({"type": "integer"}), &json!({"type": "integer", "minimum": 5}), None; "unbounded against bounded")]
fn is_subset_of_decides(left: &Value, right: &Value, expected: Option<bool>) {
    let left = canonicalize(left).expect("canonicalizes");
    let right = canonicalize(right).expect("canonicalizes");
    assert_eq!(left.is_subset_of(&right).expect("compares"), expected);
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
fn is_subset_of_proves_a_schema_a_subset_of_itself(schema: &Value) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    assert_eq!(
        canonical.is_subset_of(&canonical).expect("compares"),
        Some(true)
    );
}

// Two symbolic references are not compared through their targets.
#[test]
fn is_subset_of_declines_distinct_references() {
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
    assert_eq!(left.is_subset_of(&right).expect("compares"), None);
}

#[test_case(false; "raw on the left")]
#[test_case(true; "raw on the right")]
fn is_subset_of_rejects_a_raw_operand(swap: bool) {
    let raw = canonicalize(&unmodeled()).expect("canonicalizes");
    let modeled = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    let (left, right) = if swap {
        (&modeled, &raw)
    } else {
        (&raw, &modeled)
    };
    assert!(matches!(
        left.is_subset_of(right),
        Err(CanonicalizationError::UnmodeledOperand)
    ));
}

#[test]
fn is_subset_of_rejects_operands_from_different_drafts() {
    let draft7 = options()
        .with_draft(Draft::Draft7)
        .canonicalize(&json!({"type": "string"}))
        .expect("canonicalizes");
    let latest = canonicalize(&json!({"type": "string"})).expect("canonicalizes");
    assert!(matches!(
        draft7.is_subset_of(&latest),
        Err(CanonicalizationError::IncompatibleOperands(
            OperandMismatch::Drafts {
                left: Draft::Draft7,
                right: Draft::Draft202012
            }
        ))
    ));
}

#[test]
fn is_subset_of_rejects_operands_with_distinct_definition_maps() {
    let left = canonicalize(&json!({
        "$defs": {"A": {"type": "string"}},
        "$ref": "#/$defs/A"
    }))
    .expect("canonicalizes");
    let right = canonicalize(&json!({
        "$defs": {"B": {"minLength": 4}},
        "$ref": "#/$defs/B"
    }))
    .expect("canonicalizes");
    assert!(matches!(
        left.is_subset_of(&right),
        Err(CanonicalizationError::IncompatibleOperands(
            OperandMismatch::Definitions
        ))
    ));
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
    &json!({
        "$defs": {"a": {"type": "string", "minLength": 3}},
        "anyOf": [
            {"type": ["null", "boolean", "number", "array", "object"]},
            {"allOf": [{"type": "string", "maxLength": 5}, {"$ref": "#/$defs/a"}]}
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

// The decline set is contract: a caller sizes its fallback on it, so widening it is a visible change.
#[test_case(&json!({"if": {}, "unevaluatedProperties": false}); "raw document")]
#[test_case(
    &json!({"type": "array", "contains": {"type": "string"}, "minContains": 2});
    "counted array existential demand"
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
    assert_eq!(canonicalize(schema).expect("canonicalizes").negate(), None);
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
    assert_eq!(canonicalize(&schema).expect("canonicalizes").negate(), None);
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
        .is_some());
    assert_eq!(
        canonicalize(&choice_over_pointers(12))
            .expect("canonicalizes")
            .negate(),
        None
    );
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
    assert_eq!(barred.negate(), None);
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
    false;
    "no room for the name-fails demand beyond the required key"
)]
#[test_case(
    &json!({"type": "object", "maxProperties": 2, "minProperties": 1, "required": ["a"],
            "properties": {"a": {"type": "string"}},
            "not": {"type": "object", "propertyNames": {"enum": ["a", "b"]}}}),
    true;
    "a second slot admits the violating key"
)]
#[test_case(
    &json!({"type": "object", "maxProperties": 1, "minProperties": 1, "required": ["c"],
            "properties": {"a": {"type": "string"}},
            "not": {"type": "object", "propertyNames": {"enum": ["a", "b"]}}}),
    true;
    "the required key itself already carries the violation"
)]
#[test_case(
    &json!({"type": "object", "maxProperties": 1, "required": ["a"],
            "properties": {"a": {"type": "integer"}},
            "not": {"type": "object", "properties": {"a": {"type": "integer"}},
                    "additionalProperties": {"type": "string"}}}),
    false;
    "no room for the undeclared-value-fails demand beyond the required key"
)]
fn a_required_key_the_demand_admits_needs_room_for_another(schema: &Value, satisfiable: bool) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    assert_eq!(canonical.is_satisfiable(), satisfiable);
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
    assert!(canonical.is_satisfiable());
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
    assert!(canonical.is_satisfiable());
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
    assert!(!meet.is_satisfiable());
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

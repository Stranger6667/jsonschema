use std::{cmp::Ordering, collections::HashSet};

use jsonschema::{
    canonical::{options, CanonicalKind, CanonicalSchema, CanonicalView},
    canonicalize, CanonicalizationError, Draft, JsonType, PatternOptions, Registry,
};
use serde_json::{json, Map, Number, Value};
use test_case::test_case;

#[test_case(&json!({"anyOf": [{}], "unevaluatedProperties": false}); "unevaluated properties beside an applicator")]
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
        "$defs": {"value": {"anyOf": [{}], "unevaluatedProperties": false}}
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
        "$defs": {"other": {"type": "integer"}}
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
    assert!(view.unique_items);
    assert!(view.prefix_items.is_empty());
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

// A format the canonicalizer cannot check may still be checked at validation, so a union must not
// absorb a value into a leaf carrying one.
#[test_case(&json!({"type": "object", "propertyNames": {"format": "only-ok"}}), &json!({"nope": 1}); "key constraint")]
#[test_case(&json!({"type": "object", "properties": {"a": {"type": "string", "format": "only-ok"}}}), &json!({"a": "nope"}); "property schema")]
#[test_case(&json!({"type": "object", "properties": {"a": {"format": "only-ok"}}}), &json!({"a": "nope"}); "untyped property schema")]
#[test_case(&json!({"type": "object", "properties": {"a": {"type": "object", "propertyNames": {"format": "only-ok"}}}}), &json!({"a": {"nope": 1}}); "nested object")]
#[test_case(&json!({"type": "array", "items": {"type": "string", "format": "only-ok"}}), &json!(["nope"]); "item schema")]
#[test_case(&json!({"type": "array", "contains": {"type": "string", "format": "only-ok"}}), &json!(["nope"]); "contains schema")]
#[test_case(&json!({"type": "object", "properties": {"a": {"type": "array", "contains": {"type": "string", "format": "only-ok"}}}}), &json!({"a": ["nope"]}); "contains under a property")]
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
        r#"{"anyOf": [{}], "unevaluatedProperties": {"enum": [1, null, true, "x", [2], {"b": 3}]}}"#,
    );
    let float = canonical(
        r#"{"anyOf": [{}], "unevaluatedProperties": {"enum": [1.0, null, true, "x", [2], {"b": 3}]}}"#,
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
#[test_case(&json!({"anyOf": [{}], "unevaluatedProperties": false}), CanonicalKind::Raw, "raw"; "raw")]
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
        map.insert("anyOf".to_string(), json!([{}]));
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
    let raw_one = raw(r#"{"anyOf":[{}],"unevaluatedProperties":{"const":1}}"#);
    let raw_two = raw(r#"{"anyOf":[{}],"unevaluatedProperties":{"const":2}}"#);
    assert_eq!(raw_one.partial_cmp(&raw_two), Some(Ordering::Less));
    assert!(raw_one < raw_two);

    #[cfg(feature = "arbitrary-precision")]
    assert!(
        raw(r#"{"anyOf":[{}],"unevaluatedProperties":{"const":1e400}}"#)
            < raw(r#"{"anyOf":[{}],"unevaluatedProperties":{"const":2e400}}"#)
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

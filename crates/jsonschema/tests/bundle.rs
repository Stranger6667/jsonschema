use jsonschema::ReferencingError;
use referencing::Resource;
use serde_json::{json, Value};

#[cfg(all(feature = "resolve-async", not(target_arch = "wasm32")))]
mod async_tests {
    use super::*;

    #[tokio::test]
    async fn test_async_bundle_single_external_ref() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$ref": "https://example.com/person.json"
        });
        let bundled = jsonschema::async_options()
            .with_resource(
                "https://example.com/person.json",
                Resource::from_contents(person_schema()),
            )
            .bundle(&schema)
            .await
            .expect("async bundle failed");

        assert_eq!(
            bundled.get("$ref"),
            Some(&json!("https://example.com/person.json"))
        );
        let defs = bundled.get("$defs").unwrap().as_object().unwrap();
        assert!(defs.contains_key("https://example.com/person.json"));
    }

    #[tokio::test]
    async fn test_async_bundle_no_external_refs() {
        let schema = json!({"type": "integer", "minimum": 0});
        let bundled = jsonschema::async_bundle(&schema)
            .await
            .expect("async bundle failed");
        assert_eq!(bundled, schema);
        assert!(bundled.get("$defs").is_none());
    }

    #[tokio::test]
    async fn test_async_bundle_unresolvable_ref() {
        let schema = json!({"$ref": "https://example.com/missing.json"});
        let result = jsonschema::async_bundle(&schema).await;
        assert!(
            matches!(result, Err(ReferencingError::Unretrievable { .. })),
            "expected Unretrievable, got: {result:?}"
        );
    }
}

fn person_schema() -> Value {
    json!({
        "$id": "https://example.com/person.json",
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": { "name": { "type": "string" } },
        "required": ["name"]
    })
}

#[test]
fn test_bundle_no_external_refs() {
    let schema = json!({"type": "string"});
    let bundled = jsonschema::bundle(&schema).expect("bundle failed");
    assert!(bundled.get("$defs").is_none());
    assert_eq!(bundled.get("type"), Some(&json!("string")));
}

#[test]
fn test_bundle_single_external_ref() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/person.json"
    });
    let bundled = jsonschema::options()
        .with_resource(
            "https://example.com/person.json",
            Resource::from_contents(person_schema()),
        )
        .bundle(&schema)
        .expect("bundle failed");

    // $ref MUST NOT be rewritten (spec requirement)
    assert_eq!(
        bundled.get("$ref"),
        Some(&json!("https://example.com/person.json"))
    );
    let defs = bundled.get("$defs").expect("no $defs").as_object().unwrap();
    assert!(defs.contains_key("https://example.com/person.json"));
    // embedded resource MUST have $id
    let embedded = &defs["https://example.com/person.json"];
    assert_eq!(
        embedded.get("$id"),
        Some(&json!("https://example.com/person.json"))
    );
}

#[test]
fn test_bundle_validates_identically() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/person.json"
    });
    let bundled = jsonschema::options()
        .with_resource(
            "https://example.com/person.json",
            Resource::from_contents(person_schema()),
        )
        .bundle(&schema)
        .expect("bundle failed");

    let validator = jsonschema::validator_for(&bundled).expect("compile bundled failed");
    assert!(validator.is_valid(&json!({"name": "Alice"})));
    assert!(!validator.is_valid(&json!({"age": 30})));
}

#[test]
fn test_bundle_unresolvable_ref() {
    let schema = json!({"$ref": "https://example.com/missing.json"});
    let result = jsonschema::bundle(&schema);
    assert!(matches!(
        result,
        Err(ReferencingError::Unretrievable { .. })
    ));
}

#[test]
fn test_bundle_transitive_refs() {
    let address_schema = json!({
        "$id": "https://example.com/address.json",
        "type": "object",
        "properties": { "street": { "type": "string" } }
    });
    let person_with_address = json!({
        "$id": "https://example.com/person.json",
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "address": { "$ref": "https://example.com/address.json" }
        }
    });
    let root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/person.json"
    });
    let bundled = jsonschema::options()
        .with_resource(
            "https://example.com/person.json",
            Resource::from_contents(person_with_address),
        )
        .with_resource(
            "https://example.com/address.json",
            Resource::from_contents(address_schema),
        )
        .bundle(&root)
        .expect("bundle failed");

    let defs = bundled.get("$defs").unwrap().as_object().unwrap();
    assert!(
        defs.contains_key("https://example.com/person.json"),
        "person missing"
    );
    assert!(
        defs.contains_key("https://example.com/address.json"),
        "address missing"
    );
}

#[test]
fn test_bundle_circular_ref() {
    let node_schema = json!({
        "$id": "https://example.com/node.json",
        "type": "object",
        "properties": {
            "child": { "$ref": "https://example.com/node.json" }
        }
    });
    let root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/node.json"
    });
    let bundled = jsonschema::options()
        .with_resource(
            "https://example.com/node.json",
            Resource::from_contents(node_schema),
        )
        .bundle(&root)
        .expect("bundle failed");

    let defs = bundled.get("$defs").unwrap().as_object().unwrap();
    assert_eq!(defs.len(), 1, "node.json should appear exactly once");
    assert!(defs.contains_key("https://example.com/node.json"));
}

/// A `$ref` like `https://example.com/schema.json#/$defs/Name` should embed
/// the entire schema.json document (not just the fragment).
#[test]
fn test_bundle_fragment_qualified_external_ref() {
    let schemas = json!({
        "$id": "https://example.com/schema.json",
        "$defs": {
            "Name": { "type": "string" }
        }
    });
    let root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "properties": {
            "name": { "$ref": "https://example.com/schema.json#/$defs/Name" }
        }
    });
    let bundled = jsonschema::options()
        .with_resource(
            "https://example.com/schema.json",
            referencing::Resource::from_contents(schemas),
        )
        .bundle(&root)
        .expect("bundle failed");

    // $ref must NOT be rewritten
    let name_prop = bundled["properties"]["name"].as_object().unwrap();
    assert_eq!(
        name_prop["$ref"],
        json!("https://example.com/schema.json#/$defs/Name")
    );
    // The whole schema.json document is embedded
    let defs = bundled.get("$defs").expect("no $defs").as_object().unwrap();
    assert!(defs.contains_key("https://example.com/schema.json"));
}

/// An external schema that internally uses a relative $ref should have its
/// transitive dependency collected correctly.
#[test]
fn test_bundle_relative_ref_inside_external_schema() {
    // address.json uses a relative $ref to country.json
    let country_schema = json!({
        "$id": "https://example.com/schemas/country.json",
        "type": "string",
        "enum": ["US", "UK", "CA"]
    });
    let address_schema = json!({
        "$id": "https://example.com/schemas/address.json",
        "type": "object",
        "properties": {
            "street": { "type": "string" },
            "country": { "$ref": "country.json" }
        }
    });
    let root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/schemas/address.json"
    });
    let bundled = jsonschema::options()
        .with_resource(
            "https://example.com/schemas/address.json",
            referencing::Resource::from_contents(address_schema),
        )
        .with_resource(
            "https://example.com/schemas/country.json",
            referencing::Resource::from_contents(country_schema),
        )
        .bundle(&root)
        .expect("bundle failed");

    let defs = bundled.get("$defs").expect("no $defs").as_object().unwrap();
    assert!(
        defs.contains_key("https://example.com/schemas/address.json"),
        "address missing"
    );
    assert!(
        defs.contains_key("https://example.com/schemas/country.json"),
        "country missing (transitive)"
    );
}

#[test]
fn test_bundle_inner_ref_not_rewritten() {
    // $ref values inside embedded schemas must not be rewritten — this is a core spec invariant
    let leaf = json!({ "$id": "https://example.com/leaf", "type": "number", "minimum": 0 });
    let middle = json!({ "$id": "https://example.com/middle", "$ref": "https://example.com/leaf", "maximum": 100 });
    let root = json!({ "$schema": "https://json-schema.org/draft/2020-12/schema", "$ref": "https://example.com/middle" });

    let bundled = jsonschema::options()
        .with_resource(
            "https://example.com/leaf",
            referencing::Resource::from_contents(leaf),
        )
        .with_resource(
            "https://example.com/middle",
            referencing::Resource::from_contents(middle),
        )
        .bundle(&root)
        .expect("bundle failed");

    assert_eq!(
        bundled["$ref"],
        json!("https://example.com/middle"),
        "root $ref must not be rewritten"
    );
    assert_eq!(
        bundled["$defs"]["https://example.com/middle"]["$ref"],
        json!("https://example.com/leaf"),
        "inner $ref inside embedded resource must not be rewritten"
    );
}

#[test]
fn test_bundle_resolves_ref_with_nested_id_scope() {
    let nested_dependency = json!({
        "$id": "https://example.com/A/b.json",
        "type": "integer"
    });
    let root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$defs": {
            "A": {
                "$id": "https://example.com/A/",
                "$ref": "b.json"
            }
        }
    });

    let bundled = jsonschema::options()
        .with_resource(
            "https://example.com/A/b.json",
            Resource::from_contents(nested_dependency),
        )
        .bundle(&root)
        .expect("bundle failed");

    let defs = bundled.get("$defs").expect("no $defs").as_object().unwrap();
    assert!(defs.contains_key("A"));
    assert!(
        defs.contains_key("https://example.com/A/b.json"),
        "nested dependency was not embedded"
    );
}

#[test]
fn test_bundle_ignores_ref_inside_const_annotation_payload() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "const": {
            "$ref": "https://example.com/not-a-schema"
        }
    });

    let bundled = jsonschema::bundle(&schema).expect("bundle failed");
    assert_eq!(bundled, schema);
    assert!(bundled.get("$defs").is_none());
}

#[test]
fn test_bundle_supports_legacy_drafts_using_definitions() {
    for schema_uri in [
        "http://json-schema.org/draft-04/schema#",
        "http://json-schema.org/draft-06/schema#",
        "http://json-schema.org/draft-07/schema#",
    ] {
        let schema = json!({
            "$schema": schema_uri,
            "$ref": "https://example.com/person.json"
        });

        let bundled = jsonschema::options()
            .with_resource(
                "https://example.com/person.json",
                Resource::from_contents(json!({
                    "$id": "https://example.com/person.json",
                    "$schema": schema_uri,
                    "type": "object",
                    "properties": { "name": { "type": "string" } }
                })),
            )
            .bundle(&schema)
            .expect("bundle failed");

        assert!(
            bundled.get("$defs").is_none(),
            "unexpected $defs for {schema_uri}"
        );
        let definitions = bundled
            .get("definitions")
            .and_then(Value::as_object)
            .expect("no definitions object");
        assert!(
            definitions.contains_key("https://example.com/person.json"),
            "missing bundled resource for {schema_uri}"
        );
    }
}

#[test]
fn test_bundle_draft4_embedded_resource_uses_id_keyword() {
    let root = json!({
        "$schema": "http://json-schema.org/draft-04/schema#",
        "$ref": "https://example.com/integer.json"
    });
    let bundled = jsonschema::options()
        .with_resource(
            "https://example.com/integer.json",
            Resource::from_contents(json!({
                "$schema": "http://json-schema.org/draft-04/schema#",
                "type": "integer"
            })),
        )
        .bundle(&root)
        .expect("bundle failed");

    let embedded = &bundled["definitions"]["https://example.com/integer.json"];
    assert_eq!(
        embedded.get("id"),
        Some(&json!("https://example.com/integer.json"))
    );
    assert!(embedded.get("$id").is_none());
}

#[test]
fn test_parity_legacy_drafts() {
    for schema_uri in [
        "http://json-schema.org/draft-04/schema#",
        "http://json-schema.org/draft-06/schema#",
        "http://json-schema.org/draft-07/schema#",
    ] {
        let root = json!({
            "$schema": schema_uri,
            "$ref": "https://example.com/legacy-non-negative.json"
        });
        let external = json!({
            "$schema": schema_uri,
            "type": "integer",
            "minimum": 0
        });

        assert_bundle_parity(
            &root,
            &[("https://example.com/legacy-non-negative.json", external)],
            &[json!(0), json!(5)],
            &[json!(-1), json!("x"), json!(1.5)],
        );
    }
}

#[test]
fn test_parity_mixed_root_draft7_external_draft4() {
    let root = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$ref": "https://example.com/mixed-schema.json"
    });
    let external = json!({
        "$schema": "http://json-schema.org/draft-04/schema#",
        "type": "integer",
        "minimum": 0
    });

    assert_bundle_parity(
        &root,
        &[("https://example.com/mixed-schema.json", external)],
        &[json!(0), json!(10)],
        &[json!(-1), json!("oops"), json!(1.2)],
    );
}

#[test]
fn test_parity_mixed_root_draft4_external_draft7() {
    let root = json!({
        "$schema": "http://json-schema.org/draft-04/schema#",
        "$ref": "https://example.com/mixed-schema.json"
    });
    let external = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "integer",
        "minimum": 0
    });

    assert_bundle_parity(
        &root,
        &[("https://example.com/mixed-schema.json", external)],
        &[json!(0), json!(10)],
        &[json!(-1), json!("oops"), json!(1.2)],
    );
}

#[test]
fn test_parity_mixed_root_draft4_external_draft7_const() {
    let root = json!({
        "$schema": "http://json-schema.org/draft-04/schema#",
        "$ref": "https://example.com/mixed-const.json"
    });
    let external = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "const": 1
    });

    assert_bundle_parity(
        &root,
        &[("https://example.com/mixed-const.json", external)],
        &[json!(1)],
        &[json!(2)],
    );
}

#[test]
fn test_bundle_202012_reuses_existing_definitions_container() {
    let root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "local": { "$ref": "#/definitions/localInt" },
            "external": { "$ref": "https://example.com/ext.json" }
        },
        "definitions": {
            "localInt": { "type": "integer" }
        }
    });
    let external = json!({
        "$id": "https://example.com/ext.json",
        "type": "string"
    });

    let bundled = jsonschema::options()
        .with_resource(
            "https://example.com/ext.json",
            Resource::from_contents(external.clone()),
        )
        .bundle(&root)
        .expect("bundle failed");

    assert!(bundled.get("$defs").is_none(), "unexpected $defs created");
    let definitions = bundled
        .get("definitions")
        .and_then(Value::as_object)
        .expect("missing definitions");
    assert!(definitions.contains_key("localInt"));
    assert!(definitions.contains_key("https://example.com/ext.json"));

    let validator = jsonschema::validator_for(&bundled).expect("bundled compile failed");
    assert!(validator.is_valid(&json!({"local": 1, "external": "ok"})));
    assert!(!validator.is_valid(&json!({"local": "x", "external": "ok"})));
}

#[test]
fn test_bundle_draft7_keeps_existing_defs_but_adds_definitions_for_resolution() {
    let root = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$ref": "https://example.com/ext.json",
        "$defs": {
            "kept": { "type": "string" }
        }
    });

    let bundled = jsonschema::options()
        .with_resource(
            "https://example.com/ext.json",
            Resource::from_contents(json!({
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "integer"
            })),
        )
        .bundle(&root)
        .expect("bundle failed");

    assert!(bundled.get("$defs").is_some(), "existing $defs should stay");
    assert!(
        bundled
            .get("definitions")
            .and_then(Value::as_object)
            .and_then(|defs| defs.get("https://example.com/ext.json"))
            .is_some(),
        "draft-07 bundles must embed into definitions for resolvability"
    );

    let validator = jsonschema::validator_for(&bundled).expect("bundled compile failed");
    assert!(validator.is_valid(&json!(1)));
    assert!(!validator.is_valid(&json!("x")));
}

fn assert_bundle_parity(
    root: &Value,
    resources: &[(&str, Value)],
    valid_instances: &[Value],
    invalid_instances: &[Value],
) {
    // Validator from distributed schemas (registered individually)
    let mut opts = jsonschema::options();
    for (uri, schema) in resources {
        opts = opts.with_resource(*uri, Resource::from_contents(schema.clone()));
    }
    let distributed = opts.build(root).expect("distributed compile failed");

    // Validator from bundled schema
    let mut bundle_opts = jsonschema::options();
    for (uri, schema) in resources {
        bundle_opts = bundle_opts.with_resource(*uri, Resource::from_contents(schema.clone()));
    }
    let bundled = bundle_opts.bundle(root).expect("bundle failed");
    let bundled_validator = jsonschema::validator_for(&bundled).expect("bundled compile failed");

    for instance in valid_instances {
        assert!(
            distributed.is_valid(instance),
            "distributed rejected valid: {instance}"
        );
        assert!(
            bundled_validator.is_valid(instance),
            "bundled rejected valid: {instance}"
        );
    }
    for instance in invalid_instances {
        assert!(
            !distributed.is_valid(instance),
            "distributed accepted invalid: {instance}"
        );
        assert!(
            !bundled_validator.is_valid(instance),
            "bundled accepted invalid: {instance}"
        );
    }
}

/// From: <https://json-schema.org/blog/posts/bundling-json-schema-compound-documents>
#[test]
fn test_parity_blog_post_integer_non_negative() {
    let integer = json!({
        "$id": "https://example.com/integer",
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "integer"
    });
    let non_negative = json!({
        "$id": "https://example.com/non-negative",
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/integer",
        "minimum": 0
    });
    let root = json!({
        "$id": "https://example.com/root",
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/non-negative"
    });

    assert_bundle_parity(
        &root,
        &[
            ("https://example.com/integer", integer),
            ("https://example.com/non-negative", non_negative),
        ],
        &[json!(5), json!(0), json!(100)],
        &[json!(-1), json!("hello"), json!(1.5)],
    );
}

#[test]
fn test_parity_nested_object_refs() {
    let address = json!({
        "$id": "https://example.com/address",
        "type": "object",
        "properties": {
            "street": { "type": "string" },
            "city": { "type": "string" }
        },
        "required": ["street", "city"]
    });
    let person = json!({
        "$id": "https://example.com/person",
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "address": { "$ref": "https://example.com/address" }
        },
        "required": ["name"]
    });
    let root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/person"
    });

    assert_bundle_parity(
        &root,
        &[
            ("https://example.com/address", address),
            ("https://example.com/person", person),
        ],
        &[
            json!({"name": "Alice"}),
            json!({"name": "Bob", "address": {"street": "1 Main St", "city": "NYC"}}),
        ],
        &[
            json!({"address": {"street": "x", "city": "y"}}), // missing name
            json!({"name": "Alice", "address": {"street": "x"}}), // address missing city
        ],
    );
}

/// Root schema already has $defs — bundler must merge, not overwrite.
#[test]
fn test_parity_merge_with_existing_defs() {
    let external = json!({
        "$id": "https://example.com/string-type",
        "type": "string"
    });
    let root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "a": { "$ref": "#/$defs/local" },
            "b": { "$ref": "https://example.com/string-type" }
        },
        "$defs": {
            "local": { "type": "integer" }
        }
    });

    assert_bundle_parity(
        &root,
        &[("https://example.com/string-type", external)],
        &[json!({"a": 1, "b": "hello"})],
        &[json!({"a": "x", "b": "hello"}), json!({"a": 1, "b": 42})],
    );
}

/// Walk recurses into embedded schemas; an unresolvable $ref inside one must propagate.
#[test]
fn test_bundle_error_propagates_from_recursive_walk() {
    // `middle` is registered, but it references `leaf` which is not registered.
    // The walk recurses into `middle` and fails when resolving `leaf`.
    let middle = json!({
        "$id": "https://example.com/middle.json",
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/leaf.json"
    });
    let root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/middle.json"
    });
    let result = jsonschema::options()
        .with_resource(
            "https://example.com/middle.json",
            Resource::from_contents(middle),
        )
        .bundle(&root);
    assert!(
        matches!(result, Err(ReferencingError::Unretrievable { .. })),
        "expected Unretrievable, got: {result:?}"
    );
}

#[test]
fn test_bundle_error_unresolvable_display_and_source() {
    use std::error::Error;
    let err = jsonschema::bundle(&json!({"$ref": "https://example.com/missing.json"}))
        .expect_err("unresolvable ref must fail");
    assert!(
        matches!(err, ReferencingError::Unretrievable { .. }),
        "expected Unretrievable, got: {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("https://example.com/missing.json"),
        "unexpected message: {msg}"
    );
    assert!(err.source().is_some(), "Unretrievable must expose a source");
}

#[test]
fn test_bundle_error_invalid_schema() {
    let schema = json!({
        "$schema": "https://example.com/custom-meta",
        "type": "string"
    });
    let err = jsonschema::bundle(&schema).expect_err("unknown meta-schema must fail");
    assert!(
        !matches!(err, ReferencingError::Unretrievable { .. }),
        "unexpected Unretrievable, got: {err:?}"
    );
}

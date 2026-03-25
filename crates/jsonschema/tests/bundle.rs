use jsonschema::{ReferencingError, Registry};
use serde_json::{json, Value};

const TEST_ROOT_URI: &str = "urn:jsonschema:test:root";

fn registry_from_resources<'a>(resources: &'a [(&str, Value)]) -> Registry<'a> {
    let mut registry = jsonschema::Registry::new();
    for (uri, schema) in resources {
        registry = registry
            .add(*uri, schema)
            .expect("resource should be accepted");
    }
    registry.prepare().expect("registry build failed")
}

fn try_bundle_with_resources(
    root: &Value,
    resources: &[(&str, Value)],
) -> Result<Value, ReferencingError> {
    let registry = registry_from_resources(resources);
    jsonschema::options()
        .with_registry(&registry)
        .with_base_uri(TEST_ROOT_URI)
        .bundle(root)
}

fn validator_with_resources(root: &Value, resources: &[(&str, Value)]) -> jsonschema::Validator {
    let registry = registry_from_resources(resources);
    jsonschema::options()
        .with_registry(&registry)
        .with_base_uri(TEST_ROOT_URI)
        .build(root)
        .expect("distributed compile failed")
}

#[cfg(all(feature = "resolve-async", not(target_arch = "wasm32")))]
mod async_tests {
    use super::*;

    #[tokio::test]
    async fn test_async_bundle_single_external_ref() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$ref": "https://example.com/person.json"
        });
        let registry = jsonschema::Registry::new()
            .add("https://example.com/person.json", person_schema())
            .expect("resource should be accepted")
            .prepare()
            .expect("registry build failed");
        let bundled = jsonschema::async_options()
            .with_registry(&registry)
            .with_base_uri(TEST_ROOT_URI)
            .bundle(&schema)
            .await
            .expect("async bundle failed");

        assert_eq!(bundled["$ref"], json!("https://example.com/person.json"));
        let defs = bundled["$defs"].as_object().unwrap();
        assert!(!defs["https://example.com/person.json"].is_null());
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
    let bundled = try_bundle_with_resources(
        &schema,
        &[("https://example.com/person.json", person_schema())],
    )
    .expect("bundle failed");

    // $ref MUST NOT be rewritten (spec requirement)
    assert_eq!(bundled["$ref"], json!("https://example.com/person.json"));
    let defs = bundled["$defs"].as_object().unwrap();
    assert!(!defs["https://example.com/person.json"].is_null());
    // embedded resource MUST have $id
    let embedded = &defs["https://example.com/person.json"];
    assert_eq!(embedded["$id"], json!("https://example.com/person.json"));
}

#[test]
fn test_bundle_validates_identically() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/person.json"
    });
    let bundled = try_bundle_with_resources(
        &schema,
        &[("https://example.com/person.json", person_schema())],
    )
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
    let bundled = try_bundle_with_resources(
        &root,
        &[
            ("https://example.com/person.json", person_with_address),
            ("https://example.com/address.json", address_schema),
        ],
    )
    .expect("bundle failed");

    let defs = bundled["$defs"].as_object().unwrap();
    assert!(
        !defs["https://example.com/person.json"].is_null(),
        "person missing"
    );
    assert!(
        !defs["https://example.com/address.json"].is_null(),
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
    let bundled =
        try_bundle_with_resources(&root, &[("https://example.com/node.json", node_schema)])
            .expect("bundle failed");

    let defs = bundled["$defs"].as_object().unwrap();
    assert_eq!(defs.len(), 1, "node.json should appear exactly once");
    assert!(!defs["https://example.com/node.json"].is_null());
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
    let bundled = try_bundle_with_resources(&root, &[("https://example.com/schema.json", schemas)])
        .expect("bundle failed");

    // $ref must NOT be rewritten
    let name_prop = bundled["properties"]["name"].as_object().unwrap();
    assert_eq!(
        name_prop["$ref"],
        json!("https://example.com/schema.json#/$defs/Name")
    );
    // The whole schema.json document is embedded
    let defs = bundled["$defs"].as_object().unwrap();
    assert!(!defs["https://example.com/schema.json"].is_null());
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
    let bundled = try_bundle_with_resources(
        &root,
        &[
            ("https://example.com/schemas/address.json", address_schema),
            ("https://example.com/schemas/country.json", country_schema),
        ],
    )
    .expect("bundle failed");

    let defs = bundled["$defs"].as_object().unwrap();
    assert!(
        !defs["https://example.com/schemas/address.json"].is_null(),
        "address missing"
    );
    assert!(
        !defs["https://example.com/schemas/country.json"].is_null(),
        "country missing (transitive)"
    );
}

#[test]
fn test_bundle_inner_ref_not_rewritten() {
    // $ref values inside embedded schemas must not be rewritten — this is a core spec invariant
    let leaf = json!({ "$id": "https://example.com/leaf", "type": "number", "minimum": 0 });
    let middle = json!({ "$id": "https://example.com/middle", "$ref": "https://example.com/leaf", "maximum": 100 });
    let root = json!({ "$schema": "https://json-schema.org/draft/2020-12/schema", "$ref": "https://example.com/middle" });

    let bundled = try_bundle_with_resources(
        &root,
        &[
            ("https://example.com/leaf", leaf),
            ("https://example.com/middle", middle),
        ],
    )
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

    let bundled = try_bundle_with_resources(
        &root,
        &[("https://example.com/A/b.json", nested_dependency)],
    )
    .expect("bundle failed");

    let defs = bundled["$defs"].as_object().unwrap();
    assert!(!defs["A"].is_null());
    assert!(
        !defs["https://example.com/A/b.json"].is_null(),
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
        let bundled = try_bundle_with_resources(
            &schema,
            &[(
                "https://example.com/person.json",
                json!({
                    "$id": "https://example.com/person.json",
                    "$schema": schema_uri,
                    "type": "object",
                    "properties": { "name": { "type": "string" } }
                }),
            )],
        )
        .expect("bundle failed");

        assert!(
            bundled.get("$defs").is_none(),
            "unexpected $defs for {schema_uri}"
        );
        let definitions = bundled["definitions"].as_object().unwrap();
        assert!(
            !definitions["https://example.com/person.json"].is_null(),
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
    let bundled = try_bundle_with_resources(
        &root,
        &[(
            "https://example.com/integer.json",
            json!({
                "$schema": "http://json-schema.org/draft-04/schema#",
                "type": "integer"
            }),
        )],
    )
    .expect("bundle failed");

    let embedded = &bundled["definitions"]["https://example.com/integer.json"];
    assert_eq!(embedded["id"], json!("https://example.com/integer.json"));
    assert!(embedded["$id"].is_null());
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

    let bundled = try_bundle_with_resources(&root, &[("https://example.com/ext.json", external)])
        .expect("bundle failed");

    assert!(bundled.get("$defs").is_none(), "unexpected $defs created");
    let definitions = bundled["definitions"].as_object().unwrap();
    assert!(!definitions["localInt"].is_null());
    assert!(!definitions["https://example.com/ext.json"].is_null());

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

    let bundled = try_bundle_with_resources(
        &root,
        &[(
            "https://example.com/ext.json",
            json!({
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "integer"
            }),
        )],
    )
    .expect("bundle failed");

    assert!(bundled["$defs"].is_object(), "existing $defs should stay");
    assert!(
        !bundled["definitions"]["https://example.com/ext.json"].is_null(),
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
    let distributed = validator_with_resources(root, resources);
    let bundled = try_bundle_with_resources(root, resources).expect("bundle failed");
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

/// Missing refs reachable from the shared registry fail during registry preparation.
#[test]
fn test_registry_prepare_error_propagates_for_missing_transitive_ref() {
    // `middle` is registered, but it references `leaf` which is not registered.
    // Preparation should fail before bundling starts.
    let middle = json!({
        "$id": "https://example.com/middle.json",
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/leaf.json"
    });
    let result = jsonschema::Registry::new()
        .add("https://example.com/middle.json", middle)
        .expect("resource should be accepted")
        .prepare();
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

import jsonschema_rs
import pytest

PERSON_SCHEMA = {
    "$id": "https://example.com/person.json",
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "properties": {"name": {"type": "string"}},
    "required": ["name"],
}


def test_bundle_no_external_refs():
    bundled = jsonschema_rs.bundle({"type": "string"})
    assert isinstance(bundled, dict)
    assert "$defs" not in bundled


def test_bundle_single_external_ref():
    root = {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/person.json",
    }
    registry = jsonschema_rs.Registry(
        resources=[("https://example.com/person.json", PERSON_SCHEMA)]
    )
    bundled = jsonschema_rs.bundle(root, registry=registry)
    assert bundled["$ref"] == "https://example.com/person.json"  # must not be rewritten
    assert "https://example.com/person.json" in bundled["$defs"]


def test_bundle_validates_identically():
    root = {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/person.json",
    }
    registry = jsonschema_rs.Registry(
        resources=[("https://example.com/person.json", PERSON_SCHEMA)]
    )
    bundled = jsonschema_rs.bundle(root, registry=registry)
    validator = jsonschema_rs.validator_for(bundled)
    assert validator.is_valid({"name": "Alice"})
    assert not validator.is_valid({"age": 30})


def test_bundle_with_registry_and_explicit_draft4_legacy_id_root():
    root = {
        "id": "urn:root",
        "type": "object",
        "properties": {"value": {"$ref": "urn:string"}},
        "required": ["value"],
    }
    registry = jsonschema_rs.Registry(
        resources=[("urn:string", {"type": "string"})],
        draft=jsonschema_rs.Draft4,
    )

    bundled = jsonschema_rs.bundle(root, registry=registry, draft=jsonschema_rs.Draft4)

    assert bundled["properties"]["value"]["$ref"] == "urn:string"
    assert "urn:string" in bundled["definitions"]


def test_bundle_unresolvable_raises():
    with pytest.raises(jsonschema_rs.ReferencingError):
        jsonschema_rs.bundle({"$ref": "https://example.com/missing.json"})


def test_bundle_resolves_ref_with_nested_id_scope():
    root = {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$defs": {"A": {"$id": "https://example.com/A/", "$ref": "b.json"}},
    }
    registry = jsonschema_rs.Registry(
        resources=[("https://example.com/A/b.json", {"type": "integer"})]
    )
    bundled = jsonschema_rs.bundle(root, registry=registry)
    assert "A" in bundled["$defs"]
    assert "https://example.com/A/b.json" in bundled["$defs"]


def test_bundle_ignores_ref_inside_const_annotation_payload():
    schema = {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "const": {"$ref": "https://example.com/not-a-schema"},
    }
    bundled = jsonschema_rs.bundle(schema)
    assert bundled == schema
    assert "$defs" not in bundled


def test_bundle_supports_legacy_drafts():
    resource_uri = "https://example.com/legacy/person.json"
    for schema_uri in (
        "http://json-schema.org/draft-04/schema#",
        "http://json-schema.org/draft-06/schema#",
        "http://json-schema.org/draft-07/schema#",
    ):
        root = {"$schema": schema_uri, "$ref": resource_uri}
        registry = jsonschema_rs.Registry(
            resources=[
                (
                    resource_uri,
                    {"$schema": schema_uri, "type": "integer", "minimum": 0},
                )
            ]
        )
        distributed = jsonschema_rs.validator_for(root, registry=registry)
        bundled = jsonschema_rs.bundle(root, registry=registry)
        bundled_validator = jsonschema_rs.validator_for(bundled)

        assert "$defs" not in bundled
        assert resource_uri in bundled["definitions"]
        embedded = bundled["definitions"][resource_uri]
        assert embedded.get("id") == resource_uri or embedded.get("$id") == resource_uri

        for instance in (0, 5):
            assert distributed.is_valid(instance)
            assert bundled_validator.is_valid(instance)
        for instance in (-1, "x", 1.5):
            assert not distributed.is_valid(instance)
            assert not bundled_validator.is_valid(instance)


def test_bundle_mixed_draft_refs_validate_identically():
    resource_uri = "https://example.com/mixed/schema.json"
    root = {"$schema": "http://json-schema.org/draft-07/schema#", "$ref": resource_uri}
    registry = jsonschema_rs.Registry(
        resources=[
            (
                resource_uri,
                {"$schema": "http://json-schema.org/draft-04/schema#", "type": "integer"},
            )
        ]
    )

    distributed = jsonschema_rs.validator_for(root, registry=registry)
    bundled = jsonschema_rs.bundle(root, registry=registry)
    bundled_validator = jsonschema_rs.validator_for(bundled)

    embedded = bundled["definitions"][resource_uri]
    assert embedded["id"] == resource_uri
    assert embedded["$id"] == resource_uri
    assert distributed.is_valid(1)
    assert bundled_validator.is_valid(1)
    assert not distributed.is_valid("x")
    assert not bundled_validator.is_valid("x")


def test_bundle_mixed_draft_const_semantics_are_preserved():
    resource_uri = "https://example.com/mixed/const.json"
    root = {"$schema": "http://json-schema.org/draft-04/schema#", "$ref": resource_uri}
    registry = jsonschema_rs.Registry(
        resources=[
            (
                resource_uri,
                {"$schema": "http://json-schema.org/draft-07/schema#", "const": 1},
            )
        ]
    )

    distributed = jsonschema_rs.validator_for(root, registry=registry)
    bundled = jsonschema_rs.bundle(root, registry=registry)
    bundled_validator = jsonschema_rs.validator_for(bundled)

    assert distributed.is_valid(1)
    assert bundled_validator.is_valid(1)
    assert not distributed.is_valid(2)
    assert not bundled_validator.is_valid(2)

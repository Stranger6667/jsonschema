import subprocess
import sys

import pytest

from jsonschema_rs import ReferencingError, Registry, ValidationError, meta


@pytest.mark.parametrize(
    "schema",
    [
        {"type": "string"},
        {"type": "number", "minimum": 0},
        {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]},
        # Boolean schemas are valid
        True,
        False,
    ],
)
def test_valid_schemas(schema):
    assert meta.is_valid(schema)
    meta.validate(schema)  # Should not raise


@pytest.mark.parametrize(
    ["schema", "expected"],
    [
        ({"type": "invalid_type"}, "is not valid"),
        ({"type": "number", "minimum": "0"}, 'is not of type "number"'),
        ({"type": "object", "required": "name"}, 'is not of type "array"'),
    ],
)
def test_invalid_schemas(schema, expected):
    assert not meta.is_valid(schema)
    with pytest.raises(ValidationError, match=expected):
        meta.validate(schema)


def test_unknown_schema_requires_known_meta():
    schema = {"$schema": "invalid-uri", "type": "string"}
    with pytest.raises(ReferencingError, match="invalid-uri"):
        meta.is_valid(schema)
    with pytest.raises(ReferencingError, match="invalid-uri"):
        meta.validate(schema)


def test_validation_error_details():
    schema = {"type": "invalid_type"}

    with pytest.raises(ValidationError) as exc_info:
        meta.validate(schema)

    error = exc_info.value
    assert hasattr(error, "message")
    assert hasattr(error, "instance_path")
    assert hasattr(error, "schema_path")
    assert "invalid_type" in str(error)


@pytest.mark.parametrize(
    "invalid_input",
    [
        None,
        lambda: None,
        object(),
        {1, 2, 3},
    ],
)
def test_type_errors(invalid_input):
    with pytest.raises((ValueError, ValidationError)):
        meta.validate(invalid_input)


@pytest.mark.parametrize(
    ["custom_meta", "schema", "valid"],
    [
        # Simple custom metaschema that only allows type: object
        (
            {"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object", "properties": {"type": {"const": "object"}}},
            {"$schema": "http://example.com/meta", "type": "object"},
            True,
        ),
        # Schema violating custom metaschema
        (
            {"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object", "properties": {"type": {"const": "object"}}},
            {"$schema": "http://example.com/meta", "type": "string"},
            False,
        ),
        # Custom metaschema requiring specific property
        (
            {
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "required": ["customRequired"],
            },
            {"$schema": "http://example.com/meta", "customRequired": "value"},
            True,
        ),
        # Schema missing required property
        (
            {
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "required": ["customRequired"],
            },
            {"$schema": "http://example.com/meta", "type": "object"},
            False,
        ),
    ],
)
def test_custom_metaschema_validation(custom_meta, schema, valid):
    registry = Registry([("http://example.com/meta", custom_meta)])

    assert meta.is_valid(schema, registry=registry) == valid

    if valid:
        meta.validate(schema, registry=registry)  # Should not raise
    else:
        with pytest.raises(ValidationError):
            meta.validate(schema, registry=registry)


def test_custom_metaschema_with_nested_references():
    # Custom metaschema that extends Draft 2020-12 with additional constraints
    custom_meta = {
        "$id": "http://example.com/custom-meta",
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "allOf": [
            {
                "properties": {
                    "type": {"type": "string"},
                    "customKeyword": {"type": "string"},
                }
            }
        ],
        "required": ["customKeyword"],
    }

    registry = Registry([("http://example.com/custom-meta", custom_meta)])

    # Valid schema with custom keyword
    valid_schema = {
        "$schema": "http://example.com/custom-meta",
        "type": "object",
        "customKeyword": "value",
    }
    assert meta.is_valid(valid_schema, registry=registry)
    meta.validate(valid_schema, registry=registry)

    # Invalid schema missing custom keyword
    invalid_schema = {
        "$schema": "http://example.com/custom-meta",
        "type": "object",
    }
    assert not meta.is_valid(invalid_schema, registry=registry)
    with pytest.raises(ValidationError, match="customKeyword"):
        meta.validate(invalid_schema, registry=registry)


def test_custom_metaschema_not_in_registry():
    # Registry doesn't contain the referenced metaschema
    registry = Registry([("http://example.com/meta1", {"type": "object"})])

    schema = {"$schema": "http://example.com/meta2", "type": "string"}

    # Should raise ReferencingError because metaschema not found
    with pytest.raises(ReferencingError, match="meta2"):
        meta.is_valid(schema, registry=registry)

    with pytest.raises(ReferencingError, match="meta2"):
        meta.validate(schema, registry=registry)


def test_custom_metaschema_circular_reference():
    # Create circular reference: meta1 -> meta2 -> meta1
    meta1 = {
        "$id": "http://example.com/meta1",
        "$schema": "http://example.com/meta2",
        "type": "object",
    }

    meta2 = {
        "$id": "http://example.com/meta2",
        "$schema": "http://example.com/meta1",
        "type": "object",
    }

    registry = Registry([
        ("http://example.com/meta1", meta1),
        ("http://example.com/meta2", meta2),
    ])

    schema = {"$schema": "http://example.com/meta1", "type": "string"}

    # Should raise ReferencingError about circular reference
    with pytest.raises(ReferencingError, match="[Cc]ircular"):
        meta.is_valid(schema, registry=registry)

    with pytest.raises(ReferencingError, match="[Cc]ircular"):
        meta.validate(schema, registry=registry)


def test_custom_metaschema_chain_resolution():
    # Test resolving a chain: custom -> draft2020-12 (built-in)
    custom_meta = {
        "$id": "http://example.com/meta",
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "$schema": {"type": "string"},
            "customProp": {"type": "string"},
        },
        "unevaluatedProperties": False,
    }

    registry = Registry([("http://example.com/meta", custom_meta)])

    # Valid schema
    valid_schema = {
        "$schema": "http://example.com/meta",
        "customProp": "value",
    }
    assert meta.is_valid(valid_schema, registry=registry)

    # Invalid schema with extra properties
    invalid_schema = {
        "$schema": "http://example.com/meta",
        "customProp": "value",
        "extraProp": "not allowed",
    }
    assert not meta.is_valid(invalid_schema, registry=registry)


def test_builtin_schemas_work_without_registry():
    # Ensure backward compatibility - built-in schemas work without registry
    schemas = [
        {"type": "string"},
        {"$schema": "http://json-schema.org/draft-04/schema#", "type": "number"},
        {"$schema": "http://json-schema.org/draft-06/schema#", "type": "boolean"},
        {"$schema": "http://json-schema.org/draft-07/schema#", "type": "array"},
        {"$schema": "https://json-schema.org/draft/2019-09/schema", "type": "object"},
        {"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "null"},
    ]

    for schema in schemas:
        assert meta.is_valid(schema)
        meta.validate(schema)


def test_builtin_schemas_work_with_empty_registry():
    # Built-in schemas should still work even if registry is provided but doesn't contain them
    registry = Registry([("http://example.com/other", {"type": "object"})])

    builtin_schema = {"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "string"}
    assert meta.is_valid(builtin_schema, registry=registry)
    meta.validate(builtin_schema, registry=registry)


def test_complex_custom_metaschema():
    # More complex metaschema similar to the Rust test
    meta_schema = {
        "$id": "http://example.com/meta/schema",
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Core schema definition",
        "type": "object",
        "allOf": [
            {"$ref": "#/$defs/core"}
        ],
        "properties": {
            "properties": {
                "type": "object",
                "patternProperties": {
                    ".*": {
                        "type": "object",
                        "properties": {
                            "type": {
                                "type": "string",
                                "enum": ["array", "boolean", "integer", "number", "object", "string", "null"],
                            }
                        },
                    }
                },
                "propertyNames": {
                    "type": "string",
                    "pattern": "^[A-Za-z_][A-Za-z0-9_]*$",
                },
            }
        },
        "unevaluatedProperties": False,
        "required": ["properties"],
        "$defs": {
            "core": {
                "type": "object",
                "properties": {
                    "$id": {"type": "string"},
                    "$schema": {"type": "string"},
                    "type": {"const": "object"},
                    "title": {"type": "string"},
                    "description": {"type": "string"},
                },
                "required": ["$id", "$schema", "type"],
            }
        },
    }

    registry = Registry([("http://example.com/meta/schema", meta_schema)])

    # Valid schema according to custom metaschema
    valid_schema = {
        "$schema": "http://example.com/meta/schema",
        "$id": "http://example.com/schemas/element",
        "title": "Element",
        "properties": {
            "value": {"type": "string"},
            "count": {"type": "number"},
        },
        "type": "object",
    }
    assert meta.is_valid(valid_schema, registry=registry)
    meta.validate(valid_schema, registry=registry)

    # Invalid - missing required 'properties'
    invalid_schema1 = {
        "$schema": "http://example.com/meta/schema",
        "$id": "http://example.com/schemas/element",
        "type": "object",
    }
    assert not meta.is_valid(invalid_schema1, registry=registry)
    with pytest.raises(ValidationError, match="properties"):
        meta.validate(invalid_schema1, registry=registry)

    # Invalid - property name doesn't match pattern
    invalid_schema2 = {
        "$schema": "http://example.com/meta/schema",
        "$id": "http://example.com/schemas/element",
        "type": "object",
        "properties": {
            "123invalid": {"type": "string"},  # Starts with number
        },
    }
    assert not meta.is_valid(invalid_schema2, registry=registry)

    # Invalid - unevaluated property
    invalid_schema3 = {
        "$schema": "http://example.com/meta/schema",
        "$id": "http://example.com/schemas/element",
        "type": "object",
        "properties": {"value": {"type": "string"}},
        "extraKeyword": "not allowed",
    }
    assert not meta.is_valid(invalid_schema3, registry=registry)


def test_custom_metaschema_with_retriever():
    # Test that registry with retriever works for custom metaschemas
    def retrieve(uri: str):
        if uri == "http://example.com/dynamic-meta":
            return {
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {"dynamicProp": {"type": "boolean"}},
            }
        raise KeyError(f"Schema not found: {uri}")

    # Register a schema that references the dynamic metaschema
    static_meta = {
        "$id": "http://example.com/meta",
        "$schema": "http://example.com/dynamic-meta",
        "type": "object",
    }

    registry = Registry(
        [("http://example.com/meta", static_meta)],
        retriever=retrieve,
    )

    schema = {
        "$schema": "http://example.com/meta",
        "dynamicProp": True,
    }

    assert meta.is_valid(schema, registry=registry)
    meta.validate(schema, registry=registry)


def test_exceptions_remain_consistent_after_reload():
    # Module reloading affects global state and can contaminate other tests that import
    # exception classes at module level. Run in subprocess for complete isolation.
    result = subprocess.run(
        [
            sys.executable,
            "-c",
            """
import importlib
import jsonschema_rs

schema = {"type": "integer"}

# Test before reload
try:
    jsonschema_rs.validate(schema, "not-int")
    raise AssertionError("Expected ValidationError")
except jsonschema_rs.ValidationError:
    pass

# Reload the module
reloaded = importlib.reload(jsonschema_rs)
assert reloaded is jsonschema_rs

# Test after reload - exceptions should still work
try:
    jsonschema_rs.validate(schema, "still-not-int")
    raise AssertionError("Expected ValidationError")
except jsonschema_rs.ValidationError:
    pass

# Test meta-validation - invalid schema should raise ValidationError
try:
    jsonschema_rs.meta.validate({"type": "invalid_type"})
    raise AssertionError("Expected ValidationError")
except jsonschema_rs.ValidationError:
    pass
""",
        ],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, f"Test failed with stdout:\n{result.stdout}\nstderr:\n{result.stderr}"

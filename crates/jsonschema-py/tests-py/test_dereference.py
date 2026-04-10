import jsonschema_rs
import pytest


@pytest.mark.parametrize(
    ("schema", "expected"),
    [
        pytest.param(
            {"type": "string", "minLength": 1},
            {"type": "string", "minLength": 1},
            id="no_refs_unchanged",
        ),
        pytest.param(
            {
                "$defs": {
                    "address": {
                        "type": "object",
                        "properties": {
                            "street": {"type": "string"},
                            "city": {"type": "string"},
                        },
                    }
                },
                "properties": {"home": {"$ref": "#/$defs/address"}},
            },
            {
                "$defs": {
                    "address": {
                        "type": "object",
                        "properties": {
                            "street": {"type": "string"},
                            "city": {"type": "string"},
                        },
                    }
                },
                "properties": {
                    "home": {
                        "type": "object",
                        "properties": {
                            "street": {"type": "string"},
                            "city": {"type": "string"},
                        },
                    }
                },
            },
            id="simple_fragment_ref_inlined",
        ),
        pytest.param(
            {
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$id": "https://example.com/node.json",
                "type": "object",
                "properties": {
                    "children": {
                        "type": "array",
                        "items": {"$ref": "https://example.com/node.json"},
                    }
                },
            },
            {
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
                                    "items": {"$ref": "https://example.com/node.json"},
                                }
                            },
                        },
                    }
                },
            },
            id="circular_ref_left_in_place",
        ),
        pytest.param(
            {
                "$defs": {"base": {"type": "integer"}},
                "properties": {
                    "count": {"$ref": "#/$defs/base", "description": "how many"}
                },
            },
            {
                "$defs": {"base": {"type": "integer"}},
                "properties": {
                    "count": {"type": "integer", "description": "how many"}
                },
            },
            id="sibling_keys_merged",
        ),
    ],
)
def test_dereference(schema, expected):
    assert jsonschema_rs.dereference(schema) == expected


def test_unknown_ref_raises_referencing_error():
    schema = {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/does-not-exist.json",
    }
    with pytest.raises(jsonschema_rs.ReferencingError):
        jsonschema_rs.dereference(schema)


def test_dereference_with_retriever():
    def retrieve(uri):
        if uri == "https://example.com/string.json":
            return {"type": "string"}
        raise KeyError(uri)

    schema = {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "https://example.com/string.json",
    }
    result = jsonschema_rs.dereference(schema, retriever=retrieve)
    assert result == {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "string",
    }

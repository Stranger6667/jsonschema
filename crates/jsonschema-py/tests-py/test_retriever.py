import json

import pytest

from jsonschema_rs import ValidationError, validator_for


def test_basic_retriever():
    def retrieve(uri: str):
        schemas = {
            "https://example.com/person.json": {
                "type": "object",
                "properties": {"name": {"type": "string"}, "age": {"type": "integer"}},
                "required": ["name", "age"],
            }
        }
        return schemas[uri]

    schema = {"$ref": "https://example.com/person.json"}
    validator = validator_for(schema, retriever=retrieve)

    assert validator.is_valid({"name": "Alice", "age": 30})
    assert not validator.is_valid({"name": "Bob"})
    assert not validator.is_valid({"age": 25})


def test_retriever_error():
    def retrieve(uri: str):
        raise KeyError(f"Schema not found: {uri}")

    schema = {"$ref": "https://example.com/nonexistent.json"}
    with pytest.raises(ValidationError) as exc:
        validator_for(schema, retriever=retrieve)
    assert "Schema not found" in str(exc.value)


def test_nested_references():
    def retrieve(uri: str):
        schemas = {
            "https://example.com/address.json": {
                "type": "object",
                "properties": {"street": {"type": "string"}, "city": {"type": "string"}},
                "required": ["street", "city"],
            },
            "https://example.com/person.json": {
                "type": "object",
                "properties": {"name": {"type": "string"}, "address": {"$ref": "https://example.com/address.json"}},
                "required": ["name", "address"],
            },
        }
        return schemas[uri]

    schema = {"$ref": "https://example.com/person.json"}
    validator = validator_for(schema, retriever=retrieve)

    assert validator.is_valid({"name": "Alice", "address": {"street": "123 Main St", "city": "Springfield"}})
    assert not validator.is_valid({"name": "Bob", "address": {"street": "456 Oak Rd"}})


def test_retriever_type_error():
    schema = {"$ref": "https://example.com/schema.json"}
    with pytest.raises(ValueError):
        validator_for(schema, retriever="not_a_function")


def test_circular_references():
    def retrieve(uri: str):
        schemas = {
            "https://example.com/person.json": {
                "type": "object",
                "properties": {"name": {"type": "string"}, "friend": {"$ref": "https://example.com/person.json"}},
                "required": ["name"],
            }
        }
        return schemas[uri]

    schema = {"$ref": "https://example.com/person.json"}
    validator = validator_for(schema, retriever=retrieve)

    assert validator.is_valid({"name": "Alice", "friend": {"name": "Bob", "friend": {"name": "Charlie"}}})


def test_base_uri_resolution(tmp_path):
    b_schema = {"type": "object", "properties": {"age": {"type": "number"}}, "required": ["age"]}
    b_file = tmp_path / "b.json"
    b_file.write_text(json.dumps(b_schema))

    a_schema = {"$schema": "https://json-schema.org/draft/2020-12/schema", "$ref": "./b.json", "type": "object"}
    a_file = tmp_path / "a.json"
    a_file.write_text(json.dumps(a_schema))

    valid = {"age": 30}
    invalid = {"age": "thirty"}

    base_uri = tmp_path.absolute().as_uri() + "/"

    validator = validator_for(a_schema, base_uri=base_uri)

    assert validator.is_valid(valid)
    assert not validator.is_valid(invalid)

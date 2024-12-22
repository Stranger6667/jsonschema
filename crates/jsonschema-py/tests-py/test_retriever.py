import pytest
from jsonschema_rs import validator_for, ValidationError


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

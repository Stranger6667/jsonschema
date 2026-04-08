import pytest

from jsonschema_rs import (
    Draft4,
    Draft4Validator,
    Draft6,
    Draft6Validator,
    Draft7,
    Draft7Validator,
    Draft201909,
    Draft201909Validator,
    Draft202012,
    Draft202012Validator,
    Registry,
    ValidationError,
    is_valid,
    iter_errors,
    validate,
    validator_for,
)

BASIC_RESOURCES = [("https://example.com/string-schema", {"type": "string"})]
NESTED_RESOURCES = [
    (
        "https://example.com/address.json",
        {
            "type": "object",
            "properties": {"street": {"type": "string"}, "city": {"type": "string"}},
            "required": ["street", "city"],
        },
    ),
    (
        "https://example.com/person.json",
        {
            "type": "object",
            "properties": {"name": {"type": "string"}, "address": {"$ref": "https://example.com/address.json"}},
            "required": ["name", "address"],
        },
    ),
]

VALID_PERSON = {"name": "John Doe", "address": {"street": "123 Main St", "city": "Springfield"}}
INVALID_PERSON = {"name": "John Doe", "address": {"street": "123 Main St"}}

VALIDATOR_CLASSES = [
    (Draft4Validator, Draft4),
    (Draft6Validator, Draft6),
    (Draft7Validator, Draft7),
    (Draft201909Validator, Draft201909),
    (Draft202012Validator, Draft202012),
]


@pytest.mark.parametrize("validator_class, draft", VALIDATOR_CLASSES)
def test_validator_classes_with_registry(validator_class, draft):
    registry = Registry(BASIC_RESOURCES, draft=draft)
    schema = {"$ref": "https://example.com/string-schema"}

    validator = validator_class(schema, registry=registry)

    assert validator.is_valid("test")
    assert not validator.is_valid(123)

    validator.validate("test")
    with pytest.raises(ValidationError):
        validator.validate(123)

    assert list(validator.iter_errors("test")) == []
    assert list(validator.iter_errors(123)) != []


@pytest.mark.parametrize("function", [validate, is_valid, iter_errors])
def test_top_level_functions_with_registry(function):
    registry = Registry(NESTED_RESOURCES)
    schema = {"$ref": "https://example.com/person.json"}

    if function == is_valid:
        assert function(schema, VALID_PERSON, registry=registry)
        assert not function(schema, INVALID_PERSON, registry=registry)
    elif function == validate:
        function(schema, VALID_PERSON, registry=registry)
        with pytest.raises(ValidationError):
            function(schema, INVALID_PERSON, registry=registry)
    else:
        assert list(function(schema, VALID_PERSON, registry=registry)) == []
        assert list(function(schema, INVALID_PERSON, registry=registry)) != []


def test_validator_for_with_registry():
    registry = Registry(NESTED_RESOURCES)
    schema = {"$ref": "https://example.com/person.json"}

    validator = validator_for(schema, registry=registry)

    assert validator.is_valid(VALID_PERSON)
    assert not validator.is_valid(INVALID_PERSON)


def test_validator_for_with_registry_and_explicit_draft4_legacy_id_root():
    registry = Registry([("urn:string", {"type": "string"})], draft=Draft4)
    schema = {
        "id": "urn:root",
        "type": "object",
        "properties": {"value": {"$ref": "urn:string"}},
        "required": ["value"],
    }

    validator = Draft4Validator(schema, registry=registry)

    assert validator.is_valid({"value": "ok"})
    assert not validator.is_valid({"value": 42})


def test_registry_with_retriever_and_validation():
    def retrieve(uri: str):
        if uri == "https://example.com/dynamic.json":
            return {"type": "number"}
        raise KeyError(f"Schema not found: {uri}")

    registry = Registry(
        BASIC_RESOURCES + [("https://example.com/inner.json", {"$ref": "https://example.com/dynamic.json"})],
        retriever=retrieve,
    )

    static_schema = {"$ref": "https://example.com/string-schema"}
    static_validator = validator_for(static_schema, registry=registry)
    assert static_validator.is_valid("test")
    assert not static_validator.is_valid(123)

    dynamic_schema = {"$ref": "https://example.com/inner.json"}
    dynamic_validator = validator_for(dynamic_schema, registry=registry)
    assert dynamic_validator.is_valid(123)
    assert not dynamic_validator.is_valid("test")


def test_validator_for_uses_call_retriever_when_inline_root_adds_external_ref():
    def retrieve(uri: str):
        if uri == "urn:external":
            return {"type": "string"}
        raise KeyError(f"Schema not found: {uri}")

    registry = Registry([("urn:seed", {"type": "integer"})])
    schema = {
        "type": "object",
        "properties": {"value": {"$ref": "urn:external"}},
        "required": ["value"],
    }

    validator = validator_for(schema, registry=registry, retriever=retrieve)
    assert validator.is_valid({"value": "ok"})
    assert not validator.is_valid({"value": 42})


def test_validator_for_with_registry_accepts_equivalent_base_uri_with_empty_fragment():
    registry = Registry([("urn:shared", {"type": "integer"})])
    schema = {"$id": "urn:root", "$ref": "urn:shared"}

    validator = validator_for(schema, registry=registry, base_uri="urn:root#")

    assert validator.is_valid(1)
    assert not validator.is_valid("x")


def test_registry_error_propagation():
    registry = Registry(NESTED_RESOURCES)

    schema = {"$ref": "https://example.com/non-existent.json"}
    with pytest.raises(ValidationError) as exc:
        validator_for(schema, registry=registry)
    assert "non-existent.json" in str(exc.value)


def test_registry_error_on_retrieval():
    def retrieve(uri: str):
        raise KeyError(f"Schema not found: {uri}")

    with pytest.raises(ValueError):
        Registry(
            [("https://example.com/inner.json", {"$ref": "https://example.com/dynamic.json"})],
            retriever=retrieve,
        )

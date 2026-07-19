import pytest

from jsonschema_rs import CanonicalSchema, ValidationError, canonical, canonicalize


@pytest.mark.parametrize(
    "schema",
    [
        {"type": "string", "minLength": 3},
        {"allOf": [{"type": "integer"}, {"minimum": 0}]},
        {"$defs": {"a": {"type": "null"}}, "$ref": "#/$defs/a"},
        {},
        True,
        False,
    ],
)
def test_round_trip_verbatim(schema):
    result = canonicalize(schema)
    assert isinstance(result, CanonicalSchema)
    assert result.to_json_schema() == schema


def test_view_is_raw():
    match canonicalize({"type": "string"}).view():
        case canonical.RawView(schema=payload):
            assert payload == {"type": "string"}
        case other:
            pytest.fail(f"unexpected view: {other!r}")


@pytest.mark.parametrize(
    ("schema", "draft", "expected"),
    [
        ({}, None, 20),
        ({"$schema": "http://json-schema.org/draft-07/schema#"}, None, 7),
        ({"$schema": "https://json-schema.org/draft/2019-09/schema"}, None, 19),
        ({}, 4, 4),
        ({}, 7, 7),
    ],
)
def test_draft(schema, draft, expected):
    kwargs = {} if draft is None else {"draft": draft}
    assert canonicalize(schema, **kwargs).draft == expected


def test_kind():
    assert canonicalize({}).kind == "raw"


def test_equality_is_document_identity():
    assert canonicalize({"const": 1}) == canonicalize({"const": 1})
    assert canonicalize({"const": 1}) != canonicalize({"const": 1.0})
    assert hash(canonicalize({"const": 1})) == hash(canonicalize({"const": 1}))


def test_definitions_is_empty():
    assert canonicalize({"$defs": {"a": {}}, "$ref": "#/$defs/a"}).definitions() == {}


def test_invalid_schema_raises_validation_error():
    with pytest.raises(ValidationError):
        canonicalize({"type": 123})


@pytest.mark.parametrize("schema", [42, "string", [1], None])
def test_invalid_schema_type(schema):
    with pytest.raises(canonical.InvalidSchemaType):
        canonicalize(schema)


def test_exception_hierarchy():
    assert issubclass(canonical.CanonicalizationError, ValueError)
    assert issubclass(canonical.InvalidSchemaType, canonical.CanonicalizationError)

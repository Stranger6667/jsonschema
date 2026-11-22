import json
import sys

import pytest

import jsonschema_rs
from jsonschema_rs import JsonTextValidator, ValidationError

SCHEMA = {
    "type": "object",
    "properties": {"age": {"type": "integer", "minimum": 0}, "tags": {"type": "array", "uniqueItems": True}},
    "required": ["age"],
}


@pytest.mark.parametrize("text", ['{"age": 5}', b'{"age": 5}'])
def test_accepts_str_and_bytes(text):
    assert JsonTextValidator(SCHEMA).is_valid(text)


@pytest.mark.parametrize(
    "text",
    [
        '{"age": 5, "tags": ["a", "b"]}',
        '{"age": -1}',
        '{"tags": [1, 1.0]}',
        '{"age": "five", "tags": [{"a": 1}, {"a": 1}]}',
        "[]",
        '"text"',
        '{"age": 5, "age": "five"}',
        '{"age": "five", "age": 5}',
    ],
)
def test_verdicts_and_errors_match_validator(text):
    expected = jsonschema_rs.validator_for(SCHEMA)
    validator = JsonTextValidator(SCHEMA)
    instance = json.loads(text)
    assert validator.is_valid(text) is expected.is_valid(instance)
    errors = [
        (error.message, error.instance, error.instance_path, error.schema_path, error.kind.name)
        for error in validator.iter_errors(text)
    ]
    assert errors == [
        (error.message, error.instance, error.instance_path, error.schema_path, error.kind.name)
        for error in expected.iter_errors(instance)
    ]
    assert validator.evaluate(text).list() == expected.evaluate(instance).list()


def test_validate_raises_the_first_error():
    with pytest.raises(ValidationError, match="-1 is less than the minimum of 0") as excinfo:
        JsonTextValidator(SCHEMA).validate('{"age": -1}')
    assert excinfo.value.instance == -1
    assert excinfo.value.instance_path == ["age"]


@pytest.mark.parametrize("text", ['{"age": ', "", "NaN", "{'age': 5}", b"\xff", '{"age": 5} trailing'])
def test_malformed_text_raises_value_error(text):
    with pytest.raises(ValueError, match="^Invalid JSON: "):
        JsonTextValidator(SCHEMA).is_valid(text)


@pytest.mark.parametrize("instance", [5, None, {"age": 5}, ["{}"], bytearray(b"{}")])
def test_other_instances_raise_type_error(instance):
    with pytest.raises(TypeError, match="^JSON text must be str or bytes, not "):
        JsonTextValidator(SCHEMA).is_valid(instance)


@pytest.mark.parametrize(
    "schema, draft, valid",
    [
        ({"format": "email"}, None, True),
        ({"format": "email"}, jsonschema_rs.Draft7, False),
        ({"$schema": "http://json-schema.org/draft-07/schema#", "format": "email"}, None, False),
    ],
)
def test_draft_comes_from_the_argument_or_the_schema(schema, draft, valid):
    assert JsonTextValidator(schema, draft=draft).is_valid('"not an email"') is valid


def test_schema_as_text():
    assert not JsonTextValidator('{"type": "integer"}').is_valid('"5"')


def test_invalid_schema_raises_validation_error():
    with pytest.raises(ValidationError):
        JsonTextValidator({"type": "unknown"})


class Even:
    seen = []

    def __init__(self, parent_schema, value, schema_path):
        pass

    def validate(self, instance):
        Even.seen.append(instance)
        if instance % 2:
            raise ValueError(f"{instance} is odd")


def test_custom_keywords_receive_python_objects():
    validator = JsonTextValidator({"properties": {"n": {"even": True}}}, keywords={"even": Even})
    Even.seen.clear()
    assert validator.is_valid('{"n": 4}')
    assert not validator.is_valid('{"n": 3}')
    with pytest.raises(ValidationError, match="3 is odd") as excinfo:
        validator.validate('{"n": 3}')
    assert isinstance(excinfo.value.__cause__, ValueError)
    assert Even.seen == [4, 3, 3]


def test_custom_formats():
    validator = JsonTextValidator({"format": "upper"}, formats={"upper": str.isupper}, validate_formats=True)
    assert validator.is_valid('"ABC"')
    assert not validator.is_valid('"abc"')


def test_number_formats():
    is_even = jsonschema_rs.number_format(lambda number: number % 2 == 0)
    validator = JsonTextValidator({"format": "even"}, formats={"even": is_even}, validate_formats=True)
    assert validator.is_valid("4")
    assert not validator.is_valid("3")
    assert validator.is_valid('"3"')


def test_mask_hides_the_instance():
    with pytest.raises(ValidationError) as excinfo:
        JsonTextValidator({"type": "integer"}, mask="[REDACTED]").validate('"secret"')
    assert "secret" not in str(excinfo.value)
    assert "[REDACTED]" in str(excinfo.value)


@pytest.mark.parametrize(
    "uri, name",
    [
        ("http://json-schema.org/draft-04/schema#", "Draft4"),
        ("http://json-schema.org/draft-06/schema#", "Draft6"),
        ("http://json-schema.org/draft-07/schema#", "Draft7"),
        ("https://json-schema.org/draft/2019-09/schema", "Draft201909"),
        ("https://json-schema.org/draft/2020-12/schema", "Draft202012"),
    ],
)
def test_repr(uri, name):
    assert repr(JsonTextValidator({"$schema": uri})) == f"<JsonTextValidator: {name}>"


class OddMembers:
    def __init__(self, parent_schema, value, schema_path):
        pass

    def validate(self, instance):
        pass

    def iter_errors(self, instance):
        for key, value in instance.items():
            if value % 2:
                yield ValueError(f"{key} is odd")


def test_custom_keywords_report_every_error():
    schema = {"oddMembers": True}
    text = '{"a": 1, "b": 2, "c": 3}'
    keywords = {"oddMembers": OddMembers}
    expected = [
        error.message for error in jsonschema_rs.validator_for(schema, keywords=keywords).iter_errors(json.loads(text))
    ]
    assert expected == ["a is odd", "c is odd"]
    assert [error.message for error in JsonTextValidator(schema, keywords=keywords).iter_errors(text)] == expected


@pytest.mark.parametrize(
    "encoding", ["utf-8-sig", "utf-16", "utf-16-le", "utf-16-be", "utf-32", "utf-32-le", "utf-32-be"]
)
def test_bytes_in_the_encodings_json_loads_reads(encoding):
    data = '{"age": 5, "name": "é"}'.encode(encoding)
    assert json.loads(data) == {"age": 5, "name": "é"}
    assert JsonTextValidator(SCHEMA).is_valid(data)
    assert not JsonTextValidator({"properties": {"name": {"maxLength": 0}}}).is_valid(data)


DOCUMENT = '{"b": 1, "a": [2.50, 1E2, 12345678901234567890123, -0, "x"], "c": {"z": null, "y": true}}'


class Record:
    seen = []

    def __init__(self, parent_schema, value, schema_path):
        pass

    def validate(self, instance):
        Record.seen.append(instance)


def test_custom_keywords_receive_what_json_loads_builds():
    Record.seen.clear()
    assert JsonTextValidator({"record": True}, keywords={"record": Record}).is_valid(DOCUMENT)
    expected = json.loads(DOCUMENT)
    assert Record.seen == [expected]
    seen = Record.seen[0]
    assert [list(seen), list(seen["c"])] == [["b", "a", "c"], ["z", "y"]]
    assert [type(item) for item in seen["a"]] == [type(item) for item in expected["a"]]


def test_error_instances_are_what_json_loads_builds():
    error = next(JsonTextValidator({"maxProperties": 1}).iter_errors(DOCUMENT))
    expected = json.loads(DOCUMENT)
    assert error.instance == expected
    assert list(error.instance) == list(expected)
    assert [type(item) for item in error.instance["a"]] == [type(item) for item in expected["a"]]


def test_repeated_keys_report_like_json_loads():
    schema = {"properties": {"c": {}}, "additionalProperties": False}
    text = '{"a": 1, "b": 2, "a": 3}'
    expected = [error.message for error in jsonschema_rs.validator_for(schema).iter_errors(json.loads(text))]
    assert [error.message for error in JsonTextValidator(schema).iter_errors(text)] == expected


# `json.loads` raises for integers past `sys.get_int_max_str_digits()`, so a keyword cannot get one.
@pytest.mark.skipif(not hasattr(sys, "get_int_max_str_digits"), reason="no integer string limit")
@pytest.mark.parametrize("method", ["is_valid", "validate", "iter_errors", "evaluate"])
def test_custom_keywords_raise_for_integers_python_cannot_build(method):
    text = '{"n": ' + "1" * (sys.get_int_max_str_digits() + 1) + "}"
    with pytest.raises(ValueError, match="Exceeds the limit"):
        json.loads(text)
    validator = JsonTextValidator({"properties": {"n": {"record": True}}}, keywords={"record": Record})
    with pytest.raises(ValueError, match="Exceeds the limit") as excinfo:
        result = getattr(validator, method)(text)
        if method == "iter_errors":
            list(result)
    assert type(excinfo.value) is ValueError


# An error builds only its own instance, so values elsewhere in the document never need converting.
@pytest.mark.skipif(not hasattr(sys, "get_int_max_str_digits"), reason="no integer string limit")
def test_errors_build_only_their_own_instance():
    text = '{"age": 5, "big": ' + "1" * (sys.get_int_max_str_digits() + 1) + ', "tags": [1, 1]}'
    validator = JsonTextValidator(SCHEMA)
    assert not validator.is_valid(text)
    with pytest.raises(ValidationError) as excinfo:
        validator.validate(text)
    assert excinfo.value.instance == [1, 1]
    assert [error.instance for error in validator.iter_errors(text)] == [[1, 1]]

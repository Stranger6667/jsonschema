import sys
import uuid
from collections import OrderedDict, namedtuple
from contextlib import suppress
import enum as E
from functools import partial
from decimal import Decimal

import pytest
from hypothesis import given
from hypothesis import strategies as st

from jsonschema_rs import (
    Draft4Validator,
    Draft6Validator,
    Draft7Validator,
    Draft201909Validator,
    Draft202012Validator,
    Registry,
    ValidationError,
    ValidationErrorKind,
    is_valid,
    iter_errors,
    validate,
    validator_for,
)

json = st.recursive(
    st.none()
    | st.booleans()
    | st.floats()
    | st.integers(min_value=-sys.maxsize - 1, max_value=sys.maxsize)
    | st.text(),
    lambda children: st.lists(children, min_size=1) | st.dictionaries(st.text(), children, min_size=1),
)


@pytest.mark.parametrize("func", (is_valid, validate))
@given(instance=json)
def test_instance_processing(func, instance):
    with suppress(Exception):
        func(True, instance)


@pytest.mark.parametrize("func", (is_valid, validate))
@given(instance=json)
def test_schema_processing(func, instance):
    try:
        func(instance, True)
    except Exception:
        pass


@pytest.mark.parametrize("func", (is_valid, validate))
def test_invalid_schema(func):
    with pytest.raises(ValueError):
        func(2**64, True)


@pytest.mark.parametrize("func", (is_valid, validate))
def test_invalid_type(func):
    with pytest.raises(ValueError, match="Unsupported type: 'set'"):
        func(set(), True)


def test_repr():
    assert repr(validator_for({"minimum": 5})) == "<Draft202012Validator>"


@pytest.mark.parametrize(
    "func",
    (
        validator_for({"minimum": 5}).validate,
        validator_for('{"minimum": 5}').validate,
        partial(validate, {"minimum": 5}),
    ),
)
def test_validate(func):
    with pytest.raises(ValidationError, match="2 is less than the minimum of 5") as exc:
        func(2)
    assert isinstance(exc.value.kind, ValidationErrorKind.Minimum)
    assert exc.value.instance == 2


def test_from_str_error():
    with pytest.raises(ValidationError, match='42 is not of types "boolean", "object"'):
        validator_for(42)  # type: ignore


@pytest.mark.parametrize(
    "val",
    (
        ("A", "B", "C"),
        ["A", "B", "C"],
    ),
)
def test_array_tuple(val):
    schema = {"type": "array", "items": {"type": "string"}}
    validate(schema, val)


@pytest.mark.parametrize(
    "val",
    ((1, 2, 3), [1, 2, 3], {"foo": 1}),
)
def test_array_tuple_invalid(val):
    schema = {"type": "array", "items": {"type": "string"}}
    with pytest.raises(ValueError):
        validate(schema, val)


def test_named_tuple():
    Person = namedtuple("Person", "first_name last_name")
    person_a = Person("Joe", "Smith")
    schema = {"type": "array", "items": {"type": "string"}}
    with pytest.raises(ValueError):
        validate(schema, person_a)


def test_recursive_dict():
    instance = {}
    instance["foo"] = instance
    with pytest.raises(ValueError):
        is_valid(True, instance)


def test_recursive_list():
    instance = []
    instance.append(instance)
    with pytest.raises(ValueError):
        is_valid(True, instance)


def test_paths():
    with pytest.raises(ValidationError) as exc:
        validate({"prefixItems": [{"type": "string"}]}, [1])
    assert exc.value.schema_path == ["prefixItems", 0, "type"]
    assert exc.value.instance_path == [0]
    assert exc.value.message == '1 is not of type "string"'
    assert isinstance(exc.value.kind, ValidationErrorKind.Type)
    assert exc.value.instance == 1


def test_validation_error_handles_huge_scientific_numbers():
    validator = validator_for('{"const": 1e10000}')
    with pytest.raises(ValidationError) as exc:
        validator.validate(0)
    assert isinstance(exc.value.kind, ValidationErrorKind.Constant)
    assert exc.value.kind.expected_value == Decimal("1e10000")


@pytest.mark.parametrize(
    ["schema", "instance", "kind", "attrs"],
    [
        ({"maxItems": 1}, [1, 2], ValidationErrorKind.MaxItems, {"limit": 1}),
        ({"const": "test"}, "wrong", ValidationErrorKind.Constant, {"expected_value": "test"}),
        ({"contains": {"type": "string"}}, [1, 2, 3], ValidationErrorKind.Contains, {}),
        ({"enum": [1, 2, 3]}, 4, ValidationErrorKind.Enum, {"options": [1, 2, 3]}),
        ({"exclusiveMaximum": 5}, 5, ValidationErrorKind.ExclusiveMaximum, {"limit": 5}),
        ({"exclusiveMinimum": 5}, 5, ValidationErrorKind.ExclusiveMinimum, {"limit": 5}),
        (False, "anything", ValidationErrorKind.FalseSchema, {}),
        ({"format": "email"}, "not-an-email", ValidationErrorKind.Format, {"format": "email"}),
        ({"maximum": 5}, 6, ValidationErrorKind.Maximum, {"limit": 5}),
        ({"maxLength": 5}, "too long string", ValidationErrorKind.MaxLength, {"limit": 5}),
        ({"maxProperties": 1}, {"a": 1, "b": 2}, ValidationErrorKind.MaxProperties, {"limit": 1}),
        ({"minItems": 2}, [1], ValidationErrorKind.MinItems, {"limit": 2}),
        ({"minimum": 5}, 4, ValidationErrorKind.Minimum, {"limit": 5}),
        ({"minLength": 5}, "foo", ValidationErrorKind.MinLength, {"limit": 5}),
        ({"minProperties": 2}, {"a": 1}, ValidationErrorKind.MinProperties, {"limit": 2}),
        ({"multipleOf": 2}, 3, ValidationErrorKind.MultipleOf, {"multiple_of": 2.0}),
        ({"not": {"type": "string"}}, "string", ValidationErrorKind.Not, {"schema": {"type": "string"}}),
        ({"oneOf": [{"type": "string"}, {"type": "string"}]}, "1", ValidationErrorKind.OneOfMultipleValid, {}),
        ({"pattern": "^test$"}, "wrong", ValidationErrorKind.Pattern, {"pattern": "^test$"}),
        ({"required": ["missing"]}, {}, ValidationErrorKind.Required, {"property": "missing"}),
        ({"type": "string"}, 1, ValidationErrorKind.Type, {"types": ["string"]}),
        ({"uniqueItems": True}, [1, 1], ValidationErrorKind.UniqueItems, {}),
    ],
)
def test_validation_error_kinds(schema, instance, kind, attrs):
    with pytest.raises(ValidationError) as exc:
        validate(schema, instance, validate_formats=True)

    assert isinstance(exc.value.kind, kind)

    for attr, expected_value in attrs.items():
        assert hasattr(exc.value.kind, attr)
        assert getattr(exc.value.kind, attr) == expected_value


@pytest.mark.parametrize(
    ["schema", "instance", "kind", "context"],
    [
        (
            {"anyOf": [{"type": "string"}, {"type": "number"}]},
            True,
            ValidationErrorKind.AnyOf,
            [
                [
                    ValidationError(
                        'true is not of type "string"',
                        "",
                        ["anyOf", 0, "type"],
                        [],
                        ValidationErrorKind.Type(["string"]),
                        True,
                    )
                ],
                [
                    ValidationError(
                        'true is not of type "number"',
                        "",
                        ["anyOf", 1, "type"],
                        [],
                        ValidationErrorKind.Type(["number"]),
                        True,
                    )
                ],
            ],
        ),
        (
            {"oneOf": [{"type": "number"}, {"type": "number"}]},
            "1",
            ValidationErrorKind.OneOfNotValid,
            [
                [
                    ValidationError(
                        '"1" is not of type "number"',
                        "",
                        ["oneOf", 0, "type"],
                        [],
                        ValidationErrorKind.Type(["number"]),
                        "1",
                    )
                ],
                [
                    ValidationError(
                        '"1" is not of type "number"',
                        "",
                        ["oneOf", 1, "type"],
                        [],
                        ValidationErrorKind.Type(["number"]),
                        "1",
                    )
                ],
            ],
        ),
    ],
)
def test_validation_error_kinds_with_context(schema, instance, kind, context):
    with pytest.raises(ValidationError) as exc:
        validate(schema, instance, validate_formats=True)

    assert isinstance(exc.value.kind, kind)

    for schema_id, errors in enumerate(context):
        for index, expected_error in enumerate(errors):
            actual_error = exc.value.kind.context[schema_id][index]
            assert actual_error.message == expected_error.message
            assert actual_error.schema_path == expected_error.schema_path
            assert actual_error.instance_path == expected_error.instance_path
            assert isinstance(actual_error.kind, type(expected_error.kind))
            assert actual_error.instance == expected_error.instance


def test_one_of_multiple_valid_with_context():
    schema = {
        "oneOf": [
            {"type": "string"},
            {"minLength": 1},
            {"maxLength": 3},
        ]
    }
    instance = "hello"

    with pytest.raises(ValidationError) as exc:
        validate(schema, instance)

    assert isinstance(exc.value.kind, ValidationErrorKind.OneOfMultipleValid)
    assert len(exc.value.kind.context) == 3
    assert len(exc.value.kind.context[0]) == 0  # First schema (type: string) passed
    assert len(exc.value.kind.context[1]) == 0  # Second schema (minLength: 1) passed
    assert len(exc.value.kind.context[2]) == 1  # Third schema (maxLength: 3) failed


@given(minimum=st.integers().map(abs))
def test_minimum(minimum):
    with suppress(SystemError, ValueError):
        assert is_valid({"minimum": minimum}, minimum)
        assert is_valid({"minimum": minimum}, minimum - 1) is False


@given(maximum=st.integers().map(abs))
def test_maximum(maximum):
    with suppress(SystemError, ValueError):
        assert is_valid({"maximum": maximum}, maximum)
        assert is_valid({"maximum": maximum}, maximum + 1) is False


@pytest.mark.parametrize("method", ("is_valid", "validate"))
def test_invalid_value(method):
    schema = validator_for({"minimum": 42})
    with pytest.raises(ValueError, match="Unsupported type: 'object'"):
        getattr(schema, method)(object())


def test_invalid_schema_keyword():
    # Use a genuinely invalid/unknown meta-schema URL
    schema = {"$schema": "https://json-schema.org/draft-99/schema"}
    with pytest.raises(ValidationError, match="Unknown meta-schema: 'https://json-schema.org/draft-99/schema'"):
        validator_for(schema)


def test_error_message():
    schema = {"properties": {"foo": {"type": "integer"}}}
    instance = {"foo": None}
    try:
        validate(schema, instance)
        pytest.fail("Validation error should happen")
    except ValidationError as exc:
        assert (
            str(exc)
            == """null is not of type "integer"

Failed validating "type" in schema["properties"]["foo"]

On instance["foo"]:
    null"""
        )
        assert isinstance(exc.kind, ValidationErrorKind.Type)
        assert exc.instance is None


def test_error_instance():
    instance = {"a": [42]}
    try:
        validate({"type": "array"}, instance)
        pytest.fail("Validation error should happen")
    except ValidationError as exc:
        assert isinstance(exc.kind, ValidationErrorKind.Type)
        assert exc.instance == instance
    try:
        validate({"properties": {"a": {"type": "object"}}}, instance)
        pytest.fail("Validation error should happen")
    except ValidationError as exc:
        assert isinstance(exc.kind, ValidationErrorKind.Type)
        assert exc.instance == instance["a"]


SCHEMA = {"properties": {"foo": {"type": "integer"}, "bar": {"type": "string"}}}


@pytest.mark.parametrize(
    "func",
    (
        validator_for(SCHEMA).iter_errors,
        partial(iter_errors, SCHEMA),
    ),
)
def test_iter_err_message(func):
    errors = func({"foo": None, "bar": None})

    first = next(errors)
    assert first.message == 'null is not of type "string"'

    second = next(errors)
    assert second.message == 'null is not of type "integer"'

    with suppress(StopIteration):
        next(errors)
        pytest.fail("Validation error should happen")


@pytest.mark.parametrize(
    "func",
    (
        validator_for({"properties": {"foo": {"type": "integer"}}}).iter_errors,
        partial(iter_errors, {"properties": {"foo": {"type": "integer"}}}),
    ),
)
def test_iter_err_empty(func):
    instance = {"foo": 1}
    errs = func(instance)
    with suppress(StopIteration):
        next(errs)
        pytest.fail("Validation error should happen")


class StrEnum(E.Enum):
    bar = "bar"
    foo = "foo"


class IntEnum(E.Enum):
    bar = 1
    foo = 2


@pytest.mark.parametrize(
    "type_, value, expected",
    (
        ("number", IntEnum.bar, True),
        ("number", StrEnum.bar, False),
        ("string", IntEnum.bar, False),
        ("string", StrEnum.bar, True),
    ),
)
def test_enums(type_, value, expected):
    schema = {"properties": {"foo": {"type": type_}}}
    instance = {"foo": value}
    assert is_valid(schema, instance) is expected


def test_dict_with_non_str_keys():
    schema = {"type": "object"}
    instance = {uuid.uuid4(): "foo"}
    with pytest.raises(ValueError, match="Dict key must be str or str enum. Got 'UUID'"):
        validate(schema, instance)


class MyDict(dict):
    pass


class MyDict2(MyDict):
    pass


@pytest.mark.parametrize(
    "type_, value, expected",
    (
        (dict, 1, True),
        (dict, "bar", False),
        (OrderedDict, 1, True),
        (OrderedDict, "bar", False),
        (MyDict, 1, True),
        (MyDict, "bar", False),
        (MyDict2, 1, True),
        (MyDict2, "bar", False),
    ),
)
def test_dict_subclasses(type_, value, expected):
    schema = {"type": "object", "properties": {"foo": {"type": "integer"}}}
    document = type_({"foo": value})
    assert is_valid(schema, document) is expected


def test_custom_format():
    def is_currency(value):
        return len(value) == 3 and value.isascii()

    validator = validator_for(
        {"type": "string", "format": "currency"}, formats={"currency": is_currency}, validate_formats=True
    )
    assert validator.is_valid("USD")
    assert not validator.is_valid(42)
    assert not validator.is_valid("invalid")


def test_custom_format_invalid_callback():
    with pytest.raises(ValueError, match="Format checker for 'currency' must be a callable"):
        validator_for({"type": "string", "format": "currency"}, formats={"currency": 42})


def test_custom_format_with_exception():
    def is_currency(_):
        raise ValueError("Invalid currency")

    schema = {"type": "string", "format": "currency"}
    formats = {"currency": is_currency}
    validator = validator_for(schema, formats=formats, validate_formats=True)
    with pytest.raises(ValueError, match="Invalid currency"):
        validator.is_valid("USD")
    with pytest.raises(ValueError, match="Invalid currency"):
        validator.validate("USD")
    with pytest.raises(ValueError, match="Invalid currency"):
        for _ in validator.iter_errors("USD"):
            pass
    with pytest.raises(ValueError, match="Invalid currency"):
        is_valid(schema, "USD", formats=formats, validate_formats=True)
    with pytest.raises(ValueError, match="Invalid currency"):
        validate(schema, "USD", formats=formats, validate_formats=True)
    with pytest.raises(ValueError, match="Invalid currency"):
        for _ in iter_errors(schema, "USD", formats=formats, validate_formats=True):
            pass


@pytest.mark.parametrize(
    "cls,validate_formats,input,expected",
    [
        (Draft202012Validator, None, "2023-05-17", True),
        (Draft202012Validator, True, "2023-05-17", True),
        (Draft202012Validator, True, "not a date", False),
        (Draft202012Validator, False, "2023-05-17", True),
        (Draft202012Validator, False, "not a date", True),
        (Draft201909Validator, None, "2023-05-17", True),
        # Formats are not validated at all by default in these drafts
        (Draft202012Validator, None, "not a date", True),
        (Draft201909Validator, None, "not a date", True),
        (Draft7Validator, None, "2023-05-17", True),
        (Draft7Validator, None, "not a date", False),
        (Draft6Validator, None, "2023-05-17", True),
        (Draft6Validator, None, "not a date", False),
        (Draft4Validator, None, "2023-05-17", True),
        (Draft4Validator, None, "not a date", False),
    ],
)
def test_validate_formats(cls, validate_formats, input, expected):
    schema = {"type": "string", "format": "date"}
    validator = cls(schema, validate_formats=validate_formats)
    assert validator.is_valid(input) == expected


@pytest.mark.parametrize(
    "cls,ignore_unknown_formats,should_raise",
    [
        (Draft202012Validator, None, False),
        (Draft202012Validator, True, False),
        (Draft201909Validator, None, False),
        (Draft201909Validator, True, False),
        (Draft7Validator, None, False),
        (Draft7Validator, False, True),
        (Draft6Validator, None, False),
        (Draft6Validator, False, True),
        (Draft4Validator, None, False),
        (Draft4Validator, False, True),
        # Formats are not validated at all by default in these drafts
        (Draft202012Validator, False, False),
        (Draft201909Validator, False, False),
    ],
)
def test_ignore_unknown_formats(cls, ignore_unknown_formats, should_raise):
    unknown_format_schema = {"type": "string", "format": "unknown"}
    if should_raise:
        with pytest.raises(ValidationError):
            cls(unknown_format_schema, ignore_unknown_formats=ignore_unknown_formats)
    else:
        validator = cls(unknown_format_schema, ignore_unknown_formats=ignore_unknown_formats)
        assert validator.is_valid("any string")


def test_unicode_pattern():
    validator = Draft202012Validator({"pattern": "aaaaaaaèaaéaaaaéè"})
    assert not validator.is_valid("a")


enum_type_dec = pytest.mark.parametrize(
    "enum_type",
    (
        "old",
        pytest.param(
            "new",
            marks=pytest.mark.skipif(sys.version_info < (3, 11), reason="New style enums are supported from 3.11"),
        ),
    ),
)


@enum_type_dec
@pytest.mark.parametrize("valid", (True, False))
def test_enum_as_keys(enum_type, valid):
    if enum_type == "old":
        bases = (str, E.Enum)
    else:
        bases = (E.StrEnum,)

    class EnumCls(*bases):
        A = "a"
        B = "b"

    schema = {
        "properties": {
            "a": {"type": "string"},
        },
        "additionalProperties": False,
    }
    if valid:
        schema["properties"]["b"] = {"type": "number"}

    assert is_valid(schema, {EnumCls.A: "xyz", EnumCls.B: 42}) is valid


@enum_type_dec
def test_enum_as_keys_invalid(enum_type):
    if enum_type == "old":

        class EnumCls(E.IntEnum):
            A = 42
    else:

        class EnumCls(E.Enum):
            A = "a"

    schema = {"properties": {"a": {"type": "string"}}}
    with pytest.raises(ValueError, match=f"Dict key must be str or str enum. Got '{EnumCls.__name__}'"):
        is_valid(schema, {EnumCls.A: "xyz"})


def test_validate_error_instance_path_traverses_instance():
    schema = {
        "type": "object",
        "properties": {
            "table-node": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {"~": {"type": "string", "minLength": 1}},
                    "required": ["~"],
                },
            }
        },
        "$schema": "https://json-schema.org/draft/2020-12/schema",
    }

    instance = {
        "table-node": [
            {"~": ""},
            {"other-value": ""},
        ],
    }

    with pytest.raises(ValidationError) as excinfo:
        validate(schema, instance)

    error = excinfo.value

    # Traverse instance using the `instance_path`` segments
    current = instance
    for segment in error.instance_path:
        if isinstance(segment, int):
            current = current[segment]
        elif isinstance(segment, str):
            current = current[segment]

    assert current == instance["table-node"][0]["~"]


def test_custom_meta_schema_unregistered():
    schema = {
        "$schema": "http://example.com/meta/custom-schema",
        "type": "object",
        "properties": {"name": {"type": "string"}},
        "required": ["name"],
    }

    with pytest.raises(ValidationError, match="Unknown meta-schema"):
        validator_for(schema)


def test_custom_meta_schema_registered():
    meta_schema = {
        "$id": "http://example.com/meta/schema",
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Core schema definition",
        "type": "object",
        "allOf": [{"$ref": "#/$defs/editable"}, {"$ref": "#/$defs/core"}],
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
                "propertyNames": {"type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_]*$"},
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
                    "additionalProperties": {"type": "boolean", "const": False},
                },
                "required": ["$id", "$schema", "type"],
            },
            "editable": {
                "type": "object",
                "properties": {
                    "creationDate": {"type": "string", "format": "date-time"},
                    "updateDate": {"type": "string", "format": "date-time"},
                },
                "required": ["creationDate"],
            },
        },
    }

    element_schema = {
        "$schema": "http://example.com/meta/schema",
        "$id": "http://example.com/schemas/element",
        "title": "Element",
        "description": "An element",
        "creationDate": "2024-12-31T12:31:53+01:00",
        "properties": {"value": {"type": "string"}},
        "type": "object",
    }

    registry = Registry(
        [("http://example.com/meta/schema", meta_schema), ("http://example.com/schemas/element", element_schema)]
    )

    validator = validator_for(element_schema, registry=registry)

    assert validator.is_valid({"value": "test string"})
    assert not validator.is_valid({"value": 123})


@pytest.mark.parametrize(
    "schema, instance, expected",
    [
        # minimum with large integers
        ({"minimum": 18446744073709551616}, 18446744073709551617, True),
        ({"minimum": 18446744073709551616}, 18446744073709551615, False),
        # maximum with large integers
        ({"maximum": 18446744073709551616}, 18446744073709551615, True),
        ({"maximum": 18446744073709551616}, 18446744073709551617, False),
        # exclusiveMinimum with large integers
        ({"exclusiveMinimum": 18446744073709551616}, 18446744073709551617, True),
        ({"exclusiveMinimum": 18446744073709551616}, 18446744073709551616, False),
        # exclusiveMaximum with large integers
        ({"exclusiveMaximum": 18446744073709551616}, 18446744073709551615, True),
        ({"exclusiveMaximum": 18446744073709551616}, 18446744073709551616, False),
        # negative large integers
        ({"minimum": -9223372036854775809}, -9223372036854775808, True),
        ({"minimum": -9223372036854775809}, -9223372036854775810, False),
        # multipleOf with large integers
        ({"multipleOf": 18446744073709551616}, 36893488147419103232, True),
        ({"multipleOf": 18446744073709551616}, 18446744073709551617, False),
    ],
)
def test_large_integer_validation(schema, instance, expected):
    assert is_valid(schema, instance) == expected


@pytest.mark.parametrize(
    ["schema", "instance", "expected_instance_in_error", "expected_schema_in_error"],
    [
        # const with large integer - both values should be exact
        (
            {"const": 18446744073709551616},
            18446744073709551617,
            "18446744073709551617",
            "18446744073709551616",
        ),
        # const with negative large integer
        (
            {"const": -9223372036854775809},
            -9223372036854775808,
            "-9223372036854775808",
            "-9223372036854775809",
        ),
        # minimum with large integer - both should be exact
        (
            {"minimum": 18446744073709551616},
            18446744073709551615,
            "18446744073709551615",
            "18446744073709551616",
        ),
        # maximum with large integer
        (
            {"maximum": 18446744073709551616},
            18446744073709551617,
            "18446744073709551617",
            "18446744073709551616",
        ),
        # multipleOf with large integers
        (
            {"multipleOf": 18446744073709551616},
            18446744073709551617,
            "18446744073709551617",
            "18446744073709551616",
        ),
        # very large integer beyond u64 range
        (
            {"const": 99999999999999999999999999999999999999},
            99999999999999999999999999999999999998,
            "99999999999999999999999999999999999998",
            "99999999999999999999999999999999999999",
        ),
    ],
)
def test_large_number_precision_in_errors(schema, instance, expected_instance_in_error, expected_schema_in_error):
    with pytest.raises(ValidationError) as exc:
        validate(schema, instance)

    error_msg = str(exc.value)
    # Both the instance value and schema value should appear with full precision
    assert expected_instance_in_error in error_msg, (
        f"Instance value '{expected_instance_in_error}' not found in error message: {error_msg}"
    )
    assert expected_schema_in_error in error_msg, (
        f"Schema value '{expected_schema_in_error}' not found in error message: {error_msg}"
    )


@pytest.mark.parametrize(
    "schema, instance",
    [
        ({"minimum": 18446744073709551616}, 18446744073709551617),
        ({"multipleOf": 18446744073709551616}, 36893488147419103232),
        ({"const": 99999999999999999999}, 99999999999999999999),
        ({"multipleOf": 0.1}, 0.3),
    ],
)
def test_large_number_iter_errors(schema, instance):
    errors = list(iter_errors(schema, instance))
    assert len(errors) == 0


@pytest.mark.parametrize(
    "schema, instance",
    [
        ({"minimum": 18446744073709551616}, 18446744073709551615),
        ({"multipleOf": 18446744073709551616}, 18446744073709551617),
        ({"const": 99999999999999999999}, 99999999999999999998),
    ],
)
def test_large_number_validation_error_attributes(schema, instance):
    errors = list(iter_errors(schema, instance))
    assert len(errors) == 1
    error = errors[0]
    assert error.instance == instance
    assert isinstance(error.message, str)
    assert len(error.message) > 0


@pytest.mark.parametrize(
    "instance, expected",
    [
        (0.01, True),
        (0.02, True),
        (1.23, True),
        (0.015, False),
    ],
)
def test_float_precision_from_string_schema(instance, expected):
    # Schema values are no longer converted to f64, maintaining better precision
    validator = validator_for('{"multipleOf": 0.01}')
    assert validator.is_valid(instance) == expected

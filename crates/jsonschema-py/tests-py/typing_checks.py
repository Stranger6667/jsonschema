# Checked by type checkers in CI, not collected by pytest.
from typing import Any, Callable

from typing_extensions import assert_type

import jsonschema_rs
from jsonschema_rs import ValidationErrorKind
from jsonschema_rs.canonical import MultiTypeView, StringView

Schema = dict[str, Any] | bool


def is_currency(value: str) -> bool:
    return len(value) == 3


def build_through_class(
    cls: type[jsonschema_rs.Validator], schema: Schema, formats: dict[str, Callable[[Any], bool]] | None
) -> jsonschema_rs.Validator:
    return cls(schema, validate_formats=True, formats=formats)


def build_from_union(schema: Schema) -> list[jsonschema_rs.Validator]:
    return [
        jsonschema_rs.validator_for(schema, formats={"currency": is_currency}),
        jsonschema_rs.validator_for(
            schema,
            formats={"currency": is_currency, "int32": jsonschema_rs.number_format(lambda number: number < 2**31)},
        ),
        jsonschema_rs.Draft4Validator(schema),
        jsonschema_rs.validator_cls_for(schema)(schema),
    ]


def json_text(schema: Schema, text: str | bytes) -> list[str]:
    validator = jsonschema_rs.JsonTextValidator(schema, draft=jsonschema_rs.Draft7, validate_formats=True)
    assert_type(validator.is_valid(text), bool)
    validator.validate(text)
    return [error.message for error in validator.iter_errors(text)]


def module_functions(schema: Schema, instance: object) -> list[str]:
    jsonschema_rs.validate(schema, instance, vocabularies=("https://json-schema.org/draft/2020-12/vocab/core",))
    assert jsonschema_rs.is_valid(schema, instance)
    return [error.message for error in jsonschema_rs.iter_errors(schema, instance)]


def retrieve(url: str) -> dict[str, object]:
    return {}


def registry() -> jsonschema_rs.Registry:
    jsonschema_rs.Registry([("http://example.com/a", True)])
    return jsonschema_rs.Registry(((uri, {}) for uri in ["http://example.com/b"]), retriever=retrieve)


def bundle_keeps_type(schema: dict[str, Any], flag: bool) -> None:
    assert_type(jsonschema_rs.bundle(flag), bool)
    assert_type(jsonschema_rs.bundle(schema), dict[str, Any])
    assert_type(jsonschema_rs.dereference(flag), bool)
    assert_type(jsonschema_rs.dereference(schema), dict[str, Any])


def match_kind(error: jsonschema_rs.ValidationError) -> str:
    match error.kind:
        case ValidationErrorKind.Type(types):
            assert_type(types, list[str])
            return ",".join(types)
        case ValidationErrorKind.Required(property=name):
            assert_type(name, str)
            return name
    return ""


def match_view(schema: jsonschema_rs.CanonicalSchema) -> None:
    match schema.view():
        case MultiTypeView(types):
            assert_type(types, list[str])
        case StringView(min_length=min_length):
            assert_type(min_length, int | None)


def catch_referencing_error() -> str:
    try:
        jsonschema_rs.dereference({"$ref": "http://example.com/missing"})
    except jsonschema_rs.ReferencingError as error:
        return error.message
    return ""


def canonical_helpers(value: object) -> str:
    assert_type(jsonschema_rs.canonical.schema.clone({"a": 1}), dict[str, int])
    return jsonschema_rs.canonical.json.to_string(value)

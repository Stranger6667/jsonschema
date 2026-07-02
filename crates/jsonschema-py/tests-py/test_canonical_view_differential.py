import jsonschema_rs
import pytest


# Minimal acceptance decided by walking views.
def accepts(view, value):
    match view:
        case jsonschema_rs.canonical.TrueView():
            return True
        case jsonschema_rs.canonical.FalseView():
            return False
        case jsonschema_rs.canonical.IntegerView():
            if not isinstance(value, int) or isinstance(value, bool):
                return False
            return _numeric_ok(view, value)
        case jsonschema_rs.canonical.NumberView():
            if isinstance(value, bool) or not isinstance(value, (int, float)):
                return False
            return _numeric_ok(view, value)
        case jsonschema_rs.canonical.MultiTypeView(types=types):
            return _json_type(value) in set(types)
        case jsonschema_rs.canonical.ConstView(value=expected):
            return value == expected
        case jsonschema_rs.canonical.EnumView(values=values):
            return value in values
        case _:
            raise AssertionError(f"unhandled view: {view!r}")


def _numeric_ok(view, value):
    if view.minimum is not None and value < view.minimum:
        return False
    if view.maximum is not None and value > view.maximum:
        return False
    if view.exclusive_minimum is not None and value <= view.exclusive_minimum:
        return False
    if view.exclusive_maximum is not None and value >= view.exclusive_maximum:
        return False
    if view.multiple_of is not None and (value % view.multiple_of) != 0:
        return False
    return True


def _json_type(value):
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, int):
        return "integer"
    if isinstance(value, float):
        return "number"
    if isinstance(value, str):
        return "string"
    if isinstance(value, list):
        return "array"
    if isinstance(value, dict):
        return "object"
    return "null"


SCHEMAS = [
    {"type": "integer", "minimum": 0, "maximum": 10},
    {"type": "integer", "exclusiveMinimum": 0, "multipleOf": 2},
    # exclusiveMinimum survives canonicalization (no multipleOf to promote it)
    {"type": "integer", "exclusiveMinimum": 0},
    {"type": "number", "exclusiveMaximum": 5},
    {"type": ["integer", "string"]},
    {"const": "x"},
    {"enum": ["a", "b", 3]},
    True,
    False,
]

VALUES = [0, 1, 2, 5, 10, 11, -1, 2.5, "a", "x", "b", 3, True, None, [], {}]


@pytest.mark.parametrize("schema", SCHEMAS)
def test_view_acceptance_agrees_with_is_valid(schema):
    canonical = jsonschema_rs.canonicalize(schema)
    view = canonical.view()
    canon_schema = canonical.to_json_schema()
    for value in VALUES:
        decided = accepts(view, value)
        expected = jsonschema_rs.is_valid(canon_schema, value)
        assert decided == expected, f"view acceptance {decided} != is_valid {expected} for {value} / {schema}"

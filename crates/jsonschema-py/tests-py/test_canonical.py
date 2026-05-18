import copy
import enum
import gc
import json
import pickle
import sys
from decimal import Decimal

import pytest
from hypothesis import given
from hypothesis import strategies as st

import jsonschema_rs
from jsonschema_rs import canonical

to_string = canonical.json.to_string
LARGE_INTEGER = 2**128
SERIALIZATION_ATTEMPTS = 25

# Recursive strategy for JSON-compatible values (no NaN/Inf — those don't roundtrip)
_json_scalars = st.one_of(
    st.none(),
    st.booleans(),
    st.integers(),
    st.floats(allow_nan=False, allow_infinity=False),
    st.text(),
)
_json_values = st.recursive(
    _json_scalars,
    lambda children: st.one_of(
        st.lists(children),
        st.dictionaries(st.text(), children),
    ),
)


class Color(enum.Enum):
    RED = "red"
    GREEN = 1


class StrColor(str, enum.Enum):
    BLUE = "blue"


def large_integer_overflow_error_count():
    gc.collect()
    return sum(
        1
        for obj in gc.get_objects()
        if type(obj) is OverflowError and "convert" in str(obj) and ("too big" in str(obj) or "too large" in str(obj))
    )


@pytest.mark.parametrize(
    "value, expected",
    [
        (None, "null"),
        (True, "true"),
        (False, "false"),
        (0, "0"),
        (42, "42"),
        (-7, "-7"),
        ("hello", '"hello"'),
        ("", '""'),
        (1.0, "1"),
        (0.0, "0"),
        (-5.0, "-5"),
        (1.5, "1.5"),
        (float("nan"), "null"),
        (float("inf"), "null"),
        (float("-inf"), "null"),
        (2**128, str(2**128)),
        (-(2**128), str(-(2**128))),
        ({"b": 1, "a": 2}, '{"a":2,"b":1}'),
        ({"a": 1, "b": 2}, '{"a":1,"b":2}'),
        ({}, "{}"),
        ({"x": 99}, '{"x":99}'),
        ({"z": {"b": 1, "a": 2}, "a": 0}, '{"a":0,"z":{"a":2,"b":1}}'),
        ([], "[]"),
        ([1, 2, 3], "[1,2,3]"),
        ([{"b": 1, "a": 2}], '[{"a":2,"b":1}]'),
        ((), "[]"),
        ((1, 2, 3), "[1,2,3]"),
        (Color.RED, '"red"'),
        (Color.GREEN, "1"),
        ({StrColor.BLUE: "sky"}, '{"blue":"sky"}'),
        (Decimal("1.0"), "1"),
        (Decimal("100"), "100"),
        (Decimal("NaN"), "null"),
        (Decimal("Infinity"), "null"),
        (Decimal("-Infinity"), "null"),
        (Decimal(2**128), str(2**128)),
    ],
)
def test_to_string(value, expected):
    assert to_string(value) == expected


def test_float_large_integer_valued():
    result = to_string(1e300)
    assert result == str(int(1e300))


def test_large_integer_serialization_does_not_leak_overflow_errors():
    baseline = large_integer_overflow_error_count()
    for _ in range(SERIALIZATION_ATTEMPTS):
        assert to_string(LARGE_INTEGER) == str(LARGE_INTEGER)
    assert large_integer_overflow_error_count() == baseline


@pytest.mark.parametrize("value", [float(2**63), float(-(2**63)), float(2**64)])
def test_float_integer_boundary_values(value):
    assert to_string(value) == str(int(value))


def test_decimal_fractional():
    assert json.loads(to_string(Decimal("1.5"))) == pytest.approx(1.5)


@pytest.mark.parametrize(
    "value, expected",
    [
        (Decimal("100E-2"), "1"),
        (Decimal("1E-2"), "0.01"),
        (Decimal("0E-1000"), "0"),
        (Decimal("-0E-1000"), "0"),
        (Decimal("-0"), "0"),
    ],
)
def test_decimal_exponent_integrality(value, expected):
    assert to_string(value) == expected


# One canonical text per numeric value, matching the core canonicalization normal form.
@pytest.mark.parametrize(
    "value, expected",
    [
        (Decimal("0.10"), "0.1"),
        (Decimal("1.50"), "1.5"),
        (Decimal("-0.50"), "-0.5"),
        (Decimal("3.1400E-3"), "0.00314"),
        (Decimal("1.5E-7"), "0.00000015"),
        (Decimal("1E+3"), "1000"),
    ],
)
def test_decimal_canonical_text(value, expected):
    assert to_string(value) == expected


@pytest.mark.parametrize("value", [object(), {1, 2, 3}])
def test_unsupported_type_raises(value):
    with pytest.raises(ValueError):
        to_string(value)


@pytest.mark.parametrize(
    "value",
    [
        None,
        True,
        False,
        0,
        42,
        -7,
        "hello",
        1.5,
        [1, 2, 3],
        {"b": 1, "a": 2},
        {"nested": {"z": 0, "a": 1}},
    ],
)
def test_roundtrip(value):
    assert json.loads(to_string(value)) == value


def test_same_dict_different_order_produces_same_output():
    assert to_string({"a": 1, "b": 2, "c": 3}) == to_string({"c": 3, "a": 1, "b": 2})


@pytest.mark.parametrize("float_val, int_val", [(1.0, 1), (0.0, 0)])
def test_integer_float_same_as_int(float_val, int_val):
    assert to_string(float_val) == to_string(int_val)


@pytest.mark.parametrize("value", ["\ud800", {"\ud800": 1}])
def test_lone_surrogate_raises(value):
    with pytest.raises(ValueError, match="surrogates not allowed"):
        to_string(value)


def test_str_enum_key_value_lookup_error():
    class BrokenStrEnum(str, enum.Enum):
        A = "a"

        def __getattribute__(self, name):
            if name == "value":
                raise RuntimeError("boom")
            return super().__getattribute__(name)

    with pytest.raises(ValueError, match="Failed to access enum key value"):
        to_string({BrokenStrEnum.A: 1})


def test_enum_value_lookup_error():
    class BrokenEnum(enum.Enum):
        A = "a"

        def __getattribute__(self, name):
            if name == "value":
                raise RuntimeError("boom")
            return super().__getattribute__(name)

    with pytest.raises(ValueError, match="boom"):
        to_string(BrokenEnum.A)


@given(_json_values)
def test_roundtrip_hypothesis(value):
    # json.loads(to_string(v)) == v holds for all JSON-compatible values.
    # Python's == handles int/float equivalence (1 == 1.0), so integer-valued
    # floats that become ints after the round-trip still compare equal.
    assert json.loads(to_string(value)) == value


@given(_json_values)
def test_idempotent(value):
    # Re-encoding the parsed output produces the same string.
    first = to_string(value)
    assert to_string(json.loads(first)) == first


@given(st.dictionaries(st.text(), _json_values))
def test_dict_key_order_invariance(d):
    reversed_d = dict(reversed(list(d.items())))
    assert to_string(d) == to_string(reversed_d)


@given(st.integers())
def test_integer_float_equivalence(n):
    f = float(n)
    if f == n:  # skip integers outside the exact float range
        assert to_string(f) == to_string(n)


@pytest.mark.skipif(not hasattr(sys, "getrefcount"), reason="PyPy does not have sys.getrefcount")
def test_enum_value_refcount_is_stable():
    class PayloadEnum(enum.Enum):
        ITEM = [1]

    payload = PayloadEnum.ITEM.value
    baseline = sys.getrefcount(payload)
    for _ in range(200):
        assert to_string(PayloadEnum.ITEM) == "[1]"
    assert sys.getrefcount(payload) == baseline


def test_canonicalize_meta_failure_raises_validation_error():
    with pytest.raises(jsonschema_rs.ValidationError) as exc:
        jsonschema_rs.canonicalize({"type": "int"})
    assert exc.value.schema_path == ["properties", "type", "anyOf"]


def test_canonicalize_invalid_pattern_raises_invalid_pattern():
    with pytest.raises(jsonschema_rs.canonical.InvalidPattern) as exc:
        jsonschema_rs.canonicalize({"pattern": "["})
    assert exc.value.location == "/pattern"
    assert isinstance(exc.value, jsonschema_rs.canonical.CanonicalizationError)
    assert isinstance(exc.value, ValueError)


def test_canonicalize_non_object_root_raises_invalid_schema_type():
    with pytest.raises(jsonschema_rs.canonical.InvalidSchemaType):
        jsonschema_rs.canonicalize(42)


def test_canonicalize_accepts_validate_formats():
    # date and email are both rigid in all drafts (uuid is only rigid from 2019-09+).
    # date forbids '@'; email requires '@' — so they are always disjoint when format assertions are on.
    schema = {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "allOf": [
            {"type": "string", "format": "date"},
            {"type": "string", "format": "email"},
        ],
    }

    assert jsonschema_rs.canonicalize(schema).is_satisfiable() is True
    assert jsonschema_rs.canonicalize(schema, validate_formats=True).is_satisfiable() is False
    assert jsonschema_rs.canonicalize(schema, draft=7).is_satisfiable() is False
    assert jsonschema_rs.canonicalize(schema, draft=7, validate_formats=False).is_satisfiable() is True


def test_canonicalize_resolves_relative_refs_against_base_uri():
    registry = jsonschema_rs.Registry([("https://example.com/schemas/other", {"type": "integer"})])

    canonical = jsonschema_rs.canonicalize(
        {"$ref": "other"},
        registry=registry,
        base_uri="https://example.com/schemas/root",
    )

    assert canonical.to_json_schema()["type"] == "integer"


@pytest.mark.parametrize("transform", [copy.deepcopy, lambda e: pickle.loads(pickle.dumps(e))])
def test_invalid_pattern_survives_pickle_and_deepcopy(transform):
    with pytest.raises(jsonschema_rs.canonical.InvalidPattern) as exc:
        jsonschema_rs.canonicalize({"pattern": "["})
    revived = transform(exc.value)
    assert revived.location == exc.value.location
    assert revived.message == exc.value.message


def test_canonicalize_uses_registry_retriever_when_only_registry_provided():
    registry = jsonschema_rs.Registry(
        [],
        retriever=lambda uri: {"type": "string"} if uri == "https://example.com/string.json" else None,
    )

    canonical = jsonschema_rs.canonicalize({"$ref": "https://example.com/string.json"}, registry=registry)

    assert canonical.to_json_schema()["type"] == "string"


def test_canonicalize_infinite_recursion_raises():
    schema = {
        "$defs": {
            "node": {
                "type": "object",
                "required": ["child"],
                "properties": {"child": {"$ref": "#/$defs/node"}},
            }
        },
        "$ref": "#/$defs/node",
    }
    with pytest.raises(jsonschema_rs.canonical.InfiniteRecursion) as exc:
        jsonschema_rs.canonicalize(schema)
    assert isinstance(exc.value, jsonschema_rs.canonical.CanonicalizationError)
    assert isinstance(exc.value, ValueError)

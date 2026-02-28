import enum
import json
import sys
from decimal import Decimal

import pytest
from hypothesis import given
from hypothesis import strategies as st

from jsonschema_rs import canonical_dumps

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


@pytest.mark.parametrize("value, expected", [
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
])
def test_canonical_dumps(value, expected):
    assert canonical_dumps(value) == expected


def test_float_large_integer_valued():
    result = canonical_dumps(1e300)
    assert result == str(int(1e300))


def test_decimal_fractional():
    assert json.loads(canonical_dumps(Decimal("1.5"))) == pytest.approx(1.5)


@pytest.mark.parametrize("value", [object(), {1, 2, 3}])
def test_unsupported_type_raises(value):
    with pytest.raises(ValueError):
        canonical_dumps(value)


@pytest.mark.parametrize("value", [
    None, True, False, 0, 42, -7, "hello", 1.5,
    [1, 2, 3], {"b": 1, "a": 2}, {"nested": {"z": 0, "a": 1}},
])
def test_roundtrip(value):
    assert json.loads(canonical_dumps(value)) == value


def test_same_dict_different_order_produces_same_output():
    assert canonical_dumps({"a": 1, "b": 2, "c": 3}) == canonical_dumps({"c": 3, "a": 1, "b": 2})


@pytest.mark.parametrize("float_val, int_val", [(1.0, 1), (0.0, 0)])
def test_integer_float_same_as_int(float_val, int_val):
    assert canonical_dumps(float_val) == canonical_dumps(int_val)


@pytest.mark.parametrize("value", ["\ud800", {"\ud800": 1}])
def test_lone_surrogate_raises(value):
    with pytest.raises(ValueError, match="surrogates not allowed"):
        canonical_dumps(value)


def test_str_enum_key_value_lookup_error():
    class BrokenStrEnum(str, enum.Enum):
        A = "a"

        def __getattribute__(self, name):
            if name == "value":
                raise RuntimeError("boom")
            return super().__getattribute__(name)

    with pytest.raises(ValueError, match="Failed to access enum key value"):
        canonical_dumps({BrokenStrEnum.A: 1})


def test_enum_value_lookup_error():
    class BrokenEnum(enum.Enum):
        A = "a"

        def __getattribute__(self, name):
            if name == "value":
                raise RuntimeError("boom")
            return super().__getattribute__(name)

    with pytest.raises(ValueError, match="boom"):
        canonical_dumps(BrokenEnum.A)


@given(_json_values)
def test_roundtrip_hypothesis(value):
    # json.loads(canonical_dumps(v)) == v holds for all JSON-compatible values.
    # Python's == handles int/float equivalence (1 == 1.0), so integer-valued
    # floats that become ints after the round-trip still compare equal.
    assert json.loads(canonical_dumps(value)) == value


@given(_json_values)
def test_idempotent(value):
    # Re-encoding the parsed output produces the same string.
    first = canonical_dumps(value)
    assert canonical_dumps(json.loads(first)) == first


@given(st.dictionaries(st.text(), _json_values))
def test_dict_key_order_invariance(d):
    reversed_d = dict(reversed(list(d.items())))
    assert canonical_dumps(d) == canonical_dumps(reversed_d)


@given(st.integers())
def test_integer_float_equivalence(n):
    f = float(n)
    if f == n:  # skip integers outside the exact float range
        assert canonical_dumps(f) == canonical_dumps(n)


@pytest.mark.skipif(not hasattr(sys, "getrefcount"), reason="PyPy does not have sys.getrefcount")
def test_enum_value_refcount_is_stable():
    class PayloadEnum(enum.Enum):
        ITEM = [1]

    payload = PayloadEnum.ITEM.value
    baseline = sys.getrefcount(payload)
    for _ in range(200):
        assert canonical_dumps(PayloadEnum.ITEM) == "[1]"
    assert sys.getrefcount(payload) == baseline

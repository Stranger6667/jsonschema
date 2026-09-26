import enum
import gc
import json
import sys
from decimal import Decimal

import pytest
from hypothesis import given
from hypothesis import strategies as st

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


class RenamedColor(str, enum.Enum):
    BLUE = "blue"

    @property
    def value(self):
        return "navy"


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
        ({StrColor.BLUE: "sky", "a": 1}, '{"a":1,"blue":"sky"}'),
        ({RenamedColor.BLUE: "sky"}, '{"navy":"sky"}'),
        ({RenamedColor.BLUE: "sky", "a": 1}, '{"a":1,"navy":"sky"}'),
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
    ],
)
def test_decimal_exponent_integrality(value, expected):
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


@pytest.mark.parametrize("value", [{1: "a"}, {1: "a", "b": 2}])
def test_non_string_key_raises(value):
    with pytest.raises(ValueError, match="Dict key must be str or str enum. Got 'int'"):
        to_string(value)


@pytest.mark.parametrize("value", ["\ud800", {"\ud800": 1}])
def test_lone_surrogate_raises(value):
    with pytest.raises(ValueError, match="surrogates not allowed"):
        to_string(value)


@pytest.mark.parametrize("siblings", [{}, {"b": 2}])
def test_str_enum_key_value_lookup_error(siblings):
    class BrokenStrEnum(str, enum.Enum):
        A = "a"

        def __getattribute__(self, name):
            if name == "value":
                raise RuntimeError("boom")
            return super().__getattribute__(name)

    with pytest.raises(ValueError, match="Failed to access enum key value"):
        to_string({BrokenStrEnum.A: 1, **siblings})


def test_enum_value_lookup_error():
    class BrokenEnum(enum.Enum):
        A = "a"

        def __getattribute__(self, name):
            if name == "value":
                raise RuntimeError("boom")
            return super().__getattribute__(name)

    with pytest.raises(ValueError, match="boom"):
        to_string(BrokenEnum.A)


# Objects allocated into the memory a callback frees, kept alive so a read through a dangling
# reference sees them (or crashes) instead of the stale contents.
CHURN = []


def empty_and_churn(container):
    container.clear()
    CHURN.append([({f"k{j}": object() for j in range(8)}, [object()] * 64) for _ in range(2000)])


def emptying_member(container):
    class Emptying(enum.Enum):
        A = "a"

        @property
        def value(self):
            empty_and_churn(container)
            return "a"

    return Emptying.A


def many_entries(first):
    return {"a": first, **{f"k{i}-" + "x" * 50: [f"v{i}" * 50] * 50 for i in range(200)}}


def nested_lists(first):
    return [[first] + [f"v{i}" * 50 for i in range(200)]] + [[f"w{i}"] * 10 for i in range(10)]


def test_enum_value_emptying_sorted_dict():
    document = {}
    document.update(many_entries(emptying_member(document)))
    # Entries are collected before any value runs, so all of them are written
    assert to_string(document) == json.dumps(many_entries("a"), sort_keys=True, separators=(",", ":"))


def test_enum_value_emptying_outer_list():
    outer = []
    outer.extend(nested_lists(emptying_member(outer)))
    # The inner list is written in full; the outer one stops where it was emptied
    assert to_string(outer) == json.dumps(nested_lists("a")[:1], separators=(",", ":"))


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

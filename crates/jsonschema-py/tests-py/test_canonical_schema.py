import sys

import pytest
from hypothesis import given
from hypothesis import strategies as st

from jsonschema_rs import canonical

clone = canonical.schema.clone


@pytest.mark.parametrize("value", [None, True, False, 0, 1, -7, 3.14, "hello", ""])
def test_immutables_return_same_object(value):
    assert clone(value) is value


def test_empty_dict_is_new_object():
    d = {}
    assert clone(d) is not d
    assert clone(d) == d


def test_empty_list_is_new_object():
    lst = []
    assert clone(lst) is not lst
    assert clone(lst) == lst


def test_dict_is_new_object():
    d = {"a": 1}
    result = clone(d)
    assert result is not d
    assert result == d


def test_list_is_new_object():
    lst = [1, 2, 3]
    result = clone(lst)
    assert result is not lst
    assert result == lst


def test_string_values_are_shared():
    s = "shared"
    d = {"key": s}
    result = clone(d)
    assert result["key"] is s


def test_string_items_in_list_are_shared():
    s = "shared"
    lst = [s, s]
    result = clone(lst)
    assert result[0] is s
    assert result[1] is s


def test_nested_dict_is_deep_cloned():
    inner = {"x": 1}
    outer = {"inner": inner}
    result = clone(outer)
    assert result is not outer
    assert result["inner"] is not inner
    assert result["inner"] == inner


def test_nested_list_is_deep_cloned():
    inner = [1, 2]
    outer = [inner]
    result = clone(outer)
    assert result is not outer
    assert result[0] is not inner
    assert result[0] == inner


def test_complex_schema():
    schema = {
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "age": {"type": "integer", "minimum": 0},
            "tags": {"type": "array", "items": {"type": "string"}},
        },
        "required": ["name"],
    }
    result = clone(schema)
    assert result == schema
    assert result is not schema
    assert result["properties"] is not schema["properties"]
    assert result["required"] is not schema["required"]
    # string leaves are shared
    assert result["type"] is schema["type"]
    assert result["required"][0] is schema["required"][0]


def test_mutating_clone_does_not_affect_original():
    original = {"a": {"b": 1}}
    cloned = clone(original)
    cloned["a"]["b"] = 999
    assert original["a"]["b"] == 1


def test_mutating_original_does_not_affect_clone():
    original = {"a": [1, 2, 3]}
    cloned = clone(original)
    original["a"].append(4)
    assert cloned["a"] == [1, 2, 3]


def test_tuple_is_returned_as_is():
    t = (1, 2, 3)
    assert clone(t) is t


def test_unknown_object_is_returned_as_is():
    class Custom:
        pass

    obj = Custom()
    assert clone(obj) is obj


def test_recursion_limit_raises():
    value = {}
    node = value
    for _ in range(255):
        inner = {}
        node["x"] = inner
        node = inner
    with pytest.raises(ValueError, match="Recursion limit reached"):
        clone(value)


def test_recursion_limit_does_not_raise_at_254():
    value = {}
    node = value
    for _ in range(254):
        inner = {}
        node["x"] = inner
        node = inner
    # One level short of the limit — must not raise
    clone(value)


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
    max_leaves=50,
)


@given(_json_values)
def test_clone_equals_original(value):
    assert clone(value) == value


@given(_json_values)
def test_clone_is_independent(value):
    # Cloning a clone should produce equal output
    assert clone(clone(value)) == value


@pytest.mark.skipif(not hasattr(sys, "getrefcount"), reason="CPython only")
def test_refcount_stable_for_string():
    s = "stable"
    d = {"key": s}
    baseline = sys.getrefcount(s)
    for _ in range(100):
        clone(d)
    assert sys.getrefcount(s) == baseline

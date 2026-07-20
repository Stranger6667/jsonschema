import copy
import enum
from decimal import Decimal

import pytest

from jsonschema_rs import canonical

to_string = canonical.json.to_string

clone = canonical.schema.clone


def deepclone(value):
    """Specialized deepcopy that copies only dict and list (unrolled 3 levels)."""
    if isinstance(value, dict):
        return {
            k1: (
                {
                    k2: (
                        {k3: deepclone(v3) for k3, v3 in v2.items()}
                        if isinstance(v2, dict)
                        else [deepclone(v3) for v3 in v2]
                        if isinstance(v2, list)
                        else v2
                    )
                    for k2, v2 in v1.items()
                }
                if isinstance(v1, dict)
                else [deepclone(v2) for v2 in v1]
                if isinstance(v1, list)
                else v1
            )
            for k1, v1 in value.items()
        }
    if isinstance(value, list):
        return [
            {k2: deepclone(v2) for k2, v2 in v1.items()}
            if isinstance(v1, dict)
            else [deepclone(v2) for v2 in v1]
            if isinstance(v1, list)
            else v1
            for v1 in value
        ]
    return value


# Reversed insertion order keeps the input intentionally non-canonical
# and exercises key collection + sorting in canonical.json.to_string.
LARGE_DICT_1024 = {f"key-{i:04d}": i for i in range(1023, -1, -1)}


EnumKey = enum.Enum(
    "EnumKey",
    {f"K{i:04d}": f"key-{i:04d}" for i in range(512)},
    type=str,
)
ENUM_KEY_DICT_512 = {member: idx for idx, member in enumerate(list(EnumKey)[::-1])}


INT_FLOAT_LIST_4096 = [float(i) for i in range(4096)]
FRACTIONAL_FLOAT_LIST_2048 = [i + (i % 997) / 1000.0 for i in range(1, 2049)]
SMALL_FLOAT_LIST_2048 = [i * 1e-7 for i in range(1, 2049)]
DECIMAL_FRACTIONAL_LIST_2048 = [Decimal(f"{i}.{(i * 37) % 1000:03d}") for i in range(1, 2049)]
DECIMAL_SPECIAL_LIST_2048 = [Decimal("NaN"), Decimal("Infinity"), Decimal("-Infinity")] * 682 + [
    Decimal("NaN"),
    Decimal("Infinity"),
]


MIXED_NESTED = {
    "meta": {"version": 1, "source": "benchmark", "active": True},
    "items": [
        {
            "id": i,
            "name": f"item-{i}",
            "scores": [float(i), float(i + 1), float(i + 2)],
            "props": {"z": i, "a": i + 1, "m": i + 2},
        }
        for i in range(300)
    ],
}


SMALL_SCHEMA = {
    "type": "object",
    "properties": {
        "name": {"type": "string"},
        "age": {"type": "integer", "minimum": 0},
        "active": {"type": "boolean"},
    },
    "required": ["name"],
    "additionalProperties": False,
}

MEDIUM_SCHEMA = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "object",
    "properties": {
        "id": {"type": "integer"},
        "user": {
            "type": "object",
            "properties": {
                "name": {"type": "string", "minLength": 1, "maxLength": 100},
                "email": {"type": "string", "format": "email"},
                "roles": {
                    "type": "array",
                    "items": {"type": "string", "enum": ["admin", "user", "guest"]},
                },
                "address": {
                    "type": "object",
                    "properties": {
                        "street": {"type": "string"},
                        "city": {"type": "string"},
                        "country": {"type": "string", "minLength": 2, "maxLength": 2},
                    },
                },
            },
            "required": ["name", "email"],
        },
        "tags": {"type": "array", "items": {"type": "string"}, "uniqueItems": True},
        "metadata": {"type": "object", "additionalProperties": {"type": "string"}},
    },
    "required": ["id", "user"],
}

_prop = lambda t: {"type": t}
LARGE_SCHEMA = {
    "type": "object",
    "properties": {
        f"field_{i}": {
            "type": "object",
            "properties": {
                "value": _prop("string"),
                "count": _prop("integer"),
                "enabled": _prop("boolean"),
                "tags": {"type": "array", "items": _prop("string")},
                "nested": {
                    "type": "object",
                    "properties": {
                        "a": _prop("string"),
                        "b": _prop("integer"),
                        "c": {"type": "array", "items": _prop("number")},
                    },
                },
            },
        }
        for i in range(50)
    },
}


@pytest.mark.benchmark(group="canonical-json-dict-sort")
def test_canonical_json_large_dict_1024(benchmark):
    benchmark(to_string, LARGE_DICT_1024)


@pytest.mark.benchmark(group="canonical-json-enum-keys")
def test_canonical_json_enum_key_dict_512(benchmark):
    benchmark(to_string, ENUM_KEY_DICT_512)


@pytest.mark.benchmark(group="canonical-json-int-floats")
def test_canonical_json_int_float_list_4096(benchmark):
    benchmark(to_string, INT_FLOAT_LIST_4096)


@pytest.mark.benchmark(group="canonical-json-fractional-floats")
def test_canonical_json_fractional_float_list_2048(benchmark):
    benchmark(to_string, FRACTIONAL_FLOAT_LIST_2048)


@pytest.mark.benchmark(group="canonical-json-small-floats")
def test_canonical_json_small_float_list_2048(benchmark):
    benchmark(to_string, SMALL_FLOAT_LIST_2048)


@pytest.mark.benchmark(group="canonical-json-decimal-fractional")
def test_canonical_json_decimal_fractional_list_2048(benchmark):
    benchmark(to_string, DECIMAL_FRACTIONAL_LIST_2048)


@pytest.mark.benchmark(group="canonical-json-decimal-special")
def test_canonical_json_decimal_special_list_2048(benchmark):
    benchmark(to_string, DECIMAL_SPECIAL_LIST_2048)


@pytest.mark.benchmark(group="canonical-json-mixed-nested")
def test_canonical_json_mixed_nested(benchmark):
    benchmark(to_string, MIXED_NESTED)


_CLONE_IMPLS = [
    pytest.param(copy.deepcopy, id="deepcopy"),
    pytest.param(deepclone, id="deepclone"),
    pytest.param(clone, id="canonical"),
]


@pytest.mark.parametrize("fn", _CLONE_IMPLS)
@pytest.mark.benchmark(group="clone-small")
def test_clone_small(benchmark, fn):
    benchmark(fn, SMALL_SCHEMA)


@pytest.mark.parametrize("fn", _CLONE_IMPLS)
@pytest.mark.benchmark(group="clone-medium")
def test_clone_medium(benchmark, fn):
    benchmark(fn, MEDIUM_SCHEMA)


@pytest.mark.parametrize("fn", _CLONE_IMPLS)
@pytest.mark.benchmark(group="clone-large")
def test_clone_large(benchmark, fn):
    benchmark(fn, LARGE_SCHEMA)

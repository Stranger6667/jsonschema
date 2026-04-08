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


BENCHMARK_CONFIG = {
    "min_rounds": 20,
    "warmup": True,
}


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


@pytest.mark.benchmark(group="canonical-json-dict-sort", **BENCHMARK_CONFIG)
def test_canonical_json_large_dict_1024(benchmark):
    benchmark(to_string, LARGE_DICT_1024)


@pytest.mark.benchmark(group="canonical-json-enum-keys", **BENCHMARK_CONFIG)
def test_canonical_json_enum_key_dict_512(benchmark):
    benchmark(to_string, ENUM_KEY_DICT_512)


@pytest.mark.benchmark(group="canonical-json-int-floats", **BENCHMARK_CONFIG)
def test_canonical_json_int_float_list_4096(benchmark):
    benchmark(to_string, INT_FLOAT_LIST_4096)


@pytest.mark.benchmark(group="canonical-json-decimal-fractional", **BENCHMARK_CONFIG)
def test_canonical_json_decimal_fractional_list_2048(benchmark):
    benchmark(to_string, DECIMAL_FRACTIONAL_LIST_2048)


@pytest.mark.benchmark(group="canonical-json-decimal-special", **BENCHMARK_CONFIG)
def test_canonical_json_decimal_special_list_2048(benchmark):
    benchmark(to_string, DECIMAL_SPECIAL_LIST_2048)


@pytest.mark.benchmark(group="canonical-json-mixed-nested", **BENCHMARK_CONFIG)
def test_canonical_json_mixed_nested(benchmark):
    benchmark(to_string, MIXED_NESTED)


_CLONE_IMPLS = [
    pytest.param(copy.deepcopy, id="deepcopy"),
    pytest.param(deepclone, id="deepclone"),
    pytest.param(clone, id="canonical"),
]


@pytest.mark.parametrize("fn", _CLONE_IMPLS)
@pytest.mark.benchmark(group="clone-small", **BENCHMARK_CONFIG)
def test_clone_small(benchmark, fn):
    benchmark(fn, SMALL_SCHEMA)


@pytest.mark.parametrize("fn", _CLONE_IMPLS)
@pytest.mark.benchmark(group="clone-medium", **BENCHMARK_CONFIG)
def test_clone_medium(benchmark, fn):
    benchmark(fn, MEDIUM_SCHEMA)


@pytest.mark.parametrize("fn", _CLONE_IMPLS)
@pytest.mark.benchmark(group="clone-large", **BENCHMARK_CONFIG)
def test_clone_large(benchmark, fn):
    benchmark(fn, LARGE_SCHEMA)

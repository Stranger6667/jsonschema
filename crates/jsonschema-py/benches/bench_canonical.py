import enum
from decimal import Decimal

import pytest

from jsonschema_rs import canonical

to_string = canonical.json.to_string


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
DECIMAL_SPECIAL_LIST_2048 = [Decimal("NaN"), Decimal("Infinity"), Decimal("-Infinity")] * 682 + [Decimal("NaN"), Decimal("Infinity")]


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

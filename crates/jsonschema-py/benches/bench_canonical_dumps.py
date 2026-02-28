import enum
from decimal import Decimal

import pytest

from jsonschema_rs import canonical_dumps


BENCHMARK_CONFIG = {
    "min_rounds": 20,
    "warmup": True,
}


# Reversed insertion order keeps the input intentionally non-canonical
# and exercises key collection + sorting in canonical_dumps.
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


@pytest.mark.benchmark(group="canonical-dumps-dict-sort", **BENCHMARK_CONFIG)
def test_canonical_dumps_large_dict_1024(benchmark):
    benchmark(canonical_dumps, LARGE_DICT_1024)


@pytest.mark.benchmark(group="canonical-dumps-enum-keys", **BENCHMARK_CONFIG)
def test_canonical_dumps_enum_key_dict_512(benchmark):
    benchmark(canonical_dumps, ENUM_KEY_DICT_512)


@pytest.mark.benchmark(group="canonical-dumps-int-floats", **BENCHMARK_CONFIG)
def test_canonical_dumps_int_float_list_4096(benchmark):
    benchmark(canonical_dumps, INT_FLOAT_LIST_4096)


@pytest.mark.benchmark(group="canonical-dumps-decimal-fractional", **BENCHMARK_CONFIG)
def test_canonical_dumps_decimal_fractional_list_2048(benchmark):
    benchmark(canonical_dumps, DECIMAL_FRACTIONAL_LIST_2048)


@pytest.mark.benchmark(group="canonical-dumps-decimal-special", **BENCHMARK_CONFIG)
def test_canonical_dumps_decimal_special_list_2048(benchmark):
    benchmark(canonical_dumps, DECIMAL_SPECIAL_LIST_2048)


@pytest.mark.benchmark(group="canonical-dumps-mixed-nested", **BENCHMARK_CONFIG)
def test_canonical_dumps_mixed_nested(benchmark):
    benchmark(canonical_dumps, MIXED_NESTED)

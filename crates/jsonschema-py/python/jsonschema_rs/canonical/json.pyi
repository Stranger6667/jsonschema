__all__ = ["to_string"]

def to_string(object: object) -> str:
    """Serialize a Python object to a canonical JSON string.

    JSON values that differ only in key order or number form give equal strings, so the output
    works as a dedup key.

    - Dict keys are sorted by their UTF-8 bytes.
    - Integer-valued floats are serialized as integers (1.0 becomes 1).
    - NaN and Infinity are serialized as null.
    - Output is always compact.

    Raises ValueError on serialization failure (e.g., recursion limit exceeded,
    unsupported type, lone surrogates in strings).
    """
    ...

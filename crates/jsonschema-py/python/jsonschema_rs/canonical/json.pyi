from .. import JSONType

def to_string(object: JSONType) -> str:
    """Serialize a Python object to a canonical JSON string.

    Main use case: deduplicating equivalent JSON Schemas.

    - Dict keys are sorted lexicographically (byte order).
    - Integer-valued floats are serialized as integers (1.0 → 1).
    - NaN and Infinity are serialized as null.
    - Output is always compact.

    Raises ValueError on serialization failure (e.g., recursion limit exceeded,
    unsupported type, lone surrogates in strings).
    """
    ...

from .. import JSONType

def clone(object: JSONType) -> JSONType:
    """Deep-clone a JSON-compatible Python object.

    Only ``dict`` and ``list`` are copied. All other values (str, int, float,
    bool, None) are shared by reference since they are immutable.

    Dict subclasses (e.g. ``CaseInsensitiveDict``) are cloned into plain
    ``dict``. All other non-JSON types are returned as-is (treated as
    immutable).

    Faster than ``copy.deepcopy`` for JSON Schema documents.

    :raises ValueError: if the object exceeds 255 levels of nesting.
    """
    ...

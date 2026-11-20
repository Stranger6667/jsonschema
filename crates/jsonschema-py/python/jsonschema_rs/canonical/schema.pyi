from typing import TypeVar

__all__ = ["clone"]

_T = TypeVar("_T")

def clone(object: _T) -> _T:
    """Deep-clone a JSON-compatible Python object.

    ``dict``, its subclasses and ``list`` are copied; dict subclasses (e.g.
    ``CaseInsensitiveDict``) become plain ``dict``. All other values are shared by
    reference.

    Faster than ``copy.deepcopy`` for JSON Schema documents.

    :raises ValueError: if the object exceeds 255 levels of nesting.
    """
    ...

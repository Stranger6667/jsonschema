from typing import TypeAlias, final

from . import json as json
from . import schema as schema
from .. import JsonValue

@final
class RawView:
    """A schema the canonical form does not model structurally, kept verbatim."""

    __match_args__: tuple[str, ...]
    @property
    def schema(self) -> JsonValue: ...

CanonicalViewType: TypeAlias = RawView

class CanonicalizationError(ValueError):
    """A schema could not be reduced to canonical form."""

    message: str
    def __init__(self, message: str) -> None: ...

class InvalidSchemaType(CanonicalizationError):
    """The schema root is neither a boolean nor an object."""

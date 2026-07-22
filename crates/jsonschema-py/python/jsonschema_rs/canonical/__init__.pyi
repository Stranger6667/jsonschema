from typing import TypeAlias, final

from . import json as json
from . import schema as schema
from .. import CanonicalSchema, JsonValue

@final
class TrueView:
    """Matches any value."""

@final
class FalseView:
    """Matches no value."""

@final
class MultiTypeView:
    """A value matches iff its JSON type is in ``types``."""

    __match_args__: tuple[str, ...]
    @property
    def types(self) -> list[str]: ...

@final
class TypedGroupView:
    """A value matches iff its JSON type is ``type_name`` and it satisfies ``body``."""

    __match_args__: tuple[str, ...]
    @property
    def type_name(self) -> str: ...
    @property
    def body(self) -> CanonicalSchema: ...

@final
class StringView:
    """A string value within a length window matching every pattern and format."""

    __match_args__: tuple[str, ...]
    @property
    def min_length(self) -> int | None: ...
    @property
    def max_length(self) -> int | None: ...
    @property
    def patterns(self) -> list[str]: ...
    @property
    def formats(self) -> list[str]: ...

@final
class NumberView:
    """A number value within a real interval."""

    __match_args__: tuple[str, ...]
    @property
    def minimum(self) -> int | float | None: ...
    @property
    def exclusive_minimum(self) -> bool: ...
    @property
    def maximum(self) -> int | float | None: ...
    @property
    def exclusive_maximum(self) -> bool: ...

@final
class IntegerView:
    """An integer value within a range, optionally a multiple of a divisor."""

    __match_args__: tuple[str, ...]
    @property
    def minimum(self) -> int | None: ...
    @property
    def maximum(self) -> int | None: ...
    @property
    def multiple_of(self) -> int | None: ...

@final
class AnyOfView:
    """A value matches iff at least one branch matches."""

    __match_args__: tuple[str, ...]
    @property
    def branches(self) -> list[CanonicalSchema]: ...

@final
class ConstView:
    """Exactly one admitted value."""

    __match_args__: tuple[str, ...]
    @property
    def value(self) -> JsonValue: ...

@final
class EnumView:
    """A sorted, deduplicated finite set of admitted values."""

    __match_args__: tuple[str, ...]
    @property
    def values(self) -> list[JsonValue]: ...

@final
class RawView:
    """A schema the canonical form does not model structurally, kept verbatim."""

    __match_args__: tuple[str, ...]
    @property
    def schema(self) -> JsonValue: ...

CanonicalViewType: TypeAlias = (
    TrueView
    | FalseView
    | MultiTypeView
    | TypedGroupView
    | StringView
    | NumberView
    | IntegerView
    | AnyOfView
    | ConstView
    | EnumView
    | RawView
)

class CanonicalizationError(ValueError):
    """A schema could not be reduced to canonical form."""

    message: str
    def __init__(self, message: str) -> None: ...

class InvalidSchemaType(CanonicalizationError):
    """The schema root is neither a boolean nor an object."""

class InvalidPattern(CanonicalizationError):
    """A ``pattern`` value is not a valid regular expression."""

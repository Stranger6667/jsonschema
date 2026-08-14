from decimal import Decimal
from typing import TypeAlias, final

from . import json as json
from . import schema as schema
from .. import CanonicalSchema, JsonValue

@final
class Containment:
    """Whether one schema admits every value another does."""

    YES: Containment
    NO: Containment
    UNKNOWN: Containment
    @property
    def value(self) -> str: ...

@final
class Satisfiability:
    """Whether any value satisfies a schema."""

    YES: Satisfiability
    NO: Satisfiability
    UNKNOWN: Satisfiability
    @property
    def value(self) -> str: ...

@final
class Distinctness:
    """What an array requires of its elements: all distinct, some repeated, or neither."""

    UNCONSTRAINED: Distinctness
    ALL_DISTINCT: Distinctness
    SOME_REPEATED: Distinctness
    @property
    def value(self) -> str: ...

@final
class CanonicalKind:
    """Structural discriminant of a canonical node, one member per view class."""

    MULTI_TYPE: CanonicalKind
    TYPED_GROUP: CanonicalKind
    STRING: CanonicalKind
    INTEGER: CanonicalKind
    NUMBER: CanonicalKind
    ARRAY: CanonicalKind
    OBJECT: CanonicalKind
    CONST: CanonicalKind
    ENUM: CanonicalKind
    NOT: CanonicalKind
    ALL_OF: CanonicalKind
    ANY_OF: CanonicalKind
    ONE_OF: CanonicalKind
    REFERENCE: CanonicalKind
    TRUE: CanonicalKind
    FALSE: CanonicalKind
    RAW: CanonicalKind
    @property
    def value(self) -> str: ...

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
    """A string value within a length window, matching every required facet and no barred one."""

    __match_args__: tuple[str, ...]
    @property
    def min_length(self) -> int | None: ...
    @property
    def max_length(self) -> int | None: ...
    @property
    def patterns(self) -> list[str]: ...
    @property
    def excluded_patterns(self) -> list[str]: ...
    @property
    def formats(self) -> list[str]: ...
    @property
    def excluded_formats(self) -> list[str]: ...
    @property
    def content_media_types(self) -> list[str]: ...
    @property
    def content_encodings(self) -> list[str]: ...
    @property
    def excluded(self) -> list[str]: ...

@final
class NumberView:
    """A number value within a real interval."""

    __match_args__: tuple[str, ...]
    @property
    def minimum(self) -> int | float | Decimal | None: ...
    @property
    def exclusive_minimum(self) -> bool: ...
    @property
    def maximum(self) -> int | float | Decimal | None: ...
    @property
    def exclusive_maximum(self) -> bool: ...
    @property
    def multiple_of(self) -> list[int | float | Decimal]: ...
    @property
    def not_multiple_of(self) -> list[int | float | Decimal]: ...
    @property
    def excludes_integers(self) -> bool: ...

@final
class IntegerView:
    """An integer value within a range, optionally a multiple of a divisor."""

    __match_args__: tuple[str, ...]
    @property
    def minimum(self) -> int | None: ...
    @property
    def maximum(self) -> int | None: ...
    @property
    def multiple_of(self) -> list[int | float | Decimal]: ...
    @property
    def not_multiple_of(self) -> list[int | float | Decimal]: ...

@final
class ArrayView:
    """An array value's constraints."""

    __match_args__: tuple[str, ...]
    @property
    def min_items(self) -> int | None: ...
    @property
    def max_items(self) -> int | None: ...
    @property
    def distinctness(self) -> Distinctness: ...
    @property
    def prefix_items(self) -> list[CanonicalSchema]: ...
    @property
    def items(self) -> CanonicalSchema | None: ...
    @property
    def contains(self) -> list[ContainsView]: ...

@final
class ContainsView:
    """One `contains` requirement of an array. An absent minimum means the default of one."""

    __match_args__: tuple[str, ...]
    @property
    def schema(self) -> CanonicalSchema: ...
    @property
    def min_contains(self) -> int | None: ...
    @property
    def max_contains(self) -> int | None: ...

@final
class ObjectView:
    """An object value's constraints."""

    __match_args__: tuple[str, ...]
    @property
    def min_properties(self) -> int | None: ...
    @property
    def max_properties(self) -> int | None: ...
    @property
    def required(self) -> list[str]: ...
    @property
    def property_names(self) -> CanonicalSchema | None: ...
    @property
    def properties(self) -> dict[str, CanonicalSchema]: ...
    @property
    def pattern_properties(self) -> dict[str, CanonicalSchema]: ...
    @property
    def additional_properties(self) -> CanonicalSchema | None: ...
    @property
    def violations(self) -> list[NameFailsView | UndeclaredValueFailsView]: ...

@final
class NameFailsView:
    """Some key's name fails the schema."""

    __match_args__: tuple[str, ...]
    @property
    def schema(self) -> CanonicalSchema: ...

@final
class UndeclaredValueFailsView:
    """Some key outside `names` and matching none of `patterns` has a value failing `additional`."""

    __match_args__: tuple[str, ...]
    @property
    def names(self) -> list[str]: ...
    @property
    def patterns(self) -> list[str]: ...
    @property
    def additional(self) -> CanonicalSchema: ...

@final
class NotView:
    """The exact complement of ``schema``."""

    __match_args__: tuple[str, ...]
    @property
    def schema(self) -> CanonicalSchema: ...

@final
class AllOfView:
    """A value matches iff every branch matches."""

    __match_args__: tuple[str, ...]
    @property
    def branches(self) -> list[CanonicalSchema]: ...

@final
class AnyOfView:
    """A value matches iff at least one branch matches."""

    __match_args__: tuple[str, ...]
    @property
    def branches(self) -> list[CanonicalSchema]: ...

@final
class OneOfView:
    """A value matches iff exactly one branch matches."""

    __match_args__: tuple[str, ...]
    @property
    def branches(self) -> list[CanonicalSchema]: ...

@final
class ReferenceView:
    """A symbolic JSON Schema reference."""

    __match_args__: tuple[str, ...]
    @property
    def uri(self) -> str: ...

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
    """A schema the canonical form does not support structurally, kept verbatim."""

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
    | ArrayView
    | ObjectView
    | NotView
    | AllOfView
    | AnyOfView
    | OneOfView
    | ReferenceView
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

class IncompatibleOperands(CanonicalizationError):
    """Operands of a set operation cannot be combined."""

class UnsupportedOperand(CanonicalizationError):
    """A set operation reached an operand the canonical form does not support."""

class UnsupportedResult(CanonicalizationError):
    """A set operation ran, and the canonical form does not support its result."""

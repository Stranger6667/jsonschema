from decimal import Decimal
from typing import Any

from . import json as json
from . import schema as schema
from .. import CanonicalSchema, JsonValue

_Number = int | float | Decimal

class NullView: ...
class TrueView: ...
class FalseView: ...

class BooleanView:
    __match_args__ = ("variant",)
    @property
    def variant(self) -> str:
        """One of ``"any"``, ``"just_true"``, ``"just_false"``."""

class IntegerView:
    """``minimum``/``exclusive_minimum`` are mutually exclusive (likewise the maxima): the set one carries the bound, the other is ``None``."""

    __match_args__ = (
        "minimum",
        "maximum",
        "exclusive_minimum",
        "exclusive_maximum",
        "multiple_of",
        "not_multiple_of",
    )
    @property
    def minimum(self) -> _Number | None: ...
    @property
    def maximum(self) -> _Number | None: ...
    @property
    def exclusive_minimum(self) -> _Number | None: ...
    @property
    def exclusive_maximum(self) -> _Number | None: ...
    @property
    def multiple_of(self) -> _Number | None: ...
    @property
    def not_multiple_of(self) -> list[_Number]: ...

class NumberView:
    """``minimum``/``exclusive_minimum`` are mutually exclusive (likewise the maxima): the set one carries the bound, the other is ``None``."""

    __match_args__ = (
        "minimum",
        "maximum",
        "exclusive_minimum",
        "exclusive_maximum",
        "multiple_of",
        "not_multiple_of",
    )
    @property
    def minimum(self) -> _Number | None: ...
    @property
    def maximum(self) -> _Number | None: ...
    @property
    def exclusive_minimum(self) -> _Number | None: ...
    @property
    def exclusive_maximum(self) -> _Number | None: ...
    @property
    def multiple_of(self) -> _Number | None: ...
    @property
    def not_multiple_of(self) -> list[_Number]: ...

class ContentFacetView:
    __match_args__ = ("content_encoding", "content_media_type", "content_schema")
    @property
    def content_encoding(self) -> str | None: ...
    @property
    def content_media_type(self) -> str | None: ...
    @property
    def content_schema(self) -> JsonValue | None: ...

class StringView:
    __match_args__ = (
        "min_length",
        "max_length",
        "patterns",
        "not_patterns",
        "format",
        "content",
        "extended_regex",
    )
    @property
    def min_length(self) -> int | None: ...
    @property
    def max_length(self) -> int | None: ...
    @property
    def patterns(self) -> list[str]: ...
    @property
    def not_patterns(self) -> list[str]: ...
    @property
    def format(self) -> str | None: ...
    @property
    def content(self) -> list[ContentFacetView]: ...
    @property
    def extended_regex(self) -> bool: ...

class ContainsView:
    __match_args__ = ("schema", "min_contains", "max_contains")
    @property
    def schema(self) -> CanonicalSchema: ...
    @property
    def min_contains(self) -> int: ...
    @property
    def max_contains(self) -> int | None: ...

class ArrayView:
    __match_args__ = (
        "prefix",
        "tail",
        "min_items",
        "max_items",
        "unique_items",
        "repeated_items",
        "contains",
    )
    @property
    def prefix(self) -> list[CanonicalSchema]: ...
    @property
    def tail(self) -> CanonicalSchema | None: ...
    @property
    def min_items(self) -> int: ...
    @property
    def max_items(self) -> int | None: ...
    @property
    def unique_items(self) -> bool: ...
    @property
    def repeated_items(self) -> bool: ...
    @property
    def contains(self) -> list[ContainsView]: ...

class RequiredProperty:
    __match_args__ = ("name",)
    @property
    def name(self) -> str: ...

class PatternPropertyRequirement:
    __match_args__ = ("pattern", "schema")
    @property
    def pattern(self) -> str: ...
    @property
    def schema(self) -> CanonicalSchema: ...

class AdditionalPropertiesRequirement:
    __match_args__ = ("schema",)
    @property
    def schema(self) -> CanonicalSchema: ...

class DependentPropertiesRequirement:
    __match_args__ = ("property", "required_properties")
    # `property` is defined last so it does not shadow the builtin for later `@property` decorators.
    @property
    def required_properties(self) -> list[str]: ...
    @property
    def property(self) -> str: ...

class DependentSchemaRequirement:
    __match_args__ = ("property", "schema")
    # `property` is defined last so it does not shadow the builtin for later `@property` decorators.
    @property
    def schema(self) -> CanonicalSchema: ...
    @property
    def property(self) -> str: ...

class NamedPropertyConstraint:
    __match_args__ = ("name", "schema")
    @property
    def name(self) -> str: ...
    @property
    def schema(self) -> CanonicalSchema: ...

class PatternPropertyConstraint:
    __match_args__ = ("pattern", "schema")
    @property
    def pattern(self) -> str: ...
    @property
    def schema(self) -> CanonicalSchema: ...

class AdditionalPropertiesConstraint:
    __match_args__ = ("schema",)
    @property
    def schema(self) -> CanonicalSchema: ...

_ObjectRequirementView = (
    RequiredProperty
    | PatternPropertyRequirement
    | AdditionalPropertiesRequirement
    | DependentPropertiesRequirement
    | DependentSchemaRequirement
)
_ObjectConstraintView = NamedPropertyConstraint | PatternPropertyConstraint | AdditionalPropertiesConstraint

class ObjectView:
    __match_args__ = (
        "requirements",
        "constraints",
        "property_names",
        "min_properties",
        "max_properties",
    )
    @property
    def requirements(self) -> list[_ObjectRequirementView]: ...
    @property
    def constraints(self) -> list[_ObjectConstraintView]: ...
    @property
    def property_names(self) -> CanonicalSchema | None: ...
    @property
    def min_properties(self) -> int | None: ...
    @property
    def max_properties(self) -> int | None: ...

class MultiTypeView:
    __match_args__ = ("types",)
    @property
    def types(self) -> list[str]: ...

class AllOfView:
    __match_args__ = ("schemas",)
    @property
    def schemas(self) -> list[CanonicalSchema]: ...

class AnyOfView:
    __match_args__ = ("schemas",)
    @property
    def schemas(self) -> list[CanonicalSchema]: ...

class OneOfView:
    __match_args__ = ("schemas",)
    @property
    def schemas(self) -> list[CanonicalSchema]: ...

class NotView:
    __match_args__ = ("schema",)
    @property
    def schema(self) -> CanonicalSchema: ...

class TypedGroupView:
    """A value matches iff its JSON type is ``type_name`` *and* it satisfies ``body``; other types do not match."""

    __match_args__ = ("type_name", "body")
    @property
    def type_name(self) -> str: ...
    @property
    def body(self) -> CanonicalSchema: ...

class TypeGuardView:
    """Constrains only values of JSON type ``type_name`` (they must satisfy ``body``); any other type matches unconditionally."""

    __match_args__ = ("type_name", "body")
    @property
    def type_name(self) -> str: ...
    @property
    def body(self) -> CanonicalSchema: ...

class ConstView:
    __match_args__ = ("value",)
    @property
    def value(self) -> JsonValue: ...

class EnumView:
    __match_args__ = ("values",)
    @property
    def values(self) -> list[JsonValue]: ...

class ReferenceView:
    __match_args__ = ("uri",)
    @property
    def uri(self) -> str: ...

class RecursiveView:
    __match_args__ = ("uri",)
    @property
    def uri(self) -> str: ...

class DynamicRefView:
    __match_args__ = ("name",)
    @property
    def name(self) -> str: ...

class RawView:
    __match_args__ = ("schema",)
    @property
    def schema(self) -> JsonValue: ...

CanonicalViewType = (
    NullView
    | TrueView
    | FalseView
    | BooleanView
    | IntegerView
    | NumberView
    | StringView
    | ArrayView
    | ObjectView
    | MultiTypeView
    | AllOfView
    | AnyOfView
    | OneOfView
    | NotView
    | TypedGroupView
    | TypeGuardView
    | ConstView
    | EnumView
    | ReferenceView
    | RecursiveView
    | DynamicRefView
    | RawView
)

class CanonicalizationError(ValueError):
    message: str

class InvalidSchemaType(CanonicalizationError): ...
class InvalidJsonValue(CanonicalizationError): ...

class InvalidPattern(CanonicalizationError):
    location: str

class UnguardedRecursion(CanonicalizationError): ...
class InfiniteRecursion(CanonicalizationError): ...

from __future__ import annotations

from ..jsonschema_rs import canonical as _canonical

NullView = _canonical.NullView
TrueView = _canonical.TrueView
FalseView = _canonical.FalseView
BooleanView = _canonical.BooleanView
IntegerView = _canonical.IntegerView
NumberView = _canonical.NumberView
StringView = _canonical.StringView
ContentFacetView = _canonical.ContentFacetView
ContainsView = _canonical.ContainsView
ArrayView = _canonical.ArrayView
RequiredProperty = _canonical.RequiredProperty
PatternPropertyRequirement = _canonical.PatternPropertyRequirement
AdditionalPropertiesRequirement = _canonical.AdditionalPropertiesRequirement
DependentPropertiesRequirement = _canonical.DependentPropertiesRequirement
DependentSchemaRequirement = _canonical.DependentSchemaRequirement
NamedPropertyConstraint = _canonical.NamedPropertyConstraint
PatternPropertyConstraint = _canonical.PatternPropertyConstraint
AdditionalPropertiesConstraint = _canonical.AdditionalPropertiesConstraint
ObjectView = _canonical.ObjectView
MultiTypeView = _canonical.MultiTypeView
AllOfView = _canonical.AllOfView
AnyOfView = _canonical.AnyOfView
OneOfView = _canonical.OneOfView
NotView = _canonical.NotView
TypedGroupView = _canonical.TypedGroupView
TypeGuardView = _canonical.TypeGuardView
ConstView = _canonical.ConstView
EnumView = _canonical.EnumView
ReferenceView = _canonical.ReferenceView
RecursiveView = _canonical.RecursiveView
DynamicRefView = _canonical.DynamicRefView
RawView = _canonical.RawView


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

json = _canonical.json
schema = _canonical.schema


class CanonicalizationError(ValueError):
    """A schema could not be reduced to canonical form."""

    message: str

    def __init__(self, message: str) -> None:
        super().__init__(message)
        self.message = message

    def __str__(self) -> str:
        return self.message


class InvalidSchemaType(CanonicalizationError):
    """The schema root is neither a boolean nor an object."""


class InvalidJsonValue(CanonicalizationError):
    """A schema literal value cannot be represented as canonical JSON."""


class InvalidPattern(CanonicalizationError):
    """A `pattern` / `patternProperties` regex failed to compile."""

    location: str

    def __init__(self, message: str, location: str) -> None:
        super().__init__(message)
        self.location = location

    def __reduce__(self) -> tuple[type[InvalidPattern], tuple[str, str]]:
        # `args` only carries `message`, so the default reduce would reconstruct without `location`
        # and raise a TypeError under pickle/deepcopy. Supply both required arguments.
        return (type(self), (self.message, self.location))


class UnguardedRecursion(CanonicalizationError):
    """A `$ref` cycle that never crosses a typed operator (no base case).

    Ill-founded: refers to itself without consuming the instance, e.g.
    ``a = allOf: [$ref a]``.
    """


class InfiniteRecursion(CanonicalizationError):
    """A recursive schema with no finite instance (recursion in a required position).

    Well-founded but unsatisfiable: every value must nest another forever, e.g.
    ``node = {required: [child], properties: {child: $ref node}}``.
    """


__all__ = [
    "NullView",
    "TrueView",
    "FalseView",
    "BooleanView",
    "IntegerView",
    "NumberView",
    "StringView",
    "ContentFacetView",
    "ContainsView",
    "ArrayView",
    "RequiredProperty",
    "PatternPropertyRequirement",
    "AdditionalPropertiesRequirement",
    "DependentPropertiesRequirement",
    "DependentSchemaRequirement",
    "NamedPropertyConstraint",
    "PatternPropertyConstraint",
    "AdditionalPropertiesConstraint",
    "ObjectView",
    "MultiTypeView",
    "AllOfView",
    "AnyOfView",
    "OneOfView",
    "NotView",
    "TypedGroupView",
    "TypeGuardView",
    "ConstView",
    "EnumView",
    "ReferenceView",
    "RecursiveView",
    "DynamicRefView",
    "RawView",
    "CanonicalViewType",
    "json",
    "schema",
    "CanonicalizationError",
    "InvalidSchemaType",
    "InvalidJsonValue",
    "InvalidPattern",
    "UnguardedRecursion",
    "InfiniteRecursion",
]

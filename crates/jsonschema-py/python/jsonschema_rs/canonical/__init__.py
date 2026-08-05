from __future__ import annotations

from ..jsonschema_rs import canonical as _canonical

TrueView = _canonical.TrueView
FalseView = _canonical.FalseView
MultiTypeView = _canonical.MultiTypeView
TypedGroupView = _canonical.TypedGroupView
StringView = _canonical.StringView
NumberView = _canonical.NumberView
IntegerView = _canonical.IntegerView
ArrayView = _canonical.ArrayView
ContainsView = _canonical.ContainsView
ObjectView = _canonical.ObjectView
NameFailsView = _canonical.NameFailsView
UndeclaredValueFailsView = _canonical.UndeclaredValueFailsView
NotView = _canonical.NotView
AllOfView = _canonical.AllOfView
AnyOfView = _canonical.AnyOfView
OneOfView = _canonical.OneOfView
ReferenceView = _canonical.ReferenceView
ConstView = _canonical.ConstView
EnumView = _canonical.EnumView
RawView = _canonical.RawView

CanonicalViewType = (
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


class InvalidPattern(CanonicalizationError):
    """A ``pattern`` value is not a valid regular expression."""


class IncompatibleOperands(CanonicalizationError):
    """Operands of a set operation cannot be combined."""


class UnmodeledOperand(CanonicalizationError):
    """A set operation reached a schema the canonical form does not model."""


__all__ = [
    "AllOfView",
    "AnyOfView",
    "ArrayView",
    "CanonicalViewType",
    "CanonicalizationError",
    "ConstView",
    "ContainsView",
    "EnumView",
    "FalseView",
    "IncompatibleOperands",
    "IntegerView",
    "InvalidPattern",
    "InvalidSchemaType",
    "MultiTypeView",
    "NameFailsView",
    "NotView",
    "ObjectView",
    "OneOfView",
    "RawView",
    "ReferenceView",
    "StringView",
    "NumberView",
    "TrueView",
    "TypedGroupView",
    "UndeclaredValueFailsView",
    "UnmodeledOperand",
    "json",
    "schema",
]

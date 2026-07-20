from __future__ import annotations

from ..jsonschema_rs import canonical as _canonical

TrueView = _canonical.TrueView
FalseView = _canonical.FalseView
MultiTypeView = _canonical.MultiTypeView
TypedGroupView = _canonical.TypedGroupView
AnyOfView = _canonical.AnyOfView
ConstView = _canonical.ConstView
EnumView = _canonical.EnumView
RawView = _canonical.RawView

CanonicalViewType = TrueView | FalseView | MultiTypeView | TypedGroupView | AnyOfView | ConstView | EnumView | RawView

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


__all__ = [
    "AnyOfView",
    "CanonicalViewType",
    "CanonicalizationError",
    "ConstView",
    "EnumView",
    "FalseView",
    "InvalidSchemaType",
    "MultiTypeView",
    "RawView",
    "TrueView",
    "TypedGroupView",
    "json",
    "schema",
]

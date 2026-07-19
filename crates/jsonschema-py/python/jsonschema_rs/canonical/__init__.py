from __future__ import annotations

from ..jsonschema_rs import canonical as _canonical

RawView = _canonical.RawView

CanonicalViewType = RawView

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
    "CanonicalViewType",
    "CanonicalizationError",
    "InvalidSchemaType",
    "RawView",
    "json",
    "schema",
]

from typing import Any

from .jsonschema_rs import (
    Draft4,
    Draft4Validator,
    Draft6,
    Draft6Validator,
    Draft7,
    Draft7Validator,
    Draft201909,
    Draft201909Validator,
    Draft202012,
    Draft202012Validator,
    FancyRegexOptions,
    RegexOptions,
    Registry,
    ValidationErrorKind,
    is_valid,
    iter_errors,
    meta,
    validate,
    validator_for,
)


class ValidationError(ValueError):
    """An instance is invalid under a provided schema."""

    message: str
    verbose_message: str
    schema_path: list[str | int]
    instance_path: list[str | int]
    kind: ValidationErrorKind
    instance: Any

    def __init__(
        self,
        message: str,
        verbose_message: str,
        schema_path: list[str | int],
        instance_path: list[str | int],
        kind: ValidationErrorKind,
        instance: Any,
    ) -> None:
        super().__init__(verbose_message)
        self.message = message
        self.verbose_message = verbose_message
        self.schema_path = schema_path
        self.instance_path = instance_path
        self.kind = kind
        self.instance = instance

    def __str__(self) -> str:
        return self.verbose_message

    def __repr__(self) -> str:
        return f"<ValidationError: '{self.message}'>"


class ReferencingError(Exception):
    """Errors that can occur during reference resolution and resource handling."""

    message: str

    def __init__(self, message: str) -> None:
        super().__init__(message)
        self.message = message

    def __str__(self) -> str:
        return self.message

    def __repr__(self) -> str:
        return f"<ReferencingError: '{self.message}'>"


__all__ = [
    "ReferencingError",
    "ValidationError",
    "ValidationErrorKind",
    "is_valid",
    "validate",
    "iter_errors",
    "validator_for",
    "Draft4",
    "Draft6",
    "Draft7",
    "Draft201909",
    "Draft202012",
    "Draft4Validator",
    "Draft6Validator",
    "Draft7Validator",
    "Draft201909Validator",
    "Draft202012Validator",
    "Registry",
    "FancyRegexOptions",
    "RegexOptions",
    "meta",
]

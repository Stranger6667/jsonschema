from collections.abc import Iterator
from typing import Any, Callable, TypeVar

_SchemaT = TypeVar("_SchemaT", bool, dict[str, Any])
_FormatFunc = TypeVar("_FormatFunc", bound=Callable[[str], bool])

def is_valid(
    schema: _SchemaT,
    instance: Any,
    draft: int | None = None,
    with_meta_schemas: bool | None = None,
    formats: dict[str, _FormatFunc] | None = None,
    validate_formats: bool | None = None,
    ignore_unknown_formats: bool = True,
) -> bool: ...
def validate(
    schema: _SchemaT,
    instance: Any,
    draft: int | None = None,
    with_meta_schemas: bool | None = None,
    formats: dict[str, _FormatFunc] | None = None,
    validate_formats: bool | None = None,
    ignore_unknown_formats: bool = True,
) -> None: ...
def iter_errors(
    schema: _SchemaT,
    instance: Any,
    draft: int | None = None,
    with_meta_schemas: bool | None = None,
    formats: dict[str, _FormatFunc] | None = None,
    validate_formats: bool | None = None,
    ignore_unknown_formats: bool = True,
) -> Iterator[ValidationError]: ...

class ValidationErrorKind: ...

class ValidationError(ValueError):
    message: str
    schema_path: list[str | int]
    instance_path: list[str | int]
    kind: ValidationErrorKind
    instance: list | dict | str | int | float | bool | None

Draft4: int
Draft6: int
Draft7: int
Draft201909: int
Draft202012: int

class Draft4Validator:
    def __init__(
        self,
        schema: _SchemaT | str,
        formats: dict[str, _FormatFunc] | None = None,
        validate_formats: bool | None = None,
        ignore_unknown_formats: bool = True,
    ) -> None: ...
    def is_valid(self, instance: Any) -> bool: ...
    def validate(self, instance: Any) -> None: ...
    def iter_errors(self, instance: Any) -> Iterator[ValidationError]: ...

class Draft6Validator:
    def __init__(
        self,
        schema: _SchemaT | str,
        formats: dict[str, _FormatFunc] | None = None,
        validate_formats: bool | None = None,
        ignore_unknown_formats: bool = True,
    ) -> None: ...
    def is_valid(self, instance: Any) -> bool: ...
    def validate(self, instance: Any) -> None: ...
    def iter_errors(self, instance: Any) -> Iterator[ValidationError]: ...

class Draft7Validator:
    def __init__(
        self,
        schema: _SchemaT | str,
        formats: dict[str, _FormatFunc] | None = None,
        validate_formats: bool | None = None,
        ignore_unknown_formats: bool = True,
    ) -> None: ...
    def is_valid(self, instance: Any) -> bool: ...
    def validate(self, instance: Any) -> None: ...
    def iter_errors(self, instance: Any) -> Iterator[ValidationError]: ...

class Draft201909Validator:
    def __init__(
        self,
        schema: _SchemaT | str,
        formats: dict[str, _FormatFunc] | None = None,
        validate_formats: bool | None = None,
        ignore_unknown_formats: bool = True,
    ) -> None: ...
    def is_valid(self, instance: Any) -> bool: ...
    def validate(self, instance: Any) -> None: ...
    def iter_errors(self, instance: Any) -> Iterator[ValidationError]: ...

class Draft202012Validator:
    def __init__(
        self,
        schema: _SchemaT | str,
        formats: dict[str, _FormatFunc] | None = None,
        validate_formats: bool | None = None,
        ignore_unknown_formats: bool = True,
    ) -> None: ...
    def is_valid(self, instance: Any) -> bool: ...
    def validate(self, instance: Any) -> None: ...
    def iter_errors(self, instance: Any) -> Iterator[ValidationError]: ...

def validator_for(
    schema: _SchemaT,
    formats: dict[str, _FormatFunc] | None = None,
    validate_formats: bool | None = None,
    ignore_unknown_formats: bool = True,
) -> Draft4Validator | Draft6Validator | Draft7Validator | Draft201909Validator | Draft202012Validator: ...

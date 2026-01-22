from collections.abc import Iterator
from decimal import Decimal
from typing import Any, Callable, List, Protocol, TypeAlias, TypeVar, TypedDict, Union

_SchemaT = TypeVar("_SchemaT", bool, dict[str, Any])
_FormatFunc = TypeVar("_FormatFunc", bound=Callable[[str], bool])
JSONType: TypeAlias = dict[str, Any] | list | str | int | float | Decimal | bool | None
JSONPrimitive: TypeAlias = str | int | float | Decimal | bool | None

class KeywordValidator(Protocol):
    """Protocol for custom keyword validators.

    Custom keywords are classes instantiated with (parent_schema, value, schema_path)
    that implement a validate(instance) method which raises an exception on failure.

    Example:
        class DivisibleBy:
            def __init__(self, parent_schema, value, schema_path):
                self.divisor = value

            def validate(self, instance):
                if isinstance(instance, int) and instance % self.divisor != 0:
                    raise ValueError(f"{instance} is not divisible by {self.divisor}")

        validator = jsonschema_rs.validator_for(
            {"divisibleBy": 3},
            keywords={"divisibleBy": DivisibleBy},
        )

    """

    def __init__(self, parent_schema: dict[str, Any], value: Any, schema_path: list[str | int]) -> None: ...
    def validate(self, instance: JSONType) -> None: ...

class EvaluationAnnotation(TypedDict):
    schemaLocation: str
    absoluteKeywordLocation: str | None
    instanceLocation: str
    annotations: JSONType

class EvaluationErrorEntry(TypedDict):
    schemaLocation: str
    absoluteKeywordLocation: str | None
    instanceLocation: str
    error: str

class FlagOutput(TypedDict):
    """JSON Schema Output v1 - Flag format."""

    valid: bool

class OutputUnit(TypedDict, total=False):
    """A single output unit in list/hierarchical formats."""

    valid: bool
    evaluationPath: str
    schemaLocation: str
    instanceLocation: str
    errors: dict[str, str]
    annotations: JSONType
    droppedAnnotations: JSONType
    details: List["OutputUnit"]

class ListOutput(TypedDict):
    """JSON Schema Output v1 - List format."""

    valid: bool
    details: List[OutputUnit]

class Evaluation:
    valid: bool
    def flag(self) -> FlagOutput: ...
    def list(self) -> ListOutput: ...
    def hierarchical(self) -> OutputUnit: ...
    def annotations(self) -> List[EvaluationAnnotation]: ...
    def errors(self) -> List[EvaluationErrorEntry]: ...
    def __repr__(self) -> str: ...

class FancyRegexOptions:
    def __init__(
        self, backtrack_limit: int | None = None, size_limit: int | None = None, dfa_size_limit: int | None = None
    ) -> None: ...
    def __repr__(self) -> str: ...

class RegexOptions:
    def __init__(self, size_limit: int | None = None, dfa_size_limit: int | None = None) -> None: ...
    def __repr__(self) -> str: ...

class EmailOptions:
    """Configuration for email format validation."""

    def __init__(
        self,
        require_tld: bool = False,
        allow_domain_literal: bool = True,
        allow_display_text: bool = True,
        minimum_sub_domains: int | None = None,
    ) -> None: ...
    def __repr__(self) -> str: ...

class HttpOptions:
    """Configuration for HTTP client used in schema retrieval."""

    timeout: float | None
    connect_timeout: float | None
    tls_verify: bool
    ca_cert: str | None

    def __init__(
        self,
        timeout: float | None = None,
        connect_timeout: float | None = None,
        tls_verify: bool = True,
        ca_cert: str | None = None,
    ) -> None: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...

PatternOptionsType = Union[FancyRegexOptions, RegexOptions]

class RetrieverProtocol(Protocol):
    def __call__(self, uri: str) -> JSONType: ...

def is_valid(
    schema: _SchemaT,
    instance: Any,
    draft: int | None = None,
    formats: dict[str, _FormatFunc] | None = None,
    validate_formats: bool | None = None,
    ignore_unknown_formats: bool = True,
    retriever: RetrieverProtocol | None = None,
    registry: Registry | None = None,
    mask: str | None = None,
    base_uri: str | None = None,
    pattern_options: PatternOptionsType | None = None,
    email_options: EmailOptions | None = None,
    http_options: HttpOptions | None = None,
    keywords: dict[str, type[KeywordValidator]] | None = None,
) -> bool:
    """Check if a JSON instance is valid against a schema.

    Returns True if valid, False otherwise. Stops at the first error.
    """
    ...

def validate(
    schema: _SchemaT,
    instance: Any,
    draft: int | None = None,
    formats: dict[str, _FormatFunc] | None = None,
    validate_formats: bool | None = None,
    ignore_unknown_formats: bool = True,
    retriever: RetrieverProtocol | None = None,
    registry: Registry | None = None,
    mask: str | None = None,
    base_uri: str | None = None,
    pattern_options: PatternOptionsType | None = None,
    email_options: EmailOptions | None = None,
    http_options: HttpOptions | None = None,
    keywords: dict[str, type[KeywordValidator]] | None = None,
) -> None:
    """Validate a JSON instance against a schema.

    Raises ValidationError if invalid. Stops at the first error.
    """
    ...

def iter_errors(
    schema: _SchemaT,
    instance: Any,
    draft: int | None = None,
    formats: dict[str, _FormatFunc] | None = None,
    validate_formats: bool | None = None,
    ignore_unknown_formats: bool = True,
    retriever: RetrieverProtocol | None = None,
    registry: Registry | None = None,
    mask: str | None = None,
    base_uri: str | None = None,
    pattern_options: PatternOptionsType | None = None,
    email_options: EmailOptions | None = None,
    http_options: HttpOptions | None = None,
    keywords: dict[str, type[KeywordValidator]] | None = None,
) -> Iterator[ValidationError]:
    """Iterate over all validation errors.

    Returns an iterator of ValidationError objects for all errors found.
    """
    ...

def evaluate(
    schema: _SchemaT,
    instance: Any,
    draft: int | None = None,
    formats: dict[str, _FormatFunc] | None = None,
    validate_formats: bool | None = None,
    ignore_unknown_formats: bool = True,
    retriever: RetrieverProtocol | None = None,
    registry: Registry | None = None,
    base_uri: str | None = None,
    pattern_options: PatternOptionsType | None = None,
    email_options: EmailOptions | None = None,
    http_options: HttpOptions | None = None,
    keywords: dict[str, type[KeywordValidator]] | None = None,
) -> Evaluation:
    """Evaluate an instance and return structured output.

    Returns an Evaluation object with flag(), list(), and hierarchical() output formats.
    """
    ...

class ReferencingError:
    message: str

class ValidationErrorKind:
    @property
    def name(self) -> str:
        """The JSON Schema keyword that triggered this error."""
        ...
    @property
    def value(self) -> Any: ...
    def as_dict(self) -> dict[str, Any]: ...

    class AdditionalItems:
        limit: int

    class AdditionalProperties:
        unexpected: list[str]

    class AnyOf:
        context: list[list["ValidationError"]]

    class BacktrackLimitExceeded:
        error: str

    class Constant:
        expected_value: JSONType

    class Contains: ...

    class ContentEncoding:
        content_encoding: str

    class ContentMediaType:
        content_media_type: str

    class Custom:
        keyword: str
        message: str

    class Enum:
        options: list[JSONType]

    class ExclusiveMaximum:
        limit: JSONPrimitive

    class ExclusiveMinimum:
        limit: JSONPrimitive

    class FalseSchema: ...

    class Format:
        format: str

    class FromUtf8:
        error: str

    class MaxItems:
        limit: int

    class Maximum:
        limit: JSONPrimitive

    class MaxLength:
        limit: int

    class MaxProperties:
        limit: int

    class MinItems:
        limit: int

    class Minimum:
        limit: JSONPrimitive

    class MinLength:
        limit: int

    class MinProperties:
        limit: int

    class MultipleOf:
        multiple_of: int | float | Decimal

    class Not:
        schema: JSONType

    class OneOfMultipleValid:
        context: list[list["ValidationError"]]

    class OneOfNotValid:
        context: list[list["ValidationError"]]

    class Pattern:
        pattern: str

    class PropertyNames:
        error: "ValidationError"

    class Required:
        property: str

    class Type:
        types: list[str]

    class UnevaluatedItems:
        unexpected: list[int]

    class UnevaluatedProperties:
        unexpected: list[str]

    class UniqueItems: ...

    class Referencing:
        error: ReferencingError

class ValidationError(ValueError):
    message: str
    verbose_message: str
    schema_path: list[str | int]
    instance_path: list[str | int]
    evaluation_path: list[str | int]
    kind: ValidationErrorKind
    instance: JSONType

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
        retriever: RetrieverProtocol | None = None,
        registry: Registry | None = None,
        mask: str | None = None,
        base_uri: str | None = None,
        pattern_options: PatternOptionsType | None = None,
        email_options: EmailOptions | None = None,
        http_options: HttpOptions | None = None,
        keywords: dict[str, type[KeywordValidator]] | None = None,
    ) -> None: ...
    def is_valid(self, instance: Any) -> bool: ...
    def validate(self, instance: Any) -> None: ...
    def iter_errors(self, instance: Any) -> Iterator[ValidationError]: ...
    def evaluate(self, instance: Any) -> Evaluation: ...
    def __repr__(self) -> str: ...

class Draft6Validator:
    def __init__(
        self,
        schema: _SchemaT | str,
        formats: dict[str, _FormatFunc] | None = None,
        validate_formats: bool | None = None,
        ignore_unknown_formats: bool = True,
        retriever: RetrieverProtocol | None = None,
        registry: Registry | None = None,
        mask: str | None = None,
        base_uri: str | None = None,
        pattern_options: PatternOptionsType | None = None,
        email_options: EmailOptions | None = None,
        http_options: HttpOptions | None = None,
        keywords: dict[str, type[KeywordValidator]] | None = None,
    ) -> None: ...
    def is_valid(self, instance: Any) -> bool: ...
    def validate(self, instance: Any) -> None: ...
    def iter_errors(self, instance: Any) -> Iterator[ValidationError]: ...
    def evaluate(self, instance: Any) -> Evaluation: ...
    def __repr__(self) -> str: ...

class Draft7Validator:
    def __init__(
        self,
        schema: _SchemaT | str,
        formats: dict[str, _FormatFunc] | None = None,
        validate_formats: bool | None = None,
        ignore_unknown_formats: bool = True,
        retriever: RetrieverProtocol | None = None,
        registry: Registry | None = None,
        mask: str | None = None,
        base_uri: str | None = None,
        pattern_options: PatternOptionsType | None = None,
        email_options: EmailOptions | None = None,
        http_options: HttpOptions | None = None,
        keywords: dict[str, type[KeywordValidator]] | None = None,
    ) -> None: ...
    def is_valid(self, instance: Any) -> bool: ...
    def validate(self, instance: Any) -> None: ...
    def iter_errors(self, instance: Any) -> Iterator[ValidationError]: ...
    def evaluate(self, instance: Any) -> Evaluation: ...
    def __repr__(self) -> str: ...

class Draft201909Validator:
    def __init__(
        self,
        schema: _SchemaT | str,
        formats: dict[str, _FormatFunc] | None = None,
        validate_formats: bool | None = None,
        ignore_unknown_formats: bool = True,
        retriever: RetrieverProtocol | None = None,
        registry: Registry | None = None,
        mask: str | None = None,
        base_uri: str | None = None,
        pattern_options: PatternOptionsType | None = None,
        email_options: EmailOptions | None = None,
        http_options: HttpOptions | None = None,
        keywords: dict[str, type[KeywordValidator]] | None = None,
    ) -> None: ...
    def is_valid(self, instance: Any) -> bool: ...
    def validate(self, instance: Any) -> None: ...
    def iter_errors(self, instance: Any) -> Iterator[ValidationError]: ...
    def evaluate(self, instance: Any) -> Evaluation: ...
    def __repr__(self) -> str: ...

class Draft202012Validator:
    def __init__(
        self,
        schema: _SchemaT | str,
        formats: dict[str, _FormatFunc] | None = None,
        validate_formats: bool | None = None,
        ignore_unknown_formats: bool = True,
        retriever: RetrieverProtocol | None = None,
        registry: Registry | None = None,
        mask: str | None = None,
        base_uri: str | None = None,
        pattern_options: PatternOptionsType | None = None,
        email_options: EmailOptions | None = None,
        http_options: HttpOptions | None = None,
        keywords: dict[str, type[KeywordValidator]] | None = None,
    ) -> None: ...
    def is_valid(self, instance: Any) -> bool: ...
    def validate(self, instance: Any) -> None: ...
    def iter_errors(self, instance: Any) -> Iterator[ValidationError]: ...
    def evaluate(self, instance: Any) -> Evaluation: ...
    def __repr__(self) -> str: ...

Validator: TypeAlias = Draft4Validator | Draft6Validator | Draft7Validator | Draft201909Validator | Draft202012Validator

def validator_for(
    schema: _SchemaT,
    formats: dict[str, _FormatFunc] | None = None,
    validate_formats: bool | None = None,
    ignore_unknown_formats: bool = True,
    retriever: RetrieverProtocol | None = None,
    registry: Registry | None = None,
    mask: str | None = None,
    base_uri: str | None = None,
    pattern_options: PatternOptionsType | None = None,
    email_options: EmailOptions | None = None,
    http_options: HttpOptions | None = None,
    keywords: dict[str, type[KeywordValidator]] | None = None,
) -> Validator:
    """Create a validator for the given schema.

    Automatically detects the JSON Schema draft from the $schema keyword.
    Returns a Draft-specific validator instance.
    """
    ...

class Registry:
    def __init__(
        self,
        resources: list[tuple[str, JSONType]],
        draft: int | None = None,
        retriever: RetrieverProtocol | None = None,
    ) -> None: ...
    def __repr__(self) -> str: ...

class _Meta:
    def is_valid(self, schema: _SchemaT, registry: Registry | None = None) -> bool: ...
    def validate(self, schema: _SchemaT, registry: Registry | None = None) -> None: ...

meta: _Meta

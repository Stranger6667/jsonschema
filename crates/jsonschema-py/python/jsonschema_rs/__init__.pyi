from collections.abc import Iterable, Iterator, Sequence
from decimal import Decimal
from typing import Any, Callable, List, Literal, Protocol, TypeAlias, TypeVar, TypedDict, Union, final

from typing_extensions import Self

from . import canonical as canonical

_SchemaT = TypeVar("_SchemaT", bound=dict[str, Any] | bool)
_Schema: TypeAlias = dict[str, Any] | bool
_FormatFunc: TypeAlias = Callable[..., bool]
# Loose JSON for inputs the caller supplies
JSONType: TypeAlias = dict[str, Any] | list[Any] | tuple[Any, ...] | str | int | float | Decimal | bool | None
# Precise recursive JSON the library returns
JsonValue: TypeAlias = dict[str, "JsonValue"] | list["JsonValue"] | str | int | float | Decimal | bool | None
JSONPrimitive: TypeAlias = str | int | float | Decimal | bool | None

class KeywordValidator(Protocol):
    """Protocol for custom keyword validators.

    Custom keywords are classes instantiated with (parent_schema, value, schema_path)
    that implement a validate(instance) method which raises an exception on failure.

    A keyword may also implement an optional iter_errors(instance) method that yields one
    exception per problem, so a single keyword can report multiple errors at once. When it
    is absent, iterating errors yields at most the single error from validate.

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
    annotations: JsonValue

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
    annotations: JsonValue
    droppedAnnotations: JsonValue
    details: List["OutputUnit"]

class ListOutput(TypedDict):
    """JSON Schema Output v1 - List format."""

    valid: bool
    details: List[OutputUnit]

@final
class Evaluation:
    valid: bool
    """Whether the evaluated instance is valid."""

    def flag(self) -> FlagOutput:
        """Return the flag output as a dict."""
        ...
    def list(self) -> ListOutput:
        """Return the list output as a dict."""
        ...
    def hierarchical(self) -> OutputUnit:
        """Return the hierarchical output as a dict."""
        ...
    def annotations(self) -> List[EvaluationAnnotation]:
        """Return collected annotations for all evaluated nodes."""
        ...
    def errors(self) -> List[EvaluationErrorEntry]:
        """Return collected errors for all evaluated nodes."""
        ...
    def __repr__(self) -> str: ...

@final
class FancyRegexOptions:
    """Configuration for the fancy-regex engine, which supports lookaround and backreferences.

        >>> validator = validator_for(
        ...     {"type": "string", "pattern": "^(a+)+$"},  # Potentially problematic pattern
        ...     pattern_options=FancyRegexOptions(backtrack_limit=10000)
        ... )

    Args:
        backtrack_limit: Maximum number of backtracking steps
        size_limit: Maximum compiled pattern size in bytes
        dfa_size_limit: Maximum regex DFA cache size in bytes
        syntax: "ecma262" translates patterns for the engine, "engine" passes them to it unchanged

    """

    def __new__(
        cls,
        backtrack_limit: int | None = None,
        size_limit: int | None = None,
        dfa_size_limit: int | None = None,
        syntax: Literal["ecma262", "engine"] = "ecma262",
    ) -> Self: ...
    def __repr__(self) -> str: ...

@final
class RegexOptions:
    """Configuration for the regex engine, which matches in linear time and supports no lookaround or backreferences.

        >>> validator = validator_for(
        ...     schema,
        ...     pattern_options=RegexOptions()
        ... )

    Args:
        size_limit: Maximum compiled pattern size in bytes
        dfa_size_limit: Maximum regex DFA cache size in bytes
        syntax: "ecma262" translates patterns for the engine, "engine" passes them to it unchanged

    """

    def __new__(
        cls,
        size_limit: int | None = None,
        dfa_size_limit: int | None = None,
        syntax: Literal["ecma262", "engine"] = "ecma262",
    ) -> Self: ...
    def __repr__(self) -> str: ...

@final
class EmailOptions:
    """Configuration for email format validation.

        >>> from jsonschema_rs import validator_for, EmailOptions
        >>> validator = validator_for(
        ...     {"format": "email", "type": "string"},
        ...     validate_formats=True,
        ...     email_options=EmailOptions(require_tld=True)
        ... )
        >>> validator.is_valid("user@localhost")
        False
        >>> validator.is_valid("user@example.com")
        True

    Args:
        require_tld: Require a top-level domain (e.g., reject "user@localhost")
        allow_domain_literal: Allow IP address literals like "user@[127.0.0.1]"
        allow_display_text: Allow display names like "Name <user@example.com>"
        minimum_sub_domains: Minimum number of domain segments required

    """

    def __new__(
        cls,
        require_tld: bool = False,
        allow_domain_literal: bool = True,
        allow_display_text: bool = True,
        minimum_sub_domains: int | None = None,
    ) -> Self: ...
    def __repr__(self) -> str: ...

@final
class HttpOptions:
    """Configuration for the HTTP client that fetches external schemas.

        >>> from jsonschema_rs import validator_for, HttpOptions
        >>> validator = validator_for(
        ...     {"$ref": "https://example.com/schema.json"},
        ...     http_options=HttpOptions(timeout=30.0, connect_timeout=10.0)
        ... )

    Args:
        timeout: Total request timeout in seconds (default: None - no timeout)
        connect_timeout: Connection timeout in seconds (default: None - no timeout)
        tls_verify: Whether to verify TLS certificates (default: True)
        ca_cert: Path to a custom CA certificate file in PEM format (default: None)

    """

    timeout: float | None
    connect_timeout: float | None
    tls_verify: bool
    ca_cert: str | None

    def __new__(
        cls,
        timeout: float | None = None,
        connect_timeout: float | None = None,
        tls_verify: bool = True,
        ca_cert: str | None = None,
    ) -> Self: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object, /) -> bool: ...
    def __hash__(self) -> int: ...

PatternOptionsType = Union[FancyRegexOptions, RegexOptions]

class RetrieverProtocol(Protocol):
    def __call__(self, uri: str, /) -> JSONType: ...

def is_valid(
    schema: _Schema,
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
    vocabularies: Sequence[str] | None = None,
    offline: bool | None = None,
) -> bool:
    """Return whether `instance` is valid under `schema`. Stops at the first error.

        >>> is_valid({"minimum": 5}, 3)
        False

    To validate many instances against one schema, use `validator_for(schema).is_valid`.
    """
    ...

def validate(
    schema: _Schema,
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
    vocabularies: Sequence[str] | None = None,
    offline: bool | None = None,
) -> None:
    """Raise `ValidationError` for the first error of `instance` under `schema`.

        >>> validate({"minimum": 5}, 3)
        ...
        ValidationError: 3 is less than the minimum of 5

    To validate many instances against one schema, use `validator_for(schema).validate`.
    """
    ...

def iter_errors(
    schema: _Schema,
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
    vocabularies: Sequence[str] | None = None,
    offline: bool | None = None,
) -> Iterator[ValidationError]:
    """Iterate over every `ValidationError` of `instance` under `schema`.

        >>> print(next(iter_errors({"minimum": 5}, 3)).message)
        3 is less than the minimum of 5

    To validate many instances against one schema, use `validator_for(schema).iter_errors`.
    """
    ...

def evaluate(
    schema: _Schema,
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
    vocabularies: Sequence[str] | None = None,
    offline: bool | None = None,
) -> Evaluation:
    """Evaluate `instance` against `schema` and return an `Evaluation` holding the output formats.

    >>> evaluation = evaluate({"type": "integer"}, "oops")
    >>> evaluation.flag()["valid"]
    False
    >>> print(evaluation.errors()[0]["error"])
    "oops" is not of type "integer"
    """
    ...

class ReferencingError(Exception):
    """A reference could not be resolved or its resource could not be retrieved."""

    message: str
    def __init__(self, message: str) -> None: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...

class ValidationErrorKind:
    """Type of validation failure with its contextual data."""

    @property
    def name(self) -> str:
        """The keyword that failed, or ``"falseSchema"`` for a ``false`` schema."""
        ...
    @property
    def value(self) -> Any: ...
    def as_dict(self) -> dict[str, Any]: ...

    @final
    class AdditionalItems(ValidationErrorKind):
        __match_args__ = ("limit",)
        limit: int

    @final
    class AdditionalProperties(ValidationErrorKind):
        __match_args__ = ("unexpected",)
        unexpected: list[str]

    @final
    class AnyOf(ValidationErrorKind):
        __match_args__ = ("context",)
        context: list[list["ValidationError"]]

    @final
    class BacktrackLimitExceeded(ValidationErrorKind):
        __match_args__ = ("error",)
        error: str

    @final
    class Constant(ValidationErrorKind):
        __match_args__ = ("expected_value",)
        expected_value: JsonValue

    @final
    class Contains(ValidationErrorKind):
        __match_args__ = ()

    @final
    class ContentEncoding(ValidationErrorKind):
        __match_args__ = ("content_encoding",)
        content_encoding: str

    @final
    class ContentMediaType(ValidationErrorKind):
        __match_args__ = ("content_media_type",)
        content_media_type: str

    @final
    class Custom(ValidationErrorKind):
        __match_args__ = ("keyword", "message")
        keyword: str
        message: str

    @final
    class Enum(ValidationErrorKind):
        __match_args__ = ("options",)
        options: list[JsonValue]

    @final
    class ExclusiveMaximum(ValidationErrorKind):
        __match_args__ = ("limit",)
        limit: JSONPrimitive

    @final
    class ExclusiveMinimum(ValidationErrorKind):
        __match_args__ = ("limit",)
        limit: JSONPrimitive

    @final
    class FalseSchema(ValidationErrorKind):
        __match_args__ = ()

    @final
    class Format(ValidationErrorKind):
        __match_args__ = ("format",)
        format: str

    @final
    class FromUtf8(ValidationErrorKind):
        __match_args__ = ("error",)
        error: str

    @final
    class MaxContains(ValidationErrorKind):
        __match_args__ = ("limit",)
        limit: int

    @final
    class MaxItems(ValidationErrorKind):
        __match_args__ = ("limit",)
        limit: int

    @final
    class Maximum(ValidationErrorKind):
        __match_args__ = ("limit",)
        limit: JSONPrimitive

    @final
    class MaxLength(ValidationErrorKind):
        __match_args__ = ("limit",)
        limit: int

    @final
    class MaxProperties(ValidationErrorKind):
        __match_args__ = ("limit",)
        limit: int

    @final
    class MinContains(ValidationErrorKind):
        __match_args__ = ("limit",)
        limit: int

    @final
    class MinItems(ValidationErrorKind):
        __match_args__ = ("limit",)
        limit: int

    @final
    class Minimum(ValidationErrorKind):
        __match_args__ = ("limit",)
        limit: JSONPrimitive

    @final
    class MinLength(ValidationErrorKind):
        __match_args__ = ("limit",)
        limit: int

    @final
    class MinProperties(ValidationErrorKind):
        __match_args__ = ("limit",)
        limit: int

    @final
    class MultipleOf(ValidationErrorKind):
        __match_args__ = ("multiple_of",)
        multiple_of: int | float | Decimal

    @final
    class Not(ValidationErrorKind):
        __match_args__ = ("schema",)
        schema: JsonValue

    @final
    class OneOfMultipleValid(ValidationErrorKind):
        __match_args__ = ("context",)
        context: list[list["ValidationError"]]

    @final
    class OneOfNotValid(ValidationErrorKind):
        __match_args__ = ("context",)
        context: list[list["ValidationError"]]

    @final
    class Pattern(ValidationErrorKind):
        __match_args__ = ("pattern",)
        pattern: str

    @final
    class PropertyNames(ValidationErrorKind):
        __match_args__ = ("error",)
        error: "ValidationError"

    @final
    class Required(ValidationErrorKind):
        __match_args__ = ("property",)
        property: str

    @final
    class Type(ValidationErrorKind):
        __match_args__ = ("types",)
        types: list[str]

    @final
    class UnevaluatedItems(ValidationErrorKind):
        __match_args__ = ("unexpected",)
        unexpected: list[int]

    @final
    class UnevaluatedProperties(ValidationErrorKind):
        __match_args__ = ("unexpected",)
        unexpected: list[str]

    @final
    class UniqueItems(ValidationErrorKind):
        __match_args__ = ()

    @final
    class Referencing(ValidationErrorKind):
        __match_args__ = ("error",)
        error: ReferencingError

class ValidationError(ValueError):
    """An instance is invalid under a provided schema."""

    message: str
    verbose_message: str
    schema_path: list[str | int]
    instance_path: list[str | int]
    evaluation_path: list[str | int]
    kind: ValidationErrorKind
    instance: JsonValue
    absolute_keyword_location: str | None
    def __init__(
        self,
        message: str,
        verbose_message: str,
        schema_path: list[str | int],
        instance_path: list[str | int],
        evaluation_path: list[str | int],
        kind: ValidationErrorKind,
        instance: Any,
        absolute_keyword_location: str | None = None,
    ) -> None: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...

Draft4: int
Draft6: int
Draft7: int
Draft201909: int
Draft202012: int

class Validator:
    def __new__(
        cls,
        schema: _Schema | str,
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
        vocabularies: Sequence[str] | None = None,
        offline: bool | None = None,
    ) -> Self: ...
    def is_valid(self, instance: Any) -> bool:
        """Return whether `instance` is valid. Stops at the first error.

        >>> validator = validator_for({"minimum": 5})
        >>> validator.is_valid(3)
        False
        """
        ...
    def validate(self, instance: Any) -> None:
        """Raise `ValidationError` for the first error of `instance`.

        >>> validator = validator_for({"minimum": 5})
        >>> validator.validate(3)
        ...
        ValidationError: 3 is less than the minimum of 5
        """
        ...
    def iter_errors(self, instance: Any) -> Iterator[ValidationError]:
        """Iterate over every `ValidationError` of `instance`.

        >>> validator = validator_for({"minimum": 5})
        >>> print(next(validator.iter_errors(3)).message)
        3 is less than the minimum of 5
        """
        ...
    def evaluate(self, instance: Any) -> Evaluation:
        """Evaluate `instance` and return an `Evaluation` holding the output formats.

        >>> validator = validator_for({"type": "integer"})
        >>> validator.evaluate("oops").flag()["valid"]
        False
        """
        ...
    def __repr__(self) -> str: ...

class Draft4Validator(Validator):
    """A JSON Schema Draft 4 validator.

    >>> validator = Draft4Validator({"minimum": 5})
    >>> validator.is_valid(3)
    False
    """

class Draft6Validator(Validator):
    """A JSON Schema Draft 6 validator.

    >>> validator = Draft6Validator({"minimum": 5})
    >>> validator.is_valid(3)
    False
    """

class Draft7Validator(Validator):
    """A JSON Schema Draft 7 validator.

    >>> validator = Draft7Validator({"minimum": 5})
    >>> validator.is_valid(3)
    False
    """

class Draft201909Validator(Validator):
    """A JSON Schema Draft 2019-09 validator.

    >>> validator = Draft201909Validator({"minimum": 5})
    >>> validator.is_valid(3)
    False
    """

class Draft202012Validator(Validator):
    """A JSON Schema Draft 2020-12 validator.

    >>> validator = Draft202012Validator({"minimum": 5})
    >>> validator.is_valid(3)
    False
    """

def validator_for(
    schema: _Schema | str,
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
    vocabularies: Sequence[str] | None = None,
    offline: bool | None = None,
) -> Validator:
    """Create a validator for `schema`, an instance of the draft class its `$schema` names (Draft 2020-12 without one).

    >>> validator = validator_for({"minimum": 5})
    >>> validator.is_valid(3)
    False
    """
    ...

@final
class ValidatorMap:
    """Compiled validators keyed by URI-fragment JSON pointer (e.g. ``"#/$defs/User"``), from :func:`validator_map_for`."""

    def get(self, pointer: str) -> Validator | None:
        """Return the validator for ``pointer``, or ``None`` if not found.

        >>> m = validator_map_for({"$defs": {"T": {"type": "string"}}})
        >>> v = m.get("#/$defs/T")
        """
        ...

    def __getitem__(self, pointer: str, /) -> Validator: ...
    def __contains__(self, pointer: object, /) -> bool: ...
    def keys(self) -> list[str]:
        """Return all URI-fragment JSON pointer keys in this map."""
        ...
    def __len__(self) -> int: ...

def validator_map_for(
    schema: _Schema | str,
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
    vocabularies: Sequence[str] | None = None,
    offline: bool | None = None,
) -> ValidatorMap:
    """Compile all subschemas in ``schema`` into a map keyed by URI-fragment JSON pointer.

    The root schema is under ``"#"``.

        >>> schema = {"$defs": {"User": {"type": "object"}}}
        >>> m = validator_map_for(schema)
        >>> v = m.get("#/$defs/User")
        >>> v.is_valid({})
        True
    """
    ...

def bundle(
    schema: _SchemaT,
    /,
    *,
    retriever: RetrieverProtocol | None = None,
    registry: Registry | None = None,
    draft: int | None = None,
    base_uri: str | None = None,
    offline: bool | None = None,
) -> _SchemaT:
    """Bundle a JSON Schema into a Compound Schema Document.

    All externally-referenced schemas reachable via $ref are embedded in a
    draft-appropriate container (definitions for Draft 4/6/7, $defs for
    Draft 2019-09/2020-12).
    Original $ref values are preserved unchanged.

    :raises ReferencingError: if a referenced document cannot be retrieved.
    :raises ValueError: for any other `$ref` that cannot be resolved.
    """
    ...

def dereference(
    schema: _SchemaT,
    /,
    *,
    retriever: RetrieverProtocol | None = None,
    registry: Registry | None = None,
    draft: int | None = None,
    base_uri: str | None = None,
    offline: bool | None = None,
) -> _SchemaT:
    """Recursively inline all `$ref` values in a JSON Schema.

    Circular references are left in place as `$ref` strings.

    :param schema: The JSON Schema to dereference.
    :param retriever: Optional callable ``(uri: str) -> dict`` for fetching external schemas.
    :param registry: Optional pre-built :class:`Registry` of known schemas.
    :param offline: Refuse to fetch anything outside ``registry``.
    :param draft: Optional draft version (e.g. ``jsonschema_rs.Draft202012``).
    :param base_uri: Optional base URI for the root schema.
    :raises ReferencingError: if a referenced document cannot be retrieved.
    :raises ValueError: for any other `$ref` that cannot be resolved.

        >>> result = dereference({"$defs": {"t": {"type": "string"}}, "properties": {"x": {"$ref": "#/$defs/t"}}})
    """
    ...

def validator_cls_for(schema: _Schema | str) -> type[Validator]:
    """Detect the JSON Schema draft for a schema and return the corresponding validator class.

    Draft is detected automatically from the $schema field. Defaults to Draft202012Validator.
    """
    ...

@final
class Registry:
    """A registry of JSON Schema resources, each identified by their canonical URIs."""

    def __new__(
        cls,
        resources: Iterable[tuple[str, JSONType]],
        draft: int | None = None,
        retriever: RetrieverProtocol | None = None,
    ) -> Self: ...
    def resolver(self, base_uri: str) -> Resolver: ...
    def __repr__(self) -> str: ...

@final
class Resolver:
    @property
    def base_uri(self) -> str: ...
    @property
    def dynamic_scope(self) -> tuple[str, ...]: ...
    def lookup(self, reference: str) -> Resolved: ...
    def lookup_dynamic_ref(self, reference: str) -> Resolved: ...

@final
class Resolved:
    @property
    def contents(self) -> JsonValue: ...
    @property
    def resolver(self) -> Resolver: ...
    @property
    def draft(self) -> int: ...

class _Meta:
    def is_valid(self, schema: _Schema, registry: Registry | None = None) -> bool:
        """Validate a JSON Schema document against its meta-schema.

        Draft version is detected automatically. Schemas with unknown `$schema` values raise `jsonschema_rs.ReferencingError`.

            >>> jsonschema_rs.meta.is_valid({"type": "string"})
            True
            >>> jsonschema_rs.meta.is_valid({"type": "invalid_type"})
            False

        For custom meta-schemas, provide a registry:

            >>> custom_meta = {"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object"}
            >>> registry = Registry([("http://example.com/meta", custom_meta)])
            >>> schema = {"$schema": "http://example.com/meta", "customKeyword": "value"}
            >>> jsonschema_rs.meta.is_valid(schema, registry=registry)
            True
        """
        ...
    def validate(self, schema: _Schema, registry: Registry | None = None) -> None:
        """Validate a JSON Schema document against its meta-schema and raise ValidationError if invalid.

        Draft version is detected automatically. Schemas with custom/unknown `$schema` values raise `jsonschema_rs.ReferencingError`.

            >>> jsonschema_rs.meta.validate({"type": "string"})
            >>> jsonschema_rs.meta.validate({"type": "invalid_type"})
            Traceback (most recent call last):
                ...
            jsonschema_rs.ValidationError: ...
            >>> jsonschema_rs.meta.validate({"$schema": "http://custom.example.com/schema", "type": "object"})
            Traceback (most recent call last):
                ...
            jsonschema_rs.ReferencingError: Unknown meta-schema: 'http://custom.example.com/schema'. Custom meta-schemas must be registered in the registry before use

        For custom meta-schemas, provide a registry:

            >>> custom_meta = {"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object"}
            >>> registry = Registry([("http://example.com/meta", custom_meta)])
            >>> schema = {"$schema": "http://example.com/meta", "customKeyword": "value"}
            >>> jsonschema_rs.meta.validate(schema, registry=registry)
        """
        ...

meta: _Meta

@final
class CanonicalSchema:
    """Canonical representation of a JSON Schema, obtained via :func:`canonicalize`."""

    @property
    def draft(self) -> int:
        """The JSON Schema draft as an integer: 4, 6, 7, 19 (2019-09), or 20 (2020-12)."""
        ...

    @property
    def kind(self) -> canonical.CanonicalKind:
        """Structural kind of this node."""
        ...

    def view(self) -> canonical.CanonicalViewType:
        """Return the single view object for this node; dispatch with ``match``."""
        ...

    def intersect(self, other: CanonicalSchema) -> CanonicalSchema:
        """Every value both schemas accept."""
        ...

    def union(self, other: CanonicalSchema) -> CanonicalSchema:
        """Every value either schema accepts."""
        ...

    def subtract(self, other: CanonicalSchema) -> CanonicalSchema:
        """Every value this schema accepts and ``other`` rejects.

        ``old.subtract(new)`` is unsatisfiable when ``new`` accepts every value ``old`` accepts.
        """
        ...

    def covers(self, other: CanonicalSchema) -> canonical.Containment:
        """Whether this schema accepts every value ``other`` accepts.

        Only ``YES`` proves containment; ``UNKNOWN`` means undecided, not disproved. Test for
        ``YES``, and treat ``UNKNOWN`` like ``NO``.
        """
        ...

    def negate(self) -> CanonicalSchema:
        """Every value this schema rejects.

        Raises ``UnsupportedOperand`` when this schema is a ``Raw`` pass-through, and
        ``UnsupportedResult`` when the canonical form does not support the negation.
        """
        ...

    def definition(self, uri: str) -> CanonicalSchema | None:
        """The reference target registered under ``uri``, or the document itself under ``#``.

        A ``#`` inside the target names the document the target was written in.
        """
        ...

    def definitions(self) -> dict[str, CanonicalSchema]:
        """Map of reference URI -> canonical target for every known target reachable from this schema."""
        ...

    def to_json_schema(self) -> JsonValue:
        """Convert this canonical schema back to a plain Python JSON value.

        A node below the document root carries that root along, so a ``#`` inside it keeps naming
        the document.
        """
        ...

    def satisfiability(self) -> canonical.Satisfiability:
        """Whether any value satisfies this schema.

        Only ``NO`` proves the schema accepts nothing; ``UNKNOWN`` means undecided, not empty. Test
        for ``NO``, and treat ``UNKNOWN`` like ``YES``.
        """
        ...

    def __hash__(self) -> int: ...
    def __eq__(self, other: object, /) -> bool: ...
    def __repr__(self) -> str: ...

def canonicalize(
    schema: _Schema,
    /,
    *,
    draft: int | None = None,
    validate_formats: bool | None = None,
    pattern_options: PatternOptionsType | None = None,
    retriever: RetrieverProtocol | None = None,
    registry: Registry | None = None,
    base_uri: str | None = None,
    offline: bool | None = None,
) -> CanonicalSchema:
    """Parse and normalize a JSON Schema to its canonical form.

    Returns a :class:`CanonicalSchema` that accepts the same values as ``schema``. Raises
    :class:`ValidationError` when the schema fails meta-schema validation and
    :class:`canonical.CanonicalizationError` when the root is not a boolean or an object.
    """
    ...

__all__ = [
    "ReferencingError",
    "ValidationError",
    "CanonicalSchema",
    "canonical",
    "canonicalize",
    "ValidationErrorKind",
    "Evaluation",
    "bundle",
    "dereference",
    "is_valid",
    "validate",
    "iter_errors",
    "evaluate",
    "validator_cls_for",
    "validator_for",
    "validator_map_for",
    "ValidatorMap",
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
    "Validator",
    "Registry",
    "Resolver",
    "Resolved",
    "EmailOptions",
    "FancyRegexOptions",
    "HttpOptions",
    "RegexOptions",
    "meta",
]

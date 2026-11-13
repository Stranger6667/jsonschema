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
    def flag(self) -> FlagOutput: ...
    def list(self) -> ListOutput: ...
    def hierarchical(self) -> OutputUnit: ...
    def annotations(self) -> List[EvaluationAnnotation]: ...
    def errors(self) -> List[EvaluationErrorEntry]: ...
    def __repr__(self) -> str: ...

@final
class FancyRegexOptions:
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
    def __new__(
        cls,
        size_limit: int | None = None,
        dfa_size_limit: int | None = None,
        syntax: Literal["ecma262", "engine"] = "ecma262",
    ) -> Self: ...
    def __repr__(self) -> str: ...

@final
class EmailOptions:
    """Configuration for email format validation."""

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
    """Configuration for HTTP client used in schema retrieval."""

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
    """Check if a JSON instance is valid against a schema.

    Returns True if valid, False otherwise. Stops at the first error.
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
    """Validate a JSON instance against a schema.

    Raises ValidationError if invalid. Stops at the first error.
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
    """Iterate over all validation errors.

    Returns an iterator of ValidationError objects for all errors found.
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
    """Evaluate an instance and return structured output.

    Returns an Evaluation object with flag(), list(), and hierarchical() output formats.
    """
    ...

class ReferencingError(Exception):
    message: str
    def __init__(self, message: str) -> None: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...

class ValidationErrorKind:
    @property
    def name(self) -> str:
        """The JSON Schema keyword that triggered this error."""
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
    class RegexEngineFailure(ValidationErrorKind):
        __match_args__ = ("message",)
        message: str

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
    def is_valid(self, instance: Any) -> bool: ...
    def validate(self, instance: Any) -> None: ...
    def iter_errors(self, instance: Any) -> Iterator[ValidationError]: ...
    def evaluate(self, instance: Any) -> Evaluation: ...
    def __repr__(self) -> str: ...

class Draft4Validator(Validator): ...
class Draft6Validator(Validator): ...
class Draft7Validator(Validator): ...
class Draft201909Validator(Validator): ...
class Draft202012Validator(Validator): ...

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
    """Create a validator for the given schema.

    Automatically detects the JSON Schema draft from the $schema keyword.
    Returns a Draft-specific validator instance.
    """
    ...

@final
class ValidatorMap:
    """Map of compiled validators keyed by URI-fragment JSON pointer.

    Obtained via :func:`validator_map_for`. Use it to validate instances against
    named subschemas identified by JSON pointer (e.g. ``"#/$defs/User"``).
    """

    def get(self, pointer: str) -> Validator | None:
        """Return the validator for *pointer*, or ``None`` if not found."""
        ...

    def __getitem__(self, pointer: str, /) -> Validator:
        """Return the validator for *pointer*, raising ``KeyError`` if not found."""
        ...

    def __contains__(self, pointer: object, /) -> bool: ...
    def keys(self) -> list[str]: ...
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
    """Compile all subschemas in *schema* into a map keyed by URI-fragment JSON pointer.

    The root schema is always included as ``"#"``.
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
    """Recursively inline all $ref values in a JSON Schema.

    Circular references are left in place as `$ref` strings.

    Raises ReferencingError if any $ref cannot be resolved.
    """
    ...

def validator_cls_for(schema: _Schema | str) -> type[Validator]:
    """Detect the JSON Schema draft for a schema and return the corresponding validator class.

    Draft is detected automatically from the $schema field. Defaults to Draft202012Validator.
    """
    ...

@final
class Registry:
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

@final
class Resolved:
    @property
    def contents(self) -> JsonValue: ...
    @property
    def resolver(self) -> Resolver: ...
    @property
    def draft(self) -> int: ...

class _Meta:
    def is_valid(self, schema: _Schema, registry: Registry | None = None) -> bool: ...
    def validate(self, schema: _Schema, registry: Registry | None = None) -> None: ...

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
        """Every value both schemas admit."""
        ...

    def union(self, other: CanonicalSchema) -> CanonicalSchema:
        """Every value either schema admits."""
        ...

    def subtract(self, other: CanonicalSchema) -> CanonicalSchema:
        """Every value this schema admits and ``other`` rejects.

        ``old.subtract(new)`` admits exactly the values ``old`` took and ``new`` turns away, and is
        unsatisfiable iff ``new`` still takes everything ``old`` did.
        """
        ...

    def covers(self, other: CanonicalSchema) -> canonical.Containment:
        """Whether this schema admits every value ``other`` admits.

        Only ``YES`` proves containment. Test for ``YES``, and treat ``UNKNOWN`` like ``NO``.
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

        The target keeps the document it was written in, so a ``#`` inside it names that document
        and not the target standing in for it.
        """
        ...

    def definitions(self) -> dict[str, CanonicalSchema]:
        """Map of reference uri -> canonical target."""
        ...

    def to_json_schema(self) -> JsonValue:
        """Convert this canonical schema back to a plain Python JSON value.

        A node below the document root carries that root along, so a ``#`` inside it keeps naming
        the document.
        """
        ...

    def satisfiability(self) -> canonical.Satisfiability:
        """Whether any value satisfies this schema.

        Only ``NO`` proves the schema admits nothing. Test for ``NO``, and treat ``UNKNOWN``
        like ``YES``.
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

    Raises :class:`ValidationError` when the schema fails meta-schema validation and
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

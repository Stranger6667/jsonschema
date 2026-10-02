# jsonschema-rs

[![Build](https://img.shields.io/github/actions/workflow/status/Stranger6667/jsonschema/ci.yml?branch=master&style=flat-square)](https://github.com/Stranger6667/jsonschema/actions)
[![Version](https://img.shields.io/pypi/v/jsonschema-rs.svg?style=flat-square)](https://pypi.org/project/jsonschema-rs/)
[![Python versions](https://img.shields.io/pypi/pyversions/jsonschema-rs.svg?style=flat-square)](https://pypi.org/project/jsonschema-rs/)
[![License](https://img.shields.io/pypi/l/jsonschema-rs.svg?style=flat-square)](https://opensource.org/licenses/MIT)
[<img alt="Supported Dialects" src="https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fsupported_versions.json&style=flat-square">](https://bowtie.report/#/implementations/rust-jsonschema)

A high-performance JSON Schema validator for Python.

```python
import jsonschema_rs

schema = {"maxLength": 5}
instance = "foo"

# One-off validation
try:
    jsonschema_rs.validate(schema, "incorrect")
except jsonschema_rs.ValidationError as exc:
    assert str(exc) == '''"incorrect" is longer than 5 characters

Failed validating "maxLength" in schema

On instance:
    "incorrect"'''

# Build & reuse (faster)
validator = jsonschema_rs.validator_for(schema)

# Iterate over errors
for error in validator.iter_errors(instance):
    print(f"Error: {error}")
    print(f"Location: {error.instance_path}")

# Boolean result
assert validator.is_valid(instance)

# Structured output (JSON Schema Output v1)
evaluation = validator.evaluate(instance)
for error in evaluation.errors():
    print(f"Error at {error['instanceLocation']}: {error['error']}")
```

> ⚠️ **Upgrading from older versions?** See the [Migration Guide](https://github.com/Stranger6667/jsonschema/blob/master/crates/jsonschema-py/MIGRATION.md) for breaking changes.

> **Migrating from `jsonschema`?** See the [jsonschema migration guide](https://github.com/Stranger6667/jsonschema/blob/master/crates/jsonschema-py/MIGRATION_FROM_JSONSCHEMA.md).

## Highlights

- 📚 Drafts 4, 6, 7, 2019-09 and 2020-12
- 🔧 Custom keywords and format validators
- ⚡ Compile-time validators for your own extension modules, via the Rust crate
- 🌐 `$ref` resolution over HTTP and from files
- 📦 Schema bundling into Compound Schema Documents, and `$ref` dereferencing
- 🎨 Structured Output v1 reports (flag/list/hierarchical)
- ✨ Meta-schema validation for schema documents, including custom metaschemas
- 🧮 Experimental schema canonicalization

### Supported drafts

- [![Draft 2020-12](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft2020-12.json)](https://bowtie.report/#/implementations/rust-jsonschema)
- [![Draft 2019-09](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft2019-09.json)](https://bowtie.report/#/implementations/rust-jsonschema)
- [![Draft 7](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft7.json)](https://bowtie.report/#/implementations/rust-jsonschema)
- [![Draft 6](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft6.json)](https://bowtie.report/#/implementations/rust-jsonschema)
- [![Draft 4](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft4.json)](https://bowtie.report/#/implementations/rust-jsonschema)

Per-draft compliance results are on the [Bowtie Report](https://bowtie.report/#/implementations/rust-jsonschema).

## Playground

Try schemas in the browser with the WebAssembly [playground](https://jsonschema.dygalo.dev/).

## Installation

```bash
pip install jsonschema-rs
```

## Usage

Pass a schema as a JSON string to skip parsing it in Python:

```python
import jsonschema_rs

validator = jsonschema_rs.validator_for('{"minimum": 42}')
...
```

`validator_for` detects the draft from `$schema`. To pick one yourself, use a draft-specific class:

```python
import jsonschema_rs

# Automatic draft detection
validator = jsonschema_rs.validator_for({"minimum": 42})

# Draft-specific validators
validator = jsonschema_rs.Draft4Validator({"minimum": 42})
validator = jsonschema_rs.Draft6Validator({"minimum": 42})
validator = jsonschema_rs.Draft7Validator({"minimum": 42})
validator = jsonschema_rs.Draft201909Validator({"minimum": 42})
validator = jsonschema_rs.Draft202012Validator({"minimum": 42})
```

`jsonschema-rs` ships validators for the standard `format` values. To add your own, pass a function that takes a `str` and returns a `bool` via `formats`.
Drafts 2019-09 and 2020-12 treat `format` as an annotation, so set `validate_formats=True` to get it checked:

```python
import jsonschema_rs

def is_currency(value):
    # The input value is always a string
    return len(value) == 3 and value.isascii()


validator = jsonschema_rs.validator_for(
    {"type": "string", "format": "currency"}, 
    formats={"currency": is_currency},
    validate_formats=True  # Important for Draft 2019-09 and 2020-12
)
validator.is_valid("USD")  # True
validator.is_valid("invalid")  # False
```

### Custom Keywords

A custom keyword is a class. The validator builds one instance per occurrence of the keyword, passing the parent schema, the keyword value and its schema path.
Its `validate` method rejects a value by raising any exception, whose message becomes the error message:

```python
import jsonschema_rs

class DivisibleBy:
    def __init__(self, parent_schema, value, schema_path):
        self.divisor = value

    def validate(self, instance):
        if isinstance(instance, int) and instance % self.divisor != 0:
            raise ValueError(f"{instance} is not divisible by {self.divisor}")


validator = jsonschema_rs.validator_for(
    {"type": "integer", "divisibleBy": 3},
    keywords={"divisibleBy": DivisibleBy},
)
validator.is_valid(9)   # True
validator.is_valid(10)  # False
```

The resulting `ValidationError` keeps that exception as `__cause__`:

```python
try:
    validator.validate(instance)
except jsonschema_rs.ValidationError as e:
    print(type(e.__cause__))   # <class 'ValueError'>
    print(e.__cause__)         # original message
```

Other options:

- `validate_formats`: check `format` regardless of the draft default.
- `ignore_unknown_formats`: set to `False` to raise on a `format` value with no validator.
- `base_uri`: base URI for relative `$ref`s in the schema.
- `vocabularies`: vocabularies your custom keywords implement.

```python
import jsonschema_rs

validator = jsonschema_rs.Draft202012Validator(
    {"type": "string", "format": "date"},
    validate_formats=True,
    ignore_unknown_formats=False
)

# This will validate the "date" format
validator.is_valid("2023-05-17")  # True
validator.is_valid("not a date")  # False

# With ignore_unknown_formats=False, an unknown format raises an error
invalid_schema = {"type": "string", "format": "unknown"}
try:
    jsonschema_rs.Draft202012Validator(
        invalid_schema, validate_formats=True, ignore_unknown_formats=False
    )
except jsonschema_rs.ValidationError as exc:
    assert exc.message == (
        "Unknown format: 'unknown'. "
        "Adjust configuration to ignore unrecognized formats"
    )
```

### Structured Output with `evaluate`

`evaluate` returns the JSON Schema Output v1 formats instead of a boolean:

```python
import jsonschema_rs

schema = {
    "type": "array",
    "prefixItems": [{"type": "string"}],
    "items": {"type": "integer"},
}
evaluation = jsonschema_rs.evaluate(schema, ["hello", "oops"])
type_error = {"type": '"oops" is not of type "integer"'}

assert evaluation.flag() == {"valid": False}
assert evaluation.list() == {
    "valid": False,
    "details": [
        {
            "evaluationPath": "",
            "instanceLocation": "",
            "schemaLocation": "",
            "valid": False,
        },
        {
            "valid": True,
            "evaluationPath": "/type",
            "instanceLocation": "",
            "schemaLocation": "/type",
        },
        {
            "valid": False,
            "evaluationPath": "/items",
            "instanceLocation": "",
            "schemaLocation": "/items",
            "droppedAnnotations": True,
        },
        {
            "valid": False,
            "evaluationPath": "/items",
            "instanceLocation": "/1",
            "schemaLocation": "/items",
        },
        {
            "valid": False,
            "evaluationPath": "/items/type",
            "instanceLocation": "/1",
            "schemaLocation": "/items/type",
            "errors": type_error,
        },
        {
            "valid": True,
            "evaluationPath": "/prefixItems",
            "instanceLocation": "",
            "schemaLocation": "/prefixItems",
            "annotations": 0,
        },
        {
            "valid": True,
            "evaluationPath": "/prefixItems/0",
            "instanceLocation": "/0",
            "schemaLocation": "/prefixItems/0",
        },
        {
            "valid": True,
            "evaluationPath": "/prefixItems/0/type",
            "instanceLocation": "/0",
            "schemaLocation": "/prefixItems/0/type",
        },
    ],
}

hierarchical = evaluation.hierarchical()
assert hierarchical == {
    "valid": False,
    "evaluationPath": "",
    "instanceLocation": "",
    "schemaLocation": "",
    "details": [
        {
            "valid": True,
            "evaluationPath": "/type",
            "instanceLocation": "",
            "schemaLocation": "/type",
        },
        {
            "valid": False,
            "evaluationPath": "/items",
            "instanceLocation": "",
            "schemaLocation": "/items",
            "droppedAnnotations": True,
            "details": [
                {
                    "valid": False,
                    "evaluationPath": "/items",
                    "instanceLocation": "/1",
                    "schemaLocation": "/items",
                    "details": [
                        {
                            "valid": False,
                            "evaluationPath": "/items/type",
                            "instanceLocation": "/1",
                            "schemaLocation": "/items/type",
                            "errors": type_error,
                        }
                    ],
                }
            ],
        },
        {
            "valid": True,
            "evaluationPath": "/prefixItems",
            "instanceLocation": "",
            "schemaLocation": "/prefixItems",
            "annotations": 0,
            "details": [
                {
                    "valid": True,
                    "evaluationPath": "/prefixItems/0",
                    "instanceLocation": "/0",
                    "schemaLocation": "/prefixItems/0",
                    "details": [
                        {
                            "valid": True,
                            "evaluationPath": "/prefixItems/0/type",
                            "instanceLocation": "/0",
                            "schemaLocation": "/prefixItems/0/type",
                        }
                    ],
                }
            ],
        },
    ],
}

assert evaluation.errors() == [
    {
        "schemaLocation": "/items/type",
        "absoluteKeywordLocation": None,
        "instanceLocation": "/1",
        "error": '"oops" is not of type "integer"',
    }
]

# A failing schema produces no annotations
assert evaluation.annotations() == []
```

### Arbitrary-Precision Numbers

Numbers keep their full precision on the way to Python:

- Integers, regardless of size, are returned as regular `int` objects.
- Floating-point literals that fit into IEEE-754 become Python `float`s.
- Floating-point literals that don't fit in `float` (for example `1e10000` or extremely precise
  decimals) fall back to [`decimal.Decimal`](https://docs.python.org/3/library/decimal.html) using
  their original JSON string representation.

So `ValidationError.kind` attributes can hold `Decimal` values:

```python
from decimal import Decimal
from jsonschema_rs import ValidationError, validator_for

validator = validator_for('{"const": 1e10000}')
try:
    validator.validate(0)
except ValidationError as exc:
    assert exc.kind.expected_value == Decimal("1e10000")

# Exponents beyond ~10^1_000_000 are clamped to keep parsing predictable
```

## Schema Bundling and Dereferencing

Produce a Compound Schema Document ([Appendix B](https://json-schema.org/draft/2020-12/json-schema-core#appendix-B)) by embedding all external `$ref` targets into a draft-appropriate container. The bundle accepts the same values as the original.

```python
import jsonschema_rs

address_schema = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "$id": "https://example.com/address.json",
    "type": "object",
    "properties": {"street": {"type": "string"}, "city": {"type": "string"}},
    "required": ["street", "city"]
}

schema = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "properties": {"home": {"$ref": "https://example.com/address.json"}},
    "required": ["home"]
}

registry = jsonschema_rs.Registry(
    [("https://example.com/address.json", address_schema)]
)
bundled = jsonschema_rs.bundle(schema, registry=registry)
```

`dereference` instead replaces each `$ref` with the schema it points to, for consumers that do not resolve references. Circular references are left in place.

```python
dereferenced = jsonschema_rs.dereference(schema, registry=registry)
```

## Schema Canonicalization

> **Experimental**: the canonicalization API may change in minor releases.

`canonicalize` reduces a schema to a normal form that accepts the same values. Schemas that accept the same values reduce to equal `CanonicalSchema` objects, and a schema proven to accept nothing reduces to `false`:

```python
import jsonschema_rs

canonical = jsonschema_rs.canonicalize({
    "allOf": [
        {"type": "integer", "minimum": 0},
        {"minimum": 10, "maximum": 100},
    ]
})
assert canonical.to_json_schema() == {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "integer", "minimum": 10, "maximum": 100,
}

# However they were written, equivalent schemas compare equal
same = jsonschema_rs.canonicalize(
    {"type": "integer", "maximum": 100, "minimum": 10}
)
assert canonical == same

# A schema no value can satisfy collapses
from jsonschema_rs.canonical import Satisfiability

empty = jsonschema_rs.canonicalize(
    {"type": "integer", "minimum": 10, "maximum": 5}
)
assert empty.satisfiability() == Satisfiability.NO
```

A schema the canonical form cannot model exactly comes back unchanged, with `kind == CanonicalKind.RAW`. Its `view().reason` names what stopped the run, and `view().pointer` the subschema at fault when a single one is.

Canonical schemas combine like sets of values. Every emitted schema carries `$schema`; the comments below leave it out:

```python
positive = jsonschema_rs.canonicalize({"type": "integer", "minimum": 0})
bounded = jsonschema_rs.canonicalize({"type": "integer", "maximum": 100})

# Every value both admit
positive.intersect(bounded).to_json_schema()
# {"type": "integer", "minimum": 0, "maximum": 100}

# Every value either admits
positive.union(bounded).to_json_schema()
# {"type": "integer"}

# Every value `positive` admits and `bounded` rejects
positive.subtract(bounded).to_json_schema()
# {"type": "integer", "minimum": 101}

# Every value `positive` rejects: other types, negative integers
# and non-integer numbers
positive.negate().to_json_schema()
# {"anyOf": [{"type": ["null", "boolean", "string", "array", "object"]},
#            {"type": "integer", "maximum": -1},
#            {"type": "number", "not": {"multipleOf": 1}}]}

# Containment.NO: `bounded` takes negative integers, `positive` does not
positive.covers(bounded)
positive.satisfiability()  # Satisfiability.YES
```

### Comparing two versions of a schema

`subtract` tells you what an edit did to a schema. `old.subtract(new)` accepts exactly the values `old` accepts and `new` rejects, so it accepts nothing when the edit lost nothing.

```python
old = jsonschema_rs.canonicalize({"type": "string"})
new = jsonschema_rs.canonicalize({"type": "string", "maxLength": 50})

# What `new` stopped accepting, as a schema
assert old.subtract(new).to_json_schema() == {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "string", "minLength": 51,
}
# Nothing is accepted that was not accepted before, so the change only narrows
assert new.subtract(old).satisfiability() == Satisfiability.NO
```

The direction to check depends on who sends the value:

- For a request schema, check `old.subtract(new)`. It accepts the payloads existing callers send that the new schema rejects.
- For a response schema, check `new.subtract(old)`. It accepts the values the new schema lets a server return that callers never agreed to read.

`Satisfiability.NO` on the difference proves the edit safe in that direction. `UNKNOWN` means the canonicalizer could not decide, and proves nothing either way. Read it as the answer that keeps you safe:

- `satisfiability()`: only `Satisfiability.NO` proves a schema accepts nothing. Treat `UNKNOWN` like `YES`.
- `a.covers(b)`: only `Containment.YES` proves `a` accepts every value `b` accepts. Treat `UNKNOWN` like `NO`.

`Containment`, `Satisfiability`, `Distinctness`, `CanonicalKind`, `UnsatisfiableReason`, `Cause` and the exceptions below live in `jsonschema_rs.canonical`.

The set operations raise `IncompatibleOperands` when the operands differ in draft, `format` assertion or regular-expression engine, or resolve `#` or one external resource to different schemas. They raise `UnsupportedOperand` when an operand is `RAW`, and `UnsupportedResult` when the canonical form cannot express the result exactly. All three subclass `CanonicalizationError`.

### Finding the dead subschemas of a document

`find_unsatisfiable` walks a whole document and reports each subschema that accepts no value, with the keywords at fault and their pointers:

```python
from jsonschema_rs.canonical import UnsatisfiableReason, find_unsatisfiable

reasons = find_unsatisfiable({
    "properties": {
        "tag": {"type": "string", "minLength": 5, "maxLength": 2},
        "name": {"type": "string"},
    }
})

match reasons["/properties/tag"]:
    case UnsatisfiableReason.Conflict(causes):
        assert [(cause.pointer, cause.keywords) for cause in causes] == [
            ("/properties/tag", ["type"]),
            ("/properties/tag", ["minLength", "maxLength"]),
        ]

# A live subschema is not reported
assert "/properties/name" not in reasons
```

A reason is `Literal` (written as `false`), `Empty` (one part admits nothing by itself) or `Conflict` (each part admits values, together they admit none). A missing pointer does not prove that subschema satisfiable. For a document the canonical form cannot model, `find_unsatisfiable` reports nothing, the same way `satisfiability()` answers `UNKNOWN`.

## Meta-Schema Validation

`jsonschema_rs.meta` checks a schema against the meta-schema of its draft:

```python
import jsonschema_rs

# Valid schema
schema = {
    "type": "object",
    "properties": {
        "name": {"type": "string"},
        "age": {"type": "integer", "minimum": 0}
    },
    "required": ["name"]
}

# Validate schema (draft is auto-detected)
assert jsonschema_rs.meta.is_valid(schema)
jsonschema_rs.meta.validate(schema)  # No error raised

# Invalid schema
invalid_schema = {
    "minimum": "not_a_number"  # "minimum" must be a number
}

try:
    jsonschema_rs.meta.validate(invalid_schema)
except jsonschema_rs.ValidationError as exc:
    assert 'is not of type "number"' in str(exc)
```

## Regular Expression Configuration

`pattern_options` picks the regex engine for `pattern` and `patternProperties` and sets its limits:

```python
import jsonschema_rs
from jsonschema_rs import FancyRegexOptions, RegexOptions

# Default fancy-regex engine with backtracking limits
# (supports advanced features but needs protection against DoS)
validator = jsonschema_rs.validator_for(
    {"type": "string", "pattern": "^(a+)+$"},
    pattern_options=FancyRegexOptions(backtrack_limit=10_000)
)

# Standard regex engine for guaranteed linear-time matching
# (prevents regex DoS attacks but supports fewer features)
validator = jsonschema_rs.validator_for(
    {"type": "string", "pattern": "^a+$"},
    pattern_options=RegexOptions()
)

# Both engines support memory usage configuration
validator = jsonschema_rs.validator_for(
    {"type": "string", "pattern": "^a+$"},
    pattern_options=RegexOptions(
        size_limit=1024 * 1024,   # Maximum compiled pattern size
        dfa_size_limit=10240      # Maximum DFA cache size
    )
)
```

  - `FancyRegexOptions`: default engine, supports lookaround and backreferences

    - `backtrack_limit`: Maximum backtracking steps
    - `size_limit`: Maximum compiled regex size in bytes
    - `dfa_size_limit`: Maximum DFA cache size in bytes

  - `RegexOptions`: matches in linear time, no lookaround or backreferences

    - `size_limit`: Maximum compiled regex size in bytes
    - `dfa_size_limit`: Maximum DFA cache size in bytes

If you validate against schemas from untrusted sources, use `RegexOptions`: a crafted pattern cannot make it backtrack.

## Email Format Configuration

`email_options` makes `{"format": "email"}` stricter or looser than the spec default:

```python
import jsonschema_rs
from jsonschema_rs import EmailOptions

# Require a top-level domain (reject "user@localhost")
validator = jsonschema_rs.validator_for(
    {"format": "email", "type": "string"},
    validate_formats=True,
    email_options=EmailOptions(require_tld=True)
)
validator.is_valid("user@localhost")     # False
validator.is_valid("user@example.com")   # True

# Disallow IP address literals and display names
validator = jsonschema_rs.validator_for(
    {"format": "email", "type": "string"},
    validate_formats=True,
    email_options=EmailOptions(
        allow_domain_literal=False,  # Reject "user@[127.0.0.1]"
        allow_display_text=False     # Reject "Name <user@example.com>"
    )
)

# Require at least 3 domain segments, e.g. user@sub.example.com
validator = jsonschema_rs.validator_for(
    {"format": "email", "type": "string"},
    validate_formats=True,
    email_options=EmailOptions(minimum_sub_domains=3),
)
```

  - `require_tld`: Require a top-level domain (e.g., reject "user@localhost") (default: False)
  - `allow_domain_literal`: Allow IP address literals like "user@[127.0.0.1]" (default: True)
  - `allow_display_text`: Allow display names like "Name <user@example.com>" (default: True)
  - `minimum_sub_domains`: Minimum number of domain segments required

## External References

By default, `jsonschema-rs` fetches external `$ref` targets over HTTP and from the local file system. Pass a `retriever` to load them yourself. This one serves schemas from a dict:

```python
import jsonschema_rs

def retrieve(uri: str):
    schemas = {
        "https://example.com/person.json": {
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"}
            },
            "required": ["name", "age"]
        }
    }
    if uri not in schemas:
        raise KeyError(f"Schema not found: {uri}")
    return schemas[uri]

schema = {
    "$ref": "https://example.com/person.json"
}

validator = jsonschema_rs.validator_for(schema, retriever=retrieve)

# This is valid
validator.is_valid({
    "name": "Alice",
    "age": 30
})

# This is invalid (missing "age")
validator.is_valid({
    "name": "Bob"
})  # False
```

For schemas from untrusted sources, pass `offline=True`. The validator then refuses to fetch any `$ref` target, so a schema cannot reach your network or file system. Schemas held in a `Registry` still resolve:

```python
try:
    jsonschema_rs.validator_for(
        {"$ref": "https://example.com/other.json"}, offline=True
    )
except jsonschema_rs.ValidationError as exc:
    assert "Retrieval is disabled" in str(exc)
```

`bundle`, `dereference` and `canonicalize` take `offline` too.

## Schema Registry

A `Registry` holds schemas by URI, so a validator resolves `$ref`s to them without fetching anything:

```python
import jsonschema_rs

# Create a registry with schemas
registry = jsonschema_rs.Registry([
    ("https://example.com/address.json", {
        "type": "object",
        "properties": {
            "street": {"type": "string"},
            "city": {"type": "string"}
        }
    }),
    ("https://example.com/person.json", {
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "address": {"$ref": "https://example.com/address.json"}
        }
    })
])

# Use the registry with any validator
validator = jsonschema_rs.validator_for(
    {"$ref": "https://example.com/person.json"},
    registry=registry
)

# Validate instances
assert validator.is_valid({
    "name": "John",
    "address": {"street": "Main St", "city": "Boston"}
})
```

`Registry` also takes a default draft and a `retriever` for URIs it does not hold:

```python
import jsonschema_rs

registry = jsonschema_rs.Registry(
    resources=[
        (
            "https://example.com/address.json",
            {}
        )
    ],  # Your schemas
    draft=jsonschema_rs.Draft202012,  # Optional
    retriever=lambda uri: {}  # Optional
)
```

## Error Handling

A `ValidationError` carries the message, both locations and a `kind` with keyword-specific details:

```python
import jsonschema_rs

schema = {"type": "string", "maxLength": 5}

try:
    jsonschema_rs.validate(schema, "too long")
except jsonschema_rs.ValidationError as error:
    # Basic error information
    print(error.message)       # '"too long" is longer than 5 characters'
    print(error.instance_path) # Location in the instance that failed
    print(error.schema_path)   # Location in the schema that failed

    # Detailed error information via `kind`
    if isinstance(error.kind, jsonschema_rs.ValidationErrorKind.MaxLength):
        assert error.kind.limit == 5
        print(f"Exceeded maximum length of {error.kind.limit}")
```

The [type stubs](https://github.com/Stranger6667/jsonschema/blob/master/crates/jsonschema-py/python/jsonschema_rs/__init__.pyi) list every error kind and its attributes.

### Error Kind Properties

`kind` also has generic accessors:

```python
for error in jsonschema_rs.iter_errors({"minimum": 5}, 3):
    print(error.kind.name)      # "minimum"
    print(error.kind.value)     # 5
    print(error.kind.as_dict()) # {"limit": 5}
```

Each kind is a class you can `match` on:

```python
for error in jsonschema_rs.iter_errors({"minimum": 5}, 3):
    match error.kind:
        case jsonschema_rs.ValidationErrorKind.Minimum(limit=limit):
            print(f"Value below {limit}")
        case jsonschema_rs.ValidationErrorKind.Type(types=types):
            print(f"Expected one of {types}")
```

### Error Message Masking

Pass `mask` to replace instance values in error messages with a placeholder:

```python
import jsonschema_rs

schema = {
    "type": "object",
    "properties": {
        "password": {"type": "string", "minLength": 8},
        "api_key": {"type": "string", "pattern": "^[A-Z0-9]{32}$"}
    }
}

# Replace instance values with "[REDACTED]"
validator = jsonschema_rs.validator_for(schema, mask="[REDACTED]")

try:
    validator.validate({
        "password": "123",
        "api_key": "secret_key_123"
    })
except jsonschema_rs.ValidationError as exc:
    assert str(exc) == '''[REDACTED] is shorter than 8 characters

Failed validating "minLength" in schema["properties"]["password"]

On instance["password"]:
    [REDACTED]'''
```

## Performance

Compared with other Python validators:

- **138-10,841x** faster than `jsonschema` for complex schemas and large instances
- **8-1,848x** faster than `fastjsonschema` on CPython

Full results and methodology are in [BENCHMARKS.md](https://github.com/Stranger6667/jsonschema/blob/master/crates/jsonschema-py/BENCHMARKS.md).

### Compile-Time Validators

If you ship your own extension module and know the schema at build time, the Rust crate's
`#[jsonschema::validator(..., backend = Pyo3)]` macro compiles it into a validator that reads
Python objects directly, so nothing is parsed or compiled when your module is imported, and validation
runs up to 4.8x faster than with a validator built at run time. See the
[macro documentation](https://docs.rs/jsonschema/latest/jsonschema/#python-extension-modules).

This is not available from the `jsonschema-rs` package on PyPI, which takes its schemas at run time.
A complete extension with its build and test commands lives in
[`examples/pyo3-extension`](https://github.com/Stranger6667/jsonschema/tree/master/examples/pyo3-extension).

## Python support

`jsonschema-rs` supports CPython 3.10 through 3.14 and PyPy 3.10+.

Pre-built wheels are available for:

- **Linux**: `x86_64`, `i686`, `aarch64` (glibc and musl)
- **macOS**: `x86_64`, `aarch64`, `universal2`
- **Windows**: `x64`, `x86`, `arm64`

## Troubleshooting

If a source build on Linux fails with linking errors (e.g., undefined symbols from `ring`), use the `mold` linker:

```bash
RUSTFLAGS="-C link-arg=-fuse-ld=mold" \
  pip install jsonschema-rs --no-binary :all:
```

## Acknowledgements

The API follows the Python [`jsonschema`](https://github.com/python-jsonschema/jsonschema) package. Thanks to its maintainers and contributors.

## Support

Ask questions and suggest improvements in [GitHub Discussions](https://github.com/Stranger6667/jsonschema/discussions).

## Sponsorship

If you find `jsonschema-rs` useful, please consider [sponsoring its development](https://github.com/sponsors/Stranger6667).

## Contributing

Ways to help:

- Share your use cases
- Implement missing keywords
- Fix failing test cases from the [JSON Schema test suite](https://bowtie.report/#/implementations/rust-jsonschema)

See [CONTRIBUTING.md](https://github.com/Stranger6667/jsonschema/blob/master/CONTRIBUTING.md) for more details.

## License

Licensed under [MIT License](https://github.com/Stranger6667/jsonschema/blob/master/LICENSE).

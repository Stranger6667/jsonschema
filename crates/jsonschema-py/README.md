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
```

> ⚠️ **Upgrading from older versions?** Check our [Migration Guide](https://github.com/Stranger6667/jsonschema/blob/master/crates/jsonschema-py/MIGRATION.md) for key changes.

## Highlights

- 📚 Full support for popular JSON Schema drafts
- 🌐 Remote reference fetching (network/file)
- 🔧 Custom format validators
- ✨ Meta-schema validation for schema documents

### Supported drafts

The following drafts are supported:

- [![Draft 2020-12](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft2020-12.json)](https://bowtie.report/#/implementations/rust-jsonschema)
- [![Draft 2019-09](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft2019-09.json)](https://bowtie.report/#/implementations/rust-jsonschema)
- [![Draft 7](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft7.json)](https://bowtie.report/#/implementations/rust-jsonschema)
- [![Draft 6](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft6.json)](https://bowtie.report/#/implementations/rust-jsonschema)
- [![Draft 4](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft4.json)](https://bowtie.report/#/implementations/rust-jsonschema)

You can check the current status on the [Bowtie Report](https://bowtie.report/#/implementations/rust-jsonschema).

## Installation

To install `jsonschema-rs` via `pip` run the following command:

```bash
pip install jsonschema-rs
```

## Usage

If you have a schema as a JSON string, then you could pass it to `validator_for`
to avoid parsing on the Python side:

```python
import jsonschema_rs

validator = jsonschema_rs.validator_for('{"minimum": 42}')
...
```

You can use draft-specific validators for different JSON Schema versions:

```python
import jsonschema_rs

# Automatic draft detection
validator = jsonschema_rs.validator_for({"minimum": 42})

# Draft-specific validators
validator = jsonschema_rs.Draft7Validator({"minimum": 42})
validator = jsonschema_rs.Draft201909Validator({"minimum": 42})
validator = jsonschema_rs.Draft202012Validator({"minimum": 42})
```

JSON Schema allows for format validation through the `format` keyword. While `jsonschema-rs`
provides built-in validators for standard formats, you can also define custom format validators
for domain-specific string formats.

To implement a custom format validator:

1. Define a function that takes a `str` and returns a `bool`.
2. Pass it with the `formats` argument.
3. Ensure validate_formats is set appropriately (especially for Draft 2019-09 and 2020-12).

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

Additional configuration options are available for fine-tuning the validation process:

- `validate_formats`: Override the draft-specific default behavior for format validation.
- `ignore_unknown_formats`: Control whether unrecognized formats should be reported as errors.
- `base_uri` - a base URI for all relative `$ref` in the schema.

Example usage of these options:

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

# With ignore_unknown_formats=False, using an unknown format will raise an error
invalid_schema = {"type": "string", "format": "unknown"}
try:
    jsonschema_rs.Draft202012Validator(
        invalid_schema, validate_formats=True, ignore_unknown_formats=False
    )
except jsonschema_rs.ValidationError as exc:
    assert str(exc) == '''Unknown format: 'unknown'. Adjust configuration to ignore unrecognized formats

Failed validating "format" in schema

On instance:
    "unknown"'''
```

### Arbitrary-Precision Numbers

The Python bindings always include the `arbitrary-precision` support from the Rust validator, so numeric
values are exposed to Python using the most accurate type available:

- Integers, regardless of size, are returned as regular `int` objects.
- Floating-point literals that fit into IEEE-754 become Python `float`s.
- Floating-point literals that don't fit in `float` (for example `1e10000` or extremely precise
  decimals) fall back to [`decimal.Decimal`](https://docs.python.org/3/library/decimal.html) using
  their original JSON string representation.

This means `ValidationError.kind` attributes may contain `Decimal` instances for very large numbers.
Import `Decimal` from the standard library if you need to compare against or serialize those
values exactly:

```python
from decimal import Decimal
from jsonschema_rs import ValidationError, validator_for

validator = validator_for('{"const": 1e10000}')
try:
    validator.validate(0)
except ValidationError as exc:
    assert exc.kind.expected_value == Decimal("1e10000")

# Extremely large exponents (beyond ~10^1_000_000) are clamped internally to keep parsing
# predictable, matching the Rust implementation's guardrails.
```

## Meta-Schema Validation

JSON Schema documents can be validated against their meta-schemas to ensure they are valid schemas. `jsonschema-rs` provides this functionality through the `meta` module:

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

When validating schemas with regex patterns (in `pattern` or `patternProperties`), you can configure the underlying regex engine:

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

The available options:

  - `FancyRegexOptions`: Default engine with lookaround and backreferences support

    - `backtrack_limit`: Maximum backtracking steps
    - `size_limit`: Maximum compiled regex size in bytes
    - `dfa_size_limit`: Maximum DFA cache size in bytes

  - `RegexOptions`: Safer engine with linear-time guarantee

    - `size_limit`: Maximum compiled regex size in bytes
    - `dfa_size_limit`: Maximum DFA cache size in bytes

This configuration is crucial when working with untrusted schemas where attackers might craft malicious regex patterns.

## External References

By default, `jsonschema-rs` resolves HTTP references and file references from the local file system. You can implement a custom retriever to handle external references. Here's an example that uses a static map of schemas:

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

## Schema Registry

For applications that frequently use the same schemas, you can create a registry to store and reference them efficiently:

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

The registry can be configured with a draft version and a retriever for external references:

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

`jsonschema-rs` provides detailed validation errors through the `ValidationError` class, which includes both basic error information and specific details about what caused the validation to fail:

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

For a complete list of all error kinds and their attributes, see the [type definitions file](https://github.com/Stranger6667/jsonschema/blob/master/crates/jsonschema-py/python/jsonschema_rs/__init__.pyi)

### Error Message Masking

When working with sensitive data, you might want to hide actual values from error messages.
You can mask instance values in error messages by providing a placeholder:

```python
import jsonschema_rs

schema = {
    "type": "object",
    "properties": {
        "password": {"type": "string", "minLength": 8},
        "api_key": {"type": "string", "pattern": "^[A-Z0-9]{32}$"}
    }
}

# Use default masking (replaces values with "[REDACTED]")
validator = jsonschema_rs.validator_for(schema, mask="[REDACTED]")

try:
    validator.validate({
        "password": "123",
        "api_key": "secret_key_123"
    })
except jsonschema_rs.ValidationError as exc:
    assert str(exc) == '''[REDACTED] does not match "^[A-Z0-9]{32}$"

Failed validating "pattern" in schema["properties"]["api_key"]

On instance["api_key"]:
    [REDACTED]'''
```

## Performance

`jsonschema-rs` is designed for high performance, outperforming other Python JSON Schema validators in most scenarios:

- Up to **60-390x** faster than `jsonschema` for complex schemas and large instances
- Generally **3-7x** faster than `fastjsonschema` on CPython

For detailed benchmarks, see our [full performance comparison](https://github.com/Stranger6667/jsonschema/blob/master/crates/jsonschema-py/BENCHMARKS.md).

## Python support

`jsonschema-rs` supports CPython 3.8 through 3.14.

## Acknowledgements

This library draws API design inspiration from the Python [`jsonschema`](https://github.com/python-jsonschema/jsonschema) package. We're grateful to the Python `jsonschema` maintainers and contributors for their pioneering work in JSON Schema validation.

## Support

If you have questions, need help, or want to suggest improvements, please use [GitHub Discussions](https://github.com/Stranger6667/jsonschema/discussions).

## Sponsorship

If you find `jsonschema-rs` useful, please consider [sponsoring its development](https://github.com/sponsors/Stranger6667).

## Contributing

We welcome contributions! Here's how you can help:

- Share your use cases
- Implement missing keywords
- Fix failing test cases from the [JSON Schema test suite](https://bowtie.report/#/implementations/rust-jsonschema)

See [CONTRIBUTING.md](https://github.com/Stranger6667/jsonschema/blob/master/CONTRIBUTING.md) for more details.

## License

Licensed under [MIT License](https://github.com/Stranger6667/jsonschema/blob/master/LICENSE).

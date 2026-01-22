# Migration Guide: Python `jsonschema` to `jsonschema-rs`

## Quick Reference

| `jsonschema` | `jsonschema-rs` |
|--------------|-----------------|
| `validate(instance, schema)` | `validate(schema, instance)` |
| `Draft3Validator` | Not supported |
| `validators.validator_for(schema)` returns class | `validator_for(schema)` returns instance |
| `FormatChecker` class | `formats` dict parameter |
| `RefResolver` / `referencing.Registry` | `retriever` function / `Registry` |
| `error.path` / `error.absolute_path` | `error.instance_path` |
| `error.validator` | `error.kind` |

## What Stays the Same

- JSON Schema documents work as-is
- Method names: `is_valid()`, `iter_errors()`, `validate()`
- Draft-specific validators: `Draft4Validator`, `Draft7Validator`, etc.

## API Changes

### Validation

```python
# jsonschema
jsonschema.validate("foo", {"type": "string"})

# jsonschema-rs
jsonschema_rs.validate({"type": "string"}, "foo")
```

### Format Validation

```python
# jsonschema
checker = jsonschema.FormatChecker()

@checker.checks("currency")
def check_currency(value):
    return len(value) == 3 and value.isascii()

validator = jsonschema.Draft7Validator(schema, format_checker=checker)

# jsonschema-rs
def check_currency(value):
    return len(value) == 3 and value.isascii()

validator = jsonschema_rs.Draft7Validator(
    schema,
    formats={"currency": check_currency},
    validate_formats=True
)
```

### Reference Resolution

References are resolved when the validator is created, not during validation.

`RefResolver` is deprecated since jsonschema 4.18.0 in favor of `referencing.Registry`.

```python
# jsonschema (RefResolver, deprecated)
resolver = jsonschema.RefResolver(
    "",
    schema,
    store={"https://example.com/person.json": {"type": "object"}}
)
validator = jsonschema.Draft7Validator(schema, resolver=resolver)

# jsonschema-rs
def retriever(uri):
    schemas = {"https://example.com/person.json": {"type": "object"}}
    return schemas[uri]

validator = jsonschema_rs.Draft7Validator(schema, retriever=retriever)
```

```python
# jsonschema (referencing.Registry)
from referencing import Registry, Resource
from referencing.jsonschema import DRAFT7

registry = Registry().with_resource(
    "https://example.com/person.json",
    Resource.from_contents(person_schema, default_specification=DRAFT7)
)
validator = jsonschema.Draft7Validator(schema, registry=registry)

# jsonschema-rs
registry = jsonschema_rs.Registry([
    ("https://example.com/person.json", person_schema)
])
validator = jsonschema_rs.Draft7Validator(schema, registry=registry)
```

### Error Attributes

In jsonschema, `error.path` is relative to the current validation context and may differ from `error.absolute_path` in nested validations (e.g., inside `anyOf`). `jsonschema-rs` provides only `error.instance_path`, which is always relative to the root instance (equivalent to `error.absolute_path`).

```python
# jsonschema
print(list(error.absolute_path))
print(error.validator)
print(error.validator_value)

# jsonschema-rs
print(error.instance_path)
print(error.kind)
# Access details via error.kind attributes, e.g. error.kind.limit
```

## Migration Checklist

- [ ] Swap argument order in `validate()`, `is_valid()`, `iter_errors()` calls
- [ ] Replace `FormatChecker` with `formats` dict
- [ ] Replace `RefResolver` with `retriever` function or `Registry`
- [ ] Update error path attributes to `error.instance_path`
- [ ] Update `error.validator` to `error.kind`

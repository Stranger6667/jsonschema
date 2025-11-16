# Migration Guide

## Upgrading to 0.35.0

### Arbitrary Precision Numbers

Arbitrary precision support is now always enabled. Numeric values in `ValidationError` attributes use the most accurate Python type:

- Integers fitting in i64/u64 → `int`
- Larger integers → `int` (parsed from string)
- Floats within f64 range → `float`
- Floats outside f64 range → `decimal.Decimal`

**Breaking change**: `ValidationErrorKind.MultipleOf.multiple_of` changed from `float` to `int | float | Decimal`.

If you access numeric error fields, handle all types:

```python
from decimal import Decimal
from jsonschema_rs import ValidationError, ValidationErrorKind

try:
    validator.validate(instance)
except ValidationError as exc:
    if isinstance(exc.kind, ValidationErrorKind.MultipleOf):
        multiple = exc.kind.multiple_of  # int, float, or Decimal
        if isinstance(multiple, Decimal):
            # Handle Decimal (rare - only for huge numbers)
            multiple_str = str(multiple)
```

Performance impact is negligible for most schemas (~2x slower only for number-heavy data like GeoJSON).

## Upgrading from 0.19.x to 0.20.0

Draft-specific validators are now available:

```python
# Old (0.19.x)
validator = jsonschema_rs.JSONSchema(schema, draft=jsonschema_rs.Draft202012)

# New (0.20.0)
validator = jsonschema_rs.Draft202012Validator(schema)
```

Automatic draft detection:

```python
# Old (0.19.x)
validator = jsonschema_rs.JSONSchema(schema)

# New (0.20.0)
validator = jsonschema_rs.validator_for(schema)
```


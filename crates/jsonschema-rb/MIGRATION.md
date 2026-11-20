# Migrating from json_schemer

## Quick Reference

| json_schemer | jsonschema_rs |
|---|---|
| `JSONSchemer.schema(s)` | `JSONSchema.validator_for(s)` |
| `JSONSchemer.schema(s, meta_schema: 'draft7')` | `JSONSchema::Draft7Validator.new(s)` |
| `schemer.valid?(d)` | `validator.valid?(d)` |
| `schemer.validate(d)` | `validator.each_error(d)` |
| `schemer.validate!(d)` | `validator.validate!(d)` |
| `JSONSchemer.valid?(s, d)` | `JSONSchema.valid?(s, d)` |
| `JSONSchemer.valid_schema?(s)` | `JSONSchema::Meta.valid?(s)` |
| `error["data_pointer"]` | `error.instance_path_pointer` |
| `error["schema_pointer"]` | `error.schema_path_pointer` |
| `error["type"]` | `error.kind.name` |
| `error["error"]` | `error.message` |
| `ref_resolver: proc` | `retriever: proc` |
| `format: true` | `validate_formats: true` |

## What Stays the Same

- JSON Schema documents work as-is
- `valid?` and `validate!` method names
- Custom format validators via `formats:`, as procs that take the string
- One-off validation: `JSONSchema.valid?(schema, data)`

## Error Objects

json_schemer returns hashes, jsonschema_rs returns `ValidationError` objects:

```ruby
# json_schemer
error["data_pointer"]    # => "/foo/bar"
error["schema_pointer"]  # => "/properties/foo/minimum"
error["type"]            # => "minimum"
error["error"]           # => "value is less than 10"

# jsonschema_rs
error.instance_path            # => ["foo", "bar"]
error.instance_path_pointer    # => "/foo/bar"  (same format as data_pointer)
error.schema_path              # => ["properties", "foo", "minimum"]
error.schema_path_pointer      # => "/properties/foo/minimum"
error.kind.name                # => "minimum"
error.message                  # => "5 is less than the minimum of 10"

# Need a hash? Use to_h on error kind
error.kind.to_h        # => { name: "minimum", value: { limit: 10 } }
```

## Reference Resolution

```ruby
# json_schemer
JSONSchemer.schema(schema, ref_resolver: refs.to_proc)

# jsonschema_rs — retriever
JSONSchema.validator_for(schema, retriever: ->(uri) { fetch_schema(uri) })

# jsonschema_rs — registry
registry = JSONSchema::Registry.new([["http://example.com/s", sub_schema]])
JSONSchema.validator_for(schema, registry: registry)
```

## What You Gain

- **Structured output** — `evaluate` API with flag, list, and hierarchical output formats
- **Custom keywords** — extend JSON Schema with domain-specific validation rules
- **Error masking** — hide sensitive data in error messages with `mask:`
- **Regex engine configuration** — choose between fancy-regex (default) and linear-time regex
- **Email validation options** — TLD, IP-literal, display-name and domain-segment rules for `format: email`

## Not Supported

- `insert_property_defaults` — jsonschema_rs is a validator, not a data transformer
- OpenAPI document parsing — use a dedicated OpenAPI library

## Migration Checklist

- [ ] Replace `JSONSchemer.schema` with `JSONSchema.validator_for`
- [ ] Replace `validate` (error iteration) with `each_error`
- [ ] Replace `ref_resolver:` with `retriever:` or use `Registry`
- [ ] Replace `format: true` with `validate_formats: true`
- [ ] Replace `meta_schema: 'draft7'` with `JSONSchema::Draft7Validator.new(s)` or `draft: :draft7` on one-off functions
- [ ] Update error handling from hash access to `ValidationError` attributes

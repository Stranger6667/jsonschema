# jsonschema_rs

[![Build](https://img.shields.io/github/actions/workflow/status/Stranger6667/jsonschema/ci.yml?branch=master&style=flat-square)](https://github.com/Stranger6667/jsonschema/actions)
[![Version](https://img.shields.io/gem/v/jsonschema_rs.svg?style=flat-square)](https://rubygems.org/gems/jsonschema_rs)
[![Ruby versions](https://img.shields.io/badge/ruby-3.2%20%7C%203.4%20%7C%204.0-blue?style=flat-square)](https://rubygems.org/gems/jsonschema_rs)
[<img alt="Supported Dialects" src="https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fsupported_versions.json&style=flat-square">](https://bowtie.report/#/implementations/rust-jsonschema)

A high-performance JSON Schema validator for Ruby.

```ruby
require 'jsonschema_rs'

schema = { "maxLength" => 5 }
instance = "foo"

# One-off validation
JSONSchema.valid?(schema, instance)  # => true

begin
  JSONSchema.validate!(schema, "incorrect")
rescue JSONSchema::ValidationError => e
  puts e.message  # => "\"incorrect\" is longer than 5 characters"
end

# Build & reuse (faster)
validator = JSONSchema.validator_for(schema)

# Iterate over errors
validator.each_error(instance) do |error|
  puts "Error: #{error.message}"
  puts "Location: #{error.instance_path}"
end

# Boolean result
validator.valid?(instance)  # => true

# Structured output (JSON Schema Output v1)
evaluation = validator.evaluate(instance)
evaluation.errors.each do |err|
  puts "Error at #{err[:instanceLocation]}: #{err[:error]}"
end
```

> **Migrating from `json_schemer`?** See the [migration guide](MIGRATION.md).

## Highlights

- 📚 Full support for popular JSON Schema drafts
- 🌐 Remote reference fetching (network/file)
- 🔧 Custom keywords and format validators
- ✨ Meta-schema validation for schema documents
- ♦️ Supports Ruby 3.2, 3.4 and 4.0

### Supported drafts

The following drafts are supported:

- [![Draft 2020-12](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft2020-12.json)](https://bowtie.report/#/implementations/rust-jsonschema)
- [![Draft 2019-09](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft2019-09.json)](https://bowtie.report/#/implementations/rust-jsonschema)
- [![Draft 7](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft7.json)](https://bowtie.report/#/implementations/rust-jsonschema)
- [![Draft 6](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft6.json)](https://bowtie.report/#/implementations/rust-jsonschema)
- [![Draft 4](https://img.shields.io/endpoint?url=https%3A%2F%2Fbowtie.report%2Fbadges%2Frust-jsonschema%2Fcompliance%2Fdraft4.json)](https://bowtie.report/#/implementations/rust-jsonschema)

You can check the current status on the [Bowtie Report](https://bowtie.report/#/implementations/rust-jsonschema).

## Installation

Add to your Gemfile:

```
gem 'jsonschema_rs'
```

Pre-built native gems are available for:

- **Linux**: `x86_64`, `aarch64` (glibc and musl)
- **macOS**: `x86_64`, `arm64`
- **Windows**: `x64` (mingw-ucrt)

If no pre-built gem is available for your platform, it will be compiled from source during installation. You'll need:
- Ruby 3.2+
- Rust toolchain ([rustup](https://rustup.rs/))

## Usage

### Reusable validators

For validating multiple instances against the same schema, create a reusable validator.
`validator_for` automatically detects the draft version from the `$schema` keyword in the schema:

```ruby
validator = JSONSchema.validator_for({
  "type" => "object",
  "properties" => {
    "name" => { "type" => "string" },
    "age" => { "type" => "integer", "minimum" => 0 }
  },
  "required" => ["name"]
})

validator.valid?({ "name" => "Alice", "age" => 30 })  # => true
validator.valid?({ "age" => 30 })                      # => false
```

You can use draft-specific validators for different JSON Schema versions:

```ruby
validator = JSONSchema::Draft7Validator.new(schema)

# Available: Draft4Validator, Draft6Validator, Draft7Validator,
#            Draft201909Validator, Draft202012Validator
```

### Custom format validators

```ruby
phone_format = ->(value) { value.match?(/^\+?[1-9]\d{1,14}$/) }

validator = JSONSchema.validator_for(
  { "type" => "string", "format" => "phone" },
  validate_formats: true,
  formats: { "phone" => phone_format }
)
```

### Custom keyword validators

```ruby
class EvenValidator
  def initialize(parent_schema, value, schema_path)
    @enabled = value
  end

  def validate(instance)
    return unless @enabled && instance.is_a?(Integer)
    raise "#{instance} is not even" if instance.odd?
  end
end

validator = JSONSchema.validator_for(
  { "type" => "integer", "even" => true },
  keywords: { "even" => EvenValidator }
)
```

Each custom keyword class must implement:
- `initialize(parent_schema, value, schema_path)` - called during schema compilation
- `validate(instance)` - raise on failure, return normally on success

### Structured evaluation output

When you need more than a boolean result, use the `evaluate` API to access the [JSON Schema Output v1](https://json-schema.org/draft/2020-12/json-schema-core#name-output-formatting) formats:

```ruby
schema = {
  "type" => "object",
  "properties" => {
    "name" => { "type" => "string" },
    "age" => { "type" => "integer" }
  },
  "required" => ["name"]
}
validator = JSONSchema.validator_for(schema)

evaluation = validator.evaluate({ "age" => "not_an_integer" })

evaluation.valid?  # => false
```

**Flag output** — simplest, just valid/invalid:

```ruby
evaluation.flag
# => {valid: false}
```

**List output** — flat list of all evaluation nodes:

```ruby
evaluation.list
# => {valid: false,
#     details: [
#       {valid: false, evaluationPath: "", schemaLocation: "", instanceLocation: ""},
#       {valid: true, evaluationPath: "/type", schemaLocation: "/type", instanceLocation: ""},
#       {valid: false, evaluationPath: "/required", schemaLocation: "/required",
#        instanceLocation: "",
#        errors: {"required" => "\"name\" is a required property"}},
#       {valid: false, evaluationPath: "/properties", schemaLocation: "/properties",
#        instanceLocation: "", droppedAnnotations: ["age"]},
#       {valid: false, evaluationPath: "/properties/age", schemaLocation: "/properties/age",
#        instanceLocation: "/age"},
#       {valid: false, evaluationPath: "/properties/age/type",
#        schemaLocation: "/properties/age/type", instanceLocation: "/age",
#        errors: {"type" => "\"not_an_integer\" is not of type \"integer\""}}
#     ]}
```

**Hierarchical output** — nested tree following schema structure:

```ruby
evaluation.hierarchical
# => {valid: false, evaluationPath: "", schemaLocation: "", instanceLocation: "",
#     details: [
#       {valid: true, evaluationPath: "/type", schemaLocation: "/type", instanceLocation: ""},
#       {valid: false, evaluationPath: "/required", schemaLocation: "/required",
#        instanceLocation: "",
#        errors: {"required" => "\"name\" is a required property"}},
#       {valid: false, evaluationPath: "/properties", schemaLocation: "/properties",
#        instanceLocation: "", droppedAnnotations: ["age"],
#        details: [
#          {valid: false, evaluationPath: "/properties/age",
#           schemaLocation: "/properties/age", instanceLocation: "/age",
#           details: [
#             {valid: false, evaluationPath: "/properties/age/type",
#              schemaLocation: "/properties/age/type", instanceLocation: "/age",
#              errors: {"type" => "\"not_an_integer\" is not of type \"integer\""}}
#           ]}
#        ]}
#     ]}
```

**Collected errors** — flat list of all errors across evaluation nodes:

```ruby
evaluation.errors
# => [{schemaLocation: "/required", absoluteKeywordLocation: nil,
#      instanceLocation: "", error: "\"name\" is a required property"},
#     {schemaLocation: "/properties/age/type", absoluteKeywordLocation: nil,
#      instanceLocation: "/age",
#      error: "\"not_an_integer\" is not of type \"integer\""}]
```

**Collected annotations** — flat list of annotations from successfully validated nodes.
When a node fails validation, its annotations appear as `droppedAnnotations` in the list/hierarchical output instead.

```ruby
valid_eval = validator.evaluate({ "name" => "Alice", "age" => 30 })
valid_eval.annotations
# => [{schemaLocation: "/properties", absoluteKeywordLocation: nil,
#      instanceLocation: "", annotations: ["age", "name"]}]
```

## Meta-Schema Validation

Validate that a JSON Schema document is itself valid:

```ruby
JSONSchema::Meta.valid?({ "type" => "string" })      # => true
JSONSchema::Meta.valid?({ "type" => "invalid_type" }) # => false

begin
  JSONSchema::Meta.validate!({ "type" => 123 })
rescue JSONSchema::ValidationError => e
  e.message  # => "123 is not valid under any of the schemas listed in the 'anyOf' keyword"
end
```

## External References

By default, `jsonschema` resolves HTTP references and file references from the local file system. You can implement a custom retriever to handle external references:

```ruby
schemas = {
  "https://example.com/person.json" => {
    "type" => "object",
    "properties" => {
      "name" => { "type" => "string" },
      "age" => { "type" => "integer" }
    },
    "required" => ["name", "age"]
  }
}

retriever = ->(uri) { schemas[uri] }

schema = { "$ref" => "https://example.com/person.json" }
validator = JSONSchema.validator_for(schema, retriever: retriever)

validator.valid?({ "name" => "Alice", "age" => 30 })  # => true
validator.valid?({ "name" => "Bob" })                  # => false (missing "age")
```

## Schema Registry

For applications that frequently use the same schemas, create a registry to store and reference them:

```ruby
registry = JSONSchema::Registry.new([
  ["https://example.com/address.json", {
    "type" => "object",
    "properties" => {
      "street" => { "type" => "string" },
      "city" => { "type" => "string" }
    }
  }],
  ["https://example.com/person.json", {
    "type" => "object",
    "properties" => {
      "name" => { "type" => "string" },
      "address" => { "$ref" => "https://example.com/address.json" }
    }
  }]
])

validator = JSONSchema.validator_for(
  { "$ref" => "https://example.com/person.json" },
  registry: registry
)

validator.valid?({
  "name" => "John",
  "address" => { "street" => "Main St", "city" => "Boston" }
})  # => true
```

The registry also accepts `draft:` and `retriever:` options:

```ruby
registry = JSONSchema::Registry.new(
  [["https://example.com/person.json", schemas["https://example.com/person.json"]]],
  draft: :draft7,
  retriever: retriever
)
```

## Regular Expression Configuration

When validating schemas with regex patterns (in `pattern` or `patternProperties`), you can configure the underlying regex engine:

```ruby
# Default fancy-regex engine with backtracking limits
# (supports lookaround and backreferences but needs protection against DoS)
validator = JSONSchema.validator_for(
  { "type" => "string", "pattern" => "^(a+)+$" },
  pattern_options: JSONSchema::FancyRegexOptions.new(backtrack_limit: 10_000)
)

# Standard regex engine for guaranteed linear-time matching
# (prevents regex DoS attacks but supports fewer features)
validator = JSONSchema.validator_for(
  { "type" => "string", "pattern" => "^a+$" },
  pattern_options: JSONSchema::RegexOptions.new
)

# Both engines support memory usage configuration
validator = JSONSchema.validator_for(
  { "type" => "string", "pattern" => "^a+$" },
  pattern_options: JSONSchema::RegexOptions.new(
    size_limit: 1024 * 1024,   # Maximum compiled pattern size
    dfa_size_limit: 10240       # Maximum DFA cache size
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

## Email Format Configuration

When validating email addresses using `{"format": "email"}`, you can customize the validation behavior:

```ruby
# Require a top-level domain (reject "user@localhost")
validator = JSONSchema.validator_for(
  { "format" => "email", "type" => "string" },
  validate_formats: true,
  email_options: JSONSchema::EmailOptions.new(require_tld: true)
)
validator.valid?("user@localhost")    # => false
validator.valid?("user@example.com") # => true

# Disallow IP address literals and display names
validator = JSONSchema.validator_for(
  { "format" => "email", "type" => "string" },
  validate_formats: true,
  email_options: JSONSchema::EmailOptions.new(
    allow_domain_literal: false,  # Reject "user@[127.0.0.1]"
    allow_display_text: false     # Reject "Name <user@example.com>"
  )
)

# Require minimum domain segments
validator = JSONSchema.validator_for(
  { "format" => "email", "type" => "string" },
  validate_formats: true,
  email_options: JSONSchema::EmailOptions.new(minimum_sub_domains: 3)  # e.g., user@sub.example.com
)
```

Available options:

  - `require_tld`: Require a top-level domain (e.g., reject "user@localhost")
  - `allow_domain_literal`: Allow IP address literals like "user@[127.0.0.1]" (default: true)
  - `allow_display_text`: Allow display names like "Name <user@example.com>" (default: true)
  - `minimum_sub_domains`: Minimum number of domain segments required

## Error Handling

`jsonschema` provides detailed validation errors through the `ValidationError` class:

```ruby
schema = { "type" => "string", "maxLength" => 5 }

begin
  JSONSchema.validate!(schema, "too long")
rescue JSONSchema::ValidationError => error
  # Basic error information
  error.message         # => '"too long" is longer than 5 characters'
  error.verbose_message # => Full context with schema path and instance
  error.instance_path   # => Location in the instance that failed
  error.schema_path     # => Location in the schema that failed

  # Detailed error information via `kind`
  error.kind.name       # => "maxLength"
  error.kind.value      # => { "limit" => 5 }
  error.kind.to_h       # => { "name" => "maxLength", "value" => { "limit" => 5 } }
end
```

### Error Kind Properties

Each error has a `kind` property with convenient accessors:

```ruby
JSONSchema.each_error({ "minimum" => 5 }, 3).each do |error|
  error.kind.name   # => "minimum"
  error.kind.value  # => { "limit" => 5 }
  error.kind.to_h   # => { "name" => "minimum", "value" => { "limit" => 5 } }
  error.kind.to_s   # => "minimum"
end
```

### Error Message Masking

When working with sensitive data, you can mask instance values in error messages:

```ruby
schema = {
  "type" => "object",
  "properties" => {
    "password" => { "type" => "string", "minLength" => 8 },
    "api_key" => { "type" => "string", "pattern" => "^[A-Z0-9]{32}$" }
  }
}

validator = JSONSchema.validator_for(schema, mask: "[REDACTED]")

begin
  validator.validate!({ "password" => "123", "api_key" => "secret_key_123" })
rescue JSONSchema::ValidationError => exc
  puts exc.message
  # => '[REDACTED] does not match "^[A-Z0-9]{32}$"'
  puts exc.verbose_message
  # => '[REDACTED] does not match "^[A-Z0-9]{32}$"\n\nFailed validating...\nOn instance["api_key"]:\n    [REDACTED]'
end
```

### Exception Classes

- **`JSONSchema::ValidationError`** - raised on validation failure
  - `message`, `verbose_message`, `instance_path`, `schema_path`, `evaluation_path`, `kind`, `instance`
  - JSON Pointer helpers: `instance_path_pointer`, `schema_path_pointer`, `evaluation_path_pointer`
- **`JSONSchema::ReferencingError`** - raised when `$ref` cannot be resolved

## Options Reference

One-off validation methods (`valid?`, `validate!`, `each_error`, `evaluate`) accept these keyword arguments:

```ruby
JSONSchema.valid?(schema, instance,
  draft: :draft7,                  # Specific draft version (symbol)
  validate_formats: true,          # Enable format validation (default: false)
  ignore_unknown_formats: true,    # Don't error on unknown formats (default: true)
  base_uri: "https://example.com", # Base URI for reference resolution
  mask: "[REDACTED]",              # Mask sensitive data in error messages
  retriever: ->(uri) { ... },      # Custom schema retriever for $ref
  formats: { "name" => proc },     # Custom format validators
  keywords: { "name" => Klass },   # Custom keyword validators
  registry: registry,              # Pre-registered schemas
  pattern_options: opts,           # RegexOptions or FancyRegexOptions
  email_options: opts,             # EmailOptions
  http_options: opts               # HttpOptions
)
```

`evaluate` accepts the same options except `mask` (currently unsupported for evaluation output).

`validator_for` accepts the same options except `draft:` — use draft-specific validators (`Draft7Validator.new`, etc.) to pin a draft version.

Valid draft symbols: `:draft4`, `:draft6`, `:draft7`, `:draft201909`, `:draft202012`.

## Performance

`jsonschema` is designed for high performance, outperforming other Ruby JSON Schema validators in most scenarios:

- **30-117x** faster than `json_schemer` for complex schemas and large instances
- **206-473x** faster than `json-schema` where supported
- **7-118x** faster than `rj_schema` (RapidJSON/C++)

For detailed benchmarks, see our [full performance comparison](https://github.com/Stranger6667/jsonschema/blob/master/crates/jsonschema-rb/BENCHMARKS.md).

**Tips:** Reuse validators. Use `valid?` for boolean checks (short-circuits on first error).

## Acknowledgements

This library draws API design inspiration from the Python [`jsonschema`](https://github.com/python-jsonschema/jsonschema) package. We're grateful to the Python `jsonschema` maintainers and contributors for their pioneering work in JSON Schema validation.

## Support

If you have questions, need help, or want to suggest improvements, please use [GitHub Discussions](https://github.com/Stranger6667/jsonschema/discussions).

## Sponsorship

If you find `jsonschema` useful, please consider [sponsoring its development](https://github.com/sponsors/Stranger6667).

## Contributing

See [CONTRIBUTING.md](https://github.com/Stranger6667/jsonschema/blob/master/CONTRIBUTING.md) for details.

## License

Licensed under [MIT License](https://github.com/Stranger6667/jsonschema/blob/master/LICENSE).

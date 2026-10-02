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

- 📚 Drafts 4, 6, 7, 2019-09 and 2020-12
- 🔧 Custom keywords and format validators
- ⚡ Compile-time validators for your own native extensions, via the Rust crate
- 🌐 `$ref` resolution over HTTP and from files
- 📦 Schema bundling into Compound Schema Documents, and `$ref` dereferencing
- 🎨 Structured Output v1 reports (flag/list/hierarchical)
- ✨ Meta-schema validation for schema documents, including custom metaschemas
- 🧮 Experimental schema canonicalization
- ♦️ Supports Ruby 3.2, 3.4 and 4.0

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

Add to your Gemfile:

```
gem 'jsonschema_rs'
```

Pre-built native gems:

- **Linux**: `x86_64`, `aarch64` (glibc and musl)
- **macOS**: `x86_64`, `arm64`
- **Windows**: `x64` (mingw-ucrt)

On other platforms, `gem install` compiles from source and needs:
- Ruby 3.2+
- Rust toolchain ([rustup](https://rustup.rs/))

## Usage

### Reusable validators

Build a validator once to check many instances against one schema.
`validator_for` detects the draft from `$schema` and falls back to Draft 2020-12:

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

To pick the draft yourself, use a draft-specific class:

```ruby
validator = JSONSchema::Draft7Validator.new(schema)

# Available: Draft4Validator, Draft6Validator, Draft7Validator,
#            Draft201909Validator, Draft202012Validator
```

### Custom format validators

Pass a callable that takes a `String` and returns a boolean via `formats`. Drafts 2019-09 and 2020-12 treat `format` as an annotation, so set `validate_formats: true` to get it checked:

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
- `initialize(parent_schema, value, schema_path)`: called once when the validator is built
- `validate(instance)`: raise to reject the value, return to accept it

To report several problems from one keyword, also implement `iter_errors(instance)` returning an array of exceptions. `each_error` yields each of them. Without it, `each_error` yields the single error from `validate`:

```ruby
class AllPositive
  def initialize(parent_schema, value, schema_path); end

  def validate(instance)
    first_error = iter_errors(instance).first
    raise first_error if first_error
  end

  def iter_errors(instance)
    return [] unless instance.is_a?(Array)
    instance.each_index.select { |i| instance[i].negative? }
            .map { |i| ArgumentError.new("item #{i} is negative") }
  end
end
```

The resulting `ValidationError` keeps the raised exception as `cause`, for both `validate` and `iter_errors`:

```ruby
begin
  validator.validate!(3)
rescue JSONSchema::ValidationError => e
  puts e.cause.class    # => RuntimeError
  puts e.cause.message  # => "3 is not even"
end
```

### Structured evaluation output

`evaluate` returns the [JSON Schema Output v1](https://json-schema.org/draft/2020-12/json-schema-core#name-output-formatting) formats instead of a boolean:

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

**Flag output**: valid or invalid, nothing else:

```ruby
evaluation.flag
# => {valid: false}
```

**List output**: every evaluation node in a flat list:

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

**Hierarchical output**: the same nodes nested by schema structure:

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

**Collected errors**: every error across all nodes:

```ruby
evaluation.errors
# => [{schemaLocation: "/required", absoluteKeywordLocation: nil,
#      instanceLocation: "", error: "\"name\" is a required property"},
#     {schemaLocation: "/properties/age/type", absoluteKeywordLocation: nil,
#      instanceLocation: "/age",
#      error: "\"not_an_integer\" is not of type \"integer\""}]
```

**Collected annotations**: annotations from the nodes that passed.
A failed node reports its annotations as `droppedAnnotations` in the list and hierarchical output instead.

```ruby
valid_eval = validator.evaluate({ "name" => "Alice", "age" => 30 })
valid_eval.annotations
# => [{schemaLocation: "/properties", absoluteKeywordLocation: nil,
#      instanceLocation: "", annotations: ["age", "name"]}]
```

### Canonical JSON serialization

`Canonical::JSON.to_string` serializes equal JSON values to the same string, regardless of key order:

```ruby
schema_a = { "type" => "object", "properties" => { "b" => { "type" => "integer" }, "a" => { "type" => "string" } } }
schema_b = { "properties" => { "a" => { "type" => "string" }, "b" => { "type" => "integer" } }, "type" => "object" }

dump_a = JSONSchema::Canonical::JSON.to_string(schema_a)
dump_b = JSONSchema::Canonical::JSON.to_string(schema_b)

dump_a == dump_b # => true
```

Use it to deduplicate schemas that differ only in key order.

## Schema Canonicalization

> **Experimental**: the canonicalization API may change in minor releases.

`JSONSchema.canonicalize` reduces a schema to a normal form that accepts the same values. Schemas that accept the same values reduce to equal `CanonicalSchema` objects, and a schema proven to accept nothing reduces to `false`:

```ruby
canonical = JSONSchema.canonicalize(
  { "allOf" => [{ "type" => "integer", "minimum" => 0 }, { "minimum" => 10, "maximum" => 100 }] }
)
canonical.to_json_schema
# => {"$schema"=>"https://json-schema.org/draft/2020-12/schema",
#     "type"=>"integer", "minimum"=>10, "maximum"=>100}

# However they were written, equivalent schemas compare equal
canonical == JSONSchema.canonicalize({ "type" => "integer", "maximum" => 100, "minimum" => 10 })
# => true

# A schema no value can satisfy collapses
JSONSchema.canonicalize({ "type" => "integer", "minimum" => 10, "maximum" => 5 }).satisfiability
# => :no, which is JSONSchema::Canonical::Satisfiability::NO
```

A schema the canonical form cannot model exactly comes back unchanged, with `kind == :raw`. Its `view.reason` names what stopped the run, and `view.pointer` the subschema at fault when a single one is.

Canonical schemas combine like sets of values. Every emitted schema carries `$schema`; the comments below leave it out:

```ruby
positive = JSONSchema.canonicalize({ "type" => "integer", "minimum" => 0 })
bounded  = JSONSchema.canonicalize({ "type" => "integer", "maximum" => 100 })

# Every value both admit
positive.intersect(bounded).to_json_schema
# => {"type"=>"integer", "minimum"=>0, "maximum"=>100}

# Every value either admits
positive.union(bounded).to_json_schema
# => {"type"=>"integer"}

# Every value `positive` admits and `bounded` rejects
positive.subtract(bounded).to_json_schema
# => {"type"=>"integer", "minimum"=>101}

# Every value `positive` rejects: other types, negative integers and non-integer numbers
positive.negate.to_json_schema
# => {"anyOf"=>[{"type"=>["null", "boolean", "string", "array", "object"]},
#               {"type"=>"integer", "maximum"=>-1},
#               {"type"=>"number", "not"=>{"multipleOf"=>1}}]}

positive.covers(bounded)        # => :no: `bounded` takes negative integers, `positive` does not
positive.satisfiability         # => :yes
```

### Comparing two versions of a schema

`subtract` tells you what an edit did to a schema. `old.subtract(new)` accepts exactly the values `old` accepts and `new` rejects, so it accepts nothing when the edit lost nothing.

```ruby
old = JSONSchema.canonicalize({ "type" => "string" })
new = JSONSchema.canonicalize({ "type" => "string", "maxLength" => 50 })

# What `new` stopped accepting, as a schema
old.subtract(new).to_json_schema
# => {"$schema"=>"https://json-schema.org/draft/2020-12/schema",
#     "type"=>"string", "minLength"=>51}

# Nothing is accepted that was not accepted before, so the change only narrows
new.subtract(old).satisfiability
# => :no
```

The direction to check depends on who sends the value:

- For a request schema, check `old.subtract(new)`. It accepts the payloads existing callers send that the new schema rejects.
- For a response schema, check `new.subtract(old)`. It accepts the values the new schema lets a server return that callers never agreed to read.

`:no` on the difference proves the edit safe in that direction. `:unknown` means the canonicalizer could not decide, and proves nothing either way. Read it as the answer that keeps you safe:

- `satisfiability`: only `:no` proves a schema accepts nothing. Treat `:unknown` like `:yes`.
- `a.covers(b)`: only `:yes` proves `a` accepts every value `b` accepts. Treat `:unknown` like `:no`.

`JSONSchema::Canonical::Containment`, `Satisfiability`, `Distinctness` and `Kind` hold these symbols as constants, each with an `ALL` list.

The set operations raise `IncompatibleOperands` when the operands differ in draft, `format` assertion or regular-expression engine, or resolve `#` or one external resource to different schemas. They raise `UnsupportedOperand` when an operand is `:raw`, and `UnsupportedResult` when the canonical form cannot express the result exactly. All three live under `JSONSchema::Canonical`.

### Finding the dead subschemas of a document

`JSONSchema::Canonical.find_unsatisfiable` walks a whole document and reports each subschema that accepts no value, with the keywords at fault and their pointers:

```ruby
reasons = JSONSchema::Canonical.find_unsatisfiable(
  {
    "properties" => {
      "tag" => { "type" => "string", "minLength" => 5, "maxLength" => 2 },
      "name" => { "type" => "string" }
    }
  }
)

case reasons["/properties/tag"]
in JSONSchema::Canonical::ConflictReason[causes:]
  causes.map { |cause| [cause.pointer, cause.keywords] }
  # => [["/properties/tag", ["type"]], ["/properties/tag", ["minLength", "maxLength"]]]
end

# A live subschema is not reported
reasons.key?("/properties/name")
# => false
```

A reason is a `LiteralReason` (written as `false`), an `EmptyReason` (one part admits nothing by itself) or a `ConflictReason` (each part admits values, together they admit none). A missing pointer does not prove that subschema satisfiable. For a document the canonical form cannot model, `find_unsatisfiable` reports nothing, the same way `satisfiability` answers `:unknown`.

## Schema Bundling and Dereferencing

Produce a Compound Schema Document ([Appendix B](https://json-schema.org/draft/2020-12/json-schema-core#appendix-B)) by embedding all external `$ref` targets into a draft-appropriate container. The bundle accepts the same values as the original.

```ruby
address_schema = {
  "$schema" => "https://json-schema.org/draft/2020-12/schema",
  "$id" => "https://example.com/address.json",
  "type" => "object",
  "properties" => { "street" => { "type" => "string" }, "city" => { "type" => "string" } },
  "required" => ["street", "city"]
}

schema = {
  "$schema" => "https://json-schema.org/draft/2020-12/schema",
  "type" => "object",
  "properties" => { "home" => { "$ref" => "https://example.com/address.json" } },
  "required" => ["home"]
}

registry = JSONSchema::Registry.new([["https://example.com/address.json", address_schema]])
bundled = JSONSchema.bundle(schema, registry: registry)
```

`dereference` instead replaces each `$ref` with the schema it points to, for consumers that do not resolve references. Circular references are left in place.

```ruby
dereferenced = JSONSchema.dereference(schema, registry: registry)
```

## Meta-Schema Validation

`JSONSchema::Meta` checks a schema against the meta-schema of its draft:

```ruby
JSONSchema::Meta.valid?({ "type" => "string" })      # => true
JSONSchema::Meta.valid?({ "type" => "invalid_type" }) # => false

begin
  JSONSchema::Meta.validate!({ "type" => 123 })
rescue JSONSchema::ValidationError => e
  e.message  # => "123 is not valid under any of the schemas listed in the 'anyOf' keyword"
end
```

For a custom metaschema, register it and point `$schema` at its URI:

```ruby
metaschema = {
  "$schema" => "https://json-schema.org/draft/2020-12/schema",
  "$id" => "https://example.com/meta",
  "type" => "object",
  "required" => ["title"]
}
meta_registry = JSONSchema::Registry.new([["https://example.com/meta", metaschema]])

JSONSchema::Meta.valid?({ "$schema" => "https://example.com/meta", "title" => "Person" }, registry: meta_registry)  # => true
JSONSchema::Meta.valid?({ "$schema" => "https://example.com/meta" }, registry: meta_registry)                      # => false
```

## External References

By default, `jsonschema_rs` fetches external `$ref` targets over HTTP and from the local file system. Pass a `retriever:` to load them yourself:

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

A `Registry` holds schemas by URI, so a validator resolves `$ref`s to them without fetching anything:

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

`Registry` also takes a default `draft:` and a `retriever:` for URIs it does not hold:

```ruby
registry = JSONSchema::Registry.new(
  [["https://example.com/person.json", schemas["https://example.com/person.json"]]],
  draft: :draft7,
  retriever: retriever
)
```

## Regular Expression Configuration

`pattern_options:` picks the regex engine for `pattern` and `patternProperties` and sets its limits:

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

  - `FancyRegexOptions`: default engine, supports lookaround and backreferences

    - `backtrack_limit`: Maximum backtracking steps
    - `size_limit`: Maximum compiled regex size in bytes
    - `dfa_size_limit`: Maximum DFA cache size in bytes

  - `RegexOptions`: matches in linear time, no lookaround or backreferences

    - `size_limit`: Maximum compiled regex size in bytes
    - `dfa_size_limit`: Maximum DFA cache size in bytes

If you validate against schemas from untrusted sources, use `RegexOptions`: a crafted pattern cannot make it backtrack.

## Email Format Configuration

`email_options:` makes `{"format": "email"}` stricter or looser than the spec default:

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

  - `require_tld`: Require a top-level domain (e.g., reject "user@localhost") (default: false)
  - `allow_domain_literal`: Allow IP address literals like "user@[127.0.0.1]" (default: true)
  - `allow_display_text`: Allow display names like "Name <user@example.com>" (default: true)
  - `minimum_sub_domains`: Minimum number of domain segments required

## Error Handling

A `ValidationError` carries the message, both locations and a `kind` with keyword-specific details:

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

`kind` has generic accessors:

```ruby
JSONSchema.each_error({ "minimum" => 5 }, 3).each do |error|
  error.kind.name   # => "minimum"
  error.kind.value  # => { "limit" => 5 }
  error.kind.to_h   # => { "name" => "minimum", "value" => { "limit" => 5 } }
  error.kind.to_s   # => "minimum"
end
```

### Error Message Masking

Pass `mask:` to replace instance values in error messages with a placeholder:

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

- **`JSONSchema::ValidationError`**: raised on validation failure
  - `message`, `verbose_message`, `instance_path`, `schema_path`, `evaluation_path`, `kind`, `instance`
  - JSON Pointer helpers: `instance_path_pointer`, `schema_path_pointer`, `evaluation_path_pointer`
- **`JSONSchema::ReferencingError`**: raised when a `$ref` cannot be resolved

## Options Reference

The one-off methods `valid?`, `validate!` and `each_error` accept these keyword arguments:

```ruby
JSONSchema.valid?(schema, instance,
  draft: :draft7,                  # Specific draft version (symbol)
  validate_formats: true,          # Check `format` (default: on up to Draft 7, off for 2019-09 and 2020-12)
  ignore_unknown_formats: true,    # Don't error on unknown formats (default: true)
  base_uri: "https://example.com", # Base URI for reference resolution
  mask: "[REDACTED]",              # Mask sensitive data in error messages
  retriever: ->(uri) { ... },      # Custom schema retriever for $ref
  formats: { "name" => proc },     # Custom format validators
  keywords: { "name" => Klass },   # Custom keyword validators
  registry: registry,              # Pre-registered schemas
  vocabularies: ["https://..."],   # Vocabularies implemented by custom keywords
  pattern_options: opts,           # RegexOptions or FancyRegexOptions
  email_options: opts,             # EmailOptions
  http_options: opts               # HttpOptions
)
```

`evaluate` accepts the same options except `mask` (currently unsupported for evaluation output).

`validator_for` accepts the same options except `draft:`. To pin a draft, use a draft-specific class such as `Draft7Validator.new`.

Valid draft symbols: `:draft4`, `:draft6`, `:draft7`, `:draft201909`, `:draft202012`.

## Performance

Compared with other Ruby validators:

- **28-148x** faster than `json_schemer` for complex schemas and large instances
- **200-567x** faster than `json-schema` where supported
- **7-130x** faster than `rj_schema` (RapidJSON/C++)

Full results and methodology are in [BENCHMARKS.md](https://github.com/Stranger6667/jsonschema/blob/master/crates/jsonschema-rb/BENCHMARKS.md).

### Compile-Time Validators

If you ship your own extension and know the schema at build time, the Rust crate's
`#[jsonschema::validator(..., backend = Magnus)]` macro compiles it into a validator that reads
Ruby objects directly, so nothing is parsed or compiled when your extension is loaded, and
validation runs up to 3.5x faster than with a validator built at run time. See the
[macro documentation](https://docs.rs/jsonschema/latest/jsonschema/#ruby-extension-modules).

This is not available from the `jsonschema_rs` gem, which takes its schemas at run time.
A complete extension with its build and test commands lives in
[`examples/magnus-extension`](https://github.com/Stranger6667/jsonschema/tree/master/examples/magnus-extension).

## Acknowledgements

The API follows the Python [`jsonschema`](https://github.com/python-jsonschema/jsonschema) package. Thanks to its maintainers and contributors.

## Support

Ask questions and suggest improvements in [GitHub Discussions](https://github.com/Stranger6667/jsonschema/discussions).

## Sponsorship

If you find `jsonschema_rs` useful, please consider [sponsoring its development](https://github.com/sponsors/Stranger6667).

## Contributing

See [CONTRIBUTING.md](https://github.com/Stranger6667/jsonschema/blob/master/CONTRIBUTING.md) for details.

## License

Licensed under [MIT License](https://github.com/Stranger6667/jsonschema/blob/master/LICENSE).

# jsonschema-cli

[<img alt="crates.io" src="https://img.shields.io/crates/v/jsonschema-cli.svg?style=flat-square&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/jsonschema-cli)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-jsonschema-cli?style=flat-square&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/jsonschema-cli)

A fast command-line tool for JSON Schema validation and bundling, powered by the `jsonschema` crate.

## Playground

If you'd like to try `jsonschema`, you can check the WebAssembly-powered [playground](https://jsonschema.dygalo.dev/) to see the results instantly.

## Installation

### Pre-built Binaries

Download the latest binary for your platform from the [releases page](https://github.com/Stranger6667/jsonschema-rs/releases):

**Linux (x86_64):**
- `jsonschema-cli-x86_64-unknown-linux-gnu.tar.gz` - Standard GNU libc
- `jsonschema-cli-x86_64-unknown-linux-musl.tar.gz` - Static binary (MUSL), no dependencies

**Linux (ARM64):**
- `jsonschema-cli-aarch64-unknown-linux-gnu.tar.gz` - Standard GNU libc
- `jsonschema-cli-aarch64-unknown-linux-musl.tar.gz` - Static binary (MUSL), no dependencies

**macOS:**
- `jsonschema-cli-x86_64-apple-darwin.tar.gz` - Intel
- `jsonschema-cli-aarch64-apple-darwin.tar.gz` - Apple Silicon

**Windows:**
- `jsonschema-cli-x86_64-pc-windows-msvc.zip` - MSVC runtime
- `jsonschema-cli-x86_64-pc-windows-gnu.zip` - MinGW, no Visual Studio required

> **Note:** MUSL variants are statically linked and work across all Linux distributions, including Alpine.

Example installation on Linux/macOS:
```bash
curl -LO https://github.com/Stranger6667/jsonschema-rs/releases/download/VERSION/jsonschema-cli-x86_64-unknown-linux-gnu.tar.gz
tar xzf jsonschema-cli-x86_64-unknown-linux-gnu.tar.gz
sudo mv jsonschema-cli /usr/local/bin/
```

### From Source (requires Rust)

```bash
cargo install jsonschema-cli
```

## Usage

```
jsonschema <COMMAND>
```

Four subcommands are available: `validate`, `bundle`, `dereference` and `canonicalize`.

> ⚠️ **Deprecation notice:** The flat invocation `jsonschema schema.json -i instance.json` still works but is deprecated. Migrate to `jsonschema validate schema.json -i instance.json`.

---

## `jsonschema validate` — validate instances

```
jsonschema validate [OPTIONS] [SCHEMA]
```

`SCHEMA` may be omitted when every instance names its own schema — see
[Self-describing instances](#self-describing-instances) below.

### Options

| Flag | Description |
|---|---|
| `-i, --instance <FILE>` | Instance(s) to validate (repeatable) |
| `-d, --draft <DRAFT>` | Enforce a specific draft (`4`, `6`, `7`, `2019`, `2020`) |
| `--assert-format` / `--no-assert-format` | Enable/disable `format` keyword validation |
| `--vocabulary <URI>` | Declare support for a vocabulary the meta-schema requires (repeatable) |
| `--output <text\|flag\|list\|hierarchical>` | Output style (default: `text`) |
| `--errors-only` | Suppress successful validations |
| `--offline` | Refuse to fetch remote `$ref` targets |
| `--connect-timeout <SECONDS>` | Connection timeout for remote `$ref` retrieval |
| `--timeout <SECONDS>` | Total HTTP request timeout |
| `-k, --insecure` | Skip TLS certificate verification |
| `--cacert <FILE>` | Custom CA certificate (PEM) |

### Examples

Validate a single instance:
```
jsonschema validate schema.json -i instance.json
```

Validate multiple instances and emit structured output:
```
jsonschema validate schema.json -i a.json -i b.json --output list
{"output":"list","schema":"schema.json","instance":"a.json","payload":{"valid":true,...}}
{"output":"list","schema":"schema.json","instance":"b.json","payload":{"valid":false,...}}
```

### Self-describing instances

Omit `SCHEMA` and each instance is validated against the schema named in its own `$schema`
property — the convention editors follow for `tsconfig.json`, `renovate.json` and friends:

```
jsonschema validate -i tsconfig.json
```

> **Note:** this is *not* JSON Schema's `$schema`, which declares the dialect of a **schema**
> document. Here the file is data, and `$schema` names the schema to validate it against.

- Remote `$schema` URLs are fetched, like remote `$ref`s. `--timeout`, `--connect-timeout`,
  `-k` and `--cacert` apply.
- A relative `$schema` (`"./schema.json"`) resolves against the **instance file**, not the
  working directory. A JSON pointer fragment (`"./schemas.json#/$defs/Config"`) is honored.
- An instance without a usable `$schema` is reported as an error; the remaining instances are
  still validated and the run exits `1`.
- Passing `SCHEMA` explicitly always wins — the instance's `$schema` is then ignored.
- In structured output modes the `schema` field holds the resolved URI, which varies per
  instance.

---

## `jsonschema bundle` — embed external resources

Embeds all `$ref` targets into a draft-appropriate container:
- `definitions` for Draft 4/6/7
- `$defs` for Draft 2019-09/2020-12
- For mixed-draft bundles, embedded resources may include both `id` and `$id` for interoperability.

`$ref` values are preserved unchanged ([Appendix B](https://json-schema.org/draft/2020-12/json-schema-core#appendix-B)).

```
jsonschema bundle [OPTIONS] <SCHEMA>
```

### Options

| Flag | Description |
|---|---|
| `--resource <URI=FILE>` | Register an external schema resource (repeatable) |
| `-o, --output <FILE>` | Write result to file instead of stdout |
| `--offline` | Refuse to fetch references outside `--resource` |
| `--connect-timeout`, `--timeout`, `-k`, `--cacert` | Same as `validate` |

### Examples

With a locally registered resource:
```
jsonschema bundle root.json --resource https://example.com/address.json=address.json
```

Write to file:
```
jsonschema bundle root.json -o bundled.json
```

---

## `jsonschema dereference` — inline `$ref` targets

Replaces each `$ref` with the schema it points to. Circular references are left in place.

```
jsonschema dereference [OPTIONS] <SCHEMA>
```

Takes the same options as `bundle`.

---

## `jsonschema canonicalize` — reduce a schema to a normal form

Rewrites a schema to a normal form without changing the set of values it accepts. `allOf` folds
into a single constraint set, `$ref` targets are resolved, and contradictions collapse to `false`.
Equivalent schemas reduce to the same form, so two canonical outputs can be compared directly.

```
jsonschema canonicalize [OPTIONS] <SCHEMA>
```

`SCHEMA` may be JSON or YAML (`.yaml`/`.yml`).

> ⚠️ **Experimental:** canonicalization is experimental and its output may change in minor releases.

### Options

| Flag | Description |
|---|---|
| `--at <POINTER>` | Canonicalize only the subschema at this JSON Pointer |
| `-d, --draft <DRAFT>` | Enforce a specific draft (`4`, `6`, `7`, `2019`, `2020`) |
| `--assert-format` / `--no-assert-format` | Turn `format` validation on or off |
| `-o, --output <FILE>` | Write result to file instead of stdout |

### Examples

`allOf` branches fold into one constraint set:

```console
$ cat pet.yaml
allOf:
  - type: object
    properties: {name: {type: string}}
    required: [name]
  - type: object
    properties: {age: {type: integer, minimum: 0}}

$ jsonschema canonicalize pet.yaml
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "age": {"minimum": 0, "type": "integer"},
    "name": {"type": "string"}
  },
  "required": ["name"],
  "type": "object"
}
```

Equivalent schemas share one form. Both `{"const": 1, "type": "integer"}` and
`{"type": "integer", "minimum": 1, "maximum": 1}` canonicalize to:

```json
{"$schema": "https://json-schema.org/draft/2020-12/schema", "const": 1}
```

A schema no value can satisfy collapses to `false`, written `{"not": {}}`:

```console
$ echo '{"type": "integer", "minimum": 10, "maximum": 5}' > empty.json
$ jsonschema canonicalize empty.json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "not": {}
}
```

Constructs the canonical form cannot model exactly — `$dynamicRef` beside `unevaluatedProperties`,
a `not` over a pattern map, and the like — are passed through as the original document, unchanged.

### Selecting a subschema

`--at` answers "what does this part of the document accept?" without lifting the subschema out of
it, so references into the rest of the document keep resolving:

```console
$ jsonschema canonicalize openapi.yaml --at /components/schemas/Adult
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "age": {"minimum": 3, "type": "integer"},
    "name": {"type": "string"}
  },
  "required": ["name"],
  "type": "object"
}
```

A leading `#` is accepted, so a `$ref` value can be pasted as-is, and an empty pointer selects the
whole document. A selection that refers to itself keeps the `$defs` it needs.

---

## Output formats (`validate`)

| Mode | Description |
|---|---|
| `text` (default) | `<file> - VALID` or `<file> - INVALID. Errors: …` |
| `flag` | `{"valid": true/false}` per instance (ndjson) |
| `list` | Flat list of annotations/errors (ndjson) |
| `hierarchical` | Nested structure following schema hierarchy (ndjson) |

Structured modes emit newline-delimited JSON records:
```json
{"output":"list","schema":"schema.json","instance":"instance.json","payload":{...}}
```

## Exit Codes

- `0` — all instances valid (or no instances provided)
- `1` — one or more instances invalid, or an error occurred

## License

This project is licensed under the MIT License.

# jsonschema-cli

[<img alt="crates.io" src="https://img.shields.io/crates/v/jsonschema-cli.svg?style=flat-square&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/jsonschema-cli)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-jsonschema-cli?style=flat-square&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/jsonschema-cli)

A fast command-line tool for JSON Schema validation and bundling, powered by the `jsonschema` crate.

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

Two subcommands are available: `validate` and `bundle` (inline external refs).

> ⚠️ **Deprecation notice:** The flat invocation `jsonschema schema.json -i instance.json` still works but is deprecated. Migrate to `jsonschema validate schema.json -i instance.json`.

---

## `jsonschema validate` — validate instances

```
jsonschema validate [OPTIONS] <SCHEMA>
```

### Options

| Flag | Description |
|---|---|
| `-i, --instance <FILE>` | Instance(s) to validate (repeatable) |
| `-d, --draft <DRAFT>` | Enforce a specific draft (`4`, `6`, `7`, `2019`, `2020`) |
| `--assert-format` / `--no-assert-format` | Enable/disable `format` keyword validation |
| `--output <text\|flag\|list\|hierarchical>` | Output style (default: `text`) |
| `--errors-only` | Suppress successful validations |
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

---

## `jsonschema bundle` — inline external `$ref` targets

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

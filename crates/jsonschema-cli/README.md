# jsonschema-cli

[<img alt="crates.io" src="https://img.shields.io/crates/v/jsonschema-cli.svg?style=flat-square&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/jsonschema-cli)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-jsonschema-cli?style=flat-square&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/jsonschema-cli)

A fast command-line tool for JSON Schema validation, powered by the `jsonschema` crate.

## Installation

### Pre-built Binaries

Download the latest binary for your platform from the [releases page](https://github.com/Stranger6667/jsonschema-rs/releases):

- **Linux (x86_64)**: `jsonschema-cli-x86_64-unknown-linux-gnu.tar.gz`
- **Linux (ARM64)**: `jsonschema-cli-aarch64-unknown-linux-gnu.tar.gz`
- **macOS (Intel)**: `jsonschema-cli-x86_64-apple-darwin.tar.gz`
- **macOS (Apple Silicon)**: `jsonschema-cli-aarch64-apple-darwin.tar.gz`
- **Windows**: `jsonschema-cli-x86_64-pc-windows-msvc.zip`

Example installation on Linux/macOS:
```bash
# Download and extract (replace VERSION with actual version, e.g., cli-v0.36.0)
curl -LO https://github.com/Stranger6667/jsonschema-rs/releases/download/VERSION/jsonschema-cli-x86_64-unknown-linux-gnu.tar.gz
tar xzf jsonschema-cli-x86_64-unknown-linux-gnu.tar.gz

# Move to a directory in your PATH
sudo mv jsonschema-cli /usr/local/bin/
```

### From Source (requires Rust)

```bash
cargo install jsonschema-cli
```

## Usage

```
jsonschema [OPTIONS] <SCHEMA>
```

**NOTE**: It only supports valid JSON as input.

### Options

- `-i, --instance <FILE>`: JSON instance(s) to validate (can be used multiple times)
- `--output <text|flag|list|hierarchical>`: Select output style (default: `text`). `text` prints the human-friendly summary, while the structured modes emit newline-delimited JSON (`ndjson`) records with `schema`, `instance`, and JSON Schema Output v1 payloads.
- `-v, --version`: Show version information
- `--help`: Display help information

### Examples:

Validate a single instance:
```
jsonschema schema.json -i instance.json
```

Validate multiple instances:
```
jsonschema schema.json -i instance1.json -i instance2.json
```

Emit JSON Schema Output v1 (`list`) for multiple instances:
```
jsonschema schema.json -i instance1.json -i instance2.json --output list
{"output":"list","schema":"schema.json","instance":"instance1.json","payload":{"valid":true,...}}
{"output":"list","schema":"schema.json","instance":"instance2.json","payload":{"valid":false,...}}
```

## Features

- Validate one or more JSON instances against a single schema
- Clear, concise output with detailed error reporting
- Fast validation using the `jsonschema` Rust crate

## Output

For each instance:

- `text` (default): prints `<filename> - VALID` or `<filename> - INVALID. Errors:` followed by numbered error messages.
- `flag|list|hierarchical`: emit newline-delimited JSON objects shaped as:

```json
{
  "output": "list",
  "schema": "schema.json",
  "instance": "instance.json",
  "payload": { "... JSON Schema Output v1 data ..." }
}
```

## Exit Codes

- 0: All instances are valid (or no instances provided)
- 1: One or more instances are invalid, or there was an error

## License

This project is licensed under the MIT License.

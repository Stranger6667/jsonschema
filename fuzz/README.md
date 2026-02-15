# Fuzz Targets

This directory contains `cargo-fuzz` targets for `jsonschema`.

## Prerequisites

```bash
cargo install cargo-fuzz
```

## Run a target

From repository root:

```bash
cargo +nightly fuzz run --release <target> fuzz/seeds -- -dict=fuzz/dict -max_total_time=60
```

Available targets:

- `builder`
- `validation`
- `referencing`
- `codegen_parity`

## `codegen_parity`

`codegen_parity` differentially checks generated compile-time validators against dynamic
`jsonschema::Validator` instances for the same schemas. It decodes raw fuzzer bytes as
`serde_json::Value` and asserts:

```text
codegen_is_valid(instance) == dynamic_is_valid(instance)
```

Seeds for this target live in `fuzz/seeds/codegen_parity/`.

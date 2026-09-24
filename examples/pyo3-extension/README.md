# Python extension with a compile-time validator

`schema.json` is compiled into the extension when it is built, so importing the module parses and
compiles nothing, and the validator reads Python objects in place. The extension exposes
`is_valid`, `validate` (raises `ValueError` with the first error) and `errors` (every error message),
and `"uppercase": true` shows a custom keyword written in Rust.

Build and run the checks in `test.py` with [uv](https://docs.astral.sh/uv/):

```console
$ uv run test.py
ok
```

Outside this repository, depend on the published crate instead of the `path` in `Cargo.toml`. The
macro's attributes are documented in the
[`jsonschema` crate docs](https://docs.rs/jsonschema/latest/jsonschema/#python-extension-modules).

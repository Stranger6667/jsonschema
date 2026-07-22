# jsonschema-value

JSON value representations and semantics shared by the [`jsonschema`](https://crates.io/crates/jsonschema) validator and its bindings.

This crate is an implementation detail of `jsonschema`. To validate instances in a custom JSON representation, use the re-exports in the [`jsonschema::json`](https://docs.rs/jsonschema/latest/jsonschema/json/index.html) module instead of depending on this crate directly.

# Ruby extension with a compile-time validator

`schema.json` is compiled into the extension when it is built, so loading it parses and compiles
nothing, and the validator reads Ruby objects in place. The extension defines `Events.valid?`,
`Events.validate!` (raises `Events::Error` with the first error) and `Events.errors` (every error
message), and `"uppercase": true` shows a custom keyword written in Rust.

Build and run the checks in `test.rb`:

```console
$ bundle install
$ bundle exec rake compile
$ bundle exec ruby test.rb
ok
```

Outside this repository, depend on the published crate instead of the `path` in `Cargo.toml`. The
macro's attributes are documented in the
[`jsonschema` crate docs](https://docs.rs/jsonschema/latest/jsonschema/#ruby-extension-modules).

# Changelog

## [Unreleased]

### Performance

- Extend `pattern` prefix optimization to handle escaped slashes (`^\/`) and exact-match patterns (`^\$ref$`).
- Specialize `enum` for cases when all variants are strings.

## [0.42.1] - 2026-02-17

### Performance

- Reduce dynamic dispatch overhead for non-recursive `$ref` resolution.
- Cache ECMA regex transformations during `format: "regex"` validation.

## 0.42.0 - 2026-02-15

- Initial public release

[Unreleased]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.42.1...HEAD
[0.42.1]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.42.0...ruby-v0.42.1

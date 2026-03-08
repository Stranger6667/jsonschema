# Changelog

## [Unreleased]

### Added

- `JSONSchema.bundle(schema, ...)`: produce a Compound Schema Document with all external `$ref` targets embedded in a draft-appropriate container (`definitions` for Draft 4/6/7, `$defs` for Draft 2019-09/2020-12; [Appendix B](https://json-schema.org/draft/2020-12/json-schema-core#appendix-B)). [#791](https://github.com/Stranger6667/jsonschema/issues/791).
- `ValidationError#absolute_keyword_location` to get the absolute keyword location URI of the schema node that produced the error.

## [0.44.1] - 2026-03-03

### Fixed

- `hostname` format now applies legacy RFC 1034 semantics in Draft 4/6 and keeps IDNA A-label validation in Draft 7+.

## [0.44.0] - 2026-03-02

### Added

- `Canonical::JSON.to_string(object)` for canonical JSON serialization (stable key ordering and numeric normalization), useful for deduplicating equivalent JSON Schemas.

### Fixed

- Do not produce annotations for non-string instances from `contentMediaType`, `contentEncoding`, and `contentSchema` keywords.

## [0.43.0] - 2026-02-28

### Added

- `validator_cls_for(schema)` function to detect and return the appropriate validator class for a schema.

### Fixed

- `anyOf`, `format`, `unevaluatedProperties`, and `unevaluatedItems` now correctly collect annotations per spec.

### Performance

- Optimize `pattern` and `patternProperties` for `^(a|b|c)$` alternations via linear array scan.
- Optimize `^\S*$` patterns by replacing regex with a direct ECMA-262 whitespace check.

## [0.42.2] - 2026-02-26

### Changed

- Custom keyword validation exceptions are now chained to the resulting `ValidationError` via `cause`, preserving the original exception class and message.

### Fixed

- SWAR digit parser accepted bytes `:`–`?` (0x3A–0x3F) as valid digits during `date`, `time`, and `date-time` format validation, potentially allowing malformed values to pass.

### Performance

- Extend `pattern` prefix optimization to handle escaped slashes (`^\/`) and exact-match patterns (`^\$ref$`).
- Specialize `enum` for cases when all variants are strings.

## [0.42.1] - 2026-02-17

### Performance

- Reduce dynamic dispatch overhead for non-recursive `$ref` resolution.
- Cache ECMA regex transformations during `format: "regex"` validation.

## 0.42.0 - 2026-02-15

- Initial public release

[Unreleased]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.44.1...HEAD
[0.44.1]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.44.0...ruby-v0.44.1
[0.44.0]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.43.0...ruby-v0.44.0
[0.43.0]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.42.2...ruby-v0.43.0
[0.42.2]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.42.1...ruby-v0.42.2
[0.42.1]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.42.0...ruby-v0.42.1

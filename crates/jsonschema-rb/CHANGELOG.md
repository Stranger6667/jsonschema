# Changelog

## [Unreleased]

### Fixed

- Panic in the regex engine when matching against patterns with very large `{0,N}` quantifiers.

## [0.46.3] - 2026-04-28

### Fixed

- Memory not reclaimed when a validator for a schema with recursive `$ref` or `$dynamicRef` is dropped. [#1125](https://github.com/Stranger6667/jsonschema/issues/1125)

## [0.46.2] - 2026-04-20

### Fixed

- `required` not enforced when `additionalProperties` is a schema object and `required` lists exactly 2 keys.

## [0.46.1] - 2026-04-18

### Fixed

- `required` not enforced when `properties` has 15 or more entries and `required` lists exactly 2 keys.

## [0.46.0] - 2026-04-10

### Added

- Accept JSON strings in `validator_cls_for`.
- `Resolver` and `Resolved` types for programmatic schema resolution.
- `dereference` function to recursively inline `$ref` references. [#422](https://github.com/Stranger6667/jsonschema/issues/422)
- `ValidatorMap` for validating instances against subschemas identified by URI-fragment JSON pointer. [#1075](https://github.com/Stranger6667/jsonschema/pull/1075)

### Performance

- Avoid registry clones and document clones during validator construction. This improves real-world schema compilation by roughly 10-20% in internal benchmarks.

## [0.45.1] - 2026-04-06

### Fixed

- Incorrect handling of `multipleOf` validation for negative numeric instances.
- Incorrect handling of `duration` format when hours and seconds appear without minutes, or years and days without months.

## [0.45.0] - 2026-03-08

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

[Unreleased]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.46.3...HEAD
[0.46.3]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.46.2...ruby-v0.46.3
[0.46.2]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.46.1...ruby-v0.46.2
[0.46.1]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.46.0...ruby-v0.46.1
[0.46.0]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.45.1...ruby-v0.46.0
[0.45.1]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.45.0...ruby-v0.45.1
[0.45.0]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.44.1...ruby-v0.45.0
[0.44.1]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.44.0...ruby-v0.44.1
[0.44.0]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.43.0...ruby-v0.44.0
[0.43.0]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.42.2...ruby-v0.43.0
[0.42.2]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.42.1...ruby-v0.42.2
[0.42.1]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.42.0...ruby-v0.42.1

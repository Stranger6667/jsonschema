# Changelog

## [Unreleased]

### Fixed

- `multipleOf` validation for integer values between `2^53` and `i64::MAX`.

## [0.38.0] - 2025-12-24

### Added

- `EmailOptions` for configuring `email` format validation.

### Fixed

- `multipleOf` validation for large integers beyond `i64::MAX`.
- Incorrect `mask` parameter in `evaluate` docstring.

### Removed

- `with_meta_schemas` parameter from type stubs (was removed from the API long ago).

## [0.37.4] - 2025-11-30

### Fixed

- Stack overflow during validation of schemas with circular `$ref` chains (e.g., `a` -> `b` -> `a`).
- Local `$ref` resolution within fragment-extracted external resources. [#892](https://github.com/Stranger6667/jsonschema/issues/892)

## [0.37.3] - 2025-11-28

### Fixed

- External resources not discovered within subresources of local `$ref` targets. [#892](https://github.com/Stranger6667/jsonschema/issues/892)

## [0.37.2] - 2025-11-27

### Fixed

- External resources not discovered through chained local `$ref` references. [#892](https://github.com/Stranger6667/jsonschema/issues/892)

## [0.37.1] - 2025-11-19

### Fixed

- Stack overflow on empty `$ref` value. [#886](https://github.com/Stranger6667/jsonschema/issues/886)

## [0.37.0] - 2025-11-19

### Added

- Support for `decimal.Decimal` type in both schemas and instances. [#319](https://github.com/Stranger6667/jsonschema/issues/319)
- Support for additional `contentEncoding` values per RFC 4648: `base64url`, `base32`, `base32hex`, and `base16`. These encodings are now validated alongside the existing `base64` support in Draft 6 and 7. [#26](https://github.com/Stranger6667/jsonschema/issues/26)

### Performance

- `jsonschema_rs.validate()` and `Validator.validate()` run 5–10% faster in some workloads.
- `evaluate()`: Avoiding deep clones of unmatched keyword values (e.g., `title`, `description`, `examples`) on every schema node evaluation. Can be multiple times faster for schemas with large annotations. 

## [0.36.0] - 2025-11-18

### Added

- `jsonschema_rs.evaluate()`, `Validator.evaluate()`, and the `Evaluation` type for retrieving JSON Schema Output v1 (flag/list/hierarchical) formats along with annotations and errors.
- `Validator` type alias for union of all validator types (`Draft4Validator | Draft6Validator | Draft7Validator | Draft201909Validator | Draft202012Validator`).
- `__eq__` and `__hash__` methods to `ValidationError` and `ReferencingError`.
- `__repr__` method stubs to all validator classes, `Evaluation`, `Registry`, and regex options classes.
- Docstrings to functions in type stubs (`is_valid`, `validate`, `iter_errors`, `evaluate`, `validator_for`).

### Fixed

- Missing `verbose_message` attribute in `ValidationError` type stub.
- Missing `context` field in `ValidationErrorKind.OneOfMultipleValid` type stub.
- Incorrect type for `ValidationErrorKind.MultipleOf.multiple_of` - now correctly typed as `int | float | Decimal` to support arbitrary precision numbers.

## [0.35.0] - 2025-11-16

### Added

- Support for custom meta-schemas. Schemas with custom `$schema` URIs can now be used by registering their meta-schemas via the `registry` parameter in `validator_for()`. [#664](https://github.com/Stranger6667/jsonschema/issues/664)
- `registry` parameter to `meta.is_valid()` and `meta.validate()` for validating schemas against custom meta-schemas.
- Type stubs for the `meta` module.
- PyPy 3.11 support. [#309](https://github.com/Stranger6667/jsonschema/issues/309)
- Support for arbitrary-precision numbers, including large integers and high-precision decimals that exceed standard floating-point limits. [#103](https://github.com/Stranger6667/jsonschema/issues/103)
- Support for HTTPS `$schema` URIs for drafts 04, 06, and 07 (e.g., `https://json-schema.org/draft-07/schema`). [#802](https://github.com/Stranger6667/jsonschema/issues/802)

### Changed

- **BREAKING**: `meta.is_valid()` and `meta.validate()` now raise `ReferencingError` for unknown `$schema` values instead of defaulting to Draft 2020-12. Use the `registry` parameter to validate against custom meta-schemas.
- Migrated to abi3 so a single wheel per platform works on all supported 3.10+ interpreters.

### Removed

- Support for Python 3.8 & 3.9.

### Performance

- `required`: short-circuit when the instance object has fewer properties than required keys.
- Arbitrary precision support is always enabled. Performance impact is negligible for most schemas, with ~2x slowdown only for number-heavy instances (e.g., GeoJSON).

## [0.34.0] - 2025-11-14

### Added

- Python 3.14 support.

### Changed

- Update `pyo3` to `0.27`.

### Fixed

- Hostname and IDN hostname formats now decode `xn--` labels, reject leading combining marks/uppercase prefixes, and enforce the latest JSON Schema punycode context rules.

### Performance

- Recursive and regular `$ref` compilation deduplicates validator nodes, which decreases the memory usage and improves performance.
- Validator compilation restores the regex cache for faster builds on regex-heavy schemas and precomputes absolute schema locations, trading a bit of compile time for faster `apply` on location-heavy workloads.
- Large schema compilation is significantly faster. [#755](https://github.com/Stranger6667/jsonschema/issues/755)
- `unevaluatedProperties` validation is 25-35% faster through optimized property marking and early-exit paths.
- `unevaluatedProperties` memory usage drastically reduced by eliminating redundant registry clones during compilation.
- `unevaluatedItems` validation is ~10% faster through early-exit optimizations and eliminating redundant validations in combinators.

## [0.33.0] - 2025-08-24

### Fixed

- `instance_path` segments are now unescaped when iterating. [#788](https://github.com/Stranger6667/jsonschema/issues/788)

## [0.32.1] - 2025-08-03

### Changed

- Bump `fancy-regex` to `0.16`.

## [0.32.0] - 2025-07-29

### Added

- Added missing `context` field to `ValidationErrorKind.OneOfMultipleValid`.

### Changed

- Improved error message for `enum`.

## [0.31.0] - 2025-07-28

### Added

- Added support for old and new style string enums in object keys.
- Added `context` for `ValidationErrorKind.AnyOf` and `ValidationErrorKind.OneOfNotValid` which contains errors for all subschemas, each inside a separate list with an index matching subschema ID.

### Changed

- Update `pyo3` to `0.25`.

### Fixed

- Improve the precision of `multipleOf` for float values.

## [0.30.0] - 2025-04-16

### Added

- `base_uri` keyword argument that allows for specifying a base URI for all relative references in the schema.
- Configuration options for the underlying regex engine used by `pattern` and `patternProperties` keywords.

### Changed

- Better error messages for relative `$ref` without base URI.
- Update `pyo3` to `0.24`.

### Fixed

- Added Registry to type declarations.

### Removed

- Internal cache for regular expressions.

## [0.29.1] - 2025-02-08

### Packaging

- Fix missing source distribution.

## [0.29.0] - 2025-02-08

### Added

- Added `Registry` class for schema reuse and reference resolution.

### Performance

- Significantly improved validator compilation speed by using pointer-based references to schema fragments instead of cloning them during traversal.

## [0.28.3] - 2025-01-24

### Fixed

- Panic when schema registry base URI contains an unencoded fragment.

### Performance

- Fewer JSON pointer lookups.

## [0.28.2] - 2025-01-22

### Fixed

- Resolving external references that nested inside local references. [#671](https://github.com/Stranger6667/jsonschema/issues/671)
- Resolving relative references with fragments against base URIs that also contain fragments. [#666](https://github.com/Stranger6667/jsonschema/issues/666)

### Performance

- Faster JSON pointer resolution.

## [0.28.1] - 2024-12-31

### Fixed

- Handle fragment references within `$id`-anchored subschemas. [#640](https://github.com/Stranger6667/jsonschema/issues/640)

## [0.28.0] - 2024-12-29

### Added

- Meta-schema validation support. [#442](https://github.com/Stranger6667/jsonschema/issues/442)

## [0.27.1] - 2024-12-24

### Added

- Extend `ValidationErrorKind` with error-specific context data.
- Missing type annotations for `retriever` & `mask` arguments.

## [0.27.0] - 2024-12-23

### Added

- Custom retrievers for external references. [#372](https://github.com/Stranger6667/jsonschema/issues/372)
- Added the `mask` argument to validators for hiding sensitive data in error messages. [#434](https://github.com/Stranger6667/jsonschema/issues/434)
- Added `ValidationError.kind` and `ValidationError.instance` attributes. [#650](https://github.com/Stranger6667/jsonschema/issues/650)

### Changed

- Improved error message for unknown formats.
- Update `pyo3` to `0.23`.

## [0.26.1] - 2024-10-29

### Fixed

- Return "Unknown specification" error on `https`-prefixed `$schema` for Draft 4, 5, 6. [#629](https://github.com/Stranger6667/jsonschema/issues/629)

### Performance

- Speedup Python -> Rust data serialization.
- Speedup building error messages.

## [0.26.0] - 2024-10-26

### Performance

- Optimize error formatting in some cases.
- Speedup Python -> Rust data serialization.
- Significant improvement for the `validate` function.

## [0.25.1] - 2024-10-25

### Changed

- Schema representation inside `__repr__`.

### Fixed

- Panic on initializing a validator. [#618](https://github.com/Stranger6667/jsonschema/issues/618)

## [0.25.0] - 2024-10-24

**Important:** This release removes deprecated old APIs. See the [Migration Guide](MIGRATION.md) for details on transitioning to the new API.

### Changed

- **BREAKING**: Default to Draft 2020-12.

### Removed

- Deprecated `JSONSchema` class.
- Deprecated `with_meta_schemas` argument in multiple functions.

## [0.24.3] - 2024-10-24

### Fixed

- Infinite recursion when using mutually recursive `$ref` in `unevaluatedProperties`.

## [0.24.2] - 2024-10-24

### Fixed

- Infinite recursion in some cases. [#146](https://github.com/Stranger6667/jsonschema/issues/146)
- `$ref` interaction with `$recursiveAnchor` in Draft 2019-09.
- `unevaluatedProperties` with `$recursiveRef` & `$dynamicRef`.

## [0.24.1] - 2024-10-22

### Fixed

- Incomplete external reference resolution.

## [0.24.0] - 2024-10-20

### Added

- Support `$ref`, `$recursiveRef`, and `$dynamicRef` in `unevaluatedItems`. [#287](https://github.com/Stranger6667/jsonschema/issues/287)
- Support for `$vocabulary`. [#263](https://github.com/Stranger6667/jsonschema/issues/263)

### Changed

- Ignore `prefixItems` under Draft 2019-09 as it was introduced in Draft 2020-12.

### Fixed

- Numbers with zero fraction incorrectly handled in `uniqueItems`.

## [0.23.0] - 2024-10-12

### Added

- Partial support for `unevaluatedItems`, excluding references.
- `validate_formats` argument to all validator classes and validation functions. This allows overriding the draft-specific default behavior for format validation.
- `ignore_unknown_formats` argument to all validator classes and validation functions. When set to `False`, unrecognized formats will be reported as validation errors instead of being silently ignored.
- Python 3.13 support.

### Changed

- Improve error messages on URI resolving and parsing.

### Fixed

- Resolving file references on Windows. [#441](https://github.com/Stranger6667/jsonschema/issues/441)

### Performance

- Faster building of a validator.
- Speedup `hostname` & `idn-hostname` formats validation.

## [0.22.3] - 2024-10-05

### Performance

- Speedup resolving.

## [0.22.2] - 2024-10-04

### Fixed

- ECMAScript 262 regex support.

### Performance

- Speedup `json-pointer` and `relative-json-pointer` formats validation.

## [0.22.1] - 2024-10-03

### Fixed

- Removed `dbg!` macro.

## [0.22.0] - 2024-10-03

### Changed

- Extend email validation. [#471](https://github.com/Stranger6667/jsonschema/issues/471)

### Fixed

- `time` format validation (leap seconds and second fractions).
- `duration` format validation.
- Panic on root `$id` without base. [#547](https://github.com/Stranger6667/jsonschema/issues/547)
- `hostname` format validation (double dot).
- `idn-hostname` format validation. [#101](https://github.com/Stranger6667/jsonschema/issues/101)

### Performance

- Faster building of a validator.
- `hostname`, `date`, `time`, `date-time`, and `duration` formats validation.
- Cache regular expressions for `pattern`. [#417](https://github.com/Stranger6667/jsonschema/issues/417)

## [0.21.0] - 2024-09-30

### Added

- `$anchor` support.
- `$recursiveRef` & `$recursiveAnchor` support in Draft 2019-09.
- `$dynamicRef` & `$dynamicAnchor` support in Draft 2020-12.

### Changed

- **BREAKING**: Treat `$ref` as URI, not URL, and additionally normalize them. [#454](https://github.com/Stranger6667/jsonschema/issues/454)
- **BREAKING**: Resolve all non-recursive references eagerly.
- **BREAKING**: Disallow use of fragments in `$id`. [#264](https://github.com/Stranger6667/jsonschema/issues/264)

### Fixed

- Infinite recursion in `unevaluatedProperties`. [#420](https://github.com/Stranger6667/jsonschema/issues/420)
- Cross-draft validation from newer to older ones.
- Changing base URI in folder.
- Location-independent identifier in remote resource.
- Missing some format validation for Draft 2020-12.
- Incomplete `iri` & `iri-reference` validation.

### Performance

- Faster validation for `uri`, `iri`, `uri-reference`, and `iri-reference` formats.

## [0.20.0] - 2024-09-18

### Added

- New draft-specific validator classes: `Draft4Validator`, `Draft6Validator`, `Draft7Validator`, `Draft201909Validator`, and `Draft202012Validator`.
- `validator_for` function for automatic draft detection.

### Changed

- The `JSONSchema` class has been renamed to `Validator`. The old name is retained for backward compatibility but will be removed in a future release.

### Deprecated

- The `JSONSchema` class is deprecated. Use the `validator_for` function or draft-specific validators instead.
  You can use `validator_for` instead of `JSONSchema.from_str`.
- Constants `jsonschema_rs.DRAFT4`, `jsonschema_rs.DRAFT6`, `jsonschema_rs.DRAFT7`, `jsonschema_rs.DRAFT201909`, and `jsonschema_rs.DRAFT202012` are deprecated in favor of draft-specific validator classes.

### Fixed

- Location-independent references in remote schemas on drafts 4, 6, and 7.

## [0.19.1] - 2024-09-15

### Fixed

- `ipv4` format validation. [#512](https://github.com/Stranger6667/jsonschema/issues/512)

## [0.19.0] - 2024-09-14

### Fixed

- `uuid` format validation.
- Combination of `unevaluatedProperties` with `allOf` and `oneOf`. [#496](https://github.com/Stranger6667/jsonschema/issues/496)

### Performance

- `uuid` validation.

### Removed

- Support for Python 3.7.

## [0.18.3] - 2024-09-12

### Fixed

- Changing base URI when `$ref` is present in drafts 7 and earlier.

## [0.18.2] - 2024-09-11

### Fixed

- Ignoring ``$schema`` in resolved references.
- Support integer-valued numbers for `maxItems`, `maxLength`, `maxProperties`, `maxContains`, `minItems`, `minLength`, `minProperties`, `minContains`.

### Deprecated

- `with_meta_schemas` argument. Meta schemas are included by default.

## [0.18.1] - 2024-08-24

### Changed

- Update `pyo3` to `0.22`.

## [0.18.0] - 2024-05-07

### Added

- Defining custom format checkers. [#245](https://github.com/Stranger6667/jsonschema/issues/245)

### Changed

- Update `pyo3` to `0.21`.

### Fixed

- Incorrect `schema_path` when multiple errors coming from the `$ref` keyword [#426](https://github.com/Stranger6667/jsonschema/issues/426)

## [0.17.3] - 2024-03-22

### Added

- Support subclasses of Python `dict`s [#427](https://github.com/Stranger6667/jsonschema/issues/427)

## [0.17.2] - 2024-03-03

### Added

- Support for Python 3.12 [#439](https://github.com/Stranger6667/jsonschema/issues/439)

### Changed

- Expose drafts 2019-09 and 2020-12 to Python
- Update `pyo3` to `0.20`.

## [0.17.1] - 2023-07-05

### Changed

- Update `pyo3` to `0.19`.
- Improved error messages for `oneOf` / `anyOf` keywords. [#429](https://github.com/Stranger6667/jsonschema/issues/429)

## [0.16.3] - 2023-02-01

### Added

- Build wheels for Linux(glibc) x86_64/i686, Windows x64/x86, and macOS x86_64/aarch64.

### Changed

- Update `pyo3` to `0.18`.

## [0.16.2] - 2023-01-14

### Added

- Support for Python 3.11

## [0.16.1] - 2022-10-20

### Changed

- Raise `ValueError` on validating dicts with non-string keys. [#386](https://github.com/Stranger6667/jsonschema/issues/386)
- Update `pyo3` to `0.17`.

## [0.16.0] - 2022-05-12

### Added

- Python 3.10 support

### Fixed

- Installation error due to `pyo3-built` incompatibility
- Memory leak in `iter_errors`. [#325](https://github.com/Stranger6667/jsonschema/issues/325)

### Changed

- Update `pyo3` to `0.16`.

### Removed

- Support for Python 3.6

## [0.14.0] - 2022-01-31

### Added

- Support for resolving external schema files. [#76](https://github.com/Stranger6667/jsonschema/issues/76)

### Changed

- Update `pyo3` to `0.15`.

## [0.13.1] - 2021-11-10

### Added

- Convert `Enum` into raw values before validating.

## [0.13.0] - 2021-11-04

### Added

- `JSONSchema.from_str` method that accepts a string to construct a compiled schema.
  Useful if you have a schema as string, because you don't have to call `json.loads` on your side - parsing will happen on the Rust side.

### Fixed

- Set `jsonschema_rs.JSONSchema.__module__` to `jsonschema_rs`.
- Convert tuples into lists for validation to fix `ValueError: Unsupported type: 'tuple'`.

### Performance

- Minor performance improvements.

## [0.12.3] - 2021-10-22

### Added

- `iter_errors` to iterate all errors. [#236](https://github.com/Stranger6667/jsonschema/issues/236)

## [0.12.2] - 2021-10-21

### Fixed

- Display the original value in errors from `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`. [#215](https://github.com/Stranger6667/jsonschema/issues/215)
- Switch from `chrono` to `time==0.3.3` due to [RUSTSEC-2020-0159](https://rustsec.org/advisories/RUSTSEC-2020-0159.html) in older `time` versions that `chrono` depends on.

## [0.12.1] - 2021-07-29

### Fixed

- Allow using empty arrays or arrays with non-unique elements for the `enum` keyword in schemas. [#258](https://github.com/Stranger6667/jsonschema/issues/258)
- Inaccurate schema path in validation error messages. [#257](https://github.com/Stranger6667/jsonschema/issues/257)
- Panic on incomplete escape sequences in regex patterns. [#253](https://github.com/Stranger6667/jsonschema/issues/253)

## [0.12.0] - 2021-07-24

### Changed

- Pre-compute `JSONSchema` representation.

## [0.11.1] - 2021-07-06

### Added

- Additional attributes to `ValidationError`. They are `message`, `schema_path` and `instance_path`. [#197](https://github.com/Stranger6667/jsonschema/issues/197)

### Changed

- Update `pyo3` to `0.14.1`.

## [0.11.0] - 2021-06-19

### Added

- Report schema paths in validation errors. At the moment, it only displayed in the `ValidationError` message. [#199](https://github.com/Stranger6667/jsonschema/issues/199)

## [0.10.0] - 2021-06-17

### Added

- Meta-schema validation for input schemas. [#198](https://github.com/Stranger6667/jsonschema/issues/198)

## [0.9.1] - 2021-06-17

### Fixed

- The `format` validator incorrectly rejecting supported regex patterns. [#230](https://github.com/Stranger6667/jsonschema/issues/230)

## [0.9.0] - 2021-05-07

### Added

- Support for look-around patterns. [#183](https://github.com/Stranger6667/jsonschema/issues/183)

### Fixed

- Extend the `email` format validation. Relevant test case from the JSONSchema test suite - `email.json`.

## [0.8.0] - 2021-05-05

### Changed

- Error messages show paths to the erroneous part of the input instance. [#144](https://github.com/Stranger6667/jsonschema/issues/144)

### Fixed

- Skipped validation on an unsupported regular expression in `patternProperties`. [#213](https://github.com/Stranger6667/jsonschema/issues/213)
- Missing `array` type in error messages for `type` validators containing multiple values. [#216](https://github.com/Stranger6667/jsonschema/issues/216)

## [0.6.2] - 2021-05-03

## Changed

- Update `PyO3` to `0.13.x`.
- Improved error message for the `additionalProperties` validator. After - `Additional properties are not allowed ('faz' was unexpected)`, before - `False schema does not allow '"faz"'`.
- The `additionalProperties` validator emits a single error for all unexpected properties instead of separate errors for each unexpected property.

### Fixed

- Floating point overflow in the `multipleOf` validator. Relevant test case from the JSONSchema test suite - `float_overflow.json`

### Performance

- Various performance improvements from the underlying Rust crate.

## [0.6.1] - 2021-03-26

### Fixed

- Incorrect handling of `\w` and `\W` character groups in `pattern` keywords. [#180](https://github.com/Stranger6667/jsonschema/issues/180)
- Incorrect handling of strings that contain escaped character groups (like `\\w`) in `pattern` keywords.

## [0.6.0] - 2021-02-03

### Added

- `with_meta_schemas` argument for `is_valid` and update docstrings.
- `validate` function.

### Performance

- General performance improvements for subsets of `items` and `additionalProperties` validators.
- Defer schema & instance loading until they are used. It improves performance for cases when the user passes an nvalid draft version.

## [0.5.1] - 2021-01-29

### Changed

- Exclude unnecessary files from source code distribution.

## [0.5.0] - 2021-01-29

### Added

- Cache for documents loaded via the `$ref` keyword. [#75](https://github.com/Stranger6667/jsonschema/issues/75)
- Meta schemas for JSON Schema drafts 4, 6, and 7. [#28](https://github.com/Stranger6667/jsonschema/issues/28)

### Fixed

- Not necessary network requests for schemas with `$id` values with trailing `#` symbol. [#163](https://github.com/Stranger6667/jsonschema/issues/163)
- Source code distribution. It was missing the source code for the underlying Rust crate and were leading to
  a build error during `pip install jsonschema_rs` on platforms that we don't have wheels for.
  [#159](https://github.com/Stranger6667/jsonschema/issues/159)

### Performance

- Enum validation for input values that have a type that is not present among the enum variants. [#80](https://github.com/Stranger6667/jsonschema/issues/80)

## [0.4.3] - 2020-12-15

### Changed

- Exclude the `cli` dependency from the `jsonschema` crate & update dependencies in `Cargo.lock`.

## [0.4.2] - 2020-12-11

### Fixed

- Number comparison for `enum` and `const` keywords. [#149](https://github.com/Stranger6667/jsonschema/issues/149)
- Do not accept `date` strings with single-digit month and day values. [#151](https://github.com/Stranger6667/jsonschema/issues/151)

## [0.4.1] - 2020-12-09

### Fixed

- Integers not recognized as numbers when the `type` keyword is a list of multiple values. [#147](https://github.com/Stranger6667/jsonschema/issues/147)

## [0.4.0] - 2020-11-09

### Added

- Python 3.9 support.

### Changed

- Remove not needed `__init__.py` file. It improves performance for compiled schemas. [#121](https://github.com/Stranger6667/jsonschema/issues/121)
- Update `PyO3` to `0.12`. [#125](https://github.com/Stranger6667/jsonschema/issues/125)
- Use stable Rust.
- Set module documentation only once.

### Fixed

- ECMAScript regex support
- Formats should be associated to Draft versions (ie. `idn-hostname` is not defined on draft 4 and draft 6)
- Handle errors during conversion to `Value` instead of using `unwrap` in `JSONSchema::is_valid` and `JSONSchema::validate`. [#127](https://github.com/Stranger6667/jsonschema/issues/127)

### Removed

- Python 3.5 support.

## [0.3.3] - 2020-06-22

### Fixed

- `items` allows the presence of boolean schemas. [#115](https://github.com/Stranger6667/jsonschema/pull/115)

## [0.3.2] - 2020-06-13

### Fixed

- Packaging issue.

## [0.3.1] - 2020-06-12

### Added

- Added `jsonschema_rs.__build__` which contains useful build information. [#111](https://github.com/Stranger6667/jsonschema/pulls/111)
- Wheels for Mac OS and Windows. [#110](https://github.com/Stranger6667/jsonschema/issues/110)

### Changed

- Linux wheels are `manylinux2014` compatible. Previously they were `manylinux2010` compatible. [#111](https://github.com/Stranger6667/jsonschema/pulls/111)

## [0.3.0] - 2020-06-11

### Fixed

- Copying not needed compiled files to the wheel distribution files. [#109](https://github.com/Stranger6667/jsonschema/issues/109)

## [0.2.0] - 2020-06-11

### Added

- `JSONSchema.validate` method that raises `ValidationError` for invalid input. [#105](https://github.com/Stranger6667/jsonschema/issues/105)

### Changed

- Public functions docstrings to support PyCharm skeletons generation. Functions signatures now have proper signatures (but untyped) in PyCharm. [#107](https://github.com/Stranger6667/jsonschema/issues/107)
- Enable Link-Time Optimizations and set `codegen-units` to 1. [#104](https://github.com/Stranger6667/jsonschema/issues/104)

## 0.1.0 - 2020-06-09
- Initial public release

[Unreleased]: https://github.com/Stranger6667/jsonschema/compare/python-v0.38.0...HEAD
[0.38.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.37.4...python-v0.38.0
[0.37.4]: https://github.com/Stranger6667/jsonschema/compare/python-v0.37.3...python-v0.37.4
[0.37.3]: https://github.com/Stranger6667/jsonschema/compare/python-v0.37.2...python-v0.37.3
[0.37.2]: https://github.com/Stranger6667/jsonschema/compare/python-v0.37.1...python-v0.37.2
[0.37.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.37.0...python-v0.37.1
[0.37.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.36.0...python-v0.37.0
[0.36.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.35.0...python-v0.36.0
[0.35.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.34.0...python-v0.35.0
[0.34.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.33.0...python-v0.34.0
[0.33.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.32.1...python-v0.33.0
[0.32.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.32.0...python-v0.32.1
[0.32.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.31.0...python-v0.32.0
[0.31.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.30.0...python-v0.31.0
[0.30.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.29.1...python-v0.30.0
[0.29.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.29.0...python-v0.29.1
[0.29.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.28.3...python-v0.29.0
[0.28.3]: https://github.com/Stranger6667/jsonschema/compare/python-v0.28.2...python-v0.28.3
[0.28.2]: https://github.com/Stranger6667/jsonschema/compare/python-v0.28.1...python-v0.28.2
[0.28.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.28.0...python-v0.28.1
[0.28.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.27.1...python-v0.28.0
[0.27.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.27.0...python-v0.27.1
[0.27.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.26.1...python-v0.27.0
[0.26.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.26.0...python-v0.26.1
[0.26.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.25.1...python-v0.26.0
[0.25.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.25.0...python-v0.25.1
[0.25.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.24.3...python-v0.25.0
[0.24.3]: https://github.com/Stranger6667/jsonschema/compare/python-v0.24.2...python-v0.24.3
[0.24.2]: https://github.com/Stranger6667/jsonschema/compare/python-v0.24.1...python-v0.24.2
[0.24.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.24.0...python-v0.24.1
[0.24.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.23.0...python-v0.24.0
[0.23.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.22.3...python-v0.23.0
[0.22.3]: https://github.com/Stranger6667/jsonschema/compare/python-v0.22.2...python-v0.22.3
[0.22.2]: https://github.com/Stranger6667/jsonschema/compare/python-v0.22.1...python-v0.22.2
[0.22.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.22.0...python-v0.22.1
[0.22.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.21.0...python-v0.22.0
[0.21.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.20.0...python-v0.21.0
[0.20.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.19.1...python-v0.20.0
[0.19.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.19.0...python-v0.19.1
[0.19.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.18.3...python-v0.19.0
[0.18.3]: https://github.com/Stranger6667/jsonschema/compare/python-v0.18.2...python-v0.18.3
[0.18.2]: https://github.com/Stranger6667/jsonschema/compare/python-v0.18.1...python-v0.18.2
[0.18.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.18.0...python-v0.18.1
[0.18.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.17.3...python-v0.18.0
[0.17.3]: https://github.com/Stranger6667/jsonschema/compare/python-v0.17.2...python-v0.17.3
[0.17.2]: https://github.com/Stranger6667/jsonschema/compare/python-v0.17.1...python-v0.17.2
[0.17.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.16.3...python-v0.17.1
[0.16.3]: https://github.com/Stranger6667/jsonschema/compare/python-v0.16.2...python-v0.16.3
[0.16.2]: https://github.com/Stranger6667/jsonschema/compare/python-v0.16.1...python-v0.16.2
[0.16.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.16.0...python-v0.16.1
[0.16.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.14.0...python-v0.16.0
[0.14.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.13.1...python-v0.14.0
[0.13.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.13.0...python-v0.13.1
[0.13.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.12.1...python-v0.13.0
[0.12.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.12.0...python-v0.12.1
[0.12.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.11.1...python-v0.12.0
[0.11.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.11.0...python-v0.11.1
[0.11.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.10.0...python-v0.11.0
[0.10.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.9.1...python-v0.10.0
[0.9.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.9.0...python-v0.9.1
[0.9.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.8.0...python-v0.9.0
[0.8.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.6.2...python-v0.8.0
[0.6.2]: https://github.com/Stranger6667/jsonschema/compare/python-v0.6.1...python-v0.6.2
[0.6.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.6.0...python-v0.6.1
[0.6.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.5.1...python-v0.6.0
[0.5.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.5.0...python-v0.5.1
[0.5.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.4.3...python-v0.5.0
[0.4.3]: https://github.com/Stranger6667/jsonschema/compare/python-v0.4.2...python-v0.4.3
[0.4.2]: https://github.com/Stranger6667/jsonschema/compare/python-v0.4.1...python-v0.4.2
[0.4.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.4.0...python-v0.4.1
[0.4.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.3.3...python-v0.4.0
[0.3.3]: https://github.com/Stranger6667/jsonschema/compare/python-v0.3.2...python-v0.3.3
[0.3.2]: https://github.com/Stranger6667/jsonschema/compare/python-v0.3.1...python-v0.3.2
[0.3.1]: https://github.com/Stranger6667/jsonschema/compare/python-v0.3.0...python-v0.3.1
[0.3.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.2.0...python-v0.3.0
[0.2.0]: https://github.com/Stranger6667/jsonschema/compare/python-v0.1.0...python-v0.2.0

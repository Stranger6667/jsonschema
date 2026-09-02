# Changelog

## [Unreleased]

### Fixed

- Draft 4 `type: integer` matching a `Float` or not depending on its formatting; a `Float` is never a draft 4 integer.

### Performance

- Draft 4 `type: integer` is up to 6x faster on float-heavy instances.
- `evaluate` renders each node's locations once instead of rebuilding them.
- Building a schema resolves each `$ref` target once instead of per reference site.
- Building a schema with an `$id` encodes each absolute location once instead of re-validating it.
- `enum` listing strings or integers, with or without `null`, matches against a set instead of comparing each candidate.

## [0.52.1] - 2026-08-30

### Performance

- `minLength` and `maxLength` together now scan the string once.
- `unevaluatedProperties` is up to 2.9x faster.
- `evaluate` is up to 27% faster on reference-heavy schemas.

## [0.52.0] - 2026-08-27

### Added

- The `offline:` keyword argument, refusing to fetch references outside `registry:`.

### Fixed

- Canonicalization not idempotent where a `patternProperties` `$ref` matches a key `properties` names.
- The `draft:` keyword argument and the `Draft201909Validator` / `Draft202012Validator` classes gating keywords on the vocabularies of the `$schema` they override, so the validator accepted every instance.
- `valid?` disagreeing with `validate!`, `each_error`, and `evaluate` on a schema that reaches its own `$ref` twice, one of them under `not`.

## [0.51.0] - 2026-08-23

### Added

- `vocabularies` option, declaring support for a vocabulary this library does not implement.

### Fixed

- The `uniqueItems` length ceiling not reading an `items` or `contains` schema written as a `$ref`.
- The Draft 2020-12 Format-Assertion vocabulary read as an annotation, so a meta-schema requiring it accepted values its `format` rejects.
- An unrecognized `format` accepted under a meta-schema requiring the Format-Assertion vocabulary (it should be rejected).
- A meta-schema requiring a vocabulary this library does not implement accepted (it should be rejected).
- The Draft 2019-09 Format vocabulary declared `true` read as an annotation, so a meta-schema requiring it accepted values its `format` rejects.
- The bundled `https://json-schema.org/draft/2020-12/meta/format-assertion` meta-schema not declaring its vocabulary, so a schema written against it annotated `format` instead of asserting it.
- A meta-schema without `$id` keeping its `$vocabulary` unread, so a schema written against it got every Draft 2020-12 vocabulary.
- A schema naming a bundled vocabulary meta-schema as its `$schema` rejected as an unknown meta-schema (it should build).
- The `regex` format reading the Rust regex dialect (it should read the ECMA-262 dialect in Unicode mode).
- The `uri-template` format rejecting an apostrophe in a literal (it should accept it, per RFC 6570 errata 6937).
- The `duration` format accepting weeks combined with a time part, such as `P1WT1H` (it should reject it).
- Stack overflow compiling `unevaluatedProperties` or `unevaluatedItems` beside a `$ref` that cycles back to the node, written as `#`, through its `$id`, or through a chain of definitions.

### Performance

- Building `unevaluatedProperties` or `unevaluatedItems` over a deep `allOf` or `$ref` chain costs its depth instead of its square.

## [0.50.1] - 2026-08-22

### Changed

- `CanonicalSchema#satisfiability` answers `:yes` for a string whose `pattern` or `format` a matching value can be built from.
- `CanonicalSchema#satisfiability` answers `:no` where a `format` takes no string of the length the schema asks for.

### Fixed

- `CanonicalSchema#negate` taking apart a union that reads a definition through its own `if`, so a schema and its negation shared a value.
- `CanonicalSchema#subtract` dropping a Draft 4 array member an element demand partly takes.
- `CanonicalSchema#covers` answering `Yes` where a Draft 4 element demand refuses a member.
- A string schema whose `maxLength` is `0` keeping a `pattern`, `format`, or content facet unread, so the same constraints written in one object and written as an `allOf` reached different canonical forms.

### Performance

- `not` over a wide object schema costs its branch count instead of its square.
- `not` over an object schema is linear in its property count, not quadratic.
- `not` over an object schema with `additionalProperties: false` no longer costs cubic time in its property count.

## [0.50.0] - 2026-08-20

### Added

- `CanonicalSchema#union` and `CanonicalSchema#subtract`.
- `JSONSchema::Canonical::UnsupportedResult`, raised where the canonical form does not support a set operation's result.
- `JSONSchema::Canonical::IncompatibleOperands` where both operands read `#` and it names a different document on each side.
- `JSONSchema::Canonical::Containment`, `Satisfiability`, `Distinctness` and `Kind`: named constants for the symbols `covers`, `satisfiability`, `kind` and `ArrayView#distinctness` return, each with an `ALL` list.

### Changed

- `CanonicalSchema#satisfiable?` is now `CanonicalSchema#satisfiability`, answering `:yes`, `:no`, or `:unknown` - `:yes` wherever a value can be exhibited, not only for the forms listing their members.
- `CanonicalSchema#covers` decides through the difference as well: `:no` where the argument keeps values the receiver rejects, `:yes` where nothing is left over.
- `CanonicalSchema#is_subset_of` is now `CanonicalSchema#covers`, answering `:yes`, `:no`, or `:unknown` for whether the receiver admits every value the argument admits.
- `CanonicalSchema#negate` raises where it used to return `nil`.
- `JSONSchema::Canonical::UnmodeledOperand` is now `JSONSchema::Canonical::UnsupportedOperand`, and means only that an operand is a `Raw` pass-through.

### Fixed

- Combining nodes of two different documents repointing a `$ref` to `#` at the combined result instead of the document it was written in, including one named by a definition the result keeps.
- Intersecting a `$ref` whose target is `true` or `false`, which crashed the interpreter.
- A key constraint left un-narrowed once a run is out of intersections, which crashed the interpreter on the next read of it.
- `CanonicalSchema#covers` answering `:unknown` for a schema against itself, where that schema is a `$ref`.
- `CanonicalSchema#union` keeping a `$ref` beside the schema it names, where the other three operations read through it.
- `CanonicalSchema#covers` and `CanonicalSchema#subtract` cancelling two nodes written the same way whose `#` names a different document.
- Two results accepting the same values comparing unequal over the part of their documents neither reads.
- `additionalProperties` reaching a key the pattern map matches when a finite key constraint closes the map, which dropped values both operands accept.
- A recursive definition stopping every other pointer in the document from being read through.
- One `$defs` entry read past a wider schema written out at every use, rather than kept as the pointer it was.
- Set operations rejecting one document canonicalized twice, and a pruned result rejected against the document it came from, where the maps resolve every shared reference the same way.
- Set operations comparing a `$ref` as a pointer instead of reading through it, so a schema written with a `$ref` did not cancel against the same schema written out.
- `CanonicalSchema#subtract` asking for a complement where the difference is one of the operands or empty, declining on schemas it can subtract.
- Set operations keeping `$defs` entries the result no longer references, which then showed up in the emitted schema.
- `CanonicalSchema#union` declining over an approximated intersection, where the union itself is exact.

## [0.49.9] - 2026-08-10

### Added

- Canonicalization of a `oneOf` whose branches name object targets a required constant tells apart, which degrades to a union.

### Changed

- `CanonicalSchema#to_json_schema` on a definition emits only the definitions that one names, not the whole document's.

### Fixed

- Canonicalization running without end on a conjunction over unions; past a ceiling on the meets it takes, the document stays unmodeled.

## [0.49.8] - 2026-08-08

### Performance

- Up to 380x faster canonicalization of a `oneOf` whose overlapping branches carry many properties, which no longer removes shared regions the exactly-one spelling discards.

### Fixed

- Draft detection treating the version-less `http://json-schema.org/schema` meta-schema URI as a custom dialect, where it names the current draft.
- A `patternProperties` entry matching every key leaving `additionalProperties: false` spelled as a key constraint, where it forbids nothing.

## [0.49.7] - 2026-08-08

### Added

- `retriever`, `registry`, and `base_uri` keyword arguments to `JSONSchema.canonicalize`.
- Canonicalization of a `oneOf` whose branches name disjoint targets, which degrades to a union.
- Canonicalization of a vacuous `patternProperties` entry beside schema-valued `additionalProperties`, where matching keys escape its value constraint.
- Canonicalization of a Draft 4 closed pattern map with a reference nested under a property.
- Canonicalization of Draft 4 closed pattern maps that meet through an applicator.
- The complement of a reference back to a target already being negated, which stays symbolic instead of declining.
- `CanonicalSchema#definition` resolving `#` to the document the schema was read against.

### Performance

- Up to 150x faster canonicalization of `not` over a union of many branches.
- 2% faster emission of canonical schemas, which no longer formats copied string values.
- 5% faster emission of canonical schemas, which no longer formats object keys while copying them.
- Up to 12% faster canonicalization of schemas that repeatedly intersect the same branches.
- 13% faster canonicalization of a non-dynamic OpenAPI document, which no longer scans every schema object for dynamic references.
- Faster canonicalization of schemas with local `$defs` references, which avoid decoding unescaped definition names.
- 23% faster canonicalization of an object whose keys a finite property-name set spells.
- 3% faster intersection of schemas where one side constrains nothing.
- Up to 31x faster canonicalization of a `oneOf` whose many branches overlap.
- Up to 14x faster canonicalization of a union whose object branches share no values.
- 7% faster canonicalization of schemas that reach the same pair of nodes repeatedly.
- 5% faster canonicalization of objects, whose property maps no longer reach the allocator.
- Up to 8% faster canonicalization of schemas that reach `true` and `false` subschemas repeatedly.
- 2% faster canonicalization of schemas that assert known string formats.

### Fixed

- `CanonicalSchema#definition` and `CanonicalSchema#definitions` handing out a target that names the document root without that document, where `#` points at the target instead.
- An `items` tail beyond an array's length ceiling surviving canonicalization, where it governs no element.
- `CanonicalSchema#to_json_schema` leaving the document root out of a node emitted below it, where `#` points at that node instead.

## [0.49.6] - 2026-08-06

### Added

- Canonicalization of a negated `uniqueItems`, where the complement demands a repeated element under the length floor two elements imply.
- Canonicalization of a negated array tuple, where each position's complement stands under the length that reaches it.
- Canonicalization of `contains` demands sharing no value, where their counts add up into a length floor.
- `CanonicalSchema#negate` through references, where the complement of the resolved target takes the reference's place.
- Canonicalization of negated `propertyNames` and `additionalProperties`, where the complement spells the violating-key demand instead of a `not` residual.
- Canonicalization of negated `additionalProperties` under Draft 4, where the violating-key demand spells the closed property map.
- Canonicalization of `not` a reference, where the complement of the resolved target takes the pointer's place.
- Canonicalization of a negated `oneOf`, where the complement spells the values no branch admits beside the values two branches share.
- Canonicalization of negated `items` under Draft 4, where the violating-element demand spells the barred element schema.

### Changed

- `ArrayView` reports distinctness in three states, so an array demanding a repeated element reads apart from one demanding distinct elements.

### Fixed

- `CanonicalSchema#negate` keeping a root self-reference in the complement, where it points at the complement instead of the source.
- An `additionalItems` that tails no tuple keeping the whole document unmodeled under Draft 2020-12.

## [0.49.5] - 2026-08-05

### Added

- Canonicalization of `not` an integer schema under Draft 4, which the number leaf carries as barred integers.
- Canonicalization of `not` a `multipleOf`, which numeric leaves carry as barred divisors.
- Canonicalization of `not` an integer schema, where a barred divisor of one spells the non-integer numbers.
- Canonicalization of `not` a `pattern`, which string leaves carry as barred patterns.
- Canonicalization of `not` a typed value set under Draft 4, such as `{"type": "integer", "enum": [1, 2]}`.
- `CanonicalSchema#is_subset_of`, whether the other schema admits every value this one admits.
- Canonicalization of a reference cycle carrying no assertion, which admits every value.

### Fixed

- `CanonicalSchema#is_subset_of` failing to prove a union of array branches a subset of itself.
- Two spellings of one number canonicalizing to different texts when the fraction exceeds the expansion cap.
- Numeric bounds and `multipleOf` comparing values through a lossy `f64` conversion, such as `1e-400` passing `{"maximum": 0}`.

## [0.49.4] - 2026-08-04

### Added

- Canonicalization of `not` a string format, which the string leaf carries as barred formats.
- Canonicalization of `not` an existential demand, which an array fails exactly when no element matches.
- Canonicalization of `not` a string value set, which the string leaf carries as excluded values.
- Canonicalization of `unevaluatedItems` beside `contains`, where the elements it matches are evaluated and the tail admits either.
- Canonicalization of `unevaluated*` beside `anyOf` or `oneOf`, where every branch evaluating the same keys or indexes pins what is left over.
- Canonicalization of `not` an array element schema, which an array fails exactly when one element violates it.
- `CanonicalSchema#intersect`, the values both schemas admit.
- `CanonicalSchema#negate`, the values a schema rejects.
- `CanonicalSchema#definition`, one reference target by URI.

### Fixed

- An integer past `i64` admitted by a fractional bound `f64` rounds it onto, such as `-10000000000000000000000000` under `{"maximum": -10000000000000000000000000.1}`.
- An integer past `2^53` admitted by a bound `f64` rounds onto it, such as `9007199254740992` under `{"minimum": 9007199254740993}`.
- A `contains` subschema beside both `minContains` and `maxContains` overwriting a sibling keyword of the same name, such as `items`.

### Performance

- `CanonicalSchema` hashing and equality cost the node instead of the whole document.

## [0.49.3] - 2026-08-02

### Added

- Canonicalization of a recursive schema with no finite witness, which now folds to `false`.
- Canonicalization of `$dynamicRef` and `$recursiveRef`, which resolve through the dynamic scope and stay symbolic like any other reference. A dangling `$dynamicRef` errors rather than staying `Raw`.
- Canonicalization of `minContains` under `uniqueItems`, where a demand asking for more matches than its own schema has distinct values now folds to `false`.
- Canonicalization of a Draft 4 `patternProperties` coverage closed by `additionalProperties: false`, spelled as the closed map it was parsed from.
- Canonicalization of a `oneOf` whose branches repeat, where a repeated branch can never contribute exactly one match.
- Canonicalization of a `$ref` whose target is an empty schema, which now folds to `false`.
- Canonicalization of `unevaluatedItems` beside `allOf`, where every branch must pass and so the indexes they evaluate are known without the instance.
- Canonicalization of `unevaluatedProperties` beside `allOf`, where every branch must pass and so what they evaluate is known without the instance.
- Canonicalization of a Draft 4 `type` list holding `integer` beside other types with `enum`, which previously modeled only when spelled as an `allOf`.
- Canonicalization of `patternProperties` patterns matching finitely many keys, such as `^a$` and `^(a|b)$`.
- Canonicalization of `unevaluatedProperties` and `unevaluatedItems` when no in-place applicator sits beside them.

### Fixed

- `additionalItems` values that are not schemas silently ignored beside an array-form `items` (they should fail the build like `additionalProperties`).
- `additionalItems` beside a boolean `items` rejecting every instance, such as `{"additionalItems": false, "items": false}`.
- Draft 4 rejecting a size bound at or past `2^64`, such as `{"maxItems": 18446744073709551616}`.
- An integer past the `f64` range admitted by a fractional bound it exceeds, such as `1e400` under `{"exclusiveMaximum": 0.1}`.
- A `$ref` at the root of an `$id`-bearing subresource dropped as a self-reference when its pointer matched the one that reached that subresource.
- Canonicalization reusing one definition's body for a same-named definition in another resource, when the name spells a canonical URI.
- `unevaluatedItems` counting `prefixItems` as evaluating elements before Draft 2020-12, where it is not a keyword.

### Performance

- Up to 70% faster `evaluate`.

## [0.49.2] - 2026-07-28

### Performance

- Faster serialization of canonicalized schemas.

## [0.49.1] - 2026-07-25

### Fixed

- `JSONSchema.canonicalize` rejecting `pattern_options` (it should take the same regex configuration as validators).
- `InvalidPattern` failures raising the base `CanonicalizationError` (they should raise `JSONSchema::Canonical::InvalidPattern`).

## [0.49.0] - 2026-07-25

### Added

- **EXPERIMENTAL**: Schema canonicalization via `jsonschema::canonicalize`. It reduces a reasonable subset of JSON Schemas to their normal forms.
- Validation of recursive Ruby objects.

### Changed

- Invalid UTF-8, unsupported types, and nesting past the depth limit are reported only where a keyword reads the value.
- A hash keyed by both `:name` and `"name"` counts two properties (it previously collapsed them into one).

### Fixed

- `multipleOf` incorrectly accepted integers past `u64` that are not multiples of the divisor.

### Performance

- Up to 3x faster validation by working on Ruby objects directly instead of converting them to `serde_json`. [#239](https://github.com/Stranger6667/jsonschema/issues/239)

## [0.48.5] - 2026-07-22

### Performance

- Avoid map lookups in some `properties` validators.
- Faster validation of `{"type": "array", "items": {...}}` schemas.

## [0.48.2] - 2026-07-21

### Fixed

- `Canonical::JSON.to_string` incorrectly emitting exponent form for small `Float` values (e.g. `1e-7` instead of `0.0000001`).

### Performance

- Faster validator compilation by pre-sizing internal caches.

## [0.48.1] - 2026-07-17

### Fixed

- Missing `required` errors in `evaluate` output for schemas with `properties` and a two-entry `required` array. [#1220](https://github.com/Stranger6667/jsonschema/issues/1220)
- `contentEncoding` errors for invalid UTF-8 after decoding incorrectly had empty `instance_path` and `schema_path`.

## [0.48.0] - 2026-07-16

### Fixed

- `JSONSchema::Meta.valid?` and `JSONSchema::Meta.validate!` incorrectly accepted some Draft 2019-09 schemas that the meta-schema rejects.
- Integers just outside the `i64`/`u64` range incorrectly compared against numeric bounds through lossy `f64` rounding (e.g. `{"minimum" => -9223372036854775808}` accepted `-9223372036854775809`).

### Performance

- Faster `multipleOf` validation for integer instances with integer divisors, via integer arithmetic instead of floating-point modulo.

## [0.47.0] - 2026-07-08

### Added

- Optional `iter_errors(instance)` method on custom keyword validators for reporting multiple errors from a single keyword. [#1071](https://github.com/Stranger6667/jsonschema/discussions/1071)

### Performance

- Faster validator construction, via compile-time meta-schema validators.

### Fixed

- `type` under `items` asserted with the Validation vocabulary disabled.
- Disabled vocabularies ignored for `$ref` targets without their own `$schema` (e.g. `$defs` entries).

## [0.46.10] - 2026-07-05

### Fixed

- Stack overflow with a self-referential `$dynamicRef` combined with `unevaluatedProperties` or `unevaluatedItems`.
- Incorrect `unevaluatedProperties` and `unevaluatedItems` results when a meta-schema disables the Applicator vocabulary.

## [0.46.9] - 2026-07-02

### Fixed

- Stack overflow while preparing a registry containing deeply nested schema documents.

## [0.46.8] - 2026-07-01

### Fixed

- `idn-email` format rejected non-ASCII characters in quoted local parts (e.g. `"δοκιμή"@example.com`).

## [0.46.7] - 2026-06-30

### Fixed

- `idn-hostname` format accepted A-labels that decode to a disallowed code point (e.g. `xn--7a`).

## [0.46.6] - 2026-06-24

### Fixed

- `prefixItems` incorrectly recognised as a known keyword in Draft 2019-09 and earlier (it is 2020-12 only).
- `pattern` validation errors displayed the internally translated regex instead of the original schema pattern. [#1149](https://github.com/Stranger6667/jsonschema/issues/1149)
- Reuse registry retrievers when only `registry` is passed.

## [0.46.5] - 2026-05-13

### Fixed

- Percent-encoded characters in `$ref` URI fragments (e.g. `#/$defs/Request%20class`) are now decoded when stored as `schema_path`.

## [0.46.4] - 2026-05-02

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

[Unreleased]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.52.1...HEAD
[0.52.1]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.52.0...ruby-v0.52.1
[0.52.0]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.51.0...ruby-v0.52.0
[0.51.0]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.50.1...ruby-v0.51.0
[0.50.1]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.50.0...ruby-v0.50.1
[0.50.0]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.49.9...ruby-v0.50.0
[0.49.9]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.49.8...ruby-v0.49.9
[0.49.8]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.49.7...ruby-v0.49.8
[0.49.7]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.49.6...ruby-v0.49.7
[0.49.6]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.49.5...ruby-v0.49.6
[0.49.5]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.49.4...ruby-v0.49.5
[0.49.4]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.49.3...ruby-v0.49.4
[0.49.3]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.49.2...ruby-v0.49.3
[0.49.2]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.49.1...ruby-v0.49.2
[0.49.1]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.49.0...ruby-v0.49.1
[0.49.0]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.48.5...ruby-v0.49.0
[0.48.5]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.48.2...ruby-v0.48.5
[0.48.2]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.48.1...ruby-v0.48.2
[0.48.1]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.48.0...ruby-v0.48.1
[0.48.0]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.47.0...ruby-v0.48.0
[0.47.0]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.46.10...ruby-v0.47.0
[0.46.10]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.46.9...ruby-v0.46.10
[0.46.9]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.46.8...ruby-v0.46.9
[0.46.8]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.46.7...ruby-v0.46.8
[0.46.7]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.46.6...ruby-v0.46.7
[0.46.6]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.46.5...ruby-v0.46.6
[0.46.5]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.46.4...ruby-v0.46.5
[0.46.4]: https://github.com/Stranger6667/jsonschema/compare/ruby-v0.46.3...ruby-v0.46.4
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

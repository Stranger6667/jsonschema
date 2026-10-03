# Security Policy

## Supported Versions

Only the latest minor release receives security fixes.

| Version  | Supported          |
| -------- | ------------------ |
| 0.58.x   | :white_check_mark: |
| < 0.58   | :x:                |

This applies to the Rust crate (`jsonschema`), the Python package (`jsonschema-rs`) and the Ruby gem (`jsonschema`), which share version numbers.

## Reporting a Vulnerability

Please do not open public issues for security problems.

Report privately via [GitHub Security Advisories](https://github.com/Stranger6667/jsonschema/security/advisories/new).

- **Acknowledgement:** within 7 days.
- **Updates:** at least every 14 days until the report is resolved.
- **If accepted:** a fix is released for the supported version, followed by a GitHub Security Advisory (with a CVE where applicable) and a RustSec advisory. Reporters are credited unless they prefer otherwise.
- **If declined:** you get an explanation, usually that the behaviour is outside the scope below. You are free to discuss it publicly after that.

## Scope

Validating untrusted instances against a trusted schema is supported. Panics, unbounded recursion and excessive resource use there are in scope.

Schemas are treated as trusted code. If you compile untrusted schemas, disable the `resolve-http` and `resolve-file` features or supply a restricted `Retrieve` implementation. Otherwise, `$ref` can fetch arbitrary URLs and read local files.

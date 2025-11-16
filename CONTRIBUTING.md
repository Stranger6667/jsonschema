# Contributing to jsonschema

Thank you for your interest in contributing to jsonschema! We welcome contributions from everyone in the form of suggestions, bug reports, pull requests, and feedback. This document provides guidance if you're thinking of helping out.

## Submitting Bug Reports and Feature Requests

When reporting a bug or asking for help, please include enough details so that others can reproduce the behavior you're seeing.

To open an issue, [follow this link](https://github.com/Stranger6667/jsonschema-rs/issues/new) and fill out the appropriate template.

When making a feature request, please make it clear what problem you intend to solve with the feature and provide some ideas on how to implement it.

**NOTE**: Many commands are available via a [Justfile](https://github.com/casey/just) located in the root of the repository.

## Running the Tests

The tests in jsonschema depend on the [JSON Schema Test Suite](https://github.com/json-schema-org/JSON-Schema-Test-Suite). Before running the tests, you need to download the suite.

Initialize and update the git submodules:

   ```console
   $ git submodule init
   $ git submodule update
   ```

This will clone the JSON Schema Test Suite to `crates/jsonschema/tests/suite/`.

Then follow instructions below to run the tests either for the Rust Core or Python Bindings.

## Rust Core

### Rust Toolchain

jsonschema targets Rust 1.83.0 as its Minimum Supported Rust Version (MSRV). Please ensure your contributions are compatible with this version.

You can use [rustup](https://rustup.rs/) to manage your installed toolchains. To set up the correct version for the jsonschema project:

   ```console
   $ rustup override set 1.83.0
   ```

### Running the Tests

Run the tests with:

   ```console
   $ cargo test --all-features
   ```

Make sure all tests pass before submitting your pull request. If you've added new functionality, please include appropriate tests.

### Formatting and Linting

Format your code using:

   ```console
   $ cargo fmt --all
   ```

And lint it using:

   ```console
   $ cargo clippy --all-targets --all-features -- -D warnings
   ```

## Python Bindings

The Python bindings are located in the `crates/jsonschema-py` directory. If you're working on or testing the Python bindings, follow these steps:

### Setting Up the Python Environment

We recommend using [uv](https://github.com/astral-sh/uv) for managing the Python environment. To set up the environment:

1. Navigate to the `crates/jsonschema-py` directory:

   ```console
   $ cd crates/jsonschema-py
   ```

2. Create a virtual environment and install the package in editable mode with test dependencies:

   ```console
   $ uv venv
   $ uv pip install -e ".[tests]"
   ```

### Running Python Tests

To run the Python tests:

   ```console
   $ uv run pytest tests-py
   ```

Make sure all Python tests pass before submitting your pull request. If you've added new functionality to the Python bindings, please include appropriate Python tests as well.

### Formatting and Linting

Format your code using:

   ```console
   $ uvx ruff format benches python tests-py
   $ cargo fmt --all
   ```

And lint it using:

   ```console
   $ uvx ruff check benches python tests-py
   $ cargo clippy --all-targets --all-features -- -D warnings
   ```

### Adding New Functionality

For small changes (e.g., bug fixes), feel free to submit a PR directly.

For larger changes (e.g., new functionality or configuration options), please create an [issue](https://github.com/Stranger6667/jsonschema-rs/issues) first to discuss your proposed change.

### Improving Documentation

Contributions to documentation are always welcome. If you find any part of the documentation unclear or incomplete, please open an issue or submit a pull request.

### Implementing Missing Keywords

If you're looking to contribute code, implementing missing keywords for newer JSON Schema drafts is a great place to start. Check the [compliance badges](https://github.com/Stranger6667/jsonschema-rs#supported-drafts) to see which drafts might need work.

### Fixing Test Cases

Another way to contribute is by fixing failing test cases from the [JSON Schema Test Suite](https://github.com/json-schema-org/JSON-Schema-Test-Suite). You can check the current status on the [Bowtie Report](https://bowtie.report/#/implementations/rust-jsonschema).

## Pull Requests

1. Ensure your code passes all tests and lint checks.
2. Update the documentation as necessary.
3. Add or update tests as appropriate.
4. If you're adding new functionality, please include a description in the README.
5. If your change affects users, add an entry to the CHANGELOG.

## Getting Help

If you need help with contributing to jsonschema, you can:

1. Open a [GitHub Discussion](https://github.com/Stranger6667/jsonschema-rs/discussions).
2. Ask in the pull request or issue if you've already opened one.

Thank you for contributing to jsonschema!

default:
  @just --list

fuzz TARGET:
  mkdir -p fuzz/corpus/{{TARGET}}
  cargo +nightly fuzz run --release {{TARGET}} fuzz/corpus/{{TARGET}} fuzz/seeds -- -dict=fuzz/dict

lint-rs:
  cargo +nightly fmt --all
  cargo clippy --all-features --all-targets
  cd fuzz && cargo +nightly fmt --all
  cd fuzz && cargo clippy --all-features --all-targets
  cd profiler && cargo +nightly fmt --all
  cd profiler && cargo clippy --all-features --all-targets

lint-py:
  uvx ruff check crates/jsonschema-py/python crates/jsonschema-py/tests-py crates/jsonschema-py/benches
  uvx ruff check --select I --fix crates/jsonschema-py/python crates/jsonschema-py/tests-py crates/jsonschema-py/benches
  uvx mypy crates/jsonschema-py/python

lint: lint-rs lint-py

test-rs *FLAGS:
  cargo llvm-cov --html test {{FLAGS}}

test-py *FLAGS:
  uvx --with="crates/jsonschema-py[tests]" --refresh pytest crates/jsonschema-py/tests-py -rs {{FLAGS}}

test-py-no-rebuild *FLAGS:
  uvx --with="crates/jsonschema-py[tests]" pytest crates/jsonschema-py/tests-py -rs {{FLAGS}}

bench-py *FLAGS:
  uvx --with="crates/jsonschema-py[bench]" --refresh pytest crates/jsonschema-py/benches/bench.py --benchmark-columns=min {{FLAGS}}

miri:
  cargo +nightly miri test -p referencing

install-wasm-deps:
  rustup target add wasm32-wasip1 wasm32-unknown-unknown
  cargo install wasm-bindgen-cli --version $(cargo metadata --format-version 1 | jq -r '.packages[] | select(.name=="wasm-bindgen") | .version') --locked

test-wasm32-wasip1:
  cargo test --target wasm32-wasip1 --no-default-features -p jsonschema

test-wasm32-unknown-unknown:
  cargo test --target wasm32-unknown-unknown --no-default-features -p jsonschema

test-wasm: test-wasm32-wasip1 test-wasm32-unknown-unknown

release-rust VERSION:
  #!/usr/bin/env bash
  set -euo pipefail
  VERSION="{{VERSION}}"
  PREV=$(grep -m1 -oE '[0-9]+\.[0-9]+\.[0-9]+' crates/jsonschema/Cargo.toml)
  PREV_RE=${PREV//./\\.}
  DATE=$(date +%Y-%m-%d)
  FILES=(crates/jsonschema/Cargo.toml crates/jsonschema-referencing/Cargo.toml crates/jsonschema-cli/Cargo.toml crates/jsonschema-regex/Cargo.toml crates/jsonschema-macros-core/Cargo.toml crates/jsonschema-macros/Cargo.toml)
  sed -i "s/${PREV_RE}/${VERSION}/g" "${FILES[@]}"
  sed -i "0,/^## \[Unreleased\]$/s//## [Unreleased]\n\n## [${VERSION}] - ${DATE}/" CHANGELOG.md
  sed -i "s#compare/rust-v${PREV_RE}\.\.\.HEAD#compare/rust-v${VERSION}...HEAD#" CHANGELOG.md
  sed -i "/^\[Unreleased\]: /a [${VERSION}]: https://github.com/Stranger6667/jsonschema/compare/rust-v${PREV}...rust-v${VERSION}" CHANGELOG.md
  git add CHANGELOG.md "${FILES[@]}"
  git commit -m "chore(rust): Release ${VERSION}"
  git tag "rust-v${VERSION}"
  git push origin master
  git push origin "rust-v${VERSION}"

release-python VERSION:
  #!/usr/bin/env bash
  set -euo pipefail
  VERSION="{{VERSION}}"
  PREV=$(grep -m1 -oE '[0-9]+\.[0-9]+\.[0-9]+' crates/jsonschema-py/Cargo.toml)
  PREV_RE=${PREV//./\\.}
  DATE=$(date +%Y-%m-%d)
  CL=crates/jsonschema-py/CHANGELOG.md
  sed -i "s/${PREV_RE}/${VERSION}/g" crates/jsonschema-py/Cargo.toml
  sed -i "0,/^## \[Unreleased\]$/s//## [Unreleased]\n\n## [${VERSION}] - ${DATE}/" "$CL"
  sed -i "s#compare/python-v${PREV_RE}\.\.\.HEAD#compare/python-v${VERSION}...HEAD#" "$CL"
  sed -i "/^\[Unreleased\]: /a [${VERSION}]: https://github.com/Stranger6667/jsonschema/compare/python-v${PREV}...python-v${VERSION}" "$CL"
  git add "$CL" crates/jsonschema-py/Cargo.toml
  git commit -m "chore(python): Release ${VERSION}"
  git tag "python-v${VERSION}"
  git push origin master
  git push origin "python-v${VERSION}"

release-ruby VERSION:
  #!/usr/bin/env bash
  set -euo pipefail
  VERSION="{{VERSION}}"
  PREV=$(grep -m1 -oE '[0-9]+\.[0-9]+\.[0-9]+' crates/jsonschema-rb/lib/jsonschema/version.rb)
  PREV_RE=${PREV//./\\.}
  DATE=$(date +%Y-%m-%d)
  CL=crates/jsonschema-rb/CHANGELOG.md
  VFILES=(crates/jsonschema-rb/Cargo.toml crates/jsonschema-rb/lib/jsonschema/version.rb crates/jsonschema-rb/Gemfile.lock crates/jsonschema-rb/ext/jsonschema/Cargo.toml)
  sed -i "s/${PREV_RE}/${VERSION}/g" "${VFILES[@]}"
  sed -i "0,/^## \[Unreleased\]$/s//## [Unreleased]\n\n## [${VERSION}] - ${DATE}/" "$CL"
  sed -i "s#compare/ruby-v${PREV_RE}\.\.\.HEAD#compare/ruby-v${VERSION}...HEAD#" "$CL"
  sed -i "/^\[Unreleased\]: /a [${VERSION}]: https://github.com/Stranger6667/jsonschema/compare/ruby-v${PREV}...ruby-v${VERSION}" "$CL"
  cargo update -p jsonschema -p referencing --manifest-path crates/jsonschema-rb/ext/jsonschema/Cargo.toml
  git add "$CL" "${VFILES[@]}" crates/jsonschema-rb/ext/jsonschema/Cargo.lock
  git commit -m "chore(ruby): Release ${VERSION}"
  git tag "ruby-v${VERSION}"
  git push origin master
  git push origin "ruby-v${VERSION}"

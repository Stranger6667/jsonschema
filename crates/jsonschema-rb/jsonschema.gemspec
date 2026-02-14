# frozen_string_literal: true

require_relative "lib/jsonschema/version"

Gem::Specification.new do |spec|
  spec.name = "jsonschema"
  spec.version = JSONSchema::VERSION
  spec.authors = ["Dmitry Dygalo"]
  spec.email = ["dmitry@dygalo.dev"]

  spec.summary = "A high-performance JSON Schema validator for Ruby"
  spec.description = "High-performance JSON Schema validator with support for Draft 4, 6, 7, 2019-09, and 2020-12."
  spec.homepage = "https://github.com/Stranger6667/jsonschema/tree/master/crates/jsonschema-rb"
  spec.license = "MIT"
  spec.required_ruby_version = ">= 3.2.0"
  spec.required_rubygems_version = ">= 3.3.11"

  spec.metadata["homepage_uri"] = spec.homepage
  spec.metadata["source_code_uri"] = "https://github.com/Stranger6667/jsonschema"
  spec.metadata["changelog_uri"] = "https://github.com/Stranger6667/jsonschema/blob/master/crates/jsonschema-rb/CHANGELOG.md"
  spec.metadata["documentation_uri"] = "https://github.com/Stranger6667/jsonschema/tree/master/crates/jsonschema-rb#readme"
  spec.metadata["bug_tracker_uri"] = "https://github.com/Stranger6667/jsonschema/issues"
  spec.metadata["funding_uri"] = "https://github.com/sponsors/Stranger6667"
  spec.metadata["rubygems_mfa_required"] = "true"

  spec.files = Dir[
    "lib/**/*.rb",
    "src/**/*.rs",
    "ext/**/*.{rs,rb,toml,lock}",
    "sig/**/*.rbs",
    "Cargo.toml",
    "Cargo.lock",
    "LICENSE",
    "README.md",
    "CHANGELOG.md",
    "MIGRATION.md"
  ]
  spec.require_paths = ["lib"]
  spec.add_dependency "bigdecimal", ">= 3.1", "< 5"
  spec.add_dependency "rb_sys", "~> 0.9.124"
  # Build via the standalone extension config in ext/jsonschema.
  spec.extensions = ["ext/jsonschema/extconf.rb"]
end

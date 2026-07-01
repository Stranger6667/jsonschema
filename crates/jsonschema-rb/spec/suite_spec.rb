# frozen_string_literal: true

require "spec_helper"
require "json"
require "pathname"

module SuiteHelpers
  SUITE_PATH = Pathname.new(__dir__).join("../../jsonschema/tests/suite/tests")
  REMOTES_PATH = Pathname.new(__dir__).join("../../jsonschema/tests/suite/remotes")

  # Map draft directories to JSONSchema draft constants
  DRAFT_MAP = {
    "draft4" => :draft4,
    "draft6" => :draft6,
    "draft7" => :draft7,
    "draft2019-09" => :draft201909,
    "draft2020-12" => :draft202012
  }.freeze

  def self.sanitize_name(name)
    name.gsub(/[^a-zA-Z0-9_]/, "_").gsub(/_+/, "_").gsub(/^_|_$/, "")
  end

  # Lone surrogate: unrepresentable as a Rust string; rewrite unpaired escapes to U+FFFD.
  HIGH_SURROGATE = "d[89ab][0-9a-f]{2}"
  LOW_SURROGATE = "d[c-f][0-9a-f]{2}"

  REPLACEMENT_CHARACTER = "�"

  def self.sanitize_lone_surrogates(text)
    # Block form keeps the replacement UTF-8 instead of transcoding it to the file's ASCII.
    text
      .gsub(/\\u(#{HIGH_SURROGATE})(?!\\u#{LOW_SURROGATE})/i) { REPLACEMENT_CHARACTER }
      .gsub(/(?<!\\u#{HIGH_SURROGATE})\\u(#{LOW_SURROGATE})/i) { REPLACEMENT_CHARACTER }
  end

  def self.unencodable?(value)
    value.is_a?(String) && value.include?(REPLACEMENT_CHARACTER)
  end

  # Build a retriever proc for remote schemas
  def self.build_retriever
    return @build_retriever if defined?(@build_retriever)

    remotes = {}
    if REMOTES_PATH.exist?
      REMOTES_PATH.glob("**/*.json").each do |file|
        relative = file.relative_path_from(REMOTES_PATH).to_s
        uri = "http://localhost:1234/#{relative}"
        # Parse JSON to return Ruby hashes
        remotes[uri] = JSON.parse(file.read)
      end
    end

    @build_retriever = ->(uri) { remotes[uri] }
  end
end

RSpec.describe "JSON Schema Test Suite" do
  SuiteHelpers::DRAFT_MAP.each do |draft_name, draft_const|
    draft_path = SuiteHelpers::SUITE_PATH.join(draft_name)
    next unless draft_path.exist?

    describe draft_name do
      draft_path.glob("**/*.json").sort.each do |test_file|
        relative_path = test_file.relative_path_from(draft_path).to_s.sub(/\.json$/, "")
        is_optional = relative_path.start_with?("optional/")

        context relative_path do
          test_cases = JSON.parse(SuiteHelpers.sanitize_lone_surrogates(test_file.read))

          test_cases.each do |test_case|
            case_description = test_case["description"]
            schema = test_case["schema"]

            context case_description do
              test_case["tests"].each do |test|
                test_description = test["description"]
                data = test["data"]
                expected_valid = test["valid"]

                it test_description do
                  skip("instance has no UTF-8 representation") if SuiteHelpers.unencodable?(data)

                  opts = {
                    draft: draft_const,
                    validate_formats: is_optional,
                    retriever: SuiteHelpers.build_retriever
                  }
                  error_ctx = "Schema: #{JSON.pretty_generate(schema)}\n" \
                              "Instance: #{JSON.pretty_generate(data)}"

                  # valid?
                  result = JSONSchema.valid?(schema, data, **opts)
                  if expected_valid
                    expect(result).to be(true),
                                      "valid? expected true but got false.\n#{error_ctx}"
                  else
                    expect(result).to be(false),
                                      "valid? expected false but got true.\n#{error_ctx}"
                  end

                  # validate!
                  if expected_valid
                    expect { JSONSchema.validate!(schema, data, **opts) }.not_to raise_error
                  else
                    expect { JSONSchema.validate!(schema, data, **opts) }.to raise_error(JSONSchema::ValidationError)
                  end

                  # each_error
                  errors = JSONSchema.each_error(schema, data, **opts)
                  if expected_valid
                    expect(errors).to be_empty,
                                      "each_error expected no errors.\n#{error_ctx}"
                  else
                    expect(errors).not_to be_empty,
                                          "each_error expected errors but got none.\n#{error_ctx}"
                  end

                  # evaluate
                  eval_result = JSONSchema.evaluate(schema, data, **opts)
                  expect(eval_result.valid?).to eq(expected_valid),
                                                "evaluate.valid? expected #{expected_valid}.\n#{error_ctx}"
                end
              end
            end
          end
        end
      end
    end
  end
end

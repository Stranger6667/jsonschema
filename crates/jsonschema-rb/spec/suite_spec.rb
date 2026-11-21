# frozen_string_literal: true

require "spec_helper"
require "json"
require "bigdecimal"
require "pathname"
require_relative "support/jsonschema_testsuite_magnus"

module SuiteHelpers
  SUITE_PATH = Pathname.new(__dir__).join("../../jsonschema/tests/suite/validation/tests")

  # Draft name => draft symbol and the number the suite's `compatibility` property uses for it
  DRAFT_MAP = {
    "draft4" => [:draft4, 4],
    "draft6" => [:draft6, 6],
    "draft7" => [:draft7, 7],
    "draft2019-09" => [:draft201909, 2019],
    "draft2020-12" => [:draft202012, 2020]
  }.freeze

  def self.compatible?(compatibility, release)
    compatible = true
    compatibility.to_s.split(",").each do |constraint|
      operator, version = constraint.match(/\A(<=|=)?(\d+)\z/).captures
      version = version.to_i
      compatible = { "<=" => release <= version, "=" => release == version }.fetch(operator, release >= version)
      break if release <= version
    end
    compatible
  end

  def self.validate_formats(test_case)
    tags = test_case.fetch("tags", [])
    return true if tags.include?("format-assertion")
    return false if tags.include?("format-annotation")

    nil
  end

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

  # `JSON.generate` writes a `BigDecimal` as a string, so numbers are written out by hand.
  def self.json_text(value)
    case value
    when BigDecimal then value.to_s("F")
    when Hash then "{#{value.map { |key, item| "#{JSON.generate(key)}:#{json_text(item)}" }.join(',')}}"
    when Array then "[#{value.map { |item| json_text(item) }.join(',')}]"
    else JSON.generate(value)
    end
  end
end

RSpec.describe "JSON Schema Test Suite" do
  suite_files = SuiteHelpers::SUITE_PATH.glob("**/*.json").sort.filter_map do |test_file|
    relative_file = test_file.relative_path_from(SuiteHelpers::SUITE_PATH).to_s
    next if relative_file.start_with?("proposals/")

    [relative_file, JSON.parse(SuiteHelpers.sanitize_lone_surrogates(test_file.read), decimal_class: BigDecimal)["tests"]]
  end

  SuiteHelpers::DRAFT_MAP.each do |draft_name, (draft_const, release)|
    describe draft_name do
      suite_files.each do |relative_file, test_cases|
        context relative_file.sub(/\.json$/, "") do
          test_cases.each_with_index do |test_case, index|
            next unless SuiteHelpers.compatible?(test_case["compatibility"], release)

            case_description = test_case["description"]
            schema = test_case["schema"]
            external_schemas = test_case.fetch("externalSchemas", {})
            validate_formats = SuiteHelpers.validate_formats(test_case)
            # Matches the id `magnus_suite!` gives the validator compiled for this block.
            case_id = "#{draft_name}|#{relative_file}|#{index}"

            context case_description do
              test_case["tests"].each do |test|
                test_description = test["description"]
                data = test["data"]
                expected_valid = test["valid"]

                it test_description do
                  skip("instance has no UTF-8 representation") if SuiteHelpers.unencodable?(data)

                  retriever = ->(uri) { external_schemas[uri] }
                  opts = { draft: draft_const, retriever: retriever }
                  opts[:validate_formats] = validate_formats unless validate_formats.nil?
                  # An explicit draft never retrieves `$schema`, so a custom meta-schema must be registered.
                  meta_schema = schema.is_a?(Hash) ? external_schemas[schema["$schema"]] : nil
                  if meta_schema
                    opts[:registry] = JSONSchema::Registry.new(
                      [[schema["$schema"], meta_schema]], retriever: retriever
                    )
                  end
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

                  # The same instance as JSON text.
                  text_validator = JSONSchema::JsonTextValidator.new(schema, **opts)
                  text = SuiteHelpers.json_text(data)
                  expect(text_validator.valid?(text)).to be(expected_valid),
                                                         "JsonTextValidator valid? mismatch.\n#{error_ctx}"
                  expect(text_validator.each_error(text).empty?).to be(expected_valid),
                                                                    "JsonTextValidator each_error mismatch.\n#{error_ctx}"
                  expect(text_validator.evaluate(text).valid?).to be(expected_valid),
                                                                  "JsonTextValidator evaluate mismatch.\n#{error_ctx}"

                  # `JSONSchemaTestSuite` holds one `backend = Magnus` validator per suite case.
                  expect(JSONSchemaTestSuite.valid?(case_id, data)).to be(expected_valid),
                                                                       "codegen valid? mismatch.\n#{error_ctx}"
                  expect(JSONSchemaTestSuite.validate(case_id, data).nil?).to be(expected_valid),
                                                                              "codegen validate mismatch.\n#{error_ctx}"
                  expect(JSONSchemaTestSuite.each_error(case_id, data).empty?).to be(expected_valid),
                                                                                  "codegen each_error mismatch.\n#{error_ctx}"
                  # The outputs of the generated validator and of a runtime one built like it.
                  generated, runtime = JSON.parse(JSONSchemaTestSuite.evaluate(case_id, data))
                  expect(generated).to eq(runtime), "codegen evaluate mismatch.\n#{error_ctx}"
                end
              end
            end
          end
        end
      end
    end
  end
end

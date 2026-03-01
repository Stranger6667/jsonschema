# frozen_string_literal: true

require "spec_helper"
require "json"
require "pathname"

module AnnotationHelpers
  ANNOTATION_SUITE_PATH = Pathname.new(__dir__).join("../../jsonschema/tests/suite/annotations/tests")
  SCHEMA_VALUED_KEYWORDS = ["contentSchema"].freeze

  def self.collect_annotations(evaluation)
    evaluation.annotations.each_with_object(default_annotation_map) do |entry, result|
      instance_loc = entry[:instanceLocation].to_s
      append_entry_annotations(result, instance_loc, entry)
    end
  end

  def self.default_annotation_map
    Hash.new { |hash, key| hash[key] = [] }
  end

  def self.append_entry_annotations(result, instance_loc, entry)
    annotations = entry[:annotations]
    keyword = keyword_from_schema_location(entry[:schemaLocation])

    if !schema_valued_keyword?(keyword) && annotations.is_a?(Hash)
      append_hash_annotations(result, instance_loc, annotations)
    else
      result[[instance_loc, keyword]] << annotations
    end
  end

  def self.append_hash_annotations(result, instance_loc, annotations)
    annotations.each do |keyword, value|
      result[[instance_loc, keyword.to_s]] << value
    end
  end

  def self.keyword_from_schema_location(schema_location)
    schema_location.to_s.split("/").last.to_s
  end

  def self.schema_valued_keyword?(keyword)
    SCHEMA_VALUED_KEYWORDS.include?(keyword)
  end
end

RSpec.describe "Annotation Test Suite" do
  suite_path = AnnotationHelpers::ANNOTATION_SUITE_PATH
  next unless suite_path.exist?

  suite_path.glob("*.json").sort.each do |test_file|
    file_name = test_file.basename(".json").to_s

    context file_name do
      file_data = JSON.parse(test_file.read)

      file_data["suite"].each do |suite_case|
        description = suite_case["description"]
        schema = suite_case["schema"]

        context description do
          suite_case["tests"].each_with_index do |test_case, test_idx|
            instance = test_case["instance"]

            test_case["assertions"].each_with_index do |assertion, assert_idx|
              location = assertion["location"]
              keyword = assertion["keyword"]
              expected = assertion["expected"]

              it "test #{test_idx}, assertion #{assert_idx}: #{keyword} at #{location.inspect}" do
                eval_result = JSONSchema.evaluate(schema, instance)
                collected = AnnotationHelpers.collect_annotations(eval_result)

                key = [location, keyword]
                actual_values = collected[key] || []

                error_ctx = [
                  "Schema: #{JSON.pretty_generate(schema)}\n",
                  "Instance: #{JSON.pretty_generate(instance)}\n",
                  "Location: #{location.inspect}\n",
                  "Keyword: #{keyword.inspect}"
                ].join

                if expected.empty?
                  message = [
                    "Expected no annotation for keyword '#{keyword}' at '#{location}',",
                    "but got: #{actual_values.inspect}\n#{error_ctx}"
                  ].join(" ")
                  expect(actual_values).to(
                    be_empty,
                    message
                  )
                else
                  expected.each do |schema_loc, expected_value|
                    message = [
                      "Missing expected annotation value.\n",
                      "Schema location: #{schema_loc.inspect}\n",
                      "Expected: #{expected_value.inspect}\n",
                      "Got: #{actual_values.inspect}\n#{error_ctx}"
                    ].join
                    expect(actual_values).to(
                      include(expected_value),
                      message
                    )
                  end
                end
              end
            end
          end
        end
      end
    end
  end
end

require "spec_helper"
require "json"
require "pathname"

module AnnotationHelpers
  ANNOTATION_SUITE_PATH = Pathname.new(__dir__).join("../../jsonschema/tests/suite/annotations/tests")

  def self.collect_annotations(evaluation)
    result = Hash.new { |h, k| h[k] = [] }

    evaluation.annotations.each do |entry|
      instance_loc = entry[:instanceLocation] || ""
      annotations = entry[:annotations]

      if annotations.is_a?(Hash)
        annotations.each do |keyword, value|
          result[[instance_loc, keyword.to_s]] << value
        end
      else
        schema_loc = entry[:schemaLocation] || ""
        keyword = schema_loc.split("/").last || ""
        result[[instance_loc, keyword]] << annotations
      end
    end

    result
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

                error_ctx = "Schema: #{JSON.pretty_generate(schema)}\n" \
                            "Instance: #{JSON.pretty_generate(instance)}\n" \
                            "Location: #{location.inspect}\n" \
                            "Keyword: #{keyword.inspect}"

                if expected.empty?
                  expect(actual_values).to be_empty,
                    "Expected no annotation for keyword '#{keyword}' at '#{location}', " \
                    "but got: #{actual_values.inspect}\n#{error_ctx}"
                else
                  expected.each do |schema_loc, expected_value|
                    expect(actual_values).to include(expected_value),
                      "Missing expected annotation value.\n" \
                      "Schema location: #{schema_loc.inspect}\n" \
                      "Expected: #{expected_value.inspect}\n" \
                      "Got: #{actual_values.inspect}\n#{error_ctx}"
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

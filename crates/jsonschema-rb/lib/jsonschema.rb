# frozen_string_literal: true

require_relative "jsonschema/version"

begin
  RUBY_VERSION =~ /(\d+\.\d+)/
  require "jsonschema/#{Regexp.last_match(1)}/jsonschema_rb"
rescue LoadError
  require "jsonschema/jsonschema_rb"
end

# High-performance JSON Schema validator for Ruby.
#
# @example Quick validation with Hash
#   JSONSchema.valid?({ "type" => "string" }, "hello")  #=> true
#
# @example Quick validation with JSON string
#   JSONSchema.valid?('{"type":"string"}', "hello")  #=> true
#
# @example Reusable validator
#   validator = JSONSchema.validator_for(schema)
#   validator.valid?(data)
#
# @example Error iteration
#   validator.each_error(data) do |error|
#     puts error.message
#   end
#
# @example Structured evaluation output
#   eval = validator.evaluate(data)
#   puts eval.flag     # Simple valid/invalid
#   puts eval.list     # Flat list format
#
# @see https://json-schema.org/ JSON Schema specification
module JSONSchema
end

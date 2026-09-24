# frozen_string_literal: true

require "json"
require_relative "jsonschema_testsuite_magnus"

# `JSONSchemaTestSuite` holds a `backend = Magnus` validator for every schema in
# `spec/codegen_schemas.json`, keyed by the schema's JSON with sorted keys.
module CodegenBackend
  class ValidationError < StandardError; end

  def self.schema_key(schema)
    JSON.generate(sorted(schema))
  end

  def self.sorted(value)
    case value
    when Hash then value.map { |key, member| [key.to_s, sorted(member)] }.sort_by(&:first).to_h
    when Array then value.map { |item| sorted(item) }
    else value
    end
  end

  def self.valid?(schema, instance)
    JSONSchemaTestSuite.valid?(schema_key(schema), instance)
  end

  def self.validate!(schema, instance)
    message = JSONSchemaTestSuite.validate(schema_key(schema), instance)
    raise ValidationError, message unless message.nil?
  end

  def self.each_error(schema, instance)
    JSONSchemaTestSuite.each_error(schema_key(schema), instance)
  end
end

BACKENDS = { "runtime" => JSONSchema, "codegen" => CodegenBackend }.freeze

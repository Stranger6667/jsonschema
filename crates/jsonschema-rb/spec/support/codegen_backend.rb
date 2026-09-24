# frozen_string_literal: true

require "json"
require_relative "jsonschema_testsuite_magnus"

# `JSONSchemaTestSuite` holds a `backend = Magnus` validator for every schema in
# `spec/codegen_schemas.json`, keyed by the schema's JSON with sorted keys. Custom keywords are
# compiled in, so the `keywords:` option only the runtime reads is accepted and ignored.
module CodegenBackend
  class ValidationError < StandardError; end

  class Validator
    def initialize(schema)
      @key = CodegenBackend.schema_key(schema)
    end

    def valid?(instance)
      JSONSchemaTestSuite.valid?(@key, instance)
    end

    def validate!(instance)
      message = JSONSchemaTestSuite.validate(@key, instance)
      raise ValidationError, message unless message.nil?
    end

    def each_error(instance)
      JSONSchemaTestSuite.each_error(@key, instance).map { |message| ValidationError.new(message) }
    end
  end

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

  def self.validator_for(schema, **)
    Validator.new(schema)
  end

  def self.valid?(schema, instance, **)
    validator_for(schema).valid?(instance)
  end

  def self.validate!(schema, instance, **)
    validator_for(schema).validate!(instance)
  end

  def self.each_error(schema, instance, **)
    validator_for(schema).each_error(instance)
  end
end

BACKENDS = { "runtime" => JSONSchema, "codegen" => CodegenBackend }.freeze

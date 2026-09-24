# frozen_string_literal: true

require "json"
require "jsonschema_rs"

DATA = File.expand_path("../../benchmark/data", __dir__)

CASES = {
  "openapi" => ["openapi.json", "zuora.json"],
  "swagger" => ["swagger.json", "kubernetes.json"],
  "geojson" => ["geojson.json", "canada.json"],
  "citm" => ["citm_catalog_schema.json", "citm_catalog.json"],
  "fhir" => ["fhir.schema.json", "patient-example-d.json"],
  "recursive" => ["recursive_schema.json", "recursive_instance.json"]
}.freeze

name = ARGV.fetch(0)
mode = ARGV.fetch(1, "valid")
schema_file, instance_file = CASES.fetch(name)

schema = JSON.parse(File.read(File.join(DATA, schema_file)))
iterations = Integer(ENV.fetch("CODSPEED_ITERS", "20"))
instance = -> { JSON.parse(File.read(File.join(DATA, instance_file))) }

case mode
when "valid"
  validator = JSONSchema.validator_for(schema)
  instance = instance.call
  iterations.times { validator.valid?(instance) }
when "validate"
  validator = JSONSchema.validator_for(schema)
  instance = instance.call
  iterations.times { validator.validate!(instance) }
when "codegen-valid", "codegen-validate"
  # `JSONSchemaBench` holds a `backend = Magnus` validator per benchmark schema.
  require_relative "support/jsonschema_bench_magnus"
  compiled = JSONSchemaBench.method(:"#{name}_#{mode == 'codegen-valid' ? 'valid?' : 'validate!'}")
  instance = instance.call
  iterations.times { compiled.call(instance) }
when "meta-valid"
  iterations.times { JSONSchema::Meta.valid?(schema) }
when "meta-validate"
  iterations.times { JSONSchema::Meta.validate!(schema) }
else
  raise ArgumentError, "unknown mode: #{mode}"
end

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
schema_file, instance_file = CASES.fetch(name)

schema = JSON.parse(File.read(File.join(DATA, schema_file)))
instance = JSON.parse(File.read(File.join(DATA, instance_file)))
validator = JSONSchema.validator_for(schema)

iterations = Integer(ENV.fetch("CODSPEED_ITERS", "20"))
iterations.times { validator.valid?(instance) }

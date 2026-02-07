# frozen_string_literal: true

require "json"
require "jsonschema"
require "json_schemer"
require "json-schema"
require "rj_schema"

BENCHMARK_DATA_PATH = File.expand_path("../../benchmark/data", __dir__)

# Reusable helpers for section printing, time formatting, and measurement
module BenchHelper
  SEPARATOR = ("=" * 70).freeze
  SUB_SEPARATOR = ("-" * 70).freeze

  def self.section(title)
    puts SEPARATOR
    puts title
    puts SEPARATOR
    puts
  end

  def self.subsection(title)
    puts SUB_SEPARATOR
    puts title
    puts SUB_SEPARATOR
  end

  def self.format_time(seconds)
    if seconds < 1e-6
      format("%.2f ns", seconds * 1e9)
    elsif seconds < 1e-3
      format("%.2f \u00B5s", seconds * 1e6)
    elsif seconds < 1.0
      format("%.2f ms", seconds * 1e3)
    else
      format("%.2f s", seconds)
    end
  end

  def self.format_ratio(candidate_time, baseline_time)
    return nil unless candidate_time && baseline_time

    ratio = candidate_time / baseline_time
    format("x%.2f", ratio)
  end

  # Returns minimum time per iteration (in seconds)
  def self.measure(warmup: 3, rounds: 10, &block)
    iterations = calibrate(&block)

    warmup.times { iterations.times { block.call } }

    times = Array.new(rounds) do
      start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      iterations.times { block.call }
      elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - start
      elapsed / iterations
    end
    times.min
  end

  def self.calibrate(&block) # rubocop:disable Metrics/MethodLength
    target = 0.5
    iters = 1
    loop do
      start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      iters.times { block.call }
      elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - start
      return [iters, 1].max if elapsed >= target
      return [iters, 1].max if iters > 1_000_000

      factor = elapsed > 0.001 ? (target / elapsed).ceil : 10
      iters = [iters * factor, iters * 10].min
    end
  end
end

def load_json(filename)
  JSON.parse(File.read(File.join(BENCHMARK_DATA_PATH, filename)))
end

def load_json_str(filename)
  File.read(File.join(BENCHMARK_DATA_PATH, filename))
end

BENCHMARKS = [
  { name: "OpenAPI",        schema: "openapi.json",             instance: "zuora.json" },
  { name: "Swagger",        schema: "swagger.json",             instance: "kubernetes.json" },
  { name: "GeoJSON",        schema: "geojson.json",             instance: "canada.json" },
  { name: "CITM Catalog",   schema: "citm_catalog_schema.json", instance: "citm_catalog.json" },
  { name: "Fast (Valid)",    schema: "fast_schema.json",         instance: "fast_valid.json" },
  { name: "Fast (Invalid)",  schema: "fast_schema.json",         instance: "fast_invalid.json" },
  { name: "FHIR",           schema: "fhir.schema.json",         instance: "patient-example-d.json" },
  { name: "Recursive",      schema: "recursive_schema.json",    instance: "recursive_instance.json" }
].freeze

# Pre-load all unique JSON files (parsed and raw strings)
DATA = {} # rubocop:disable Style/MutableConstant
DATA_STR = {} # rubocop:disable Style/MutableConstant
BENCHMARKS.each do |b|
  DATA_STR[b[:schema]] ||= load_json_str(b[:schema])
  DATA_STR[b[:instance]] ||= load_json_str(b[:instance])
  DATA[b[:schema]] ||= JSON.parse(DATA_STR[b[:schema]])
  DATA[b[:instance]] ||= JSON.parse(DATA_STR[b[:instance]])
end
DATA.freeze
DATA_STR.freeze

def try_compile(lib, schema, schema_str, instance_str) # rubocop:disable Metrics/MethodLength
  case lib
  when "jsonschema"
    JSONSchema.validator_for(schema)
  when "json_schemer"
    JSONSchemer.schema(schema)
  when "json-schema"
    # fully_validate raises JSON::Schema::SchemaError for unsupported drafts
    # (e.g. Draft 7), while validate() silently returns false.
    JSON::Validator.fully_validate(schema, {})
    :class_method
  when "rj_schema"
    # rj_schema uses RapidJSON (C++); verify it produces correct results
    # by cross-checking against jsonschema (Rust)
    result = RjSchema::Validator.new.validate(schema_str, instance_str)
    rj_valid = result[:machine_errors].empty?
    rs_valid = JSONSchema.valid?(schema, JSON.parse(instance_str))
    raise "rj_schema disagrees with jsonschema" if rj_valid != rs_valid

    :rj_validator
  end
rescue StandardError => e
  warn "  #{lib}: skipped (#{e.class}: #{e.message[0..80]})"
  nil
end

def make_validate_proc(lib, validator, schema, instance, schema_str, instance_str)
  case lib
  when "jsonschema", "json_schemer"
    -> { validator.valid?(instance) }
  when "json-schema"
    -> { JSON::Validator.validate(schema, instance) }
  when "rj_schema"
    # rj_schema works with JSON strings; includes parse time
    -> { RjSchema::Validator.new.validate(schema_str, instance_str) }
  end
end

def print_lib_result(lib, value)
  puts "  #{lib.ljust(15)}  #{value}"
end

def print_table_row(name, col1, col2, col3, col4)
  puts "| #{name.to_s.ljust(20)} | #{col1.to_s.ljust(30)} | #{col2.to_s.ljust(30)} " \
       "| #{col3.to_s.ljust(30)} | #{col4.to_s.ljust(25)} |"
end

LIBRARIES = %w[json-schema rj_schema json_schemer jsonschema].freeze

BenchHelper.section("JSON Schema Validation Benchmarks")
puts "jsonschema (Rust) vs json_schemer vs json-schema vs rj_schema"
rust_version = `rustc --version 2>/dev/null`.strip.split[1] || "unknown"
puts "Ruby #{RUBY_VERSION}, Rust #{rust_version}"
puts

results = []

BENCHMARKS.each do |bench|
  BenchHelper.subsection(bench[:name])

  schema = DATA[bench[:schema]]
  instance = DATA[bench[:instance]]
  schema_str = DATA_STR[bench[:schema]]
  instance_str = DATA_STR[bench[:instance]]

  validators = {}
  LIBRARIES.each { |lib| validators[lib] = try_compile(lib, schema, schema_str, instance_str) }

  row = { name: bench[:name] }

  LIBRARIES.each do |lib|
    v = validators[lib]
    unless v
      row[lib] = nil
      print_lib_result(lib, "-")
      next
    end

    validate_proc = make_validate_proc(lib, v, schema, instance, schema_str, instance_str)
    time = BenchHelper.measure(&validate_proc)
    row[lib] = time
    print_lib_result(lib, BenchHelper.format_time(time))
  rescue StandardError => e
    row[lib] = nil
    print_lib_result(lib, "error: #{e.message[0..60]}")
  end

  results << row
  puts
end

# Print markdown summary table
BenchHelper.section("Summary")

baseline_lib = "jsonschema"
libs = %w[json-schema rj_schema json_schemer]
header = "| #{'Benchmark'.ljust(20)} | #{'json-schema'.ljust(30)} | #{'rj_schema'.ljust(30)} " \
         "| #{'json_schemer'.ljust(30)} | #{'jsonschema (validate)'.ljust(25)} |"
separator = "|#{'-' * 22}|#{'-' * 32}|#{'-' * 32}|#{'-' * 32}|#{'-' * 27}|"

puts header
puts separator

results.each do |row|
  baseline = row[baseline_lib]
  cols = libs.map do |lib|
    t = row[lib]
    if t.nil?
      "-"
    elsif baseline
      ratio = BenchHelper.format_ratio(t, baseline)
      "#{BenchHelper.format_time(t)} (**#{ratio}**)"
    else
      BenchHelper.format_time(t)
    end
  end
  jsonschema_col = baseline ? BenchHelper.format_time(baseline) : "-"
  print_table_row(row[:name], cols[0], cols[1], cols[2], jsonschema_col)
end

puts
BenchHelper.section("Benchmarks complete!")

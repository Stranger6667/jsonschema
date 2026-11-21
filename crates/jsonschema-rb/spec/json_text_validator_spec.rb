# frozen_string_literal: true

require "spec_helper"
require "json"

RSpec.describe JSONSchema::JsonTextValidator do
  let(:schema) do
    {
      "type" => "object",
      "properties" => {
        "age" => { "type" => "integer", "minimum" => 0 },
        "tags" => { "type" => "array", "uniqueItems" => true }
      },
      "required" => ["age"]
    }
  end

  it "accepts UTF-8 and binary strings" do
    validator = described_class.new(schema)
    expect(validator.valid?('{"age": 5}')).to be true
    expect(validator.valid?('{"age": 5}'.b)).to be true
  end

  [
    '{"age": 5, "tags": ["a", "b"]}',
    '{"age": -1}',
    '{"tags": [1, 1.0]}',
    '{"age": "five", "tags": [{"a": 1}, {"a": 1}]}',
    "[]",
    '"text"'
  ].each do |text|
    it "matches the object validator on #{text}" do
      expected = JSONSchema.validator_for(schema)
      validator = described_class.new(schema)
      instance = JSON.parse(text)
      expect(validator.valid?(text)).to eq(expected.valid?(instance))
      summary = ->(error) { [error.message, error.instance, error.instance_path, error.schema_path, error.kind.name] }
      expect(validator.each_error(text).map(&summary)).to eq(expected.each_error(instance).map(&summary))
      expect(validator.evaluate(text).list).to eq(expected.evaluate(instance).list)
    end
  end

  it "keeps the last value of a repeated key" do
    validator = described_class.new(schema)
    expect(validator.valid?('{"age": 5, "age": "five"}')).to be false
    expect(validator.valid?('{"age": "five", "age": 5}')).to be true
  end

  it "yields errors to a block" do
    messages = []
    described_class.new(schema).each_error('{"age": -1}') { |error| messages << error.message }
    expect(messages).to eq(["-1 is less than the minimum of 0"])
  end

  it "raises the first error" do
    error = nil
    expect { described_class.new(schema).validate!('{"age": -1}') }
      .to raise_error(JSONSchema::ValidationError) { |e| error = e }
    expect(error.message).to eq("-1 is less than the minimum of 0")
    expect(error.instance).to eq(-1)
    expect(error.instance_path).to eq(["age"])
  end

  ['{"age": ', "", "NaN", "{'age': 5}", "\xFF".b, '{"age": 5} trailing'].each do |text|
    it "raises ArgumentError for malformed text #{text.inspect}" do
      expect { described_class.new(schema).valid?(text) }
        .to raise_error(ArgumentError, /\AInvalid JSON: /)
    end
  end

  [5, nil, { "age" => 5 }, ["{}"], :sym].each do |instance|
    it "raises TypeError for #{instance.inspect}" do
      expect { described_class.new(schema).valid?(instance) }
        .to raise_error(TypeError, /\AJSON text must be a String, not /)
    end
  end

  it "takes the draft from the argument or the schema" do
    expect(described_class.new({ "format" => "email" }).valid?('"not an email"')).to be true
    expect(described_class.new({ "format" => "email" }, draft: :draft7).valid?('"not an email"')).to be false
    draft7 = { "$schema" => "http://json-schema.org/draft-07/schema#", "format" => "email" }
    expect(described_class.new(draft7).valid?('"not an email"')).to be false
  end

  it "accepts a schema as JSON text" do
    expect(described_class.new('{"type": "integer"}').valid?('"5"')).to be false
  end

  it "rejects an invalid schema" do
    expect { described_class.new({ "type" => "unknown" }) }.to raise_error(ArgumentError)
  end

  it "passes Ruby objects to custom keywords" do
    seen = []
    keyword = Class.new do
      define_method(:initialize) { |_parent_schema, _value, _schema_path| nil }
      define_method(:validate) do |instance|
        seen << instance
        raise ArgumentError, "#{instance} is odd" if instance.odd?
      end
    end
    validator = described_class.new({ "properties" => { "n" => { "even" => true } } }, keywords: { "even" => keyword })
    expect(validator.valid?('{"n": 4}')).to be true
    expect(validator.valid?('{"n": 3}')).to be false
    error = nil
    expect { validator.validate!('{"n": 3}') }
      .to raise_error(JSONSchema::ValidationError) { |e| error = e }
    expect(error.message).to eq("3 is odd")
    expect(error.cause).to be_a(ArgumentError)
    expect(seen).to eq([4, 3, 3])
  end

  it "raises a format checker's error from each_error with a block" do
    validator = described_class.new(
      { "format" => "boom" }, formats: { "boom" => ->(_value) { raise ArgumentError, "boom" } }, validate_formats: true
    )
    expect { validator.each_error('"a"') { |_error| nil } }.to raise_error(ArgumentError, "boom")
  end

  document = '{"b": 1, "a": [2.50, 1E2, 12345678901234567890123, -0, "x"], "c": {"z": null, "y": true}}'

  it "passes custom keywords what JSON.parse builds" do
    seen = []
    keyword = Class.new do
      define_method(:initialize) { |_parent_schema, _value, _schema_path| nil }
      define_method(:validate) { |instance| seen << instance }
    end
    expect(described_class.new({ "record" => true }, keywords: { "record" => keyword }).valid?(document)).to be true
    expected = JSON.parse(document)
    expect(seen).to eq([expected])
    expect(seen.first.keys).to eq(expected.keys)
    expect(seen.first["a"].map(&:class)).to eq(expected["a"].map(&:class))
  end

  it "gives errors the instance JSON.parse builds" do
    error = described_class.new({ "maxProperties" => 1 }).each_error(document).first
    expected = JSON.parse(document)
    expect(error.instance).to eq(expected)
    expect(error.instance.keys).to eq(expected.keys)
    expect(error.instance["a"].map(&:class)).to eq(expected["a"].map(&:class))
  end

  it "quotes numeric keys in verbose messages" do
    error = nil
    expect { described_class.new({ "properties" => { "123" => { "type" => "string" } } }).validate!('{"123": 42}') }
      .to raise_error(JSONSchema::ValidationError) { |e| error = e }
    expect(error.verbose_message).to include('On instance["123"]:')
  end

  it "reads strings in other encodings as JSON.parse does" do
    latin1 = "\"\xC3\xA9\"".b.force_encoding("ISO-8859-1")
    expected = JSONSchema.validator_for({ "maxLength" => 1 }).valid?(JSON.parse(latin1))
    expect(described_class.new({ "maxLength" => 1 }).valid?(latin1)).to eq(expected)
    expect(described_class.new(schema).valid?('{"age": 5}'.encode("UTF-16LE"))).to be true
  end

  it "runs custom formats" do
    validator = described_class.new(
      { "format" => "upper" }, formats: { "upper" => ->(value) { value == value.upcase } }, validate_formats: true
    )
    expect(validator.valid?('"ABC"')).to be true
    expect(validator.valid?('"abc"')).to be false
  end

  it "masks the instance in messages" do
    error = nil
    expect { described_class.new({ "type" => "integer" }, mask: "[REDACTED]").validate!('"secret"') }
      .to raise_error(JSONSchema::ValidationError) { |e| error = e }
    expect(error.message).to eq('[REDACTED] is not of type "integer"')
  end

  {
    "http://json-schema.org/draft-04/schema#" => "Draft4",
    "http://json-schema.org/draft-06/schema#" => "Draft6",
    "http://json-schema.org/draft-07/schema#" => "Draft7",
    "https://json-schema.org/draft/2019-09/schema" => "Draft201909",
    "https://json-schema.org/draft/2020-12/schema" => "Draft202012"
  }.each do |uri, name|
    it "names #{name} in inspect" do
      expect(described_class.new({ "$schema" => uri }).inspect).to eq("#<JSONSchema::JsonTextValidator: #{name}>")
    end
  end

  it "collects every error a custom keyword reports" do
    keyword = Class.new do
      define_method(:initialize) { |_parent_schema, _value, _schema_path| nil }
      define_method(:validate) { |_instance| nil }
      define_method(:iter_errors) do |instance|
        instance.select { |_key, value| value.odd? }.map { |key, _value| ArgumentError.new("#{key} is odd") }
      end
    end
    text = '{"a": 1, "b": 2, "c": 3}'
    keywords = { "oddMembers" => keyword }
    expected = JSONSchema.validator_for({ "oddMembers" => true }, keywords: keywords).each_error(JSON.parse(text)).map(&:message)
    expect(expected).to eq(["a is odd", "c is odd"])
    validator = described_class.new({ "oddMembers" => true }, keywords: keywords)
    expect(validator.each_error(text).map(&:message)).to eq(expected)
  end
end

# frozen_string_literal: true

require "spec_helper"

RSpec.describe "JSONSchema.canonicalize" do
  [
    { "type" => "string", "minLength" => 3 },
    { "allOf" => [{ "type" => "integer" }, { "minimum" => 0 }] },
    { "$defs" => { "a" => { "type" => "null" } }, "$ref" => "#/$defs/a" },
    {},
    true,
    false
  ].each do |schema|
    it "round-trips #{schema.inspect} verbatim" do
      result = JSONSchema.canonicalize(schema)
      expect(result).to be_a(JSONSchema::Canonical::CanonicalSchema)
      expect(result.to_json_schema).to eq(schema)
    end
  end

  it "view returns RawView with the document payload" do
    case JSONSchema.canonicalize({ "type" => "string" }).view
    in JSONSchema::Canonical::RawView[schema:]
      expect(schema).to eq({ "type" => "string" })
    end
  end

  it "detects draft from $schema" do
    schema = { "$schema" => "http://json-schema.org/draft-07/schema#" }
    expect(JSONSchema.canonicalize(schema).draft).to eq(:draft7)
  end

  it "defaults to draft202012" do
    expect(JSONSchema.canonicalize({}).draft).to eq(:draft202012)
  end

  it "respects the draft keyword" do
    expect(JSONSchema.canonicalize({}, draft: :draft4).draft).to eq(:draft4)
  end

  it "kind is :raw" do
    expect(JSONSchema.canonicalize({}).kind).to eq(:raw)
  end

  it "equality is document identity" do
    expect(JSONSchema.canonicalize({ "const" => 1 })).to eq(JSONSchema.canonicalize({ "const" => 1 }))
    expect(JSONSchema.canonicalize({ "const" => 1 })).not_to eq(JSONSchema.canonicalize({ "const" => 1.0 }))
    lookup = { JSONSchema.canonicalize({ "const" => 1 }) => 1 }
    expect(lookup[JSONSchema.canonicalize({ "const" => 1 })]).to eq(1)
  end

  it "definitions is empty" do
    schema = { "$defs" => { "a" => {} }, "$ref" => "#/$defs/a" }
    expect(JSONSchema.canonicalize(schema).definitions).to eq({})
  end

  it "raises ValidationError when meta-validation fails" do
    expect { JSONSchema.canonicalize({ "type" => 123 }) }.to raise_error(JSONSchema::ValidationError)
  end

  [42, "string", [1]].each do |schema|
    it "raises InvalidSchemaType for #{schema.inspect}" do
      expect { JSONSchema.canonicalize(schema) }.to raise_error(JSONSchema::Canonical::InvalidSchemaType)
    end
  end

  it "exception hierarchy is rooted in StandardError" do
    expect(JSONSchema::Canonical::InvalidSchemaType).to be < JSONSchema::Canonical::CanonicalizationError
    expect(JSONSchema::Canonical::CanonicalizationError).to be < StandardError
  end
end

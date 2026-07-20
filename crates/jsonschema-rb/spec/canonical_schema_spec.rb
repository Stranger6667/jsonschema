# frozen_string_literal: true

require "spec_helper"

DRAFT202012 = "https://json-schema.org/draft/2020-12/schema"

RSpec.describe "JSONSchema.canonicalize" do
  [
    { "type" => "string", "minLength" => 3 },
    { "allOf" => [{ "type" => "integer" }, { "minimum" => 0 }] },
    { "$defs" => { "a" => { "type" => "null" } }, "$ref" => "#/$defs/a" }
  ].each do |schema|
    it "round-trips unmodeled #{schema.inspect} verbatim" do
      result = JSONSchema.canonicalize(schema)
      expect(result).to be_a(JSONSchema::Canonical::CanonicalSchema)
      expect(result.to_json_schema).to eq(schema)
      expect(result.kind).to eq(:raw)
    end
  end

  [
    [{ "enum" => [5] }, { "$schema" => DRAFT202012, "const" => 5 }],
    [{ "enum" => ["z", 2, nil, 1] }, { "$schema" => DRAFT202012, "enum" => [nil, 1, 2, "z"] }],
    [{ "const" => nil }, { "$schema" => DRAFT202012, "type" => "null" }],
    [{ "type" => %w[integer string] }, { "$schema" => DRAFT202012, "type" => %w[integer string] }],
    [{ "type" => "boolean", "enum" => [true] }, { "$schema" => DRAFT202012, "const" => true }],
    [{ "type" => "integer", "enum" => [1, "x", 2] }, { "$schema" => DRAFT202012, "enum" => [1, 2] }]
  ].each do |schema, expected|
    it "canonicalizes #{schema.inspect}" do
      expect(JSONSchema.canonicalize(schema).to_json_schema).to eq(expected)
    end
  end

  it "view returns ConstView for a singleton enum" do
    case JSONSchema.canonicalize({ "enum" => [5] }).view
    in JSONSchema::Canonical::ConstView[value:]
      expect(value).to eq(5)
    end
  end

  it "view returns EnumView with sorted values" do
    case JSONSchema.canonicalize({ "enum" => [2, 1] }).view
    in JSONSchema::Canonical::EnumView[values:]
      expect(values).to eq([1, 2])
    end
  end

  it "view returns MultiTypeView for a type list" do
    case JSONSchema.canonicalize({ "type" => %w[string integer] }).view
    in JSONSchema::Canonical::MultiTypeView[types:]
      expect(types).to eq(%i[integer string])
    end
  end

  it "view returns TrueView and FalseView for trivial schemas" do
    expect(JSONSchema.canonicalize({}).view).to be_a(JSONSchema::Canonical::TrueView)
    expect(JSONSchema.canonicalize(false).view).to be_a(JSONSchema::Canonical::FalseView)
  end

  it "view returns TypedGroupView for a Draft 4 integer enum" do
    schema = {
      "$schema" => "http://json-schema.org/draft-04/schema#",
      "type" => "integer",
      "enum" => [1, 2]
    }
    case JSONSchema.canonicalize(schema).view
    in JSONSchema::Canonical::TypedGroupView[type_name:, body:]
      expect(type_name).to eq(:integer)
      case body.view
      in JSONSchema::Canonical::EnumView[values:]
        expect(values).to eq([1, 2])
      end
    end
  end

  it "view returns RawView with the document payload" do
    case JSONSchema.canonicalize({ "not" => {} }).view
    in JSONSchema::Canonical::RawView[schema:]
      expect(schema).to eq({ "not" => {} })
    end
  end

  [
    [{ "const" => 5 }, :const],
    [{ "enum" => [1, 2] }, :enum],
    [{ "type" => %w[integer string] }, :multi_type],
    [{}, :true], # rubocop:disable Lint/BooleanSymbol
    [false, :false], # rubocop:disable Lint/BooleanSymbol
    [{ "pattern" => "a" }, :raw]
  ].each do |schema, kind|
    it "kind of #{schema.inspect} is #{kind.inspect}" do
      expect(JSONSchema.canonicalize(schema).kind).to eq(kind)
    end
  end

  it "satisfiable? reflects provable emptiness" do
    expect(JSONSchema.canonicalize({ "const" => 5 }).satisfiable?).to be(true)
    expect(JSONSchema.canonicalize({ "type" => "integer", "enum" => ["x"] }).satisfiable?).to be(false)
  end

  it "equality is value identity" do
    expect(JSONSchema.canonicalize({ "enum" => [5] })).to eq(JSONSchema.canonicalize({ "const" => 5 }))
    expect(JSONSchema.canonicalize({ "const" => 1 })).to eq(JSONSchema.canonicalize({ "const" => 1.0 }))
    lookup = { JSONSchema.canonicalize({ "const" => 1 }) => 1 }
    expect(lookup[JSONSchema.canonicalize({ "const" => 1 })]).to eq(1)
  end

  it "detects draft from $schema" do
    schema = { "$schema" => "http://json-schema.org/draft-07/schema#" }
    expect(JSONSchema.canonicalize(schema).draft).to eq(:draft7)
  end

  it "respects the draft keyword" do
    expect(JSONSchema.canonicalize({}, draft: :draft4).draft).to eq(:draft4)
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

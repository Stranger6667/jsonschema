# frozen_string_literal: true

require "spec_helper"

RSpec.describe "JSONSchema.validator_map_for" do
  let(:schema) do
    {
      "$defs" => {
        "User" => {
          "type" => "object",
          "properties" => { "name" => { "type" => "string" } },
          "required" => ["name"]
        },
        "Address" => {
          "type" => "object",
          "properties" => { "city" => { "type" => "string" } }
        }
      },
      "type" => "object"
    }
  end

  it "returns a ValidatorMap" do
    m = JSONSchema.validator_map_for(schema)
    expect(m).to be_a(JSONSchema::ValidatorMap)
  end

  it "raises on an invalid schema" do
    expect { JSONSchema.validator_map_for({ "type" => "not-a-valid-type" }) }
      .to raise_error(ArgumentError)
  end

  describe "#[]" do
    it "returns a validator for an existing pointer" do
      m = JSONSchema.validator_map_for(schema)
      v = m["#/$defs/User"]
      expect(v).not_to be_nil
      expect(v.valid?({ "name" => "Alice" })).to be true
      expect(v.valid?(42)).to be false
    end

    it "returns nil for a missing pointer" do
      m = JSONSchema.validator_map_for(schema)
      expect(m["#/nonexistent"]).to be_nil
    end
  end

  describe "#fetch" do
    it "returns a validator for an existing pointer" do
      m = JSONSchema.validator_map_for(schema)
      v = m.fetch("#/$defs/User")
      expect(v.valid?({ "name" => "Alice" })).to be true
    end

    it "raises KeyError for a missing pointer" do
      m = JSONSchema.validator_map_for(schema)
      expect { m.fetch("#/nonexistent") }.to raise_error(KeyError)
    end
  end

  describe "#key?" do
    it "returns true for an existing pointer" do
      m = JSONSchema.validator_map_for(schema)
      expect(m.key?("#/$defs/User")).to be true
    end

    it "returns false for a missing pointer" do
      m = JSONSchema.validator_map_for(schema)
      expect(m.key?("#/nonexistent")).to be false
    end
  end

  describe "#keys" do
    it "includes root and all $defs" do
      m = JSONSchema.validator_map_for(schema)
      keys = m.keys
      expect(keys).to include("#")
      expect(keys).to include("#/$defs/User")
      expect(keys).to include("#/$defs/Address")
    end
  end

  describe "#length" do
    it "is at least 3 (root + User + Address)" do
      m = JSONSchema.validator_map_for(schema)
      expect(m.length).to be >= 3
    end
  end

  describe "#size" do
    it "equals length" do
      m = JSONSchema.validator_map_for(schema)
      expect(m.size).to eq(m.length)
    end
  end

  it "root entry validates any object" do
    m = JSONSchema.validator_map_for(schema)
    v = m["#"]
    expect(v).not_to be_nil
    expect(v.valid?({})).to be true
  end

  it "returned validator supports each_error" do
    m = JSONSchema.validator_map_for(schema)
    v = m.fetch("#/$defs/User")
    errors = []
    v.each_error(42) { |e| errors << e }
    expect(errors).not_to be_empty
  end

  it "returned validator raises on validate!" do
    m = JSONSchema.validator_map_for(schema)
    v = m.fetch("#/$defs/User")
    expect { v.validate!(42) }.to raise_error(JSONSchema::ValidationError)
  end

  it "propagates mask to returned validators" do
    m = JSONSchema.validator_map_for(schema, mask: "***")
    v = m.fetch("#/$defs/User")
    expect { v.validate!(42) }.to raise_error(JSONSchema::ValidationError) do |e|
      expect(e.message).to eq('*** is not of type "object"')
    end
  end

  it "propagates mask to returned validators via []" do
    m = JSONSchema.validator_map_for(schema, mask: "***")
    v = m["#/$defs/User"]
    expect { v.validate!(42) }.to raise_error(JSONSchema::ValidationError) do |e|
      expect(e.message).to eq('*** is not of type "object"')
    end
  end
end

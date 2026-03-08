# frozen_string_literal: true

require "spec_helper"

RSpec.describe "JSONSchema.bundle" do
  let(:person_schema) do
    {
      "$id" => "https://example.com/person.json",
      "$schema" => "https://json-schema.org/draft/2020-12/schema",
      "type" => "object",
      "properties" => { "name" => { "type" => "string" } },
      "required" => ["name"]
    }.freeze
  end

  it "returns a hash for a schema with no external refs" do
    bundled = JSONSchema.bundle({ "type" => "string" })
    expect(bundled).to be_a(Hash)
    expect(bundled).not_to have_key("$defs")
  end

  it "embeds a single external ref in $defs" do
    root = {
      "$schema" => "https://json-schema.org/draft/2020-12/schema",
      "$ref" => "https://example.com/person.json"
    }
    registry = JSONSchema::Registry.new([["https://example.com/person.json", person_schema]])
    bundled = JSONSchema.bundle(root, registry: registry)
    # $ref must NOT be rewritten (spec requirement)
    expect(bundled["$ref"]).to eq("https://example.com/person.json")
    expect(bundled.dig("$defs", "https://example.com/person.json")).not_to be_nil
  end

  it "produces a bundled schema that validates identically" do
    root = {
      "$schema" => "https://json-schema.org/draft/2020-12/schema",
      "$ref" => "https://example.com/person.json"
    }
    registry = JSONSchema::Registry.new([["https://example.com/person.json", person_schema]])
    bundled = JSONSchema.bundle(root, registry: registry)
    validator = JSONSchema.validator_for(bundled)
    expect(validator.valid?({ "name" => "Alice" })).to be true
    expect(validator.valid?({ "age" => 30 })).to be false
  end

  it "raises when a $ref cannot be resolved" do
    expect do
      JSONSchema.bundle({ "$ref" => "https://example.com/missing.json" })
    end.to raise_error(
      JSONSchema::ReferencingError,
      %r{https://example.com/missing.json}
    )
  end

  it "resolves refs with nested $id scope" do
    root = {
      "$schema" => "https://json-schema.org/draft/2020-12/schema",
      "$defs" => { "A" => { "$id" => "https://example.com/A/", "$ref" => "b.json" } }
    }
    registry = JSONSchema::Registry.new([["https://example.com/A/b.json", { "type" => "integer" }]])

    bundled = JSONSchema.bundle(root, registry: registry)
    expect(bundled.dig("$defs", "A")).not_to be_nil
    expect(bundled.dig("$defs", "https://example.com/A/b.json")).not_to be_nil
  end

  it "ignores $ref inside const annotation payloads" do
    schema = {
      "$schema" => "https://json-schema.org/draft/2020-12/schema",
      "const" => { "$ref" => "https://example.com/not-a-schema" }
    }
    bundled = JSONSchema.bundle(schema)
    expect(bundled).to eq(schema)
    expect(bundled).not_to have_key("$defs")
  end

  it "supports legacy drafts for bundling via definitions" do
    resource_uri = "https://example.com/legacy/person.json"
    [
      "http://json-schema.org/draft-04/schema#",
      "http://json-schema.org/draft-06/schema#",
      "http://json-schema.org/draft-07/schema#"
    ].each do |schema_uri|
      root = { "$schema" => schema_uri, "$ref" => resource_uri }
      registry = JSONSchema::Registry.new([
                                            [resource_uri, { "$schema" => schema_uri, "type" => "integer", "minimum" => 0 }]
                                          ])
      distributed = JSONSchema.validator_for(root, registry: registry)

      bundled = JSONSchema.bundle(root, registry: registry)
      bundled_validator = JSONSchema.validator_for(bundled)
      expect(bundled).not_to have_key("$defs")
      expect(bundled.dig("definitions", resource_uri)).not_to be_nil

      embedded = bundled.dig("definitions", resource_uri)
      expect(embedded["id"] == resource_uri || embedded["$id"] == resource_uri).to be true

      [0, 5].each do |instance|
        expect(distributed.valid?(instance)).to be true
        expect(bundled_validator.valid?(instance)).to be true
      end
      [-1, "x", 1.5].each do |instance|
        expect(distributed.valid?(instance)).to be false
        expect(bundled_validator.valid?(instance)).to be false
      end
    end
  end

  it "handles mixed draft refs by injecting both id keywords" do
    resource_uri = "https://example.com/mixed/schema.json"
    root = {
      "$schema" => "http://json-schema.org/draft-07/schema#",
      "$ref" => resource_uri
    }
    registry = JSONSchema::Registry.new([
                                          [resource_uri, { "$schema" => "http://json-schema.org/draft-04/schema#", "type" => "integer" }]
                                        ])

    distributed = JSONSchema.validator_for(root, registry: registry)
    bundled = JSONSchema.bundle(root, registry: registry)
    bundled_validator = JSONSchema.validator_for(bundled)

    embedded = bundled.dig("definitions", resource_uri)
    expect(embedded["id"]).to eq(resource_uri)
    expect(embedded["$id"]).to eq(resource_uri)
    expect(distributed.valid?(1)).to be true
    expect(bundled_validator.valid?(1)).to be true
    expect(distributed.valid?("x")).to be false
    expect(bundled_validator.valid?("x")).to be false
  end

  it "preserves mixed-draft const semantics" do
    resource_uri = "https://example.com/mixed/const.json"
    root = {
      "$schema" => "http://json-schema.org/draft-04/schema#",
      "$ref" => resource_uri
    }
    registry = JSONSchema::Registry.new([
                                          [resource_uri, { "$schema" => "http://json-schema.org/draft-07/schema#", "const" => 1 }]
                                        ])

    distributed = JSONSchema.validator_for(root, registry: registry)
    bundled = JSONSchema.bundle(root, registry: registry)
    bundled_validator = JSONSchema.validator_for(bundled)

    expect(distributed.valid?(1)).to be true
    expect(bundled_validator.valid?(1)).to be true
    expect(distributed.valid?(2)).to be false
    expect(bundled_validator.valid?(2)).to be false
  end
end

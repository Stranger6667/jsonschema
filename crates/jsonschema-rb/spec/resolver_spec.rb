# frozen_string_literal: true

require "spec_helper"

RSpec.describe "JSONSchema::Resolver" do
  let(:nested_resources) do
    [
      [
        "https://example.com/address.json",
        {
          "$schema" => "https://json-schema.org/draft/2020-12/schema",
          "type" => "object",
          "properties" => { "street" => { "type" => "string" }, "city" => { "type" => "string" } },
          "required" => %w[street city]
        }
      ],
      [
        "https://example.com/person.json",
        {
          "$schema" => "https://json-schema.org/draft/2020-12/schema",
          "type" => "object",
          "properties" => { "name" => { "type" => "string" }, "address" => { "$ref" => "address.json" } },
          "required" => %w[name address]
        }
      ]
    ]
  end

  let(:anchor_resource) do
    [
      [
        "https://example.com/anchors.json",
        {
          "$schema" => "https://json-schema.org/draft/2020-12/schema",
          "$defs" => {
            "thing" => {
              "$anchor" => "thing",
              "type" => "string"
            }
          }
        }
      ]
    ]
  end

  it "returns base_uri and empty dynamic_scope on creation" do
    registry = JSONSchema::Registry.new(nested_resources)
    resolver = registry.resolver("https://example.com/person.json")

    expect(resolver.base_uri).to eq("https://example.com/person.json")
    expect(resolver.dynamic_scope).to eq([])
  end

  it "lookup returns contents, follow-up resolver, and draft" do
    registry = JSONSchema::Registry.new(nested_resources)
    root = registry.resolver("https://example.com/person.json")
    resolved_root = root.lookup("")

    expect(resolved_root.contents["properties"]["address"]["$ref"]).to eq("address.json")
    expect(resolved_root.draft).to eq(JSONSchema::Draft202012)
    expect(resolved_root.resolver.base_uri).to eq("https://example.com/person.json")
    expect(resolved_root.resolver.dynamic_scope).to eq(["https://example.com/person.json"])

    ref = resolved_root.contents["properties"]["address"]["$ref"]
    resolved_address = resolved_root.resolver.lookup(ref)

    expect(resolved_address.contents["required"]).to eq(%w[street city])
    expect(resolved_address.resolver.base_uri).to eq("https://example.com/address.json")
    person_uri = "https://example.com/person.json"
    expect(resolved_address.resolver.dynamic_scope).to eq([person_uri, person_uri])
  end

  it "supports retriever-backed resources" do
    retrieve = lambda do |uri|
      return { "type" => "number" } if uri == "https://example.com/dynamic.json"

      raise KeyError, "Schema not found: #{uri}"
    end

    registry = JSONSchema::Registry.new(
      [["https://example.com/inner.json", { "$ref" => "https://example.com/dynamic.json" }]],
      retriever: retrieve
    )

    root = registry.resolver("https://example.com/inner.json")
    resolved_inner = root.lookup("")
    resolved_dynamic = resolved_inner.resolver.lookup(resolved_inner.contents["$ref"])

    expect(resolved_dynamic.contents).to eq({ "type" => "number" })
    expect(resolved_dynamic.resolver.base_uri).to eq("https://example.com/dynamic.json")
  end

  it "supports JSON pointer and anchor lookups" do
    registry = JSONSchema::Registry.new(anchor_resource)
    resolver = registry.resolver("https://example.com/anchors.json")

    pointer = resolver.lookup("#/$defs/thing")
    anchor = resolver.lookup("#thing")

    expect(pointer.contents).to eq({ "$anchor" => "thing", "type" => "string" })
    expect(anchor.contents).to eq({ "$anchor" => "thing", "type" => "string" })
    expect(anchor.resolver.base_uri).to eq("https://example.com/anchors.json")
  end

  it "raises ReferencingError for a missing resource" do
    registry = JSONSchema::Registry.new(nested_resources)
    resolver = registry.resolver("https://example.com/person.json")

    expect do
      resolver.lookup("https://example.com/missing.json")
    end.to raise_error(JSONSchema::ReferencingError, /missing\.json/)
  end
end

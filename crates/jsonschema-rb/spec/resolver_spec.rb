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

  [
    ["lookup", { "$id" => "inner", "$dynamicAnchor" => "item", "type" => "string" }, "https://example.com/inner"],
    ["lookup_dynamic_ref", { "$dynamicAnchor" => "item", "type" => "null" }, "https://example.com/root"]
  ].each do |method, expected, base_uri|
    it "#{method} of a dynamic anchor with the same name in an outer resource" do
      registry = JSONSchema::Registry.new(
        [
          [
            "https://example.com/root",
            {
              "$schema" => "https://json-schema.org/draft/2020-12/schema",
              "$defs" => {
                "outer" => { "$dynamicAnchor" => "item", "type" => "null" },
                "inner" => { "$id" => "inner", "$dynamicAnchor" => "item", "type" => "string" }
              }
            }
          ]
        ]
      )
      resolver = registry.resolver("https://example.com/root")

      resolved = resolver.public_send(method, "inner#item")

      expect(resolved.contents).to eq(expected)
      expect(resolved.resolver.base_uri).to eq(base_uri)
    end
  end

  it "raises ReferencingError for a missing resource" do
    registry = JSONSchema::Registry.new(nested_resources)
    resolver = registry.resolver("https://example.com/person.json")

    expect do
      resolver.lookup("https://example.com/missing.json")
    end.to raise_error(JSONSchema::ReferencingError, /missing\.json/)
  end
end

RSpec.describe JSONSchema::Registry do
  {
    "http://json-schema.org/draft-04/schema#" => JSONSchema::Draft4,
    "http://json-schema.org/draft-06/schema#" => JSONSchema::Draft6,
    "http://json-schema.org/draft-07/schema#" => JSONSchema::Draft7,
    "https://json-schema.org/draft/2019-09/schema" => JSONSchema::Draft201909
  }.each do |dialect, draft|
    it "resolves a #{dialect} resource under its own draft" do
      registry = described_class.new([["https://example.com/s.json", { "$schema" => dialect }]])
      expect(registry.resolver("https://example.com/s.json").lookup("").draft).to eq(draft)
    end
  end

  it "reads a resource without $schema under the draft keyword" do
    registry = described_class.new([["https://example.com/s.json", {}]], draft: :draft7)
    expect(registry.resolver("https://example.com/s.json").lookup("").draft).to eq(JSONSchema::Draft7)
  end

  it "ignores a nil retriever" do
    registry = described_class.new([["https://example.com/s.json", {}]], retriever: nil)
    expect(registry.resolver("https://example.com/s.json").lookup("").contents).to eq({})
  end

  [
    ["a resource that is not a pair", [[["https://example.com/s.json"]]], {}, ArgumentError,
     "Each resource must be a [uri, schema] pair"],
    ["a malformed resource URI", [[["http://[::1", {}]]], {}, ArgumentError,
     "Invalid URI reference 'http://[::1': unexpected character at index 11"],
    ["a retriever that is not callable", [[]], { retriever: 1 }, TypeError,
     "Retriever must be a callable (Proc or Lambda)"],
    [
      "a retriever result with no JSON form",
      [[["https://example.com/s.json", { "$ref" => "https://example.com/x.json" }]]],
      { retriever: ->(_uri) { Object.new } },
      ArgumentError,
      "Resource 'https://example.com/x.json' is not present in a registry and retrieving it failed: " \
      "Failed to convert retriever result: TypeError: Unsupported type: 'Object'"
    ]
  ].each do |label, args, kwargs, error, message|
    it "raises #{error} for #{label}" do
      expect { described_class.new(*args, **kwargs) }.to raise_error(error, message)
    end
  end

  it "raises ArgumentError for a malformed resolver base URI" do
    expect { described_class.new([]).resolver("http://[::1") }
      .to raise_error(ArgumentError, "Invalid URI reference 'http://[::1': unexpected character at index 11")
  end
end

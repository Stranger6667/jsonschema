# frozen_string_literal: true

require "spec_helper"

RSpec.describe "JSONSchema.dereference" do
  it "returns schema unchanged when there are no refs" do
    schema = { "type" => "string", "minLength" => 1 }
    expect(JSONSchema.dereference(schema)).to eq(schema)
  end

  it "inlines a simple fragment ref" do
    schema = {
      "$defs" => {
        "address" => {
          "type" => "object",
          "properties" => {
            "street" => { "type" => "string" },
            "city" => { "type" => "string" }
          }
        }
      },
      "properties" => { "home" => { "$ref" => "#/$defs/address" } }
    }
    expect(JSONSchema.dereference(schema)).to eq({
                                                   "$defs" => {
                                                     "address" => {
                                                       "type" => "object",
                                                       "properties" => {
                                                         "street" => { "type" => "string" },
                                                         "city" => { "type" => "string" }
                                                       }
                                                     }
                                                   },
                                                   "properties" => {
                                                     "home" => {
                                                       "type" => "object",
                                                       "properties" => {
                                                         "street" => { "type" => "string" },
                                                         "city" => { "type" => "string" }
                                                       }
                                                     }
                                                   }
                                                 })
  end

  it "leaves circular refs in place" do
    schema = {
      "$schema" => "https://json-schema.org/draft/2020-12/schema",
      "$id" => "https://example.com/node.json",
      "type" => "object",
      "properties" => {
        "children" => {
          "type" => "array",
          "items" => { "$ref" => "https://example.com/node.json" }
        }
      }
    }
    expect(JSONSchema.dereference(schema)).to eq({
                                                   "$schema" => "https://json-schema.org/draft/2020-12/schema",
                                                   "$id" => "https://example.com/node.json",
                                                   "type" => "object",
                                                   "properties" => {
                                                     "children" => {
                                                       "type" => "array",
                                                       "items" => {
                                                         "$schema" => "https://json-schema.org/draft/2020-12/schema",
                                                         "$id" => "https://example.com/node.json",
                                                         "type" => "object",
                                                         "properties" => {
                                                           "children" => {
                                                             "type" => "array",
                                                             "items" => { "$ref" => "https://example.com/node.json" }
                                                           }
                                                         }
                                                       }
                                                     }
                                                   }
                                                 })
  end

  it "merges sibling keys alongside a ref" do
    schema = {
      "$defs" => { "base" => { "type" => "integer" } },
      "properties" => {
        "count" => { "$ref" => "#/$defs/base", "description" => "how many" }
      }
    }
    expect(JSONSchema.dereference(schema)).to eq({
                                                   "$defs" => { "base" => { "type" => "integer" } },
                                                   "properties" => {
                                                     "count" => { "type" => "integer", "description" => "how many" }
                                                   }
                                                 })
  end

  it "raises ReferencingError for an unknown ref" do
    schema = {
      "$schema" => "https://json-schema.org/draft/2020-12/schema",
      "$ref" => "https://example.com/does-not-exist.json"
    }
    expect do
      JSONSchema.dereference(schema)
    end.to raise_error(JSONSchema::ReferencingError)
  end

  it "dereferences with a custom retriever" do
    retrieve = lambda do |uri|
      return { "type" => "string" } if uri == "https://example.com/string.json"

      raise KeyError, "not found: #{uri}"
    end

    schema = {
      "$schema" => "https://json-schema.org/draft/2020-12/schema",
      "$ref" => "https://example.com/string.json"
    }
    result = JSONSchema.dereference(schema, retriever: retrieve)
    expect(result).to eq({
                           "$schema" => "https://json-schema.org/draft/2020-12/schema",
                           "type" => "string"
                         })
  end
end

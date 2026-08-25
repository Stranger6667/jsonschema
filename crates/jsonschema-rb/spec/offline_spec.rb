# frozen_string_literal: true

require "spec_helper"

RSpec.describe "offline retrieval" do
  let(:remote_schema) { { "$ref" => "https://example.com/schema.json" } }

  it "refuses a remote reference" do
    expect { JSONSchema.validator_for(remote_schema, offline: true) }
      .to raise_error(JSONSchema::ReferencingError, /Retrieval is disabled/)
  end

  it "rejects a retriever" do
    expect { JSONSchema.validator_for(remote_schema, offline: true, retriever: ->(_uri) { {} }) }
      .to raise_error(ArgumentError, /`offline` cannot be used together with `retriever`/)
  end

  it "allows registry references" do
    registry = JSONSchema::Registry.new([["https://example.com/defs.json", { "type" => "integer", "minimum" => 1 }]])
    validator = JSONSchema.validator_for(
      { "$ref" => "https://example.com/defs.json" },
      registry: registry,
      offline: true
    )
    expect(validator.valid?(3)).to be true
    expect(validator.valid?(0)).to be false
  end
end

# frozen_string_literal: true

require "spec_helper"

RSpec.describe "JSONSchema::Canonical.find_unsatisfiable" do
  def reason_data(reason)
    case reason
    in JSONSchema::Canonical::LiteralReason
      "literal"
    in JSONSchema::Canonical::EmptyReason[cause:]
      ["empty", [cause.pointer, cause.keywords]]
    in JSONSchema::Canonical::ConflictReason[causes:]
      ["conflict", causes.map { |cause| [cause.pointer, cause.keywords] }]
    end
  end

  def reported(schema)
    JSONSchema::Canonical.find_unsatisfiable(schema).transform_values { |reason| reason_data(reason) }
  end

  [
    ["a live document", { "type" => "object", "properties" => { "a" => { "type" => "string" } } }, {}],
    [
      "an unmodeled document",
      { "$defs" => { "I" => { "type" => "integer" } }, "$ref" => "#/$defs/I",
        "dependencies" => {}, "unevaluatedProperties" => false },
      {}
    ],
    ["a literal false", { "properties" => { "a" => false } }, { "/properties/a" => "literal" }],
    [
      "a numeric window nothing falls in",
      { "properties" => { "a" => { "type" => "integer", "minimum" => 5, "maximum" => 3 } } },
      { "/properties/a" => ["empty", ["/properties/a", %w[minimum maximum]]] }
    ],
    [
      "siblings empty beside a reference",
      { "$defs" => { "I" => { "type" => "integer" } }, "$ref" => "#/$defs/I",
        "type" => "string", "enum" => [1] },
      { "" => ["empty", ["", %w[type enum]]] }
    ],
    [
      "an array element no schema admits",
      { "properties" => { "a" => { "type" => "array", "minItems" => 1, "items" => false } } },
      { "/properties/a" => ["conflict", [["/properties/a", %w[type]], ["/properties/a", %w[items minItems]]]],
        "/properties/a/items" => "literal" }
    ],
    [
      "a reference beside its own negation",
      { "$defs" => { "A" => { "type" => "string" } }, "$ref" => "#/$defs/A",
        "not" => { "$ref" => "#/$defs/A" } },
      { "" => ["conflict", [["/$defs/A", %w[type]], ["", %w[not]]]] }
    ]
  ].each do |label, schema, expected|
    it "names its reason for #{label}" do
      expect(reported(schema)).to eq(expected)
    end
  end

  it "matches a reason by pattern" do
    reasons = JSONSchema::Canonical.find_unsatisfiable(
      { "properties" => { "a" => { "type" => "string", "minLength" => 5, "maxLength" => 2 } } }
    )

    case reasons["/properties/a"]
    in JSONSchema::Canonical::ConflictReason[causes:]
      expect(causes.map { |cause| [cause.pointer, cause.keywords] }).to eq(
        [["/properties/a", %w[type]], ["/properties/a", %w[minLength maxLength]]]
      )
    end
  end

  it "compares causes by value" do
    schema = { "properties" => { "a" => { "type" => "string", "minLength" => 5, "maxLength" => 2 } } }
    left = JSONSchema::Canonical.find_unsatisfiable(schema)["/properties/a"].causes
    right = JSONSchema::Canonical.find_unsatisfiable(schema)["/properties/a"].causes

    expect(left).to eq(right)
    expect(left.first).not_to eq(left.last)
  end

  it "reads a cause as itself" do
    reason = JSONSchema::Canonical.find_unsatisfiable(
      { "properties" => { "a" => { "enum" => [], "unevaluatedProperties" => true } } }
    )["/properties/a"]

    expect(reason.cause.inspect).to eq(
      '#<JSONSchema::Canonical::Cause pointer="/properties/a" keywords=["enum"]>'
    )
  end

  it "reads the registry" do
    registry = JSONSchema::Registry.new([["https://example.com/int", { "type" => "integer" }]])

    reasons = JSONSchema::Canonical.find_unsatisfiable(
      { "allOf" => [{ "$ref" => "https://example.com/int" }, { "type" => "string" }] },
      registry: registry
    )

    expect(reason_data(reasons[""])).to eq(["conflict", [["/allOf/0", []], ["/allOf/1", %w[type]]]])
  end

  it "reports a schema that fails meta-validation" do
    expect { JSONSchema::Canonical.find_unsatisfiable({ "type" => 1 }) }
      .to raise_error(JSONSchema::ValidationError)
  end
end

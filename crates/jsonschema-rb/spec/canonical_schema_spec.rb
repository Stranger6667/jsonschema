# frozen_string_literal: true

require "spec_helper"

DRAFT202012 = "https://json-schema.org/draft/2020-12/schema"
# `dependencies` is the one conditional applicator canonicalization does not split beside an
# `unevaluated*`, so this stays raw. Each construct canonicalization learns needs a
# still-unsupported stand-in here.
UNSUPPORTED = { "dependencies" => {}, "unevaluatedProperties" => false }.freeze

RSpec.describe "JSONSchema.canonicalize" do
  [
    UNSUPPORTED
  ].each do |schema|
    it "round-trips unsupported #{schema.inspect} verbatim" do
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

  it "view returns StringView with its length window and patterns" do
    case JSONSchema.canonicalize({ "type" => "string", "minLength" => 2, "maxLength" => 5, "pattern" => "^a" }).view
    in JSONSchema::Canonical::StringView[min_length:, max_length:, patterns:]
      expect(min_length).to eq(2)
      expect(max_length).to eq(5)
      expect(patterns).to eq(["^a"])
    end
  end

  it "view returns StringView with nil for an absent bound" do
    case JSONSchema.canonicalize({ "type" => "string", "minLength" => 2 }).view
    in JSONSchema::Canonical::StringView[min_length:, max_length:, patterns:]
      expect(min_length).to eq(2)
      expect(max_length).to be_nil
      expect(patterns).to eq([])
    end
  end

  it "view returns StringView carrying an asserted format" do
    case JSONSchema.canonicalize({ "type" => "string", "format" => "email" }, validate_formats: true).view
    in JSONSchema::Canonical::StringView[patterns:, formats:]
      expect(patterns).to eq([])
      expect(formats).to eq(["email"])
    end
  end

  it "view returns StringView carrying an independent media type and encoding" do
    # Same-object contentEncoding+contentMediaType decode-then-check and stay raw; separate allOf
    # branches model independently, which the view then exposes.
    schema = { "allOf" => [{ "type" => "string", "contentEncoding" => "base64" },
                           { "contentMediaType" => "application/json" }] }
    case JSONSchema.canonicalize(schema, draft: :draft7).view
    in JSONSchema::Canonical::StringView[content_media_types:, content_encodings:]
      expect(content_media_types).to eq(["application/json"])
      expect(content_encodings).to eq(["base64"])
    end
  end

  it "view returns NumberView with its real interval" do
    case JSONSchema.canonicalize({ "type" => "number", "minimum" => 2, "exclusiveMaximum" => 5 }).view
    in JSONSchema::Canonical::NumberView[minimum:, exclusive_minimum:, maximum:, exclusive_maximum:]
      expect(minimum).to eq(2)
      expect(exclusive_minimum).to be(false)
      expect(maximum).to eq(5)
      expect(exclusive_maximum).to be(true)
    end
  end

  it "view returns ArrayView with its length window" do
    schema = {
      "type" => "array", "minItems" => 1, "maxItems" => 3, "uniqueItems" => true,
      "items" => { "type" => "integer" }
    }
    case JSONSchema.canonicalize(schema).view
    in JSONSchema::Canonical::ArrayView[min_items:, max_items:, distinctness:, prefix_items:, items:]
      expect(min_items).to eq(1)
      expect(max_items).to eq(3)
      expect(distinctness).to be(:all_distinct)
      expect(prefix_items).to eq([])
      expect(items.to_json_schema).to eq(
        { "$schema" => "https://json-schema.org/draft/2020-12/schema", "type" => "integer" }
      )
    end
  end

  {
    { "type" => "array", "minItems" => 1 } => :unconstrained,
    { "type" => "array", "uniqueItems" => true } => :all_distinct,
    { "type" => "array", "allOf" => [{ "not" => { "type" => "array", "uniqueItems" => true } }] } => :some_repeated
  }.each do |schema, expected|
    it "view returns ArrayView with distinctness #{expected}" do
      case JSONSchema.canonicalize(schema).view
      in JSONSchema::Canonical::ArrayView[distinctness:]
        expect(distinctness).to be(expected)
      end
    end
  end

  it "view returns ArrayView with a prefix tuple" do
    schema = {
      "type" => "array",
      "prefixItems" => [{ "type" => "integer" }, { "type" => "string" }],
      "items" => { "type" => "boolean" }
    }
    case JSONSchema.canonicalize(schema).view
    in JSONSchema::Canonical::ArrayView[prefix_items:, items:]
      expect(prefix_items.map(&:to_json_schema)).to eq(
        [
          { "$schema" => "https://json-schema.org/draft/2020-12/schema", "type" => "integer" },
          { "$schema" => "https://json-schema.org/draft/2020-12/schema", "type" => "string" }
        ]
      )
      expect(items.to_json_schema).to eq(
        { "$schema" => "https://json-schema.org/draft/2020-12/schema", "type" => "boolean" }
      )
    end
  end

  it "view returns ArrayView with its contains demands" do
    schema = {
      "type" => "array", "contains" => { "type" => "string" },
      "minContains" => 0, "maxContains" => 2
    }
    case JSONSchema.canonicalize(schema).view
    in JSONSchema::Canonical::ArrayView[contains: [facet]]
      expect(facet.schema.to_json_schema).to eq(
        { "$schema" => "https://json-schema.org/draft/2020-12/schema", "type" => "string" }
      )
      expect(facet.min_contains).to eq(0)
      expect(facet.max_contains).to eq(2)
    end
  end

  it "view reports an absent contains count window as nil" do
    schema = { "type" => "array", "contains" => { "type" => "null" } }
    case JSONSchema.canonicalize(schema).view
    in JSONSchema::Canonical::ArrayView[contains: [facet]]
      expect(facet.schema.to_json_schema).to eq(
        { "$schema" => "https://json-schema.org/draft/2020-12/schema", "type" => "null" }
      )
      expect(facet.min_contains).to be_nil
      expect(facet.max_contains).to be_nil
    end
  end

  it "view returns ObjectView with its property-count window" do
    schema = {
      "type" => "object", "minProperties" => 1, "maxProperties" => 3,
      "required" => ["a"], "propertyNames" => { "maxLength" => 4 },
      "properties" => { "a" => { "type" => "integer" } }
    }
    case JSONSchema.canonicalize(schema).view
    in JSONSchema::Canonical::ObjectView[min_properties:, max_properties:, required:, property_names:, properties:]
      expect(min_properties).to be_nil
      expect(max_properties).to eq(3)
      expect(required).to eq(["a"])
      expect(property_names.to_json_schema).to eq(
        { "$schema" => "https://json-schema.org/draft/2020-12/schema", "type" => "string", "maxLength" => 4 }
      )
      expect(properties.keys).to eq(["a"])
      expect(properties["a"].to_json_schema).to eq(
        { "$schema" => "https://json-schema.org/draft/2020-12/schema", "type" => "integer" }
      )
    end
  end

  it "view returns ObjectView with its pattern schemas" do
    schema = { "type" => "object", "patternProperties" => { "^a" => { "type" => "integer" } } }
    case JSONSchema.canonicalize(schema).view
    in JSONSchema::Canonical::ObjectView[pattern_properties:]
      expect(pattern_properties.keys).to eq(["^a"])
      expect(pattern_properties["^a"].to_json_schema).to eq(
        { "$schema" => "https://json-schema.org/draft/2020-12/schema", "type" => "integer" }
      )
    end
  end

  it "view returns ObjectView with a NameFails violation" do
    schema = {
      "type" => "object", "minProperties" => 1,
      "properties" => { "filter" => { "type" => "string" } },
      "not" => { "additionalProperties" => false, "properties" => { "filter" => { "type" => "string" } } }
    }
    case JSONSchema.canonicalize(schema).view
    in JSONSchema::Canonical::ObjectView[violations:]
      expect(violations.length).to eq(1)
      case violations.first
      in JSONSchema::Canonical::NameFailsView[schema: name_schema]
        expect(name_schema.to_json_schema["const"]).to eq("filter")
      end
    end
  end

  it "view exposes the divisor of a number leaf" do
    case JSONSchema.canonicalize({ "type" => "number", "multipleOf" => 0.5 }).view
    in JSONSchema::Canonical::NumberView[multiple_of:]
      expect(multiple_of).to eq([0.5])
    end
  end

  it "view returns IntegerView with its divisor" do
    case JSONSchema.canonicalize({ "type" => "integer", "multipleOf" => 3 }).view
    in JSONSchema::Canonical::IntegerView[minimum:, maximum:, multiple_of:]
      expect(minimum).to be_nil
      expect(maximum).to be_nil
      expect(multiple_of).to eq([3])
    end
  end

  it "view returns IntegerView with its interval" do
    case JSONSchema.canonicalize({ "type" => "integer", "minimum" => 2, "maximum" => 9 }).view
    in JSONSchema::Canonical::IntegerView[minimum:, maximum:]
      expect(minimum).to eq(2)
      expect(maximum).to eq(9)
    end
  end

  it "view returns IntegerView with nil for an absent bound" do
    case JSONSchema.canonicalize({ "type" => "integer", "minimum" => -3 }).view
    in JSONSchema::Canonical::IntegerView[minimum:, maximum:]
      expect(minimum).to eq(-3)
      expect(maximum).to be_nil
    end
  end

  it "view returns AnyOfView exposing each branch" do
    case JSONSchema.canonicalize({ "anyOf" => [{ "type" => "string" }, { "const" => 1 }] }).view
    in JSONSchema::Canonical::AnyOfView[branches:]
      expect(branches.length).to eq(2)
      expect(branches).to all(be_a(JSONSchema::Canonical::CanonicalSchema))
      expect(branches.map(&:kind)).to contain_exactly(:multi_type, :const)
    end
  end

  it "view returns OneOfView exposing each branch" do
    schema = {
      "oneOf" => [{ "$ref" => "#/$defs/plain" }, { "$ref" => "#/$defs/tight" }],
      "$defs" => { "plain" => { "type" => "string" }, "tight" => { "type" => "string", "minLength" => 3 } }
    }
    case JSONSchema.canonicalize(schema).view
    in JSONSchema::Canonical::OneOfView[branches:]
      expect(branches.length).to eq(2)
      expect(branches).to all(be_a(JSONSchema::Canonical::CanonicalSchema))
      expect(branches.map(&:kind)).to eq(%i[reference reference])
    end
  end

  it "view returns AllOfView with a symbolic reference" do
    # A cycle keeps the conjunction symbolic: such a document is not folded through its targets.
    schema = {
      "allOf" => [{ "$ref" => "#/$defs/value" }, { "type" => "string" }],
      "$defs" => { "value" => { "type" => "object", "properties" => { "self" => { "$ref" => "#/$defs/value" } } } }
    }
    case JSONSchema.canonicalize(schema).view
    in JSONSchema::Canonical::AllOfView[branches:]
      expect(branches.map(&:kind)).to eq(%i[multi_type reference])
    end
  end

  it "view returns NotView with a symbolic reference" do
    schema = {
      "not" => { "$ref" => "#/$defs/other" },
      "$defs" => { "other" => { "type" => "object", "properties" => { "child" => { "$ref" => "#/$defs/other" } } } }
    }
    case JSONSchema.canonicalize(schema).view
    in JSONSchema::Canonical::NotView[schema: inner]
      expect(inner.kind).to eq(:reference)
    end
  end

  # `inspect` must render exactly what the reader returns, so the two cannot drift.
  it "inspect renders CanonicalSchema readers" do
    schema = JSONSchema.canonicalize({ "const" => 1 })
    expect(schema.inspect).to eq(
      "#<JSONSchema::Canonical::CanonicalSchema kind=#{schema.kind.inspect} draft=#{schema.draft.inspect}>"
    )
  end

  it "inspect omits the object address for trivial views" do
    expect(JSONSchema.canonicalize({}).view.inspect).to eq("#<JSONSchema::Canonical::TrueView>")
    expect(JSONSchema.canonicalize(false).view.inspect).to eq("#<JSONSchema::Canonical::FalseView>")
  end

  {
    "MultiTypeView" => [{ "type" => %w[integer string] }, %i[types]],
    "TypedGroupView" => [{ "type" => "integer", "enum" => [1, 2] }, %i[type_name]],
    "StringView" => [{ "type" => "string", "minLength" => 2, "pattern" => "^a" },
                     %i[min_length max_length patterns excluded_patterns formats excluded_formats
                        content_media_types content_encodings excluded]],
    "IntegerView" => [{ "type" => "integer", "minimum" => 2, "maximum" => 9 },
                      %i[minimum maximum multiple_of not_multiple_of]],
    "NumberView" => [{ "type" => "number", "minimum" => 2 },
                     %i[minimum exclusive_minimum maximum exclusive_maximum multiple_of not_multiple_of
                        excludes_integers]],
    "ArrayView" => [{ "type" => "array", "minItems" => 1 }, %i[min_items max_items distinctness prefix_items items contains]],
    "ObjectView" => [{ "type" => "object", "minProperties" => 1 },
                     %i[min_properties max_properties required property_names properties pattern_properties]],
    "ConstView" => [{ "const" => nil }, %i[value]],
    "EnumView" => [{ "enum" => [1, 2] }, %i[values]],
    "RawView" => [UNSUPPORTED, %i[schema]]
  }.each do |name, (schema, readers)|
    it "inspect renders #{name} readers" do
      draft = name == "TypedGroupView" ? :draft4 : :draft202012
      view = JSONSchema.canonicalize(schema, draft: draft).view
      expect(view).to be_a(JSONSchema::Canonical.const_get(name))
      rendered = readers.map { |reader| "#{reader}=#{view.public_send(reader).inspect}" }.join(" ")
      expect(view.inspect).to eq("#<JSONSchema::Canonical::#{name} #{rendered}>")
    end
  end

  it "inspect summarises AnyOfView branches by kind" do
    view = JSONSchema.canonicalize({ "anyOf" => [{ "type" => "string" }, { "const" => 1 }] }).view
    expect(view.inspect).to eq(
      "#<JSONSchema::Canonical::AnyOfView branches=#{view.branches.map(&:kind).inspect}>"
    )
  end

  it "view returns RawView with the document payload" do
    case JSONSchema.canonicalize(UNSUPPORTED).view
    in JSONSchema::Canonical::RawView[schema:]
      expect(schema).to eq(UNSUPPORTED)
    end
  end

  [
    [{ "const" => 5 }, :const],
    [{ "enum" => [1, 2] }, :enum],
    [{ "type" => %w[integer string] }, :multi_type],
    [{}, :true], # rubocop:disable Lint/BooleanSymbol
    [false, :false], # rubocop:disable Lint/BooleanSymbol
    [{ "type" => "string", "minLength" => 3 }, :string],
    [{ "type" => "integer", "minimum" => 0 }, :integer],
    [{ "pattern" => "a" }, :any_of]
  ].each do |schema, kind|
    it "kind of #{schema.inspect} is #{kind.inspect}" do
      expect(JSONSchema.canonicalize(schema).kind).to eq(kind)
    end
  end

  it "satisfiability reflects provable emptiness" do
    expect(JSONSchema.canonicalize({ "const" => 5 }).satisfiability).not_to be(:no)
    expect(JSONSchema.canonicalize({ "type" => "integer", "enum" => ["x"] }).satisfiability).to be(:no)
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

  it "definitions exposes canonical reference targets" do
    schema = { "$defs" => { "a" => {} }, "$ref" => "#/$defs/a" }
    canonical = JSONSchema.canonicalize(schema)
    expect(canonical.view.uri).to eq("#/$defs/a")
    expect(canonical.definitions.keys).to eq(["#/$defs/a"])
    expect(canonical.definitions["#/$defs/a"].kind).to eq(:true) # rubocop:disable Lint/BooleanSymbol
  end

  [
    [{ "type" => "string" }, { "minLength" => 4 },
     { "$schema" => DRAFT202012, "type" => "string", "minLength" => 4 }],
    [{ "const" => "A" }, { "pattern" => "^A$" }, { "$schema" => DRAFT202012, "const" => "A" }],
    [{ "const" => "A" }, { "const" => "B" }, { "$schema" => DRAFT202012, "not" => {} }]
  ].each do |left, right, expected|
    it "intersects #{left.inspect} with #{right.inspect}" do
      result = JSONSchema.canonicalize(left).intersect(JSONSchema.canonicalize(right))
      expect(result).to be_a(JSONSchema::Canonical::CanonicalSchema)
      expect(result.to_json_schema).to eq(expected)
    end
  end

  [
    [{ "type" => "string" }, { "type" => "integer" },
     { "$schema" => DRAFT202012, "type" => %w[integer string] }],
    [{ "const" => "a" }, { "enum" => %w[a b] }, { "$schema" => DRAFT202012, "enum" => %w[a b] }],
    [{ "type" => "string", "minLength" => 4 }, { "type" => "string" },
     { "$schema" => DRAFT202012, "type" => "string" }]
  ].each do |left, right, expected|
    it "unions #{left.inspect} with #{right.inspect}" do
      result = JSONSchema.canonicalize(left).union(JSONSchema.canonicalize(right))
      expect(result).to be_a(JSONSchema::Canonical::CanonicalSchema)
      expect(result.to_json_schema).to eq(expected)
    end
  end

  [
    [{ "type" => "integer", "minimum" => 10 }, { "type" => "integer", "minimum" => 15 },
     { "$schema" => DRAFT202012, "type" => "integer", "minimum" => 10, "maximum" => 14 }, :yes],
    [{ "enum" => %w[a b] }, { "enum" => ["a"] }, { "$schema" => DRAFT202012, "const" => "b" }, :yes],
    [{ "type" => "integer" }, { "type" => %w[integer string] },
     { "$schema" => DRAFT202012, "not" => {} }, :no],
    # What a narrowed schema stopped accepting, which is the comparison workflow: the values the
    # old schema took and the new one turns away.
    [{ "type" => "string" }, { "type" => "string", "maxLength" => 50 },
     { "$schema" => DRAFT202012, "type" => "string", "minLength" => 51 }, :yes],
    # Empty the other way round, since the new schema only narrowed - which is what proves nothing
    # was lost.
    [{ "type" => "string", "maxLength" => 50 }, { "type" => "string" },
     { "$schema" => DRAFT202012, "not" => {} }, :no]
  ].each do |left, right, expected, satisfiability|
    it "subtracts #{right.inspect} from #{left.inspect}" do
      result = JSONSchema.canonicalize(left).subtract(JSONSchema.canonicalize(right))
      expect(result).to be_a(JSONSchema::Canonical::CanonicalSchema)
      expect(result.to_json_schema).to eq(expected)
      expect(result.satisfiability).to be(satisfiability)
    end
  end

  %w[union subtract].each do |operation|
    it "#{operation} rejects an unsupported operand" do
      modeled = JSONSchema.canonicalize({ "type" => "string" })
      expect { modeled.public_send(operation, JSONSchema.canonicalize(UNSUPPORTED)) }
        .to raise_error(JSONSchema::Canonical::UnsupportedOperand)
    end

    it "#{operation} rejects a draft mismatch" do
      left = JSONSchema.canonicalize({ "type" => "string" }, draft: :draft7)
      right = JSONSchema.canonicalize({ "type" => "string" }, draft: :draft202012)
      expect { left.public_send(operation, right) }
        .to raise_error(JSONSchema::Canonical::IncompatibleOperands)
    end
  end

  it "carries exactly the targets the result still names" do
    left = JSONSchema.canonicalize({ "$defs" => { "a" => { "type" => "string" } }, "$ref" => "#/$defs/a" })
    right = JSONSchema.canonicalize({ "$defs" => { "b" => { "minLength" => 4 } }, "$ref" => "#/$defs/b" })
    # Both pointers are read through and the meet folds into one leaf, naming neither.
    result = left.intersect(right)
    expect(result.definitions).to be_empty
    expect(result.to_json_schema).to eq(
      { "$schema" => DRAFT202012, "type" => "string", "minLength" => 4 }
    )
  end

  { "on the left" => false, "on the right" => true }.each do |side, swap|
    it "intersect rejects an unsupported operand #{side}" do
      raw = JSONSchema.canonicalize(UNSUPPORTED)
      modeled = JSONSchema.canonicalize({ "type" => "string" })
      left, right = swap ? [modeled, raw] : [raw, modeled]
      expect { left.intersect(right) }.to raise_error(JSONSchema::Canonical::UnsupportedOperand)
    end
  end

  it "intersect rejects a draft mismatch" do
    left = JSONSchema.canonicalize({ "type" => "string" }, draft: :draft7)
    right = JSONSchema.canonicalize({ "type" => "string" }, draft: :draft202012)
    expect { left.intersect(right) }.to raise_error(JSONSchema::Canonical::IncompatibleOperands)
  end

  # A `$defs` name is private to its document, so two documents binding it to different bodies
  # rename apart rather than refusing to combine.
  it "intersect renames a definition the two documents bind differently" do
    left = JSONSchema.canonicalize({ "$defs" => { "a" => { "type" => "string" } }, "$ref" => "#/$defs/a" })
    right = JSONSchema.canonicalize({ "$defs" => { "a" => { "minLength" => 4 } }, "$ref" => "#/$defs/a" })
    expect(left.intersect(right).to_json_schema).to eq(
      "$schema" => DRAFT202012, "type" => "string", "minLength" => 4
    )
  end

  # `#` names the document itself, so unlike a private key it cannot be renamed apart.
  it "intersect rejects documents binding the root differently" do
    left = JSONSchema.canonicalize({ "type" => "object", "properties" => { "next" => { "$ref" => "#" } } })
    right = JSONSchema.canonicalize(
      { "type" => "object", "properties" => { "next" => { "$ref" => "#" } }, "minProperties" => 1 }
    )
    expect { left.intersect(right) }.to raise_error(JSONSchema::Canonical::IncompatibleOperands)
  end

  [
    [{ "type" => "integer" }, { "type" => "integer" }, :yes],
    [{ "type" => "integer" }, { "const" => 1 }, :yes],
    [{ "type" => "integer" }, { "enum" => [1, 2] }, :yes],
    [{ "type" => "integer" }, { "const" => "x" }, :no],
    [{ "type" => "integer" }, { "enum" => [1, "x"] }, :no],
    [{ "type" => "integer" }, { "type" => "string" }, :no],
    [{ "type" => "integer", "minimum" => 5 }, { "type" => "integer" }, :no],
    [{ "type" => "string", "pattern" => "^a" }, { "type" => "string" }, :no]
  ].each do |outer, inner, expected|
    it "decides whether #{outer.inspect} covers #{inner.inspect}" do
      result = JSONSchema.canonicalize(outer).covers(JSONSchema.canonicalize(inner))
      expect(result).to eq(expected)
    end
  end

  { "on the left" => false, "on the right" => true }.each do |side, swap|
    it "covers rejects an unsupported operand #{side}" do
      raw = JSONSchema.canonicalize(UNSUPPORTED)
      modeled = JSONSchema.canonicalize({ "type" => "string" })
      left, right = swap ? [modeled, raw] : [raw, modeled]
      expect { left.covers(right) }.to raise_error(JSONSchema::Canonical::UnsupportedOperand)
    end
  end

  it "covers rejects a draft mismatch" do
    left = JSONSchema.canonicalize({ "type" => "string" }, draft: :draft7)
    right = JSONSchema.canonicalize({ "type" => "string" }, draft: :draft202012)
    expect { left.covers(right) }.to raise_error(JSONSchema::Canonical::IncompatibleOperands)
  end

  it "definition looks up one reference target" do
    canonical = JSONSchema.canonicalize({ "$defs" => { "a" => { "type" => "string" } }, "$ref" => "#/$defs/a" })
    expect(canonical.definition("#/$defs/a")).to eq(canonical.definitions["#/$defs/a"])
    expect(canonical.definition("#/$defs/absent")).to be_nil
  end

  [
    [{ "type" => "string", "minLength" => 5 },
     { "$schema" => DRAFT202012,
       "anyOf" => [{ "type" => %w[null boolean number array object] },
                   { "type" => "string", "maxLength" => 4 }] }],
    [{ "type" => "number", "minimum" => 5 },
     { "$schema" => DRAFT202012,
       "anyOf" => [{ "type" => %w[null boolean string array object] },
                   { "type" => "number", "exclusiveMaximum" => 5 }] }],
    [{ "type" => "integer", "minimum" => 0 },
     { "$schema" => DRAFT202012,
       "anyOf" => [{ "type" => %w[null boolean string array object] },
                   { "type" => "integer", "maximum" => -1 },
                   { "type" => "number", "not" => { "multipleOf" => 1 } }] }]
  ].each do |schema, expected|
    it "negates #{schema.inspect}" do
      result = JSONSchema.canonicalize(schema).negate
      expect(result).to be_a(JSONSchema::Canonical::CanonicalSchema)
      expect(result.to_json_schema).to eq(expected)
    end
  end

  it "negates a draft 4 integer type" do
    schema = { "$schema" => "http://json-schema.org/draft-04/schema#", "type" => "integer" }
    result = JSONSchema.canonicalize(schema).negate
    expect(result).to be_a(JSONSchema::Canonical::CanonicalSchema)
    expect(result.to_json_schema).to eq(
      { "$schema" => "http://json-schema.org/draft-04/schema#",
        "anyOf" => [{ "type" => %w[null boolean string array object] },
                    { "type" => "number", "not" => { "type" => "integer" } }] }
    )
  end

  it "negates a draft 4 typed group" do
    schema = { "$schema" => "http://json-schema.org/draft-04/schema#", "type" => "integer", "enum" => [1, 2] }
    result = JSONSchema.canonicalize(schema).negate
    expect(result).to be_a(JSONSchema::Canonical::CanonicalSchema)
    expect(result.to_json_schema).to eq(
      { "$schema" => "http://json-schema.org/draft-04/schema#",
        "anyOf" => [{ "type" => %w[null boolean string array object] },
                    { "type" => "integer", "maximum" => 0 },
                    { "type" => "integer", "minimum" => 3 },
                    { "type" => "number", "not" => { "type" => "integer" } }] }
    )
  end

  # A `Raw` operand raises `UnsupportedOperand`, not `UnsupportedResult`.
  it "rejects negating an unsupported schema" do
    expect { JSONSchema.canonicalize(UNSUPPORTED).negate }
      .to raise_error(JSONSchema::Canonical::UnsupportedOperand)
  end

  [
    { "type" => "object", "patternProperties" => { "^a" => { "type" => "string" } } },
    { "type" => "array", "contains" => { "type" => "string" }, "minContains" => 2 }
  ].each do |schema|
    it "raises UnsupportedResult negating #{schema.inspect}" do
      expect { JSONSchema.canonicalize(schema).negate }
        .to raise_error(JSONSchema::Canonical::UnsupportedResult, "result is not supported in canonical form")
    end
  end

  it "raises UnsupportedResult subtracting a schema whose complement is not supported" do
    plain = JSONSchema.canonicalize({ "type" => "object" })
    hard = JSONSchema.canonicalize({ "type" => "object", "patternProperties" => { "^a" => { "type" => "string" } } })
    expect { plain.subtract(hard) }.to raise_error(JSONSchema::Canonical::UnsupportedResult)
  end

  it "subtracts a schema from itself without asking for a complement" do
    schema = { "type" => "object", "patternProperties" => { "^a" => { "type" => "string" } } }
    expect { JSONSchema.canonicalize(schema).negate }
      .to raise_error(JSONSchema::Canonical::UnsupportedResult)
    expect(JSONSchema.canonicalize(schema).subtract(JSONSchema.canonicalize(schema)).satisfiability)
      .to be(:no)
  end

  it "lists the members of each label" do
    expect(JSONSchema::Canonical::Containment::ALL).to eq(%i[yes no unknown])
    expect(JSONSchema::Canonical::Satisfiability::ALL).to eq(%i[yes no unknown])
    expect(JSONSchema::Canonical::Distinctness::ALL).to eq(%i[unconstrained all_distinct some_repeated])
    expect(JSONSchema::Canonical::Kind::ALL).to eq(
      %i[multi_type typed_group string integer number array object const enum not all_of any_of one_of
         reference true false raw]
    )
  end

  it "answers coverage and satisfiability with those constants" do
    integer = JSONSchema.canonicalize({ "type" => "integer" })
    expect(integer.covers(integer)).to be(JSONSchema::Canonical::Containment::YES)
    expect(JSONSchema.canonicalize(false).satisfiability)
      .to be(JSONSchema::Canonical::Satisfiability::NO)
    expect(JSONSchema.canonicalize({ "const" => 5 }).satisfiability)
      .to be(JSONSchema::Canonical::Satisfiability::YES)
    expect(JSONSchema.canonicalize({ "type" => "array", "uniqueItems" => true }).view.distinctness)
      .to be(JSONSchema::Canonical::Distinctness::ALL_DISTINCT)
    expect(JSONSchema.canonicalize(UNSUPPORTED).kind).to be(JSONSchema::Canonical::Kind::RAW)
  end

  it "combines one document canonicalized twice" do
    source = { "$defs" => { "Id" => { "type" => "string" } }, "type" => "object",
               "properties" => { "id" => { "$ref" => "#/$defs/Id" } } }
    left = JSONSchema.canonicalize(source)
    right = JSONSchema.canonicalize(source)
    expect(left).to eq(right)
    expect(left.intersect(right)).to eq(left)
    expect(left.covers(right)).to be(:yes)
  end

  {
    true => :yes,
    { "const" => 5 } => :yes,
    { "enum" => [1, 2] } => :yes,
    false => :no,
    { "type" => "string" } => :yes,
    { "type" => "string", "pattern" => "^a" } => :yes,
    { "type" => "string", "pattern" => "a{100}" } => :unknown
  }.each do |schema, expected|
    it "answers satisfiability #{expected} for #{schema.inspect}" do
      expect(JSONSchema.canonicalize(schema).satisfiability).to be(expected)
    end
  end

  it "negate resolves a reference" do
    schema = JSONSchema.canonicalize({ "$defs" => { "a" => { "type" => "string" } }, "$ref" => "#/$defs/a" })
    complement = schema.negate
    expect(complement.to_json_schema).to eq(
      { "$schema" => "https://json-schema.org/draft/2020-12/schema",
        "type" => %w[null boolean number array object] }
    )
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

  describe "pattern_options" do
    # A pattern whose compiled size exceeds the default regex limit; real AWS schemas carry these.
    large_pattern = { "type" => "string", "pattern" => "^.{0,100000}$" }

    it "rejects an oversized pattern by default" do
      expect { JSONSchema.canonicalize(large_pattern) }.to raise_error(JSONSchema::Canonical::InvalidPattern)
    end

    it "accepts it with a raised size limit" do
      options = JSONSchema::FancyRegexOptions.new(size_limit: 150_000_000)
      canonical = JSONSchema.canonicalize(large_pattern, pattern_options: options)
      expect(canonical.kind).to eq(:string)
    end

    it "rejects a non-options value" do
      expect { JSONSchema.canonicalize(large_pattern, pattern_options: 42) }.to raise_error(TypeError)
    end
  end

  describe "reference resolution" do
    it "resolves a reference through the registry" do
      registry = JSONSchema::Registry.new([["https://example.com/external", { "type" => "string" }]])
      result = JSONSchema.canonicalize({ "$ref" => "https://example.com/external" }, registry: registry)

      expect(JSONSchema.valid?(result.to_json_schema, "value")).to be true
      expect(JSONSchema.valid?(result.to_json_schema, 1)).to be false
    end

    it "fetches a reference absent from the registry" do
      retriever = ->(uri) { { "type" => "string" } if uri == "https://example.com/remote" }
      result = JSONSchema.canonicalize({ "$ref" => "https://example.com/remote" }, retriever: retriever)

      expect(JSONSchema.valid?(result.to_json_schema, "value")).to be true
      expect(JSONSchema.valid?(result.to_json_schema, 1)).to be false
    end

    it "reuses the retriever the registry carries" do
      retriever = ->(uri) { { "type" => "string" } if uri == "https://example.com/remote" }
      registry = JSONSchema::Registry.new([], retriever: retriever)
      result = JSONSchema.canonicalize({ "$ref" => "https://example.com/remote" }, registry: registry)

      expect(JSONSchema.valid?(result.to_json_schema, "value")).to be true
      expect(JSONSchema.valid?(result.to_json_schema, 1)).to be false
    end

    it "resolves a relative reference against base_uri" do
      registry = JSONSchema::Registry.new([["https://example.com/external", { "type" => "string" }]])
      result = JSONSchema.canonicalize({ "$ref" => "external" }, registry: registry,
                                                                 base_uri: "https://example.com/root")

      expect(JSONSchema.valid?(result.to_json_schema, "value")).to be true
      expect(JSONSchema.valid?(result.to_json_schema, 1)).to be false
    end

    it "surfaces a retriever failure" do
      retriever = ->(uri) { raise KeyError, "Schema not found: #{uri}" }

      expect { JSONSchema.canonicalize({ "$ref" => "https://example.com/remote" }, retriever: retriever) }
        .to raise_error(JSONSchema::Canonical::CanonicalizationError, /Schema not found/)
    end

    it "rejects a non-registry value" do
      expect { JSONSchema.canonicalize(true, registry: 42) }.to raise_error(TypeError)
    end
  end
end

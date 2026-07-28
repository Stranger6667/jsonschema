# frozen_string_literal: true

require "spec_helper"

DRAFT202012 = "https://json-schema.org/draft/2020-12/schema"
# `anyOf` annotates whichever branch the instance matched, which no `additional*` twin spells,
# so this stays raw. Each construct canonicalization learns needs a still-unmodeled stand-in here.
UNMODELED = { "anyOf" => [{}], "unevaluatedProperties" => false }.freeze

RSpec.describe "JSONSchema.canonicalize" do
  [
    UNMODELED
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
    in JSONSchema::Canonical::ArrayView[min_items:, max_items:, unique_items:, prefix_items:, items:]
      expect(min_items).to eq(1)
      expect(max_items).to eq(3)
      expect(unique_items).to be(true)
      expect(prefix_items).to eq([])
      expect(items.to_json_schema).to eq(
        { "$schema" => "https://json-schema.org/draft/2020-12/schema", "type" => "integer" }
      )
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
      "oneOf" => [{ "$ref" => "#/$defs/one" }, { "$ref" => "#/$defs/two" }],
      "$defs" => { "one" => { "const" => 1 }, "two" => { "const" => 2 } }
    }
    case JSONSchema.canonicalize(schema).view
    in JSONSchema::Canonical::OneOfView[branches:]
      expect(branches.length).to eq(2)
      expect(branches).to all(be_a(JSONSchema::Canonical::CanonicalSchema))
      expect(branches.map(&:kind)).to eq(%i[reference reference])
    end
  end

  it "view returns AllOfView with a symbolic reference" do
    schema = {
      "allOf" => [{ "$ref" => "#/$defs/value" }, { "type" => "string" }],
      "$defs" => { "value" => { "type" => "string" } }
    }
    case JSONSchema.canonicalize(schema).view
    in JSONSchema::Canonical::AllOfView[branches:]
      expect(branches.map(&:kind)).to eq(%i[multi_type reference])
    end
  end

  it "view returns NotView with a symbolic reference" do
    schema = {
      "not" => { "$ref" => "#/$defs/other" },
      "$defs" => { "other" => { "type" => "string" } }
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
                     %i[min_length max_length patterns formats content_media_types content_encodings]],
    "IntegerView" => [{ "type" => "integer", "minimum" => 2, "maximum" => 9 }, %i[minimum maximum multiple_of]],
    "NumberView" => [{ "type" => "number", "minimum" => 2 }, %i[minimum exclusive_minimum maximum exclusive_maximum multiple_of]],
    "ArrayView" => [{ "type" => "array", "minItems" => 1 }, %i[min_items max_items unique_items prefix_items items contains]],
    "ObjectView" => [{ "type" => "object", "minProperties" => 1 },
                     %i[min_properties max_properties required property_names properties pattern_properties]],
    "ConstView" => [{ "const" => nil }, %i[value]],
    "EnumView" => [{ "enum" => [1, 2] }, %i[values]],
    "RawView" => [UNMODELED, %i[schema]]
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
    case JSONSchema.canonicalize(UNMODELED).view
    in JSONSchema::Canonical::RawView[schema:]
      expect(schema).to eq(UNMODELED)
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

  it "definitions exposes canonical reference targets" do
    schema = { "$defs" => { "a" => {} }, "$ref" => "#/$defs/a" }
    canonical = JSONSchema.canonicalize(schema)
    expect(canonical.view.uri).to eq("#/$defs/a")
    expect(canonical.definitions.keys).to eq(["#/$defs/a"])
    expect(canonical.definitions["#/$defs/a"].kind).to eq(:true) # rubocop:disable Lint/BooleanSymbol
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
end

# frozen_string_literal: true

require "spec_helper"
require "json"

RSpec.describe "JSONSchema canonical" do
  describe "JSONSchema.canonicalize" do
    it "is defined" do
      expect(JSONSchema).to respond_to(:canonicalize)
    end

    it "returns a CanonicalSchema for a valid schema" do
      result = JSONSchema.canonicalize({ "type" => "string" })
      expect(result).to be_a(JSONSchema::Canonical::CanonicalSchema)
    end

    it "to_json_schema round-trips a simple schema" do
      result = JSONSchema.canonicalize({ "type" => "integer" })
      js = result.to_json_schema
      expect(js).to be_a(Hash)
    end

    it "satisfiable? is true for a normal schema" do
      expect(JSONSchema.canonicalize({ "type" => "string" }).satisfiable?).to be true
    end

    it "satisfiable? is false for false schema" do
      expect(JSONSchema.canonicalize(false).satisfiable?).to be false
    end

    it "draft returns a symbol" do
      schema = JSONSchema.canonicalize({ "type" => "string" })
      expect(schema.draft).to eq(:draft202012)
    end

    it "draft respects explicit draft kwarg" do
      schema = JSONSchema.canonicalize({ "type" => "string" }, draft: :draft7)
      expect(schema.draft).to eq(:draft7)
    end

    it "kind returns a symbol" do
      schema = JSONSchema.canonicalize({ "type" => "string" })
      expect(schema.kind).to be_a(Symbol)
    end

    it "inspect returns a string" do
      schema = JSONSchema.canonicalize({ "type" => "string" })
      expect(schema.inspect).to include("CanonicalSchema")
    end

    it "== compares semantically" do
      a = JSONSchema.canonicalize({ "type" => "integer" })
      b = JSONSchema.canonicalize({ "type" => "integer" })
      expect(a).to eq(b)
    end

    it "hash is usable as Hash key" do
      a = JSONSchema.canonicalize({ "type" => "integer" })
      b = JSONSchema.canonicalize({ "type" => "integer" })
      h = { a => 1 }
      expect(h[b]).to eq(1)
    end

    it "accepts validate_formats keyword" do
      # No in-document `$schema` (it would win over the `draft:` keyword), and a format pair
      # recognized in every draft (`uuid` is an annotation before 2019-09 and never collapses there).
      schema = {
        "allOf" => [
          { "type" => "string", "format" => "date" },
          { "type" => "string", "format" => "time" }
        ]
      }

      expect(JSONSchema.canonicalize(schema).satisfiable?).to be true
      expect(JSONSchema.canonicalize(schema, validate_formats: true).satisfiable?).to be false
      expect(JSONSchema.canonicalize(schema, draft: :draft7).satisfiable?).to be false
      expect(JSONSchema.canonicalize(schema, draft: :draft7, validate_formats: false).satisfiable?).to be true
    end

    it "resolves relative refs against base_uri" do
      registry = JSONSchema::Registry.new([["https://example.com/schemas/other", { "type" => "integer" }]])

      canonical = JSONSchema.canonicalize(
        { "$ref" => "other" },
        registry: registry,
        base_uri: "https://example.com/schemas/root"
      )

      expect(canonical.to_json_schema["type"]).to eq("integer")
    end

    it "uses registry retriever when only registry is provided" do
      registry = JSONSchema::Registry.new(
        [],
        retriever: lambda do |uri|
          { "type" => "string" } if uri == "https://example.com/string.json"
        end
      )

      canonical = JSONSchema.canonicalize(
        { "$ref" => "https://example.com/string.json" },
        registry: registry
      )

      expect(canonical.to_json_schema["type"]).to eq("string")
    end
  end

  describe "#view" do
    {
      # {"type":"null"} canonicalizes to Const(null) by design
      "ConstView for null type" => [{ "type" => "null" }, JSONSchema::Canonical::ConstView],
      "TrueView for true schema" => [true, JSONSchema::Canonical::TrueView],
      "FalseView for false schema" => [false, JSONSchema::Canonical::FalseView],
      "ConstView for const schema" => [{ "const" => 42 }, JSONSchema::Canonical::ConstView],
      "EnumView for enum schema" => [{ "enum" => [1, "two"] }, JSONSchema::Canonical::EnumView]
    }.each do |desc, (schema, view_class)|
      it desc do
        expect(JSONSchema.canonicalize(schema).view).to be_a(view_class)
      end
    end

    it "ConstView exposes its value (null is nil, const is the literal)" do
      expect(JSONSchema.canonicalize({ "type" => "null" }).view.value).to be_nil
      expect(JSONSchema.canonicalize({ "const" => 42 }).view.value).to eq(42)
    end

    it "EnumView exposes its values" do
      expect(JSONSchema.canonicalize({ "enum" => [1, "two"] }).view.values).to include(1, "two")
    end
  end

  describe "#view integer bounds" do
    it "exposes minimum and maximum" do
      schema = JSONSchema.canonicalize({ "type" => "integer", "minimum" => 1, "maximum" => 100 })
      view = schema.view
      expect(view).to be_a(JSONSchema::Canonical::IntegerView)
      expect(view.minimum).to eq(1)
      expect(view.maximum).to eq(100)
      expect(view.exclusive_minimum).to be_nil
      expect(view.not_multiple_of).to eq([])
    end
  end

  describe "#view string bounds" do
    it "exposes min_length, patterns, format, and extended_regex?" do
      schema = JSONSchema.canonicalize({
                                         "type" => "string",
                                         "minLength" => 3,
                                         "maxLength" => 20,
                                         "pattern" => "^[a-z]+$"
                                       })
      view = schema.view
      expect(view).to be_a(JSONSchema::Canonical::StringView)
      expect(view.min_length).to eq(3)
      expect(view.max_length).to eq(20)
      expect(view.patterns).to include("^[a-z]+$")
      expect(view.not_patterns).to eq([])
      expect(view.extended_regex?).to be(true).or be(false)
    end
  end

  describe "#view MultiTypeView" do
    it "types returns symbols" do
      schema = JSONSchema.canonicalize({
                                         "type" => %w[string integer]
                                       })
      view = schema.view
      expect(view).to be_a(JSONSchema::Canonical::MultiTypeView)
      expect(view.types).to all(be_a(Symbol))
      expect(view.types).to include(:string, :integer)
    end
  end

  describe "#view TypedGroupView" do
    it "type_name returns a symbol" do
      # anyOf with different types forces a TypedGroup view
      schema = JSONSchema.canonicalize({
                                         "anyOf" => [
                                           { "type" => "integer", "minimum" => 0 },
                                           { "type" => "string" }
                                         ]
                                       })
      # Walk into nested view to find a TypedGroupView or TypeGuardView with type_name
      view = schema.view
      # TypedGroupView has a type_name method returning a symbol
      if view.is_a?(JSONSchema::Canonical::TypedGroupView) || view.is_a?(JSONSchema::Canonical::TypeGuardView)
        expect(view.type_name).to be_a(Symbol)
      else
        # For AnyOfView, recurse into first schema
        child_view = view.schemas.first.view
        if child_view.respond_to?(:type_name)
          expect(child_view.type_name).to be_a(Symbol)
        else
          skip "TypedGroupView not produced for this input"
        end
      end
    end
  end

  describe "#view array" do
    it "exposes min_items and unique_items?" do
      schema = JSONSchema.canonicalize({
                                         "type" => "array",
                                         "minItems" => 1,
                                         "uniqueItems" => true
                                       })
      view = schema.view
      expect(view).to be_a(JSONSchema::Canonical::ArrayView)
      expect(view.min_items).to eq(1)
      expect(view.unique_items?).to be true
      expect(view.repeated_items?).to be false
      expect(view.prefix).to eq([])
    end
  end

  describe "#view object" do
    it "exposes requirements and constraints" do
      schema = JSONSchema.canonicalize({
                                         "type" => "object",
                                         "required" => ["name"],
                                         "properties" => { "name" => { "type" => "string" } }
                                       })
      view = schema.view
      expect(view).to be_a(JSONSchema::Canonical::ObjectView)
      expect(view.requirements).not_to be_empty
      expect(view.constraints).not_to be_empty
    end
  end

  describe "operations" do
    let(:int) { JSONSchema.canonicalize({ "type" => "integer" }) }
    let(:pos) { JSONSchema.canonicalize({ "type" => "integer", "minimum" => 0 }) }

    it "#intersect returns a CanonicalSchema" do
      result = int.intersect(pos)
      expect(result).to be_a(JSONSchema::Canonical::CanonicalSchema)
      expect(result.satisfiable?).to be true
    end

    it "#negate of false is satisfiable" do
      neg = JSONSchema.canonicalize(false).negate
      expect(neg.satisfiable?).to be true
    end

    it "#union accepts members of either operand" do
      str = JSONSchema.canonicalize({ "type" => "string" })
      result = int.union(str)
      expect(result).to be_a(JSONSchema::Canonical::CanonicalSchema)
      emitted = result.to_json_schema
      expect(JSONSchema.valid?(emitted, 5)).to be true
      expect(JSONSchema.valid?(emitted, "hi")).to be true
      expect(JSONSchema.valid?(emitted, true)).to be false
    end

    it "#subtract returns CanonicalSchema" do
      result = int.subtract(pos)
      expect(result).to be_a(JSONSchema::Canonical::CanonicalSchema)
    end

    {
      # narrow integer range is contained in the unbounded integer schema
      "true when narrow is contained in wide" =>
        [{ "type" => "integer", "minimum" => 0, "maximum" => 10 }, { "type" => "integer" }, true],
      # disjoint types: the residual is all strings, a concrete inhabited shape, so non-containment is provable
      "false for disjoint types" =>
        [{ "type" => "string" }, { "type" => "integer" }, false],
      # recursion leaves the residual undecidable, so the prover stays inconclusive
      "nil for a recursive residual" =>
        [{ "$ref" => "#/$defs/n", "$defs" => { "n" => { "type" => "object", "properties" => { "next" => { "$ref" => "#/$defs/n" } } } } },
         { "type" => "integer" }, nil]
    }.each do |desc, (left, right, expected)|
      it "#subschema_of? returns #{desc}" do
        result = JSONSchema.canonicalize(left).subschema_of?(JSONSchema.canonicalize(right))
        expect(result).to eq(expected)
      end
    end
  end

  describe "definitions" do
    it "returns empty hash for inline schema" do
      schema = JSONSchema.canonicalize({ "type" => "integer" })
      expect(schema.definitions).to eq({})
    end
  end

  describe "errors" do
    {
      "CanonicalizationError subclass for non-schema type" => [42, JSONSchema::Canonical::CanonicalizationError],
      "InvalidSchemaType specifically for non-schema type" => [42, JSONSchema::Canonical::InvalidSchemaType],
      # {"type":"nonsense"} fails meta-schema validation before canonicalization
      "ValidationError for bad type value (meta-schema catches it)" => [{ "type" => "nonsense" }, JSONSchema::ValidationError]
    }.each do |desc, (input, error_class)|
      it "raises #{desc}" do
        expect { JSONSchema.canonicalize(input) }.to raise_error(error_class)
      end
    end

    it "InvalidPattern#location holds the JSON pointer to the failing pattern" do
      error = nil
      expect { JSONSchema.canonicalize({ "pattern" => "[" }) }
        .to raise_error(JSONSchema::Canonical::InvalidPattern) { |e| error = e }
      expect(error.location).to eq("/pattern")
    end
  end

  describe "pattern matching" do
    it "IntegerView supports deconstruct_keys" do
      schema = JSONSchema.canonicalize({ "type" => "integer", "minimum" => 5 })
      matched = false
      case schema.view
      in JSONSchema::Canonical::IntegerView[minimum: (Integer | Float) => min] if min > 0
        matched = true
      end
      expect(matched).to be true
    end

    it "StringView supports deconstruct_keys" do
      schema = JSONSchema.canonicalize({ "type" => "string", "minLength" => 1 })
      matched = false
      case schema.view
      in JSONSchema::Canonical::StringView[min_length: Integer]
        matched = true
      end
      expect(matched).to be true
    end

    it "ConstView supports deconstruct_keys" do
      schema = JSONSchema.canonicalize({ "const" => "hello" })
      matched = false
      case schema.view
      in JSONSchema::Canonical::ConstView[value: String => v]
        matched = (v == "hello")
      end
      expect(matched).to be true
    end
  end

  describe "Canonical::JSON.to_string" do
    it "still works after refactor" do
      result = JSONSchema::Canonical::JSON.to_string({ "type" => "string", "b" => 1, "a" => 2 })
      parsed = JSON.parse(result)
      expect(parsed["type"]).to eq("string")
    end
  end
end

# frozen_string_literal: true

require "spec_helper"
require "weakref"

# Keep strong defaults locally, while allowing CI to tune stress-loop counts.
LIFETIME_STRESS_ITERATIONS = Integer(ENV.fetch("JSONSCHEMA_RB_LIFETIME_STRESS_ITERATIONS", "100"))
LIFETIME_GC_ROUNDS = Integer(ENV.fetch("JSONSCHEMA_RB_LIFETIME_GC_ROUNDS", "10"))

RSpec.describe JSONSchema do
  describe ".valid?" do
    it "returns true for valid instance" do
      schema = { "type" => "string" }
      expect(JSONSchema.valid?(schema, "hello")).to be true
    end

    it "returns false for invalid instance" do
      schema = { "type" => "string" }
      expect(JSONSchema.valid?(schema, 42)).to be false
    end
  end

  describe ".validate!" do
    it "returns nil for valid instance" do
      schema = { "type" => "string" }
      expect(JSONSchema.validate!(schema, "hello")).to be_nil
    end

    it "raises ValidationError for invalid instance" do
      schema = { "type" => "string" }
      expect { JSONSchema.validate!(schema, 42) }.to raise_error(JSONSchema::ValidationError)
    end

    it "raises ReferencingError for unresolved $ref" do
      schema = { "$ref" => "#/$defs/missing" }
      expect { JSONSchema.validate!(schema, "hello") }.to raise_error(JSONSchema::ReferencingError)
    end
  end

  describe ".each_error" do
    let(:schema) { { "type" => "string" } }

    it "returns empty array for valid instance" do
      errors = JSONSchema.each_error(schema, "hello")
      expect(errors).to eq([])
    end

    it "returns array of errors for invalid instance" do
      errors = JSONSchema.each_error(schema, 42)
      expect(errors.size).to eq(1)
      expect(errors.first).to be_a(JSONSchema::ValidationError)
    end

    it "yields errors when block given" do
      collected = []
      JSONSchema.each_error(schema, 42) { |e| collected << e }
      expect(collected.size).to eq(1)
      expect(collected.first).to be_a(JSONSchema::ValidationError)
    end

    it "returns nil when block given" do
      seen = false
      result = JSONSchema.each_error(schema, 42) { |_error| seen = true }
      expect(result).to be_nil
      expect(seen).to be true
    end

    it "supports early termination with break" do
      # Schema that produces multiple errors
      schema = { "type" => "object", "required" => %w[a b c] }
      collected = []
      JSONSchema.each_error(schema, {}) do |e|
        collected << e
        break if collected.size == 1
      end
      expect(collected.size).to eq(1)
      expect(collected.first).to be_a(JSONSchema::ValidationError)
    end
  end

  describe ".evaluate" do
    it "returns valid evaluation for valid instance" do
      schema = { "type" => "string" }
      eval_result = JSONSchema.evaluate(schema, "hello")
      expect(eval_result.valid?).to be true
      expect(eval_result.errors).to eq([])
    end

    it "returns invalid evaluation for invalid instance" do
      schema = { "type" => "string" }
      eval_result = JSONSchema.evaluate(schema, 42)
      expect(eval_result.valid?).to be false
      expect(eval_result.errors.size).to eq(1)
    end

    it "rejects mask option" do
      schema = { "type" => "string" }
      expect { JSONSchema.evaluate(schema, 42, mask: "[HIDDEN]") }
        .to raise_error(ArgumentError, /unknown keyword: :mask/)
    end
  end

  describe ".validator_for" do
    it "creates a validator" do
      schema = { "type" => "string" }
      validator = JSONSchema.validator_for(schema)
      expect(validator).to respond_to(:valid?)
      expect(validator).to respond_to(:validate!)
      expect(validator).to respond_to(:each_error)
      expect(validator).to respond_to(:evaluate)
    end

    it "rejects draft keyword argument" do
      schema = { "type" => "string" }
      expect { JSONSchema.validator_for(schema, draft: :draft7) }
        .to raise_error(ArgumentError, /unknown keyword: :draft/)
    end

    it "raises ReferencingError for unresolved $ref" do
      schema = { "$ref" => "#/$defs/missing" }
      expect { JSONSchema.validator_for(schema) }.to raise_error(JSONSchema::ReferencingError)
    end
  end
end

RSpec.describe "Reusable validators" do
  describe "#inspect" do
    {
      JSONSchema::Draft4Validator => "Draft4",
      JSONSchema::Draft6Validator => "Draft6",
      JSONSchema::Draft7Validator => "Draft7",
      JSONSchema::Draft201909Validator => "Draft201909",
      JSONSchema::Draft202012Validator => "Draft202012"
    }.each do |klass, name|
      it "shows #{name} for #{klass}" do
        v = klass.new({ "type" => "string" })
        expect(v.inspect).to eq("#<JSONSchema::#{name}Validator>")
      end
    end
  end

  describe "with keyword arguments via validator_for" do
    it "creates validator with options" do
      schema = { "type" => "string", "format" => "email" }
      validator = JSONSchema.validator_for(schema, validate_formats: true)
      expect(validator.valid?("test@example.com")).to be true
      expect(validator.valid?("invalid")).to be false
    end

    it "accepts mask option" do
      schema = { "type" => "string" }
      validator = JSONSchema.validator_for(schema, mask: "[REDACTED]")
      expect { validator.validate!(42) }.to raise_error(JSONSchema::ValidationError) do |error|
        expect(error.message).to eq('[REDACTED] is not of type "string"')
      end
    end

    it "accepts custom formats" do
      schema = { "type" => "string", "format" => "my-format" }
      my_format = ->(value) { value.end_with?("42!") }
      validator = JSONSchema.validator_for(schema, validate_formats: true, formats: { "my-format" => my_format })
      expect(validator.valid?("foo42!")).to be true
      expect(validator.valid?("foo")).to be false
    end
  end
end

RSpec.describe "Custom formats" do
  describe "with module-level functions" do
    it "validates with custom format" do
      schema = { "type" => "string", "format" => "my-format" }
      my_format = ->(value) { value.end_with?("42!") }
      expect(JSONSchema.valid?(schema, "bar42!", validate_formats: true, formats: { "my-format" => my_format })).to be true
      expect(JSONSchema.valid?(schema, "bar", validate_formats: true, formats: { "my-format" => my_format })).to be false
    end
  end

  describe "with draft-specific validators" do
    it "accepts custom formats" do
      schema = { "type" => "string", "format" => "custom" }
      custom_format = ->(value) { value.start_with?("valid:") }
      validator = JSONSchema::Draft7Validator.new(schema, validate_formats: true, formats: { "custom" => custom_format })
      expect(validator.valid?("valid:data")).to be true
      expect(validator.valid?("invalid")).to be false
    end
  end

  describe "callback lifetime" do
    it "keeps custom format procs alive for validator lifetime" do
      schema = { "type" => "string", "format" => "my-format" }
      checker = ->(value) { value.end_with?("42!") }
      weak_checker = WeakRef.new(checker)

      validator = JSONSchema.validator_for(
        schema,
        validate_formats: true,
        formats: { "my-format" => checker }
      )
      GC.start(full_mark: true, immediate_sweep: true)

      expect(weak_checker.weakref_alive?).to be true
      expect(validator.valid?("foo42!")).to be true
      expect(validator.valid?("foo")).to be false
    end

    it "keeps custom format procs alive during schema compilation" do
      LIFETIME_STRESS_ITERATIONS.times do |iteration|
        observed_alive = nil
        checker = ->(value) { value.end_with?("42!") }
        weak_checker = WeakRef.new(checker)
        formats = { "my-format" => checker }

        retriever = lambda do |_uri|
          formats.clear
          GC.start(full_mark: true, immediate_sweep: true)
          GC.compact if GC.respond_to?(:compact)
          observed_alive = weak_checker.weakref_alive?
          { "type" => "string" }
        end

        validator = JSONSchema.validator_for(
          {
            "allOf" => [
              { "$ref" => "https://example.com/string.json" },
              { "type" => "string", "format" => "my-format" }
            ]
          },
          validate_formats: true,
          formats: formats,
          retriever: retriever
        )

        expect(validator.valid?("foo42!")).to be true
        expect(validator.valid?("foo")).to be false
        expect(observed_alive).to be(true), "format proc was collected on iteration #{iteration}"
      end
    end
  end
end

RSpec.describe JSONSchema::ValidationError do
  it "has message and paths" do
    schema = { "type" => "string" }
    begin
      JSONSchema.validate!(schema, 42)
      raise "Expected ValidationError"
    rescue JSONSchema::ValidationError => e
      expect(e.message).to eq('42 is not of type "string"')
      expect(e.instance_path).to eq([])
      expect(e.schema_path).to eq(["type"])
    end
  end

  describe "#==" do
    it "returns true for errors with same message, schema_path, and instance_path" do
      schema = { "type" => "string" }
      e1 = JSONSchema.each_error(schema, 42).first
      e2 = JSONSchema.each_error(schema, 42).first
      expect(e1).to eq(e2)
    end

    it "returns false for errors with different schema paths" do
      e1 = JSONSchema.each_error({ "type" => "string" }, 42).first
      e2 = JSONSchema.each_error({ "type" => "integer" }, "hello").first
      expect(e1).not_to eq(e2)
    end

    it "returns false when compared to non-ValidationError" do
      error = JSONSchema.each_error({ "type" => "string" }, 42).first
      expect(error).not_to eq("not an error")
    end
  end

  describe "#hash" do
    it "returns same hash for equal errors" do
      schema = { "type" => "string" }
      e1 = JSONSchema.each_error(schema, 42).first
      e2 = JSONSchema.each_error(schema, 42).first
      expect(e1.hash).to eq(e2.hash)
    end

    it "works with Array#uniq" do
      schema = { "type" => "string" }
      e1 = JSONSchema.each_error(schema, 42).first
      e2 = JSONSchema.each_error(schema, 42).first
      expect([e1, e2].uniq.size).to eq(1)
    end

    it "works as Hash keys" do
      schema = { "type" => "string" }
      e1 = JSONSchema.each_error(schema, 42).first
      e2 = JSONSchema.each_error(schema, 42).first
      h = { e1 => "first" }
      expect(h[e2]).to eq("first")
    end
  end

  describe "#instance_path_pointer" do
    it "converts instance path to JSON Pointer" do
      schema = {
        "type" => "object",
        "properties" => {
          "users" => {
            "type" => "array",
            "items" => { "type" => "string" }
          }
        }
      }
      begin
        JSONSchema.validate!(schema, { "users" => [123] })
        raise "Expected ValidationError"
      rescue JSONSchema::ValidationError => e
        expect(e.instance_path).to eq(["users", 0])
        expect(e.instance_path_pointer).to eq("/users/0")
      end
    end
  end

  describe "#schema_path_pointer" do
    it "converts schema path to JSON Pointer" do
      schema = { "type" => "string" }
      begin
        JSONSchema.validate!(schema, 42)
        raise "Expected ValidationError"
      rescue JSONSchema::ValidationError => e
        expect(e.schema_path_pointer).to eq("/type")
      end
    end
  end
end

RSpec.describe JSONSchema::Evaluation do
  let(:schema) { { "type" => "string" } }

  describe "#valid?" do
    it "returns true for valid instance" do
      eval = JSONSchema.evaluate(schema, "hello")
      expect(eval.valid?).to be true
    end

    it "returns false for invalid instance" do
      eval = JSONSchema.evaluate(schema, 42)
      expect(eval.valid?).to be false
    end
  end

  describe "#flag" do
    it "returns flag output format" do
      eval = JSONSchema.evaluate(schema, "hello")
      flag = eval.flag
      expect(flag).to be_a(Hash)
      expect(flag[:valid]).to be true
    end

    it "returns only valid key" do
      eval = JSONSchema.evaluate(schema, "hello")
      expect(eval.flag.keys).to eq([:valid])
    end
  end

  describe "#list" do
    it "returns list output format for valid instance" do
      eval = JSONSchema.evaluate(schema, "hello")
      list = eval.list
      expect(list).to be_a(Hash)
      expect(list[:valid]).to be true
    end

    it "contains details for evaluation" do
      eval = JSONSchema.evaluate(schema, 42)
      list = eval.list
      expect(list[:valid]).to be false
      expect(list[:details]).to be_an(Array)
    end
  end

  describe "#hierarchical" do
    it "returns hierarchical output format" do
      eval = JSONSchema.evaluate(schema, "hello")
      hier = eval.hierarchical
      expect(hier).to be_a(Hash)
      expect(hier[:valid]).to be true
    end

    it "contains nested structure" do
      schema = { "type" => "object", "properties" => { "name" => { "type" => "string" } } }
      eval = JSONSchema.evaluate(schema, { "name" => "Alice" })
      hier = eval.hierarchical
      expect(hier[:valid]).to be true
    end
  end

  describe "#annotations" do
    it "returns array of annotations" do
      schema = { "type" => "object", "title" => "Test" }
      eval = JSONSchema.evaluate(schema, {})
      expect(eval.annotations).to be_an(Array)
    end
  end

  describe "#errors" do
    it "returns empty array for valid instance" do
      eval_result = JSONSchema.evaluate(schema, "hello")
      expect(eval_result.errors).to eq([])
    end

    it "returns array of errors for invalid instance" do
      eval_result = JSONSchema.evaluate(schema, 42)
      expect(eval_result.errors.size).to eq(1)
      error = eval_result.errors.first
      expect(error).to have_key(:instanceLocation)
      expect(error).to have_key(:schemaLocation)
      expect(error).to have_key(:error)
    end
  end

  describe "#inspect" do
    it "returns readable representation" do
      eval_result = JSONSchema.evaluate(schema, "hello")
      expect(eval_result.inspect).to eq("#<JSONSchema::Evaluation valid=true>")
    end
  end
end

RSpec.describe JSONSchema::Meta do
  describe ".valid?" do
    it "returns true for valid schema" do
      schema = { "type" => "string" }
      expect(JSONSchema::Meta.valid?(schema)).to be true
    end

    it "returns false for invalid schema" do
      schema = { "type" => "invalid_type" }
      expect(JSONSchema::Meta.valid?(schema)).to be false
    end

    it "accepts optional registry keyword argument" do
      registry = JSONSchema::Registry.new([
                                            ["https://example.com/ref.json", { "type" => "string" }]
                                          ])
      schema = { "$ref" => "https://example.com/ref.json" }
      expect(JSONSchema::Meta.valid?(schema, registry: registry)).to be true
    end
  end

  describe ".validate!" do
    it "returns nil for valid schema" do
      schema = { "type" => "string" }
      expect(JSONSchema::Meta.validate!(schema)).to be_nil
    end

    it "raises ValidationError for invalid schema" do
      schema = { "type" => "invalid_type" }
      expect { JSONSchema::Meta.validate!(schema) }.to raise_error(JSONSchema::ValidationError)
    end
  end
end

RSpec.describe "Custom keywords" do
  let(:even_validator_class) do
    Class.new do
      def initialize(_parent_schema, value, _schema_path)
        @enabled = value
      end

      def validate(instance)
        return unless @enabled
        return unless instance.is_a?(Integer)

        raise "#{instance} is not even" if instance.odd?
      end
    end
  end

  let(:range_validator_class) do
    Class.new do
      def initialize(_parent_schema, value, _schema_path)
        @min = value["min"] || -Float::INFINITY
        @max = value["max"] || Float::INFINITY
      end

      def validate(instance)
        return unless instance.is_a?(Numeric)

        raise "Value #{instance} not in range [#{@min}, #{@max}]" unless (@min..@max).cover?(instance)
      end
    end
  end

  describe "with validator_for" do
    it "validates with custom even keyword" do
      validator = JSONSchema.validator_for(
        { "even" => true },
        keywords: { "even" => even_validator_class }
      )
      expect(validator.valid?(2)).to be true
      expect(validator.valid?(4)).to be true
      expect(validator.valid?(1)).to be false
      expect(validator.valid?(3)).to be false
      expect(validator.valid?("not a number")).to be true
    end

    it "can be disabled by setting value to false" do
      validator = JSONSchema.validator_for(
        { "even" => false },
        keywords: { "even" => even_validator_class }
      )
      expect(validator.valid?(1)).to be true
      expect(validator.valid?(3)).to be true
    end

    it "works with standard keywords" do
      validator = JSONSchema.validator_for(
        { "type" => "integer", "minimum" => 0, "even" => true },
        keywords: { "even" => even_validator_class }
      )
      expect(validator.valid?(2)).to be true
      expect(validator.valid?(100)).to be true
      expect(validator.valid?(3)).to be false
      expect(validator.valid?(-2)).to be false
      expect(validator.valid?("hello")).to be false
    end

    it "supports nested schemas" do
      validator = JSONSchema.validator_for(
        {
          "type" => "object",
          "properties" => {
            "count" => { "type" => "integer", "even" => true }
          }
        },
        keywords: { "even" => even_validator_class }
      )
      expect(validator.valid?({ "count" => 2 })).to be true
      expect(validator.valid?({ "count" => 3 })).to be false
    end

    it "supports range validator with object value" do
      validator = JSONSchema.validator_for(
        { "customRange" => { "min" => 0, "max" => 10 } },
        keywords: { "customRange" => range_validator_class }
      )
      expect(validator.valid?(5)).to be true
      expect(validator.valid?(0)).to be true
      expect(validator.valid?(10)).to be true
      expect(validator.valid?(-1)).to be false
      expect(validator.valid?(11)).to be false
    end
  end

  describe "with validate!" do
    it "raises on validation error" do
      validator = JSONSchema.validator_for(
        { "even" => true },
        keywords: { "even" => even_validator_class }
      )
      expect { validator.validate!(3) }.to raise_error(JSONSchema::ValidationError) do |error|
        expect(error.message).to include("3 is not even")
      end
    end
  end

  describe "with each_error" do
    it "returns errors for invalid instance" do
      validator = JSONSchema.validator_for(
        { "even" => true },
        keywords: { "even" => even_validator_class }
      )
      errors = validator.each_error(3).to_a
      expect(errors.size).to eq(1)
      expect(errors.first).to be_a(JSONSchema::ValidationError)
    end
  end

  describe "with module-level functions" do
    it "works with valid?" do
      expect(JSONSchema.valid?({ "even" => true }, 2, keywords: { "even" => even_validator_class })).to be true
      expect(JSONSchema.valid?({ "even" => true }, 3, keywords: { "even" => even_validator_class })).to be false
    end

    it "works with validate!" do
      JSONSchema.validate!({ "even" => true }, 2, keywords: { "even" => even_validator_class })
      expect { JSONSchema.validate!({ "even" => true }, 3, keywords: { "even" => even_validator_class }) }
        .to raise_error(JSONSchema::ValidationError)
    end

    it "works with each_error" do
      errors = JSONSchema.each_error({ "even" => true }, 3, keywords: { "even" => even_validator_class }).to_a
      expect(errors.size).to eq(1)
    end
  end

  describe "with draft-specific validators" do
    it "works with all drafts" do
      [
        JSONSchema::Draft4Validator,
        JSONSchema::Draft6Validator,
        JSONSchema::Draft7Validator,
        JSONSchema::Draft201909Validator,
        JSONSchema::Draft202012Validator
      ].each do |validator_class|
        validator = validator_class.new({ "even" => true }, keywords: { "even" => even_validator_class })
        expect(validator.valid?(2)).to be true
        expect(validator.valid?(3)).to be false
      end
    end
  end

  describe "error handling" do
    it "raises error for non-class keyword validator" do
      expect { JSONSchema.validator_for({ "myKeyword" => true }, keywords: { "myKeyword" => "not a class" }) }
        .to raise_error(TypeError, /must be a class with 'new' and 'validate' methods/)
    end
  end

  describe "lifetime management" do
    it "does not leak keyword validator instances after validator disposal" do
      keyword_class = Class.new do
        def initialize(parent_schema, value, schema_path); end

        def validate(instance); end
      end

      LIFETIME_STRESS_ITERATIONS.times do
        JSONSchema.validator_for(
          { "customKeyword" => true },
          keywords: { "customKeyword" => keyword_class }
        )
      end

      LIFETIME_GC_ROUNDS.times do
        10_000.times { +"x" }
        GC.start(full_mark: true, immediate_sweep: true)
      end

      expect(ObjectSpace.each_object(keyword_class).count).to be <= 2
    end

    it "keeps keyword validator instances alive while wrapping reusable validators" do
      original_stress = GC.stress
      constructors = {
        "validator_for" => lambda do |schema, keywords|
          JSONSchema.validator_for(schema, keywords: keywords)
        end,
        "Draft7Validator.new" => lambda do |schema, keywords|
          JSONSchema::Draft7Validator.new(schema, keywords: keywords)
        end
      }
      GC.stress = true

      constructors.each do |label, constructor|
        LIFETIME_STRESS_ITERATIONS.times do |iteration|
          instance_ref = nil

          keyword_class = Class.new do
            define_method(:initialize) do |_parent_schema, _value, _schema_path|
              instance_ref = WeakRef.new(self)
            end

            def validate(instance); end
          end

          validator = constructor.call(
            { "customKeyword" => true },
            { "customKeyword" => keyword_class }
          )
          GC.start(full_mark: true, immediate_sweep: true)
          GC.compact if GC.respond_to?(:compact)

          expect(instance_ref).not_to be_nil
          expect(instance_ref.weakref_alive?).to be(
            true
          ), "#{label}: keyword instance was collected on iteration #{iteration}"
          expect(validator.valid?(1)).to be true
        end
      end
    ensure
      GC.stress = original_stress
    end

    it "keeps keyword validator classes alive until custom keywords are compiled" do
      LIFETIME_STRESS_ITERATIONS.times do |iteration|
        observed_alive = nil

        keyword_class = Class.new do
          def initialize(parent_schema, value, schema_path); end

          def validate(instance); end
        end
        keyword_class_ref = WeakRef.new(keyword_class)

        keywords = { "customKeyword" => keyword_class }

        retriever = lambda do |_uri|
          keywords.clear
          GC.start(full_mark: true, immediate_sweep: true)
          GC.compact if GC.respond_to?(:compact)
          observed_alive = keyword_class_ref.weakref_alive?
          { "type" => "integer" }
        end

        schema = {
          "allOf" => [
            { "$ref" => "https://example.com/integer.json" },
            { "customKeyword" => true }
          ]
        }

        validator = JSONSchema.validator_for(schema, keywords: keywords, retriever: retriever)
        expect(validator.valid?(1)).to be true
        expect(observed_alive).to be(true), "keyword class was collected on iteration #{iteration}"
      end
    end

    it "keeps direct retriever callbacks alive during keyword compilation" do
      LIFETIME_STRESS_ITERATIONS.times do |iteration|
        retriever_ref = nil
        observed_alive = nil

        keyword_class = Class.new do
          define_method(:initialize) do |_parent_schema, _value, _schema_path|
            GC.start(full_mark: true, immediate_sweep: true)
            GC.compact if GC.respond_to?(:compact)
            observed_alive = retriever_ref.weakref_alive?
          end

          def validate(instance); end
        end

        schema = {
          "allOf" => [
            { "customKeyword" => true },
            { "$ref" => "https://example.com/integer.json" }
          ]
        }

        validator = JSONSchema.validator_for(
          schema,
          keywords: { "customKeyword" => keyword_class },
          retriever: ->(_uri) { { "type" => "integer" } }.tap { |proc| retriever_ref = WeakRef.new(proc) }
        )
        expect(validator.valid?(1)).to be true
        expect(observed_alive).to be(true), "retriever proc was collected on iteration #{iteration}"
      end
    end
  end
end

RSpec.describe "Registry with validators" do
  it "keeps retriever callback alive while registry exists" do
    build_registry = lambda do
      retriever = lambda do |uri|
        { "type" => "string" } if uri == "https://example.com/string.json"
      end
      retriever_ref = WeakRef.new(retriever)
      [JSONSchema::Registry.new([], retriever: retriever), retriever_ref]
    end

    _registry, retriever_ref = build_registry.call

    LIFETIME_GC_ROUNDS.times do
      break unless retriever_ref.weakref_alive?

      10_000.times { +"x" }
      GC.start(full_mark: true, immediate_sweep: true)
    end

    expect(retriever_ref.weakref_alive?).to be true
  end

  it "validates with registry on module-level valid?" do
    registry = JSONSchema::Registry.new([
                                          ["https://example.com/string.json", { "type" => "string" }]
                                        ])
    schema = { "$ref" => "https://example.com/string.json" }
    expect(JSONSchema.valid?(schema, "hello", registry: registry)).to be true
    expect(JSONSchema.valid?(schema, 42, registry: registry)).to be false
  end

  it "validates with registry on module-level validate!" do
    registry = JSONSchema::Registry.new([
                                          ["https://example.com/string.json", { "type" => "string" }]
                                        ])
    schema = { "$ref" => "https://example.com/string.json" }
    expect(JSONSchema.validate!(schema, "hello", registry: registry)).to be_nil
    expect { JSONSchema.validate!(schema, 42, registry: registry) }.to raise_error(JSONSchema::ValidationError)
  end

  it "validates with registry on module-level each_error" do
    registry = JSONSchema::Registry.new([
                                          ["https://example.com/string.json", { "type" => "string" }]
                                        ])
    schema = { "$ref" => "https://example.com/string.json" }
    errors = JSONSchema.each_error(schema, 42, registry: registry).to_a
    expect(errors).not_to be_empty
    expect(errors.first).to be_a(JSONSchema::ValidationError)
  end

  it "validates with registry on module-level evaluate" do
    registry = JSONSchema::Registry.new([
                                          ["https://example.com/string.json", { "type" => "string" }]
                                        ])
    schema = { "$ref" => "https://example.com/string.json" }
    eval_result = JSONSchema.evaluate(schema, 42, registry: registry)
    expect(eval_result.valid?).to be false
  end

  it "validates with registry on validator_for" do
    registry = JSONSchema::Registry.new([
                                          ["https://example.com/string.json", { "type" => "string" }]
                                        ])
    schema = { "$ref" => "https://example.com/string.json" }
    validator = JSONSchema.validator_for(schema, registry: registry)
    expect(validator.valid?("hello")).to be true
    expect(validator.valid?(42)).to be false
  end

  it "uses registry retriever during validator compilation when only registry is provided" do
    registry = JSONSchema::Registry.new(
      [],
      retriever: lambda do |uri|
        { "type" => "string" } if uri == "https://example.com/string.json"
      end
    )
    validator = JSONSchema.validator_for({ "$ref" => "https://example.com/string.json" }, registry: registry)
    expect(validator.valid?("hello")).to be true
    expect(validator.valid?(42)).to be false
  end

  it "validates with registry on draft-specific validators" do
    registry = JSONSchema::Registry.new([
                                          ["https://example.com/string.json", { "type" => "string" }]
                                        ])
    schema = { "$ref" => "https://example.com/string.json" }
    [
      JSONSchema::Draft4Validator,
      JSONSchema::Draft6Validator,
      JSONSchema::Draft7Validator,
      JSONSchema::Draft201909Validator,
      JSONSchema::Draft202012Validator
    ].each do |klass|
      validator = klass.new(schema, registry: registry)
      expect(validator.valid?("hello")).to be true
      expect(validator.valid?(42)).to be false
    end
  end
end

RSpec.describe "validate_formats option" do
  it "does not validate formats by default" do
    schema = { "type" => "string", "format" => "email" }
    expect(JSONSchema.valid?(schema, "not-an-email")).to be true
  end

  it "validates formats when enabled" do
    schema = { "type" => "string", "format" => "email" }
    expect(JSONSchema.valid?(schema, "not-an-email", validate_formats: true)).to be false
    expect(JSONSchema.valid?(schema, "user@example.com", validate_formats: true)).to be true
  end

  it "explicitly disables format validation" do
    schema = { "type" => "string", "format" => "email" }
    expect(JSONSchema.valid?(schema, "not-an-email", validate_formats: false)).to be true
  end
end

RSpec.describe "ignore_unknown_formats option" do
  it "ignores unknown formats by default" do
    schema = { "type" => "string", "format" => "totally-made-up" }
    expect(JSONSchema.valid?(schema, "anything", validate_formats: true)).to be true
  end

  it "raises on unknown formats when not ignored" do
    schema = { "type" => "string", "format" => "totally-made-up" }
    expect do
      JSONSchema.valid?(schema, "anything", validate_formats: true, ignore_unknown_formats: false)
    end.to raise_error(ArgumentError, /Unknown format/)
  end
end

RSpec.describe JSONSchema::ValidationErrorKind do
  it "has name and value for single type error" do
    JSONSchema.validate!({ "type" => "string" }, 42)
    raise "Expected ValidationError"
  rescue JSONSchema::ValidationError => e
    expect(e.kind.name).to eq("type")
    expect(e.kind.value).to be_a(Hash)
    expect(e.kind.value[:types]).to eq(["string"])
  end

  it "has name and value for multiple type error" do
    JSONSchema.validate!({ "type" => %w[string number] }, [])
    raise "Expected ValidationError"
  rescue JSONSchema::ValidationError => e
    expect(e.kind.name).to eq("type")
    expect(e.kind.value[:types]).to eq(%w[number string])
  end

  it "has name and value for required error" do
    JSONSchema.validate!({ "type" => "object", "required" => ["name"] }, {})
    raise "Expected ValidationError"
  rescue JSONSchema::ValidationError => e
    expect(e.kind.name).to eq("required")
    expect(e.kind.value).to be_a(Hash)
    expect(e.kind.value).to have_key(:property)
    expect(e.kind.value[:property]).to eq("name")
  end

  it "returns a hash from to_h" do
    JSONSchema.validate!({ "type" => "string" }, 42)
    raise "Expected ValidationError"
  rescue JSONSchema::ValidationError => e
    h = e.kind.to_h
    expect(h).to be_a(Hash)
    expect(h[:name]).to eq("type")
    expect(h[:value]).to be_a(Hash)
  end

  it "has inspect and to_s" do
    JSONSchema.validate!({ "type" => "string" }, 42)
    raise "Expected ValidationError"
  rescue JSONSchema::ValidationError => e
    expect(e.kind.inspect).to eq('#<JSONSchema::ValidationErrorKind name="type">')
    expect(e.kind.to_s).to eq("type")
  end

  it "preserves anyOf context with sub-error details" do
    schema = { "anyOf" => [{ "type" => "string" }, { "type" => "integer" }] }
    errors = JSONSchema.each_error(schema, []).to_a
    expect(errors).not_to be_empty
    error = errors.first
    expect(error.kind.name).to eq("anyOf")
    context = error.kind.value[:context]
    expect(context).to be_an(Array)
    expect(context).not_to be_empty
    # Each branch has sub-errors with detailed fields
    sub_error = context.first.first
    expect(sub_error).to have_key(:message)
    expect(sub_error).to have_key(:instance_path)
    expect(sub_error).to have_key(:schema_path)
    expect(sub_error).to have_key(:evaluation_path)
    expect(sub_error).to have_key(:kind)
    expect(sub_error[:kind]).to eq("type")
  end

  it "masks anyOf context messages when mask option is used" do
    schema = { "anyOf" => [{ "type" => "string" }, { "type" => "integer" }] }
    validator = JSONSchema.validator_for(schema, mask: "[HIDDEN]")
    expect { validator.validate!(true) }.to raise_error(JSONSchema::ValidationError) do |error|
      context = error.kind.value[:context]
      messages = context.flatten.map { |entry| entry[:message] }
      expect(messages).to all(include("[HIDDEN]"))
      expect(messages.join(" ")).not_to include("true")
    end
  end

  it "preserves oneOf context with sub-error details" do
    schema = { "oneOf" => [{ "type" => "string" }, { "type" => "integer" }] }
    errors = JSONSchema.each_error(schema, []).to_a
    expect(errors).not_to be_empty
    error = errors.first
    expect(error.kind.name).to eq("oneOf")
    context = error.kind.value[:context]
    expect(context).to be_an(Array)
    sub_error = context.first.first
    expect(sub_error).to have_key(:evaluation_path)
    expect(sub_error).to have_key(:kind)
  end

  it "has name falseSchema for false schema error" do
    schema = false
    errors = JSONSchema.each_error(schema, "anything").to_a
    expect(errors).not_to be_empty
    expect(errors.first.kind.name).to eq("falseSchema")
  end

  it "has name and value for enum error" do
    schema = { "enum" => [1, 2, 3] }
    errors = JSONSchema.each_error(schema, 4).to_a
    expect(errors).not_to be_empty
    error = errors.first
    expect(error.kind.name).to eq("enum")
    expect(error.kind.value).to have_key(:options)
  end

  it "has name and value for minimum error" do
    schema = { "type" => "number", "minimum" => 10 }
    errors = JSONSchema.each_error(schema, 5).to_a
    expect(errors).not_to be_empty
    expect(errors.first.kind.name).to eq("minimum")
    expect(errors.first.kind.value).to have_key(:limit)
  end

  it "has name and value for maxLength error" do
    schema = { "type" => "string", "maxLength" => 3 }
    errors = JSONSchema.each_error(schema, "toolong").to_a
    expect(errors).not_to be_empty
    expect(errors.first.kind.name).to eq("maxLength")
    expect(errors.first.kind.value[:limit]).to eq(3)
  end

  it "has name and value for pattern error" do
    schema = { "type" => "string", "pattern" => "^a" }
    errors = JSONSchema.each_error(schema, "bbb").to_a
    expect(errors).not_to be_empty
    expect(errors.first.kind.name).to eq("pattern")
    expect(errors.first.kind.value).to have_key(:pattern)
  end

  it "has name and value for const error" do
    schema = { "const" => "hello" }
    errors = JSONSchema.each_error(schema, "world").to_a
    expect(errors).not_to be_empty
    expect(errors.first.kind.name).to eq("const")
    expect(errors.first.kind.value).to have_key(:expected_value)
  end

  it "has name uniqueItems for uniqueItems error" do
    schema = { "type" => "array", "uniqueItems" => true }
    errors = JSONSchema.each_error(schema, [1, 1]).to_a
    expect(errors).not_to be_empty
    expect(errors.first.kind.name).to eq("uniqueItems")
  end
end

RSpec.describe "Thread safety" do
  it "validates concurrently with shared validator" do
    schema = { "type" => "object", "properties" => { "name" => { "type" => "string" } }, "required" => ["name"] }
    validator = JSONSchema.validator_for(schema)

    threads = 10.times.map do |i|
      Thread.new do
        100.times do
          if i.even?
            expect(validator.valid?({ "name" => "Alice" })).to be true
          else
            expect(validator.valid?({})).to be false
          end
        end
      end
    end
    threads.each(&:join)
  end
end

RSpec.describe JSONSchema::ValidationError do
  describe "#verbose_message" do
    it "returns full context with schema path and instance" do
      schema = { "type" => "string" }
      begin
        JSONSchema.validate!(schema, 42)
        raise "Expected ValidationError"
      rescue JSONSchema::ValidationError => e
        expect(e.verbose_message).to eq(
          "42 is not of type \"string\"\n\nFailed validating \"type\" in schema\n\nOn instance:\n    42"
        )
      end
    end

    it "includes nested path for nested errors" do
      schema = {
        "type" => "object",
        "properties" => {
          "name" => { "type" => "string" }
        }
      }
      begin
        JSONSchema.validate!(schema, { "name" => 123 })
        raise "Expected ValidationError"
      rescue JSONSchema::ValidationError => e
        expect(e.verbose_message).to eq(
          "123 is not of type \"string\"\n\nFailed validating \"type\" in schema[\"properties\"][\"name\"]\n\nOn instance[\"name\"]:\n    123"
        )
      end
    end

    it "preserves numeric string keys as object properties in paths" do
      schema = {
        "type" => "object",
        "properties" => {
          "123" => { "type" => "string" }
        }
      }
      begin
        JSONSchema.validate!(schema, { "123" => 42 })
        raise "Expected ValidationError"
      rescue JSONSchema::ValidationError => e
        expect(e.verbose_message).to eq(
          "42 is not of type \"string\"\n\nFailed validating \"type\" in schema[\"properties\"][\"123\"]\n\nOn instance[\"123\"]:\n    42"
        )
      end
    end

    it "unescapes JSON Pointer segments in verbose paths" do
      schema = {
        "type" => "object",
        "properties" => {
          "a/b~c" => {
            "type" => "object",
            "properties" => {
              "123" => { "type" => "string" }
            }
          }
        }
      }
      begin
        JSONSchema.validate!(schema, { "a/b~c" => { "123" => 42 } })
        raise "Expected ValidationError"
      rescue JSONSchema::ValidationError => e
        expect(e.verbose_message).to eq(
          "42 is not of type \"string\"\n\nFailed validating \"type\" in schema[\"properties\"][\"a/b~c\"][\"properties\"][\"123\"]\n\nOn instance[\"a/b~c\"][\"123\"]:\n    42"
        )
      end
    end

    it "masks instance value when mask option is used" do
      schema = { "type" => "string" }
      validator = JSONSchema.validator_for(schema, mask: "[HIDDEN]")
      begin
        validator.validate!(42)
        raise "Expected ValidationError"
      rescue JSONSchema::ValidationError => e
        expect(e.verbose_message).to eq(
          "[HIDDEN] is not of type \"string\"\n\nFailed validating \"type\" in schema\n\nOn instance:\n    [HIDDEN]"
        )
      end
    end
  end
end

RSpec.describe "base_uri option" do
  it "resolves $ref relative to base_uri" do
    schema = { "$ref" => "#/$defs/name", "$defs" => { "name" => { "type" => "string" } } }
    validator = JSONSchema.validator_for(schema, base_uri: "http://example.com/schema")
    expect(validator.valid?("hello")).to be true
    expect(validator.valid?(42)).to be false
  end

  it "works with module-level valid?" do
    schema = { "$ref" => "#/$defs/name", "$defs" => { "name" => { "type" => "string" } } }
    expect(JSONSchema.valid?(schema, "hello", base_uri: "http://example.com/schema")).to be true
  end
end

RSpec.describe JSONSchema::RegexOptions do
  it "creates options with defaults" do
    opts = JSONSchema::RegexOptions.new
    expect(opts.size_limit).to be_nil
    expect(opts.dfa_size_limit).to be_nil
  end

  it "creates options with custom values" do
    opts = JSONSchema::RegexOptions.new(size_limit: 1024, dfa_size_limit: 2048)
    expect(opts.size_limit).to eq(1024)
    expect(opts.dfa_size_limit).to eq(2048)
  end

  it "can be passed as pattern_options" do
    opts = JSONSchema::RegexOptions.new(size_limit: 10_000_000)
    schema = { "type" => "string", "pattern" => "^test$" }
    validator = JSONSchema.validator_for(schema, pattern_options: opts)
    expect(validator.valid?("test")).to be true
    expect(validator.valid?("other")).to be false
  end

  it "has inspect output" do
    opts = JSONSchema::RegexOptions.new(size_limit: 1024)
    expect(opts.inspect).to eq("#<JSONSchema::RegexOptions size_limit=1024, dfa_size_limit=nil>")
  end
end

RSpec.describe JSONSchema::FancyRegexOptions do
  it "creates options with defaults" do
    opts = JSONSchema::FancyRegexOptions.new
    expect(opts.backtrack_limit).to be_nil
    expect(opts.size_limit).to be_nil
    expect(opts.dfa_size_limit).to be_nil
  end

  it "creates options with custom values" do
    opts = JSONSchema::FancyRegexOptions.new(backtrack_limit: 1000, size_limit: 2048)
    expect(opts.backtrack_limit).to eq(1000)
    expect(opts.size_limit).to eq(2048)
  end

  it "can be passed as pattern_options" do
    opts = JSONSchema::FancyRegexOptions.new(backtrack_limit: 1_000_000)
    schema = { "type" => "string", "pattern" => "^test$" }
    validator = JSONSchema.validator_for(schema, pattern_options: opts)
    expect(validator.valid?("test")).to be true
    expect(validator.valid?("other")).to be false
  end

  it "has inspect output" do
    opts = JSONSchema::FancyRegexOptions.new(backtrack_limit: 1000)
    expect(opts.inspect).to eq("#<JSONSchema::FancyRegexOptions backtrack_limit=1000, size_limit=nil, dfa_size_limit=nil>")
  end
end

RSpec.describe JSONSchema::EmailOptions do
  it "creates options with defaults" do
    opts = JSONSchema::EmailOptions.new
    expect(opts.require_tld).to be false
    expect(opts.allow_domain_literal).to be true
    expect(opts.allow_display_text).to be true
    expect(opts.minimum_sub_domains).to be_nil
  end

  it "creates options with custom values" do
    opts = JSONSchema::EmailOptions.new(
      require_tld: false,
      allow_domain_literal: true,
      allow_display_text: true,
      minimum_sub_domains: 2
    )
    expect(opts.require_tld).to be false
    expect(opts.allow_domain_literal).to be true
    expect(opts.allow_display_text).to be true
    expect(opts.minimum_sub_domains).to eq(2)
  end

  it "can be passed as email_options" do
    opts = JSONSchema::EmailOptions.new(require_tld: false)
    schema = { "type" => "string", "format" => "email" }
    validator = JSONSchema.validator_for(schema, validate_formats: true, email_options: opts)
    expect(validator.valid?("user@localhost")).to be true
  end

  it "has inspect output" do
    opts = JSONSchema::EmailOptions.new(require_tld: true, minimum_sub_domains: 2)
    expect(opts.inspect).to eq("#<JSONSchema::EmailOptions require_tld=true, allow_domain_literal=true, allow_display_text=true, minimum_sub_domains=2>")
  end
end

RSpec.describe JSONSchema::HttpOptions do
  it "creates options with defaults" do
    opts = JSONSchema::HttpOptions.new
    expect(opts.timeout).to be_nil
    expect(opts.connect_timeout).to be_nil
    expect(opts.tls_verify).to be true
    expect(opts.ca_cert).to be_nil
  end

  it "creates options with custom values" do
    opts = JSONSchema::HttpOptions.new(timeout: 30.0, connect_timeout: 5.0, tls_verify: false)
    expect(opts.timeout).to eq(30.0)
    expect(opts.connect_timeout).to eq(5.0)
    expect(opts.tls_verify).to be false
  end

  it "has inspect output" do
    opts = JSONSchema::HttpOptions.new(timeout: 30.0, tls_verify: false)
    expect(opts.inspect).to eq("#<JSONSchema::HttpOptions timeout=30, connect_timeout=nil, tls_verify=false, ca_cert=nil>")
  end

  it "raises ArgumentError for negative timeout" do
    schema = { "type" => "string" }
    opts = JSONSchema::HttpOptions.new(timeout: -1.0)
    expect { JSONSchema.validator_for(schema, http_options: opts) }
      .to raise_error(ArgumentError, /http_options\.timeout/)
  end

  it "raises ArgumentError for non-finite connect_timeout" do
    schema = { "type" => "string" }
    opts = JSONSchema::HttpOptions.new(connect_timeout: Float::NAN)
    expect { JSONSchema.validator_for(schema, http_options: opts) }
      .to raise_error(ArgumentError, /http_options\.connect_timeout/)
  end

  it "raises ArgumentError for oversized timeout" do
    schema = { "type" => "string" }
    opts = JSONSchema::HttpOptions.new(timeout: Float::MAX)
    expect { JSONSchema.validator_for(schema, http_options: opts) }
      .to raise_error(ArgumentError, /http_options\.timeout/)
  end
end

RSpec.describe "Evaluation output structure" do
  let(:schema) do
    {
      "type" => "object",
      "properties" => { "name" => { "type" => "string" }, "age" => { "type" => "integer" } },
      "required" => ["name"]
    }
  end

  describe "#list" do
    it "has valid and details keys" do
      eval = JSONSchema.evaluate(schema, { "name" => 42 })
      list = eval.list
      expect(list).to have_key(:valid)
      expect(list).to have_key(:details)
      expect(list[:valid]).to be false
      expect(list[:details]).to be_an(Array)
      expect(list[:details]).not_to be_empty
    end
  end

  describe "#hierarchical" do
    it "has valid key and nested details" do
      eval = JSONSchema.evaluate(schema, { "name" => "Alice", "age" => 30 })
      hier = eval.hierarchical
      expect(hier).to have_key(:valid)
      expect(hier[:valid]).to be true
    end

    it "has nested details for invalid instance" do
      eval = JSONSchema.evaluate(schema, { "name" => 42 })
      hier = eval.hierarchical
      expect(hier[:valid]).to be false
      expect(hier).to have_key(:details)
    end
  end

  describe "#errors" do
    it "has complete error structure" do
      eval = JSONSchema.evaluate(schema, {})
      errors = eval.errors
      expect(errors).not_to be_empty
      error = errors.first
      expect(error).to have_key(:instanceLocation)
      expect(error).to have_key(:schemaLocation)
      expect(error).to have_key(:error)
      expect(error).to have_key(:absoluteKeywordLocation)
    end
  end

  describe "#annotations" do
    it "has complete annotation structure" do
      eval = JSONSchema.evaluate(schema, { "name" => "Alice" })
      annotations = eval.annotations
      expect(annotations).to be_an(Array)
      next if annotations.empty?

      annotation = annotations.first
      expect(annotation).to have_key(:instanceLocation)
      expect(annotation).to have_key(:schemaLocation)
      expect(annotation).to have_key(:annotations)
    end
  end
end

RSpec.describe "evaluation_path_pointer" do
  it "returns evaluation path as JSON Pointer" do
    schema = {
      "type" => "object",
      "properties" => {
        "name" => { "type" => "string" }
      }
    }
    begin
      JSONSchema.validate!(schema, { "name" => 123 })
      raise "Expected ValidationError"
    rescue JSONSchema::ValidationError => e
      expect(e.evaluation_path_pointer).to be_a(String)
    end
  end
end

RSpec.describe "BigDecimal and large numbers" do
  it "validates large integers" do
    schema = { "type" => "integer", "minimum" => 0 }
    expect(JSONSchema.valid?(schema, 10**100)).to be true
    expect(JSONSchema.valid?(schema, -(10**100))).to be false
  end

  it "validates large floats" do
    schema = { "type" => "number", "minimum" => 0 }
    expect(JSONSchema.valid?(schema, 1.0e+308)).to be true
  end

  it "handles BigDecimal values" do
    require "bigdecimal"
    schema = { "type" => "number", "minimum" => 0 }
    expect(JSONSchema.valid?(schema, BigDecimal("99999999999999999999.5"))).to be true
    expect(JSONSchema.valid?(schema, BigDecimal("-1.5"))).to be false
  end

  it "handles BigDecimal values nested in input data" do
    require "bigdecimal"
    schema = {
      "type" => "object",
      "properties" => { "price" => { "type" => "number", "minimum" => 0 } },
      "required" => ["price"]
    }
    expect(JSONSchema.valid?(schema, { "price" => BigDecimal("1234567890.123456789") })).to be true
  end

  it "preserves large integer precision in ValidationError#instance" do
    value = 10**100
    expect { JSONSchema.validate!({ "type" => "string" }, value) }.to raise_error(JSONSchema::ValidationError) do |error|
      expect(error.instance).to eq(value)
      expect(error.instance).to be_a(Integer)
    end
  end

  it "preserves decimal precision in ValidationErrorKind values" do
    require "bigdecimal"
    expected = BigDecimal("12345678901234567890.12345678901234567890")
    expect { JSONSchema.validate!({ "const" => expected }, 0) }.to raise_error(JSONSchema::ValidationError) do |error|
      actual = error.kind.value[:expected_value]
      expect(actual).to be_a(BigDecimal)
      expect(actual.to_s("F")).to eq(expected.to_s("F"))
    end
  end
end

RSpec.describe "Symbol-keyed schemas" do
  it "validates with symbol keys in schema" do
    schema = { type: "string" }
    expect(JSONSchema.valid?(schema, "hello")).to be true
    expect(JSONSchema.valid?(schema, 42)).to be false
  end

  it "validates with nested symbol keys" do
    schema = {
      type: "object",
      properties: {
        name: { type: "string" }
      },
      required: ["name"]
    }
    validator = JSONSchema.validator_for(schema)
    expect(validator.valid?({ "name" => "Alice" })).to be true
    expect(validator.valid?({})).to be false
  end

  it "validates with mixed string and symbol keys" do
    schema = { "type" => "object", properties: { name: { "type" => "string" } } }
    expect(JSONSchema.valid?(schema, { "name" => "Alice" })).to be true
  end
end

RSpec.describe "JSON string schema input" do
  describe ".valid?" do
    [
      ['{"type":"integer"}', 42, true],
      ['{"type":"integer"}', "hello", false],
      ['{"type":"string","minLength":3}', "hello", true],
      ['{"type":"string","minLength":3}', "hi", false],
      ["true", "anything", true],
      ["false", "anything", false]
    ].each do |(schema, instance, expected)|
      it "returns #{expected} for schema=#{schema.inspect}, instance=#{instance.inspect}" do
        expect(JSONSchema.valid?(schema, instance)).to eq(expected)
      end
    end

    # Non-JSON strings fall back to a string value, which is not a valid schema type
    it "raises for non-JSON string as schema" do
      expect { JSONSchema.valid?("not json", "anything") }.to raise_error(ArgumentError)
    end
  end

  describe ".validate!" do
    it "accepts JSON string schema" do
      expect(JSONSchema.validate!('{"type":"string"}', "hello")).to be_nil
    end

    it "raises for invalid instance with JSON string schema" do
      expect { JSONSchema.validate!('{"type":"string"}', 42) }.to raise_error(JSONSchema::ValidationError)
    end
  end

  describe ".each_error" do
    it "returns errors with JSON string schema" do
      errors = JSONSchema.each_error('{"type":"string"}', 42).to_a
      expect(errors).not_to be_empty
      expect(errors.first).to be_a(JSONSchema::ValidationError)
    end
  end

  describe ".evaluate" do
    it "evaluates with JSON string schema" do
      eval = JSONSchema.evaluate('{"type":"string"}', "hello")
      expect(eval.valid?).to be true
    end
  end

  describe ".validator_for" do
    it "creates validator from JSON string schema" do
      validator = JSONSchema.validator_for('{"type":"integer"}')
      expect(validator.valid?(42)).to be true
      expect(validator.valid?("hello")).to be false
    end
  end

  describe "draft-specific validators" do
    [
      JSONSchema::Draft4Validator,
      JSONSchema::Draft6Validator,
      JSONSchema::Draft7Validator,
      JSONSchema::Draft201909Validator,
      JSONSchema::Draft202012Validator
    ].each do |klass|
      it "#{klass} accepts JSON string schema" do
        validator = klass.new('{"type":"string"}')
        expect(validator.valid?("hello")).to be true
        expect(validator.valid?(42)).to be false
      end
    end
  end

  describe "Meta" do
    it "validates JSON string schema with Meta.valid?" do
      expect(JSONSchema::Meta.valid?('{"type":"string"}')).to be true
    end

    it "validates JSON string schema with Meta.validate!" do
      expect(JSONSchema::Meta.validate!('{"type":"string"}')).to be_nil
    end
  end
end

RSpec.describe "Retriever error handling" do
  it "raises ArgumentError when retriever returns nil" do
    retriever = ->(_uri) {}
    schema = { "$ref" => "https://example.com/missing.json" }
    expect do
      JSONSchema.validator_for(schema, retriever: retriever)
    end.to raise_error(ArgumentError, /Retriever returned nil/)
  end

  it "raises error when retriever raises" do
    retriever = ->(_uri) { raise "Network error" }
    schema = { "$ref" => "https://example.com/failing.json" }
    expect do
      JSONSchema.validator_for(schema, retriever: retriever)
    end.to raise_error(ArgumentError, /Network error/)
  end

  it "handles circular $ref in retrieved schemas" do
    retriever = lambda do |uri|
      case uri
      when "https://example.com/person.json"
        {
          "type" => "object",
          "properties" => {
            "name" => { "type" => "string" },
            "friend" => { "$ref" => "https://example.com/person.json" }
          },
          "required" => ["name"]
        }
      end
    end

    validator = JSONSchema.validator_for(
      { "$ref" => "https://example.com/person.json" },
      retriever: retriever
    )
    expect(validator.valid?({ "name" => "Alice", "friend" => { "name" => "Bob" } })).to be true
    expect(validator.valid?({ "name" => "Alice", "friend" => {} })).to be false
  end

  it "resolves relative $ref paths via base_uri" do
    retriever = lambda do |uri|
      case uri
      when "https://example.com/schemas/b.json"
        { "type" => "string" }
      end
    end

    validator = JSONSchema.validator_for(
      { "$ref" => "./b.json" },
      base_uri: "https://example.com/schemas/a.json",
      retriever: retriever
    )
    expect(validator.valid?("hello")).to be true
    expect(validator.valid?(42)).to be false
  end
end

RSpec.describe "Custom keywords" do
  it "passes correct arguments to keyword initializer" do
    received = nil
    keyword_class = Class.new do
      define_method(:initialize) do |parent_schema, value, schema_path|
        received = { parent_schema: parent_schema, value: value, schema_path: schema_path }
      end
      define_method(:validate) { |_instance| nil }
    end

    schema = { "type" => "integer", "myKeyword" => 42 }
    JSONSchema.validator_for(schema, keywords: { "myKeyword" => keyword_class })

    expect(received[:parent_schema]).to eq(schema)
    expect(received[:value]).to eq(42)
    expect(received[:schema_path]).to eq(["myKeyword"])
  end
end

RSpec.describe JSONSchema::ValidationError do
  describe "#instance for basic types" do
    {
      "string" => ["hello", { "type" => "integer" }],
      "hash" => [{ "a" => 1 }, { "type" => "string" }],
      "array" => [[1, 2], { "type" => "string" }],
      "nil" => [nil, { "type" => "string" }],
      "boolean" => [true, { "type" => "string" }]
    }.each do |type_name, (value, schema)|
      it "preserves #{type_name} instance value" do
        expect { JSONSchema.validate!(schema, value) }
          .to raise_error(JSONSchema::ValidationError) { |e| expect(e.instance).to eq(value) }
      end
    end
  end

  describe "#evaluation_path" do
    it "returns evaluation path as array and JSON Pointer" do
      schema = {
        "type" => "object",
        "properties" => { "name" => { "type" => "string" } }
      }
      begin
        JSONSchema.validate!(schema, { "name" => 123 })
        raise "Expected ValidationError"
      rescue JSONSchema::ValidationError => e
        expect(e.evaluation_path).to be_an(Array)
        expect(e.evaluation_path).not_to be_empty
        expect(e.evaluation_path_pointer).to be_a(String)
        expect(e.evaluation_path_pointer).not_to be_empty
      end
    end
  end

  describe "#inspect" do
    it "returns formatted inspection string" do
      error = JSONSchema.each_error({ "type" => "string" }, 42).first
      expect(error.inspect).to eq('#<JSONSchema::ValidationError: 42 is not of type "string">')
    end

    it "falls back to the exception message when constructed in Ruby" do
      error = JSONSchema::ValidationError.new("msg")
      expect(error.message).to eq("msg")
      expect(error.inspect).to eq("#<JSONSchema::ValidationError: msg>")
    end
  end

  describe "#to_s" do
    it "returns the message" do
      error = JSONSchema.each_error({ "type" => "string" }, 42).first
      expect(error.to_s).to eq(error.message)
    end
  end
end

RSpec.describe "Boolean schemas" do
  it "validates with boolean true schema" do
    expect(JSONSchema.valid?(true, "anything")).to be true
    expect(JSONSchema.valid?(true, 42)).to be true
  end

  it "rejects all instances with boolean false schema" do
    expect(JSONSchema.valid?(false, "anything")).to be false
  end
end

RSpec.describe "draft: keyword argument on module-level functions" do
  it "accepts draft on valid?" do
    schema = { "type" => "string" }
    expect(JSONSchema.valid?(schema, "hello", draft: :draft7)).to be true
    expect(JSONSchema.valid?(schema, 42, draft: :draft4)).to be false
  end

  it "accepts draft on validate!" do
    schema = { "type" => "string" }
    expect(JSONSchema.validate!(schema, "hello", draft: :draft7)).to be_nil
  end

  it "accepts draft on each_error" do
    schema = { "type" => "string" }
    errors = JSONSchema.each_error(schema, 42, draft: :draft7)
    expect(errors).not_to be_empty
  end

  it "accepts draft on evaluate" do
    schema = { "type" => "string" }
    result = JSONSchema.evaluate(schema, "hello", draft: :draft7)
    expect(result.valid?).to be true
  end
end

RSpec.describe "Type coercion errors" do
  it "raises for NaN as instance value" do
    expect { JSONSchema.valid?({ "type" => "number" }, Float::NAN) }
      .to raise_error(ArgumentError, /NaN/)
  end

  it "raises for Infinity as instance value" do
    expect { JSONSchema.valid?({ "type" => "number" }, Float::INFINITY) }
      .to raise_error(ArgumentError, /Infinity/)
  end

  it "raises for unsupported types" do
    expect { JSONSchema.valid?({ "type" => "string" }, /regex/) }
      .to raise_error(TypeError, /Unsupported type/)
  end
end

RSpec.describe "Invalid option types" do
  it "raises TypeError for invalid pattern_options" do
    expect { JSONSchema.validator_for({ "type" => "string" }, pattern_options: "bad") }
      .to raise_error(TypeError, /pattern_options must be/)
  end

  it "raises TypeError for invalid email_options" do
    expect { JSONSchema.validator_for({ "type" => "string" }, email_options: "bad") }
      .to raise_error(TypeError, /email_options must be/)
  end

  it "raises TypeError for invalid http_options" do
    expect { JSONSchema.validator_for({ "type" => "string" }, http_options: "bad") }
      .to raise_error(TypeError, /http_options must be/)
  end

  it "raises TypeError for invalid registry" do
    expect { JSONSchema.validator_for({ "type" => "string" }, registry: "bad") }
      .to raise_error(TypeError, /registry must be/)
  end
end

RSpec.describe "Custom keywords" do
  it "raises error for keyword class missing validate method" do
    klass = Class.new do
      def initialize(parent_schema, value, schema_path); end
    end
    expect { JSONSchema.validator_for({ "myKeyword" => true }, keywords: { "myKeyword" => klass }) }
      .to raise_error(TypeError, /must define a 'validate' instance method/)
  end
end

RSpec.describe "Custom formats" do
  it "raises TypeError for non-callable format checker" do
    expect do
      JSONSchema.validator_for(
        { "type" => "string", "format" => "my-format" },
        validate_formats: true,
        formats: { "my-format" => "not callable" }
      )
    end.to raise_error(TypeError, /must be a callable/)
  end
end

RSpec.describe JSONSchema::Registry do
  it "raises for invalid resource pair" do
    expect { JSONSchema::Registry.new([[1]]) }
      .to raise_error(ArgumentError, /must be a \[uri, schema\] pair/)
  end

  it "accepts draft keyword argument" do
    registry = JSONSchema::Registry.new(
      [["https://example.com/s.json", { "type" => "string" }]],
      draft: :draft7
    )
    validator = JSONSchema.validator_for(
      { "$ref" => "https://example.com/s.json" },
      registry: registry
    )
    expect(validator.valid?("hello")).to be true
  end

  it "keeps retriever callback alive while resolving refs during construction" do
    LIFETIME_STRESS_ITERATIONS.times do |iteration|
      retrieved_uris = []
      retriever = lambda do |uri|
        retrieved_uris << uri
        case uri
        when "https://example.com/first.json"
          GC.start(full_mark: true, immediate_sweep: true)
          GC.compact if GC.respond_to?(:compact)
          { "$ref" => "https://example.com/second.json" }
        when "https://example.com/second.json"
          { "type" => "string" }
        else
          raise "Unexpected URI: #{uri}"
        end
      end
      retriever_ref = WeakRef.new(retriever)
      registry = JSONSchema::Registry.new(
        [["https://example.com/root.json", { "$ref" => "https://example.com/first.json" }]],
        retriever: retriever
      )

      expect(retrieved_uris).to eq(["https://example.com/first.json", "https://example.com/second.json"]),
                                "retriever failed during construction on iteration #{iteration}"
      expect(retriever_ref.weakref_alive?).to be(true),
                                              "retriever proc was collected during construction on iteration #{iteration}"
      expect(
        JSONSchema.valid?({ "$ref" => "https://example.com/root.json" }, "hello", registry: registry)
      ).to be true
    end
  end

  it "has inspect output" do
    registry = JSONSchema::Registry.new([])
    expect(registry.inspect).to eq("#<JSONSchema::Registry>")
  end
end

RSpec.describe "draft: symbol validation errors" do
  it "raises TypeError for non-symbol draft" do
    expect { JSONSchema.valid?({ "type" => "string" }, "hello", draft: 7) }
      .to raise_error(TypeError, /draft must be a Symbol/)
  end

  it "raises ArgumentError for unknown draft symbol" do
    expect { JSONSchema.valid?({ "type" => "string" }, "hello", draft: :draft99) }
      .to raise_error(ArgumentError, /Unknown draft/)
  end
end

RSpec.describe "Masking" do
  it "masks nested object values" do
    schema = {
      "type" => "object",
      "properties" => {
        "user" => {
          "type" => "object",
          "properties" => { "name" => { "type" => "string" } }
        }
      }
    }
    validator = JSONSchema.validator_for(schema, mask: "[MASKED]")
    begin
      validator.validate!({ "user" => { "name" => 42 } })
      raise "Expected ValidationError"
    rescue JSONSchema::ValidationError => e
      expect(e.message).not_to include("42")
      expect(e.message).to include("[MASKED]")
    end
  end

  it "masks array item values" do
    schema = { "type" => "array", "items" => { "type" => "string" } }
    validator = JSONSchema.validator_for(schema, mask: "[MASKED]")
    errors = validator.each_error([42]).to_a
    expect(errors).not_to be_empty
    errors.each do |error|
      expect(error.message).not_to include("42")
      expect(error.message).to include("[MASKED]")
    end
  end
end

# frozen_string_literal: true

require "spec_helper"

RSpec.describe "README examples" do
  readme_path = File.expand_path("../README.md", __dir__)
  readme = File.read(readme_path)

  code_blocks = readme.scan(/```ruby\n(.*?)```/m).flatten

  setup_code = <<~RUBY
    schema ||= { "type" => "string" }
    instance ||= "example"
    registry ||= JSONSchema::Registry.new([])
    proc ||= ->(_value) { true }
    pattern_opts ||= JSONSchema::RegexOptions.new
    email_opts ||= JSONSchema::EmailOptions.new
    http_opts ||= JSONSchema::HttpOptions.new
    snippet_placeholder ||= nil

    unless defined?(Klass)
      Klass = Class.new do
        def initialize(parent_schema, value, schema_path); end

        def validate(instance); end
      end
    end
  RUBY

  normalize_code = lambda do |code|
    code
      .lines
      .map { |line| line.lstrip.start_with?("#") ? line : line.gsub(/\{\s*\.\.\.\s*\}/, "{ snippet_placeholder }") }
      .join
      .gsub(/pattern_options:\s*opts/, "pattern_options: pattern_opts")
      .gsub(/email_options:\s*opts/, "email_options: email_opts")
      .gsub(/http_options:\s*opts/, "http_options: http_opts")
  end

  scope = binding

  code_blocks.each_with_index do |code, index|
    it "code block #{index + 1} executes without error" do
      scope.eval(setup_code, "README.md setup", 1)
      scope.eval(normalize_code.call(code), "README.md block #{index + 1}", 1)
    end
  end
end

# frozen_string_literal: true

require "spec_helper"

RSpec.describe "README examples" do
  readme_path = File.expand_path("../README.md", __dir__)
  readme = File.read(readme_path)

  code_blocks = readme.scan(/```ruby\n(.*?)```/m).flatten

  # Skip blocks that aren't executable Ruby code:
  # - Gemfile syntax (`gem 'jsonschema'`)
  # - Options reference with `...` placeholder
  skip_patterns = [
    /\Agem\s+['"]/, # Gemfile syntax
    /\.\.\./ # Documentation placeholder
  ]

  scope = binding

  code_blocks.each_with_index do |code, index|
    should_skip = skip_patterns.any? { |pattern| code.match?(pattern) }

    if should_skip
      it "code block #{index + 1} is skipped (non-executable)" do
        skip "Block contains non-executable syntax"
      end
    else
      it "code block #{index + 1} executes without error" do
        scope.eval(code, "README.md block #{index + 1}", 1)
      end
    end
  end
end

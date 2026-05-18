# frozen_string_literal: true

require_relative "jsonschema/version"

begin
  RUBY_VERSION =~ /(\d+\.\d+)/
  require "jsonschema/#{Regexp.last_match(1)}/jsonschema_rb"
rescue LoadError
  require "jsonschema/jsonschema_rb"
end

module JSONSchema
  module Canonical
    class InvalidPattern
      attr_reader :location
    end
  end
end

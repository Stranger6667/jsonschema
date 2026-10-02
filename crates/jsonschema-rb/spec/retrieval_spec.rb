# frozen_string_literal: true

require "socket"
require "spec_helper"

# The server answers from a thread of this process, so the build must not hold the GVL while it waits
RSpec.describe "retrieval from a server in the same process" do
  let(:server) { TCPServer.new("127.0.0.1", 0) }
  let(:schema) { { "$ref" => "http://127.0.0.1:#{server.addr[1]}/item" } }

  around do |example|
    thread = Thread.new do
      loop do
        client = server.accept
        while (line = client.gets) && line != "\r\n"; end
        body = '{"type": "integer"}'
        client.write(
          "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n" \
          "Content-Length: #{body.bytesize}\r\nConnection: close\r\n\r\n#{body}"
        )
        client.close
      end
    end
    example.run
  ensure
    thread.kill
    server.close
  end

  {
    "default" => {},
    "http_options" => { http_options: JSONSchema::HttpOptions.new(timeout: 30.0) }
  }.each do |options_name, options|
    {
      "validator_for" => ->(schema, opts) { JSONSchema.validator_for(schema, **opts).valid?("a") },
      "validator" => ->(schema, opts) { JSONSchema::Draft202012Validator.new(schema, **opts).valid?("a") },
      "valid?" => ->(schema, opts) { JSONSchema.valid?(schema, "a", **opts) }
    }.each do |entry_point, check|
      it "resolves the reference with #{entry_point} (#{options_name})" do
        started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
        expect(check.call(schema, options)).to be false
        expect(Process.clock_gettime(Process::CLOCK_MONOTONIC) - started).to be < 5
      end
    end
  end
end

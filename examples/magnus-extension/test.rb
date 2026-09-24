# frozen_string_literal: true

require_relative "lib/jsonschema_example_magnus"

raise "valid event rejected" unless Events.valid?({ "id" => "evt-1", "kind" => "created", "code" => "AB" })
raise "invalid event accepted" if Events.valid?({ "kind" => "created" })
raise "symbol keys rejected" unless Events.valid?({ id: "evt-1", kind: "created" })

errors = Events.errors({ "kind" => "created" })
raise "unexpected errors: #{errors}" unless errors == ['"id" is a required property']

errors = Events.errors({ "id" => "evt-1", "kind" => "created", "code" => "ab" })
raise "unexpected errors: #{errors}" unless errors == ["ab is not uppercase"]

begin
  Events.validate!({ "id" => "evt-1", "kind" => "deleted", "extra" => 1 })
  raise "validate! accepted an invalid event"
rescue Events::Error => e
  raise "unexpected message: #{e.message}" unless e.message == "Additional properties are not allowed ('extra' was unexpected)"
end

begin
  Events.valid?({ "id" => Object.new, "kind" => "created" })
  raise "a value with no JSON form was read silently"
rescue TypeError => e
  raise "unexpected message: #{e.message}" unless e.message == "Unsupported type: 'Object'"
end

puts "ok"

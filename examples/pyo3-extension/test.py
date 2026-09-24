import jsonschema_example_pyo3 as events

assert events.is_valid({"id": "evt-1", "kind": "created", "code": "AB"})
assert not events.is_valid({"kind": "created"})
assert events.errors({"kind": "created"}) == ['"id" is a required property']
assert events.errors({"id": "evt-1", "kind": "created", "code": "ab"}) == [
    "ab is not uppercase"
]

try:
    events.validate({"id": "evt-1", "kind": "deleted", "extra": 1})
except ValueError as error:
    assert (
        str(error) == "Additional properties are not allowed ('extra' was unexpected)"
    )
else:
    raise AssertionError("validate accepted an invalid event")

try:
    events.is_valid({"id": object(), "kind": "created"})
except ValueError as error:
    assert str(error) == "Unsupported type: 'object'"
else:
    raise AssertionError("a value with no JSON form was read silently")

print("ok")

import pytest
import jsonschema_rs

SCHEMA = {
    "$defs": {
        "User": {
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"],
        },
        "Address": {
            "type": "object",
            "properties": {"city": {"type": "string"}},
        },
    },
    "type": "object",
}


def test_validator_map_for_accessible():
    m = jsonschema_rs.validator_map_for(SCHEMA)
    assert isinstance(m, jsonschema_rs.ValidatorMap)


def test_get_existing_pointer():
    m = jsonschema_rs.validator_map_for(SCHEMA)
    v = m.get("#/$defs/User")
    assert v is not None
    assert v.is_valid({"name": "Alice"})
    assert not v.is_valid(42)


def test_get_missing_pointer_returns_none():
    m = jsonschema_rs.validator_map_for(SCHEMA)
    assert m.get("#/nonexistent") is None


def test_getitem_existing():
    m = jsonschema_rs.validator_map_for(SCHEMA)
    v = m["#/$defs/User"]
    assert v.is_valid({"name": "Alice"})


def test_getitem_missing_raises_key_error():
    m = jsonschema_rs.validator_map_for(SCHEMA)
    with pytest.raises(KeyError):
        _ = m["#/nonexistent"]


def test_contains_true():
    m = jsonschema_rs.validator_map_for(SCHEMA)
    assert "#/$defs/User" in m


def test_contains_false():
    m = jsonschema_rs.validator_map_for(SCHEMA)
    assert "#/nonexistent" not in m


def test_keys_includes_defs_and_root():
    m = jsonschema_rs.validator_map_for(SCHEMA)
    keys = m.keys()
    assert "#" in keys
    assert "#/$defs/User" in keys
    assert "#/$defs/Address" in keys


def test_len():
    m = jsonschema_rs.validator_map_for(SCHEMA)
    assert len(m) >= 3  # root + User + Address


def test_root_entry_is_valid_validator():
    m = jsonschema_rs.validator_map_for(SCHEMA)
    v = m.get("#")
    assert v is not None
    assert v.is_valid({})


def test_returned_validator_iter_errors():
    m = jsonschema_rs.validator_map_for(SCHEMA)
    v = m["#/$defs/User"]
    errors = list(v.iter_errors(42))
    assert len(errors) > 0


def test_returned_validator_validate_raises():
    m = jsonschema_rs.validator_map_for(SCHEMA)
    v = m["#/$defs/User"]
    with pytest.raises(jsonschema_rs.ValidationError):
        v.validate(42)


def test_mask_propagated_to_retrieved_validator():
    schema = {
        "$defs": {
            "User": {
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"],
            }
        },
        "type": "object",
    }
    m = jsonschema_rs.validator_map_for(schema, mask="***")
    v = m["#/$defs/User"]
    with pytest.raises(jsonschema_rs.ValidationError) as exc_info:
        v.validate(42)
    assert exc_info.value.message == '*** is not of type "object"'


def test_mask_propagated_via_get():
    schema = {
        "$defs": {
            "User": {
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"],
            }
        },
        "type": "object",
    }
    m = jsonschema_rs.validator_map_for(schema, mask="***")
    v = m.get("#/$defs/User")
    assert v is not None
    with pytest.raises(jsonschema_rs.ValidationError) as exc_info:
        v.validate(42)
    assert exc_info.value.message == '*** is not of type "object"'

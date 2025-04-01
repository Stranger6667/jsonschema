import pytest

import jsonschema_rs


def test_custom_masking():
    schema = {"maxLength": 5}
    validator = jsonschema_rs.validator_for(schema, mask="[REDACTED]")

    with pytest.raises(jsonschema_rs.ValidationError) as exc:
        validator.validate("sensitive data")

    assert str(exc.value).startswith("[REDACTED] is longer than 5 characters")
    assert "sensitive data" not in str(exc.value)


def test_no_masking():
    schema = {"maxLength": 5}
    validator = jsonschema_rs.validator_for(schema)

    with pytest.raises(jsonschema_rs.ValidationError) as exc:
        validator.validate("sensitive data")

    assert '"sensitive data" is longer than 5 characters' in str(exc.value)


def test_masking_with_nested_data():
    schema = {
        "type": "object",
        "properties": {
            "credentials": {
                "type": "object",
                "properties": {
                    "password": {"type": "string", "minLength": 8},
                },
            }
        },
    }
    validator = jsonschema_rs.validator_for(schema, mask="[SECRET]")

    with pytest.raises(jsonschema_rs.ValidationError) as exc:
        validator.validate({"credentials": {"password": "123"}})

    assert "[SECRET] is shorter than 8 characters" in str(exc.value)
    assert "123" not in str(exc.value)


def test_masking_with_array():
    schema = {"items": {"type": "string"}}
    validator = jsonschema_rs.validator_for(schema, mask="[HIDDEN]")

    with pytest.raises(jsonschema_rs.ValidationError) as exc:
        validator.validate([123, 456])

    assert '[HIDDEN] is not of type "string"' in str(exc.value)


def test_masking_with_multiple_errors():
    schema = {
        "type": "object",
        "properties": {
            "password": {"type": "string", "minLength": 8},
            "api_key": {"type": "string", "pattern": "^[A-Z0-9]{32}$"},
        },
    }
    validator = jsonschema_rs.validator_for(schema, mask="[HIDDEN]")

    errors = list(validator.iter_errors({"password": "123", "api_key": "invalid"}))

    assert len(errors) == 2
    assert all("[HIDDEN]" in str(error) for error in errors)
    assert all("123" not in str(error) and "invalid" not in str(error) for error in errors)

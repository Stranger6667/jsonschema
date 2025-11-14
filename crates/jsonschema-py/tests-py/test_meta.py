import subprocess
import sys

import pytest

from jsonschema_rs import ReferencingError, ValidationError, meta


@pytest.mark.parametrize(
    "schema",
    [
        {"type": "string"},
        {"type": "number", "minimum": 0},
        {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]},
        # Boolean schemas are valid
        True,
        False,
    ],
)
def test_valid_schemas(schema):
    assert meta.is_valid(schema)
    meta.validate(schema)  # Should not raise


@pytest.mark.parametrize(
    ["schema", "expected"],
    [
        ({"type": "invalid_type"}, "is not valid"),
        ({"type": "number", "minimum": "0"}, 'is not of type "number"'),
        ({"type": "object", "required": "name"}, 'is not of type "array"'),
    ],
)
def test_invalid_schemas(schema, expected):
    assert not meta.is_valid(schema)
    with pytest.raises(ValidationError, match=expected):
        meta.validate(schema)


def test_referencing_error():
    schema = {"$schema": "invalid-uri", "type": "string"}
    with pytest.raises(ReferencingError, match="Unknown specification: invalid-uri"):
        meta.validate(schema)
    with pytest.raises(ReferencingError, match="Unknown specification: invalid-uri"):
        meta.is_valid(schema)


def test_validation_error_details():
    schema = {"type": "invalid_type"}

    with pytest.raises(ValidationError) as exc_info:
        meta.validate(schema)

    error = exc_info.value
    assert hasattr(error, "message")
    assert hasattr(error, "instance_path")
    assert hasattr(error, "schema_path")
    assert "invalid_type" in str(error)


@pytest.mark.parametrize(
    "invalid_input",
    [
        None,
        lambda: None,
        object(),
        {1, 2, 3},
    ],
)
def test_type_errors(invalid_input):
    with pytest.raises((ValueError, ValidationError)):
        meta.validate(invalid_input)


def test_exceptions_remain_consistent_after_reload():
    # Module reloading affects global state and can contaminate other tests that import
    # exception classes at module level. Run in subprocess for complete isolation.
    result = subprocess.run(
        [
            sys.executable,
            "-c",
            """
import importlib
import jsonschema_rs

schema = {"type": "integer"}

# Test before reload
try:
    jsonschema_rs.validate(schema, "not-int")
    raise AssertionError("Expected ValidationError")
except jsonschema_rs.ValidationError:
    pass

# Reload the module
reloaded = importlib.reload(jsonschema_rs)
assert reloaded is jsonschema_rs

# Test after reload - exceptions should still work
try:
    jsonschema_rs.validate(schema, "still-not-int")
    raise AssertionError("Expected ValidationError")
except jsonschema_rs.ValidationError:
    pass

try:
    jsonschema_rs.meta.validate({"$schema": "invalid", "type": "string"})
    raise AssertionError("Expected ReferencingError")
except jsonschema_rs.ReferencingError:
    pass
""",
        ],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, f"Test failed with stdout:\n{result.stdout}\nstderr:\n{result.stderr}"

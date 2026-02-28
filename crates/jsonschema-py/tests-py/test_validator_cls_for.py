import pytest

import jsonschema_rs
from jsonschema_rs import (
    Draft4Validator,
    Draft6Validator,
    Draft7Validator,
    Draft201909Validator,
    Draft202012Validator,
    validator_cls_for,
)


@pytest.mark.parametrize(
    ["schema", "expected_cls"],
    [
        # No $schema defaults to Draft202012
        ({"type": "string"}, Draft202012Validator),
        ({}, Draft202012Validator),
        # Boolean schemas (no $schema) → Draft202012
        (True, Draft202012Validator),
        (False, Draft202012Validator),
        # Draft 4 - both HTTP and HTTPS, with and without trailing #
        ({"$schema": "http://json-schema.org/draft-04/schema#"}, Draft4Validator),
        ({"$schema": "https://json-schema.org/draft-04/schema#"}, Draft4Validator),
        ({"$schema": "http://json-schema.org/draft-04/schema"}, Draft4Validator),
        # Draft 6
        ({"$schema": "http://json-schema.org/draft-06/schema#"}, Draft6Validator),
        ({"$schema": "https://json-schema.org/draft-06/schema#"}, Draft6Validator),
        # Draft 7
        ({"$schema": "http://json-schema.org/draft-07/schema#"}, Draft7Validator),
        ({"$schema": "https://json-schema.org/draft-07/schema#"}, Draft7Validator),
        # Draft 2019-09
        ({"$schema": "https://json-schema.org/draft/2019-09/schema"}, Draft201909Validator),
        ({"$schema": "http://json-schema.org/draft/2019-09/schema"}, Draft201909Validator),
        # Draft 2020-12
        ({"$schema": "https://json-schema.org/draft/2020-12/schema"}, Draft202012Validator),
        ({"$schema": "http://json-schema.org/draft/2020-12/schema"}, Draft202012Validator),
        # Unknown $schema falls back to Draft202012
        ({"$schema": "http://custom.example.com/schema"}, Draft202012Validator),
    ],
)
def test_draft_detection(schema, expected_cls):
    assert validator_cls_for(schema) is expected_cls


def test_returned_class_creates_working_validator():
    cls = validator_cls_for({"type": "string"})
    validator = cls({"type": "string"})
    assert validator.is_valid("hello")
    assert not validator.is_valid(42)


def test_no_meta_validation():
    # validator_cls_for performs no meta-validation — only draft detection.
    # Invalid schemas do not raise here; they raise only when cls(schema) is called.
    schema = {"$schema": "http://json-schema.org/draft-07/schema#", "type": "invalid_type"}
    cls = validator_cls_for(schema)
    assert cls is Draft7Validator
    with pytest.raises(jsonschema_rs.ValidationError):
        cls(schema)

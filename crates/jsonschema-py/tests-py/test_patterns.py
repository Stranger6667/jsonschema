import pytest

import jsonschema_rs
from jsonschema_rs import FancyRegexOptions, RegexOptions


@pytest.mark.xfail(reason="fancy-regex 0.16 no longer fails for this test case")
def test_fancy_regex_backtrack_limit_exceeded():
    schema = {"pattern": "(?i)(a|b|ab)*(?=c)"}

    validator = jsonschema_rs.validator_for(schema, pattern_options=FancyRegexOptions(backtrack_limit=1))

    instance = "abababababababababababababababababababababababababababab"

    with pytest.raises(jsonschema_rs.ValidationError) as excinfo:
        validator.validate(instance)

    assert "Max limit for backtracking count exceeded" in str(excinfo.value)


def test_regex_engine_validation():
    schema = {"pattern": "^[a-z]+$"}

    validator = jsonschema_rs.validator_for(schema, pattern_options=RegexOptions())

    assert validator.is_valid("hello")

    assert not validator.is_valid("Hello123")

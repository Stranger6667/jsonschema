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


# Recovery for https://github.com/rust-lang/regex/issues/1344.
@pytest.mark.parametrize(
    "options",
    [
        FancyRegexOptions(size_limit=1_000_000_000),
        RegexOptions(size_limit=1_000_000_000),
    ],
    ids=["fancy_regex", "regex"],
)
def test_pattern_panic_surfaces_as_validation_error(options):
    schema = {"type": "string", "pattern": r"^.{0,404600}$"}
    validator = jsonschema_rs.validator_for(schema, pattern_options=options)

    assert validator.is_valid("x")
    assert not validator.is_valid("")
    with pytest.raises(jsonschema_rs.ValidationError) as excinfo:
        validator.validate("")
    assert "Regex engine failed to evaluate pattern '^.{0,404600}$'" in str(excinfo.value)
    assert excinfo.value.kind.name == "pattern"


# The engine cannot finish the first alternative within the default backtrack limit; the second,
# `a+!$`, matches
UNFINISHED_PATTERN = r"^(?:((a|aa)(?=a?))+$|a+!$)"
UNFINISHED_SUBJECT = "a" * 64 + "!"
UNDECIDED = "Error executing regex: Max limit for backtracking count exceeded"


@pytest.mark.parametrize(
    ("schema", "expected"),
    [
        ({"not": {"pattern": UNFINISHED_PATTERN}}, (False, [UNDECIDED])),
        ({"anyOf": [{"pattern": UNFINISHED_PATTERN}, {"type": "string"}]}, (True, [])),
    ],
    ids=["undecided", "decided"],
)
def test_unfinished_regex_match(backend, schema, expected):
    assert (
        backend.is_valid(schema, UNFINISHED_SUBJECT),
        [error.message for error in backend.iter_errors(schema, UNFINISHED_SUBJECT)],
    ) == expected

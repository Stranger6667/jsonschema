import pytest

import jsonschema_rs


class EvenValidator:
    def __init__(self, parent_schema, value, schema_path):
        self.enabled = value

    def validate(self, instance):
        if not self.enabled:
            return
        if isinstance(instance, int) and instance % 2 != 0:
            raise ValueError(f"{instance} is not even")


class OddValidator:
    def __init__(self, parent_schema, value, schema_path):
        pass

    def validate(self, instance):
        if isinstance(instance, int) and instance % 2 == 0:
            raise ValueError(f"{instance} is not odd")


class RangeValidator:
    def __init__(self, parent_schema, value, schema_path):
        self.min = value.get("min", float("-inf"))
        self.max = value.get("max", float("inf"))

    def validate(self, instance):
        if isinstance(instance, (int, float)):
            if not (self.min <= instance <= self.max):
                raise ValueError(f"Value {instance} not in range [{self.min}, {self.max}]")


class PositiveValidator:
    def __init__(self, parent_schema, value, schema_path):
        pass

    def validate(self, instance):
        if isinstance(instance, (int, float)) and instance <= 0:
            raise ValueError(f"{instance} is not positive")


@pytest.mark.parametrize(
    "instance, expected",
    [
        (2, True),
        (4, True),
        (0, True),
        (1, False),
        (3, False),
        ("not a number", True),
    ],
)
def test_is_valid(instance, expected):
    validator = jsonschema_rs.validator_for(
        {"even": True},
        keywords={"even": EvenValidator},
    )
    assert validator.is_valid(instance) == expected


@pytest.mark.parametrize(
    "instance, should_raise",
    [
        (2, False),
        (1, True),
        (3, True),
    ],
)
def test_validate(instance, should_raise):
    validator = jsonschema_rs.validator_for(
        {"even": True},
        keywords={"even": EvenValidator},
    )
    if should_raise:
        with pytest.raises(jsonschema_rs.ValidationError):
            validator.validate(instance)
    else:
        validator.validate(instance)


def test_error_message():
    validator = jsonschema_rs.validator_for(
        {"even": True},
        keywords={"even": EvenValidator},
    )
    with pytest.raises(jsonschema_rs.ValidationError) as exc_info:
        validator.validate(3)
    assert "3 is not even" in str(exc_info.value)


def test_iter_errors():
    validator = jsonschema_rs.validator_for(
        {"even": True},
        keywords={"even": EvenValidator},
    )
    errors = list(validator.iter_errors(3))
    assert len(errors) == 1
    assert isinstance(errors[0], jsonschema_rs.ValidationError)


def test_with_standard_keywords():
    validator = jsonschema_rs.validator_for(
        {"type": "integer", "minimum": 0, "even": True},
        keywords={"even": EvenValidator},
    )
    assert validator.is_valid(2)
    assert validator.is_valid(100)
    assert not validator.is_valid(3)
    assert not validator.is_valid(-2)
    assert not validator.is_valid("hello")


def test_multiple_custom_keywords():
    validator = jsonschema_rs.validator_for(
        {"even": True, "isPositive": True},
        keywords={
            "even": EvenValidator,
            "isPositive": PositiveValidator,
        },
    )
    assert validator.is_valid(2)
    assert validator.is_valid(100)
    assert not validator.is_valid(3)
    assert not validator.is_valid(-2)
    assert not validator.is_valid(-3)


def test_nested_schema():
    validator = jsonschema_rs.validator_for(
        {
            "type": "object",
            "properties": {
                "count": {"type": "integer", "even": True},
            },
        },
        keywords={"even": EvenValidator},
    )
    assert validator.is_valid({"count": 2})
    assert validator.is_valid({"count": 100})
    assert not validator.is_valid({"count": 3})


@pytest.mark.parametrize(
    "instance, expected",
    [
        (1, True),
        (3, True),
        (5, True),
        (2, False),
        (4, False),
        ("not a number", True),
    ],
)
def test_odd_validator(instance, expected):
    validator = jsonschema_rs.validator_for(
        {"odd": True},
        keywords={"odd": OddValidator},
    )
    assert validator.is_valid(instance) == expected


@pytest.mark.parametrize(
    "instance, expected",
    [
        (5, True),
        (10, True),
        (0, True),
        (-1, False),
        (11, False),
        (100, False),
    ],
)
def test_range_validator_is_valid(instance, expected):
    validator = jsonschema_rs.validator_for(
        {"customRange": {"min": 0, "max": 10}},
        keywords={"customRange": RangeValidator},
    )
    assert validator.is_valid(instance) == expected


@pytest.mark.parametrize(
    "instance, should_raise",
    [
        (5, False),
        (-1, True),
        (20, True),
    ],
)
def test_range_validator_validate(instance, should_raise):
    validator = jsonschema_rs.validator_for(
        {"customRange": {"min": 0, "max": 10}},
        keywords={"customRange": RangeValidator},
    )
    if should_raise:
        with pytest.raises(jsonschema_rs.ValidationError):
            validator.validate(instance)
    else:
        validator.validate(instance)


def test_standalone_is_valid():
    assert jsonschema_rs.is_valid(
        {"even": True},
        2,
        keywords={"even": EvenValidator},
    )
    assert not jsonschema_rs.is_valid(
        {"even": True},
        3,
        keywords={"even": EvenValidator},
    )


def test_standalone_validate():
    jsonschema_rs.validate(
        {"even": True},
        2,
        keywords={"even": EvenValidator},
    )
    with pytest.raises(jsonschema_rs.ValidationError):
        jsonschema_rs.validate(
            {"even": True},
            3,
            keywords={"even": EvenValidator},
        )


def test_standalone_iter_errors():
    errors = list(
        jsonschema_rs.iter_errors(
            {"even": True},
            3,
            keywords={"even": EvenValidator},
        )
    )
    assert len(errors) == 1
    assert isinstance(errors[0], jsonschema_rs.ValidationError)


def test_standalone_evaluate():
    evaluation = jsonschema_rs.evaluate(
        {"even": True},
        2,
        keywords={"even": EvenValidator},
    )
    result = evaluation.flag()
    assert result["valid"] is True


def test_draft_validators():
    for validator_class in [
        jsonschema_rs.Draft4Validator,
        jsonschema_rs.Draft6Validator,
        jsonschema_rs.Draft7Validator,
        jsonschema_rs.Draft201909Validator,
        jsonschema_rs.Draft202012Validator,
    ]:
        validator = validator_class(
            {"even": True},
            keywords={"even": EvenValidator},
        )
        assert validator.is_valid(2)
        assert not validator.is_valid(3)


def test_receives_correct_arguments():
    received_args = {}

    class CaptureArgs:
        def __init__(self, parent_schema, value, schema_path):
            received_args["parent_schema"] = parent_schema
            received_args["value"] = value
            received_args["schema_path"] = list(schema_path)

        def validate(self, instance):
            pass

    jsonschema_rs.validator_for(
        {"type": "string", "myKeyword": {"foo": "bar"}},
        keywords={"myKeyword": CaptureArgs},
    )

    assert "type" in received_args["parent_schema"]
    assert received_args["parent_schema"]["type"] == "string"
    assert received_args["value"] == {"foo": "bar"}


def test_non_callable_raises_error():
    with pytest.raises(ValueError) as exc_info:
        jsonschema_rs.validator_for(
            {"myKeyword": True},
            keywords={"myKeyword": "not a callable"},
        )
    assert "callable" in str(exc_info.value).lower()


def test_init_error_propagates():
    class FailingInit:
        def __init__(self, parent_schema, value, schema_path):
            raise RuntimeError("Init failed!")

        def validate(self, instance):
            pass

    with pytest.raises(jsonschema_rs.ValidationError) as exc_info:
        jsonschema_rs.validator_for(
            {"failing": True},
            keywords={"failing": FailingInit},
        )

    assert "Init failed" in str(exc_info.value)


def test_keyword_disabled_by_value():
    validator = jsonschema_rs.validator_for(
        {"even": False},
        keywords={"even": EvenValidator},
    )
    assert validator.is_valid(1)
    assert validator.is_valid(2)
    assert validator.is_valid(3)

    validator_enabled = jsonschema_rs.validator_for(
        {"even": True},
        keywords={"even": EvenValidator},
    )
    assert validator_enabled.is_valid(2)
    assert not validator_enabled.is_valid(3)

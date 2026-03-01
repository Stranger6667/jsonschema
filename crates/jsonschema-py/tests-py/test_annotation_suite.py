import json
import os
import pytest
import jsonschema_rs


# Keywords whose annotation is the schema value itself.
_SCHEMA_VALUED_KEYWORDS = frozenset({"contentSchema"})

SUITE_PATH = os.path.join(
    os.path.dirname(__file__),
    "../../jsonschema/tests/suite/annotations/tests",
)

def load_test_cases():
    cases = []
    for filename in sorted(os.listdir(SUITE_PATH)):
        if not filename.endswith(".json"):
            continue
        filepath = os.path.join(SUITE_PATH, filename)
        with open(filepath, encoding="utf-8") as f:
            data = json.load(f)
        for suite_case in data.get("suite", []):
            schema = suite_case["schema"]
            description = suite_case.get("description", filename)
            tests = suite_case.get("tests", [])
            for test in tests:
                instance = test["instance"]
                assertions = test.get("assertions", [])
                cases.append(pytest.param(schema, instance, assertions, id=f"{filename}::{description}"))
    return cases


def get_annotations(schema, instance):
    validator = jsonschema_rs.validator_for(schema)
    evaluation = validator.evaluate(instance)
    raw = evaluation.annotations()

    result = {}
    for ann in raw:
        instance_loc = ann.get("instanceLocation", "")
        annotations = ann.get("annotations", {})

        schema_loc = ann.get("schemaLocation", "")
        segments = [s for s in schema_loc.split("/") if s]
        keyword_from_loc = segments[-1] if segments else None

        if keyword_from_loc in _SCHEMA_VALUED_KEYWORDS:
            # Keep schema-valued annotations as a single value.
            key = (instance_loc, keyword_from_loc)
            result.setdefault(key, []).append(annotations)
        elif isinstance(annotations, dict):
            # Hash bundles are {keyword: value}.
            for keyword, value in annotations.items():
                key = (instance_loc, keyword)
                result.setdefault(key, []).append(value)
        else:
            if keyword_from_loc:
                key = (instance_loc, keyword_from_loc)
                result.setdefault(key, []).append(annotations)
    return result


@pytest.mark.parametrize("schema,instance,assertions", load_test_cases())
def test_annotation(schema, instance, assertions):

    collected = get_annotations(schema, instance)

    for assertion in assertions:
        location = assertion["location"]
        keyword = assertion["keyword"]
        expected = assertion["expected"]

        key = (location, keyword)
        actual_values = collected.get(key, [])

        if not expected:
            assert not actual_values, (
                f"\nKeyword   : {keyword!r}"
                f"\nInstance  : {location!r}"
                f"\nExpected  : no annotation"
                f"\nGot       : {actual_values!r}"
            )
        else:
            for schema_loc, expected_value in expected.items():
                assert expected_value in actual_values, (
                    f"\nKeyword   : {keyword!r}"
                    f"\nInstance  : {location!r}"
                    f"\nSchema    : {schema_loc!r}"
                    f"\nExpected  : {expected_value!r}"
                    f"\nGot       : {actual_values!r}"
                )

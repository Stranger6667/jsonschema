import json
import os
import pytest
import jsonschema_rs


SUITE_PATH = os.path.join(
    os.path.dirname(__file__),
    "../../jsonschema/tests/suite/annotations/tests",
)

XFAIL_IDS = {
    "applicators.json::`anyOf`0",
    "format.json::`format` is an annotation",
    "unevaluated.json::`unevaluatedProperties` alone",
    "unevaluated.json::`unevaluatedProperties` with `properties`",
    "unevaluated.json::`unevaluatedProperties` with `patternProperties`",
    "unevaluated.json::`unevaluatedProperties` with `dependentSchemas`",
    "unevaluated.json::`unevaluatedProperties` with `if`, `then`, and `else`0",
    "unevaluated.json::`unevaluatedProperties` with `if`, `then`, and `else`1",
    "unevaluated.json::`unevaluatedProperties` with `allOf`",
    "unevaluated.json::`unevaluatedProperties` with `anyOf`",
    "unevaluated.json::`unevaluatedProperties` with `oneOf`",
    "unevaluated.json::`unevaluatedProperties` with `not`",
    "unevaluated.json::`unevaluatedItems` alone",
    "unevaluated.json::`unevaluatedItems` with `prefixItems`",
    "unevaluated.json::`unevaluatedItems` with `contains`",
    "unevaluated.json::`unevaluatedItems` with `if`, `then`, and `else`0",
    "unevaluated.json::`unevaluatedItems` with `if`, `then`, and `else`1",
    "unevaluated.json::`unevaluatedItems` with `allOf`",
    "unevaluated.json::`unevaluatedItems` with `anyOf`",
    "unevaluated.json::`unevaluatedItems` with `oneOf`",
    "unevaluated.json::`unevaluatedItems` with `not`",
}

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
            for idx, test in enumerate(tests):
                instance = test["instance"]
                assertions = test.get("assertions", [])
                suffix = str(idx) if len(tests) > 1 else ""
                test_id = f"{filename}::{description}{suffix}"
                marks = (
                    [pytest.mark.xfail(reason="Missing annotation support in the library, to be fixed upstream")]
                    if test_id in XFAIL_IDS
                    else []
                )
                cases.append(
                    pytest.param(schema, instance, assertions, id=f"{filename}::{description}", marks=marks)
                )
    return cases


def get_annotations(schema, instance):
    validator = jsonschema_rs.validator_for(schema)
    evaluation = validator.evaluate(instance)
    raw = evaluation.annotations()

    result = {}
    for ann in raw:
        instance_loc = ann.get("instanceLocation", "")
        annotations_dict = ann.get("annotations", {})

        if isinstance(annotations_dict, dict):
            for keyword, value in annotations_dict.items():
                key = (instance_loc, keyword)
                result.setdefault(key, []).append(value)
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

        for schema_loc, expected_value in expected.items():
            assert expected_value in actual_values, (
                f"\nKeyword   : {keyword!r}"
                f"\nInstance  : {location!r}"
                f"\nSchema    : {schema_loc!r}"
                f"\nExpected  : {expected_value!r}"
                f"\nGot       : {actual_values!r}"
            )

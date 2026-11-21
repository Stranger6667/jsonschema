import json
from decimal import Decimal
from pathlib import Path

import jsonschema_testsuite_pyo3
import pytest

import jsonschema_rs

TEST_SUITE_PATH = Path(__file__).parent.parent.parent / "jsonschema/tests/suite/validation/tests"

# Draft name -> the number the suite's `compatibility` property uses for it.
SUPPORTED_DRAFTS = {"4": 4, "6": 6, "7": 7, "2019-09": 2019, "2020-12": 2020}


def is_compatible(compatibility, release):
    if compatibility is None:
        return True
    compatible = True
    for constraint in compatibility.split(","):
        version = int(constraint.lstrip("<="))
        if constraint.startswith("<="):
            compatible = release <= version
        elif constraint.startswith("="):
            compatible = release == version
        else:
            compatible = release >= version
        if release <= version:
            break
    return compatible


def validate_formats(block):
    tags = block.get("tags", ())
    if "format-assertion" in tags:
        return True
    if "format-annotation" in tags:
        return False
    return None


def make_retriever(external_schemas):
    def _retriever(uri: str):
        try:
            return external_schemas[uri]
        except KeyError as exc:
            raise ValueError(f"Unknown remote schema: {uri}") from exc

    return _retriever


# A lone UTF-16 surrogate has no UTF-8 representation and cannot cross into Rust.
def unencodable_string(value):
    if not isinstance(value, str):
        return False
    try:
        value.encode("utf-8")
    except UnicodeEncodeError:
        return True
    return False


def maybe_skipped(draft, block, test, filename, case_id):
    output = (
        filename,
        draft,
        block["schema"],
        block.get("externalSchemas", {}),
        test["data"],
        test["valid"],
        test["description"],
        validate_formats(block),
        case_id,
    )
    if unencodable_string(test["data"]):
        output = pytest.param(*output, marks=pytest.mark.skip(reason="instance has no UTF-8 representation"))
    return output


def suite_cases():
    # `Decimal` keeps literals past `float` precision.
    files = [
        (
            path.relative_to(TEST_SUITE_PATH).as_posix(),
            json.loads(path.read_text(encoding="utf-8"), parse_float=Decimal)["tests"],
        )
        for path in sorted(TEST_SUITE_PATH.rglob("*.json"))
    ]
    for draft, release in SUPPORTED_DRAFTS.items():
        for relative, blocks in files:
            if relative.startswith("proposals/"):
                continue
            for index, block in enumerate(blocks):
                if not is_compatible(block.get("compatibility"), release):
                    continue
                # Matches the id `pyo3_suite!` gives the validator compiled for this block.
                case_id = f"draft{draft}|{relative}|{index}"
                for test in block["tests"]:
                    yield maybe_skipped(draft, block, test, Path(relative).name, case_id)


def pytest_generate_tests(metafunc):
    metafunc.parametrize(
        "filename, draft, schema, external_schemas, instance, expected, description, validate_formats, case_id",
        list(suite_cases()),
    )


def validator_kwargs(schema, external_schemas, validate_formats):
    retriever = make_retriever(external_schemas)
    kwargs = {"retriever": retriever}
    if validate_formats is not None:
        kwargs["validate_formats"] = validate_formats
    # An explicit draft never retrieves `$schema`, so a custom meta-schema must be registered.
    meta_schema = schema.get("$schema") if isinstance(schema, dict) else None
    if meta_schema in external_schemas:
        kwargs["registry"] = jsonschema_rs.Registry([(meta_schema, external_schemas[meta_schema])], retriever=retriever)
    return kwargs


def runtime_validator(draft, schema, external_schemas, validate_formats):
    cls = {
        "4": jsonschema_rs.Draft4Validator,
        "6": jsonschema_rs.Draft6Validator,
        "7": jsonschema_rs.Draft7Validator,
        "2019-09": jsonschema_rs.Draft201909Validator,
        "2020-12": jsonschema_rs.Draft202012Validator,
    }[draft]
    return cls(schema, **validator_kwargs(schema, external_schemas, validate_formats))


# `json.dumps` rejects `Decimal`, so literals past `float` precision are written as the suite has them.
def json_text(value):
    if isinstance(value, Decimal):
        return str(value)
    if isinstance(value, dict):
        return "{" + ",".join(f"{json.dumps(key)}:{json_text(item)}" for key, item in value.items()) + "}"
    if isinstance(value, list):
        return "[" + ",".join(json_text(item) for item in value) + "]"
    return json.dumps(value)


def test_draft_json_text(
    filename, draft, schema, external_schemas, instance, expected, description, validate_formats, case_id
):
    error_message = f"[{filename}] {description}: {schema} | {instance}"
    validator = jsonschema_rs.JsonTextValidator(
        schema,
        draft={
            "4": jsonschema_rs.Draft4,
            "6": jsonschema_rs.Draft6,
            "7": jsonschema_rs.Draft7,
            "2019-09": jsonschema_rs.Draft201909,
            "2020-12": jsonschema_rs.Draft202012,
        }[draft],
        **validator_kwargs(schema, external_schemas, validate_formats),
    )
    text = json_text(instance)
    assert validator.is_valid(text) is expected, f"is_valid mismatch: {error_message}"
    assert validator.is_valid(text.encode()) is expected, f"is_valid over bytes mismatch: {error_message}"
    if expected:
        validator.validate(text)
    else:
        with pytest.raises(jsonschema_rs.ValidationError):
            validator.validate(text)
    assert (list(validator.iter_errors(text)) == []) is expected, f"iter_errors mismatch: {error_message}"
    assert validator.evaluate(text).flag()["valid"] is expected, f"evaluate mismatch: {error_message}"


def test_draft(filename, draft, schema, external_schemas, instance, expected, description, validate_formats, case_id):
    error_message = f"[{filename}] {description}: {schema} | {instance}"
    try:
        validator = runtime_validator(draft, schema, external_schemas, validate_formats)

        result = validator.is_valid(instance)
        assert result is expected, f"is_valid mismatch: {error_message}"

        if expected:
            validator.validate(instance)
        else:
            with pytest.raises(jsonschema_rs.ValidationError):
                validator.validate(instance)

        errors = list(validator.iter_errors(instance))
        if expected:
            assert errors == [], f"iter_errors expected no errors: {error_message}"
        else:
            assert errors != [], f"iter_errors expected errors: {error_message}"

        evaluation = validator.evaluate(instance)
        assert evaluation.flag()["valid"] is expected, f"evaluate mismatch: {error_message}"
    except ValueError:
        pytest.fail(error_message)


# `jsonschema_testsuite_pyo3` holds one `backend = Pyo3` validator per suite case.
def test_draft_codegen(
    filename, draft, schema, external_schemas, instance, expected, description, validate_formats, case_id
):
    error_message = f"[{filename}] {description}: {schema} | {instance}"
    assert jsonschema_testsuite_pyo3.is_valid(case_id, instance) is expected, f"is_valid mismatch: {error_message}"
    assert (jsonschema_testsuite_pyo3.validate(case_id, instance) is None) is expected, (
        f"validate mismatch: {error_message}"
    )
    assert (jsonschema_testsuite_pyo3.iter_errors(case_id, instance) == []) is expected, (
        f"iter_errors mismatch: {error_message}"
    )
    # The outputs of the generated validator and of a runtime one built like it.
    generated, runtime = json.loads(jsonschema_testsuite_pyo3.evaluate(case_id, instance))
    assert generated == runtime, f"evaluate mismatch: {error_message}"

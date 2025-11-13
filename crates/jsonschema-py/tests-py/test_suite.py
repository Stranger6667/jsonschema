import json
import os
from pathlib import Path

import pytest

import jsonschema_rs

TEST_SUITE_PATH = Path(__file__).parent.parent.parent / "jsonschema/tests/suite"
REMOTE_PREFIXES = ("http://localhost:1234", "https://localhost:1234")


def load_remote_documents():
    remotes = TEST_SUITE_PATH / "remotes"
    documents = {}
    for root, _, files in os.walk(remotes):
        for filename in files:
            path = Path(root) / filename
            relative = path.relative_to(remotes).as_posix()
            contents = path.read_text(encoding="utf-8")
            for prefix in REMOTE_PREFIXES:
                documents[f"{prefix}/{relative}"] = contents
    return documents


REMOTE_DOCUMENTS = load_remote_documents()


def make_testsuite_retriever():
    localhost_alias = "http://127.0.0.1:1234/"

    def _retriever(uri: str):
        normalized = uri
        if normalized.startswith(localhost_alias):
            normalized = f"http://localhost:1234/{normalized[len(localhost_alias) :]}"
        try:
            return json.loads(REMOTE_DOCUMENTS[normalized])
        except KeyError as exc:
            raise ValueError(f"Unknown remote schema: {uri}") from exc

    return _retriever


TESTSUITE_RETRIEVER = make_testsuite_retriever()

SUPPORTED_DRAFTS = ("4", "6", "7", "2019-09", "2020-12")
NOT_SUPPORTED_CASES = {
    "4": ("bignum.json",),
}


def load_file(path):
    with open(path, mode="r", encoding="utf-8") as fd:
        for block in json.load(fd):
            yield block


def maybe_optional(draft, schema, instance, expected, description, filename, is_optional):
    output = (filename, draft, schema, instance, expected, description, is_optional)
    if filename in NOT_SUPPORTED_CASES.get(draft, ()):
        output = pytest.param(*output, marks=pytest.mark.skip(reason=f"{filename} is not supported"))
    return output


def pytest_generate_tests(metafunc):
    cases = [
        maybe_optional(
            draft, block["schema"], test["data"], test["valid"], test["description"], filename, "optional" in str(root)
        )
        for draft in SUPPORTED_DRAFTS
        for root, _, files in os.walk(TEST_SUITE_PATH / f"tests/draft{draft}/")
        for filename in files
        for block in load_file(os.path.join(root, filename))
        for test in block["tests"]
    ]
    metafunc.parametrize("filename, draft, schema, instance, expected, description, is_optional", cases)


def test_draft(filename, draft, schema, instance, expected, description, is_optional):
    error_message = f"[{filename}] {description}: {schema} | {instance}"
    try:
        cls = {
            "4": jsonschema_rs.Draft4Validator,
            "6": jsonschema_rs.Draft6Validator,
            "7": jsonschema_rs.Draft7Validator,
            "2019-09": jsonschema_rs.Draft201909Validator,
            "2020-12": jsonschema_rs.Draft202012Validator,
        }[draft]
        kwargs = {"retriever": TESTSUITE_RETRIEVER}
        if is_optional:
            kwargs["validate_formats"] = True
        result = cls(schema, **kwargs).is_valid(instance)
        assert result is expected, error_message
    except ValueError:
        pytest.fail(error_message)

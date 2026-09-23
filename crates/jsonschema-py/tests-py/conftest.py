import json
from types import SimpleNamespace

import jsonschema_testsuite_pyo3
import pytest

import jsonschema_rs


# `jsonschema_testsuite_pyo3` holds a `backend = Pyo3` validator for every schema in
# `codegen_schemas.json`, keyed as below.
def schema_key(schema):
    return json.dumps(schema, sort_keys=True, separators=(",", ":"))


# Custom keywords are compiled in, so `keywords` names the Python ones only the runtime reads.
class Codegen:
    @staticmethod
    def is_valid(schema, instance, keywords=None):
        return jsonschema_testsuite_pyo3.is_valid(schema_key(schema), instance)

    @staticmethod
    def validate(schema, instance, keywords=None):
        error = jsonschema_testsuite_pyo3.validate(schema_key(schema), instance)
        if error is not None:
            raise ValueError(error)

    @staticmethod
    def iter_errors(schema, instance, keywords=None):
        messages = jsonschema_testsuite_pyo3.iter_errors(schema_key(schema), instance)
        return [SimpleNamespace(message=message) for message in messages]


@pytest.fixture(params=(jsonschema_rs, Codegen), ids=("runtime", "codegen"))
def backend(request):
    return request.param

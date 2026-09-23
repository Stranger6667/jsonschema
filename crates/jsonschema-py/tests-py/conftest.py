import json

import jsonschema_testsuite_pyo3
import pytest

import jsonschema_rs


# `jsonschema_testsuite_pyo3` holds a `backend = Pyo3` validator for every schema in
# `codegen_schemas.json`, keyed as below.
def schema_key(schema):
    return json.dumps(schema, sort_keys=True, separators=(",", ":"))


class Codegen:
    @staticmethod
    def is_valid(schema, instance):
        return jsonschema_testsuite_pyo3.is_valid(schema_key(schema), instance)

    @staticmethod
    def validate(schema, instance):
        error = jsonschema_testsuite_pyo3.validate(schema_key(schema), instance)
        if error is not None:
            raise ValueError(error)


@pytest.fixture(params=(jsonschema_rs, Codegen), ids=("runtime", "codegen"))
def backend(request):
    return request.param

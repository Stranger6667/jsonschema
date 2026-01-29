import jsonschema_rs


def test_evaluate_produces_expected_outputs_for_valid_instance():
    schema = {
        "type": "object",
        "properties": {"name": {"type": "string"}, "age": {"type": "number", "minimum": 0}},
        "required": ["name"],
    }
    instance = {"name": "Alice", "age": 1}

    evaluation = jsonschema_rs.evaluate(schema, instance)

    assert evaluation.flag() == {"valid": True}

    assert evaluation.list() == {
        "valid": True,
        "details": [
            {
                "evaluationPath": "",
                "instanceLocation": "",
                "schemaLocation": "",
                "valid": True,
            },
            {
                "valid": True,
                "evaluationPath": "/type",
                "instanceLocation": "",
                "schemaLocation": "/type",
            },
            {
                "valid": True,
                "evaluationPath": "/required",
                "instanceLocation": "",
                "schemaLocation": "/required",
            },
            {
                "valid": True,
                "evaluationPath": "/properties",
                "instanceLocation": "",
                "schemaLocation": "/properties",
                "annotations": ["age", "name"],
            },
            {
                "valid": True,
                "evaluationPath": "/properties/age",
                "instanceLocation": "/age",
                "schemaLocation": "/properties/age",
            },
            {
                "valid": True,
                "evaluationPath": "/properties/age/type",
                "instanceLocation": "/age",
                "schemaLocation": "/properties/age/type",
            },
            {
                "valid": True,
                "evaluationPath": "/properties/age/minimum",
                "instanceLocation": "/age",
                "schemaLocation": "/properties/age/minimum",
            },
            {
                "valid": True,
                "evaluationPath": "/properties/name",
                "instanceLocation": "/name",
                "schemaLocation": "/properties/name",
            },
            {
                "valid": True,
                "evaluationPath": "/properties/name/type",
                "instanceLocation": "/name",
                "schemaLocation": "/properties/name/type",
            },
        ],
    }

    assert evaluation.hierarchical() == {
        "valid": True,
        "evaluationPath": "",
        "instanceLocation": "",
        "schemaLocation": "",
        "details": [
            {
                "valid": True,
                "evaluationPath": "/type",
                "instanceLocation": "",
                "schemaLocation": "/type",
            },
            {
                "valid": True,
                "evaluationPath": "/required",
                "instanceLocation": "",
                "schemaLocation": "/required",
            },
            {
                "valid": True,
                "evaluationPath": "/properties",
                "instanceLocation": "",
                "schemaLocation": "/properties",
                "annotations": ["age", "name"],
                "details": [
                    {
                        "valid": True,
                        "evaluationPath": "/properties/age",
                        "instanceLocation": "/age",
                        "schemaLocation": "/properties/age",
                        "details": [
                            {
                                "valid": True,
                                "evaluationPath": "/properties/age/type",
                                "instanceLocation": "/age",
                                "schemaLocation": "/properties/age/type",
                            },
                            {
                                "valid": True,
                                "evaluationPath": "/properties/age/minimum",
                                "instanceLocation": "/age",
                                "schemaLocation": "/properties/age/minimum",
                            },
                        ],
                    },
                    {
                        "valid": True,
                        "evaluationPath": "/properties/name",
                        "instanceLocation": "/name",
                        "schemaLocation": "/properties/name",
                        "details": [
                            {
                                "valid": True,
                                "evaluationPath": "/properties/name/type",
                                "instanceLocation": "/name",
                                "schemaLocation": "/properties/name/type",
                            }
                        ],
                    },
                ],
            },
        ],
    }

    assert evaluation.annotations() == [
        {
            "schemaLocation": "/properties",
            "absoluteKeywordLocation": None,
            "instanceLocation": "",
            "annotations": ["age", "name"],
        }
    ]
    assert evaluation.errors() == []


def test_validator_evaluate_annotations_and_errors():
    schema = {
        "type": "array",
        "prefixItems": [{"type": "string"}],
        "items": {"type": "integer"},
    }
    validator = jsonschema_rs.validator_for(schema)

    valid_eval = validator.evaluate(["hello", 1])
    assert valid_eval.annotations() == [
        {
            "schemaLocation": "/items",
            "absoluteKeywordLocation": None,
            "instanceLocation": "",
            "annotations": True,
        },
        {
            "schemaLocation": "/prefixItems",
            "absoluteKeywordLocation": None,
            "instanceLocation": "",
            "annotations": 0,
        },
    ]
    assert valid_eval.errors() == []

    invalid_eval = validator.evaluate(["hello", "oops"])
    assert invalid_eval.flag() == {"valid": False}
    assert invalid_eval.errors() == [
        {
            "schemaLocation": "/items/type",
            "absoluteKeywordLocation": None,
            "instanceLocation": "/1",
            "error": '"oops" is not of type "integer"',
        }
    ]
    assert invalid_eval.list() == {
        "valid": False,
        "details": [
            {
                "evaluationPath": "",
                "instanceLocation": "",
                "schemaLocation": "",
                "valid": False,
            },
            {
                "valid": True,
                "evaluationPath": "/type",
                "instanceLocation": "",
                "schemaLocation": "/type",
            },
            {
                "valid": False,
                "evaluationPath": "/items",
                "instanceLocation": "",
                "schemaLocation": "/items",
                "droppedAnnotations": True,
            },
            {
                "valid": False,
                "evaluationPath": "/items",
                "instanceLocation": "/1",
                "schemaLocation": "/items",
            },
            {
                "valid": False,
                "evaluationPath": "/items/type",
                "instanceLocation": "/1",
                "schemaLocation": "/items/type",
                "errors": {"type": '"oops" is not of type "integer"'},
            },
            {
                "valid": True,
                "evaluationPath": "/prefixItems",
                "instanceLocation": "",
                "schemaLocation": "/prefixItems",
                "annotations": 0,
            },
            {
                "valid": True,
                "evaluationPath": "/prefixItems/0",
                "instanceLocation": "/0",
                "schemaLocation": "/prefixItems/0",
            },
            {
                "valid": True,
                "evaluationPath": "/prefixItems/0/type",
                "instanceLocation": "/0",
                "schemaLocation": "/prefixItems/0/type",
            },
        ],
    }
    assert invalid_eval.hierarchical() == {
        "valid": False,
        "evaluationPath": "",
        "instanceLocation": "",
        "schemaLocation": "",
        "details": [
            {
                "valid": True,
                "evaluationPath": "/type",
                "instanceLocation": "",
                "schemaLocation": "/type",
            },
            {
                "valid": False,
                "evaluationPath": "/items",
                "instanceLocation": "",
                "schemaLocation": "/items",
                "droppedAnnotations": True,
                "details": [
                    {
                        "valid": False,
                        "evaluationPath": "/items",
                        "instanceLocation": "/1",
                        "schemaLocation": "/items",
                        "details": [
                            {
                                "valid": False,
                                "evaluationPath": "/items/type",
                                "instanceLocation": "/1",
                                "schemaLocation": "/items/type",
                                "errors": {"type": '"oops" is not of type "integer"'},
                            }
                        ],
                    }
                ],
            },
            {
                "valid": True,
                "evaluationPath": "/prefixItems",
                "instanceLocation": "",
                "schemaLocation": "/prefixItems",
                "annotations": 0,
                "details": [
                    {
                        "valid": True,
                        "evaluationPath": "/prefixItems/0",
                        "instanceLocation": "/0",
                        "schemaLocation": "/prefixItems/0",
                        "details": [
                            {
                                "valid": True,
                                "evaluationPath": "/prefixItems/0/type",
                                "instanceLocation": "/0",
                                "schemaLocation": "/prefixItems/0/type",
                            }
                        ],
                    }
                ],
            },
        ],
    }

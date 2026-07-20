import pytest

from jsonschema_rs import CanonicalSchema, ValidationError, canonical, canonicalize

DRAFT202012 = "https://json-schema.org/draft/2020-12/schema"


@pytest.mark.parametrize(
    "schema",
    [
        {"type": "string", "minLength": 3},
        {"allOf": [{"type": "integer"}, {"minimum": 0}]},
        {"$defs": {"a": {"type": "null"}}, "$ref": "#/$defs/a"},
    ],
)
def test_unmodeled_round_trips_verbatim(schema):
    result = canonicalize(schema)
    assert isinstance(result, CanonicalSchema)
    assert result.to_json_schema() == schema
    assert result.kind == "raw"


@pytest.mark.parametrize(
    ("schema", "expected"),
    [
        ({"enum": [5]}, {"$schema": DRAFT202012, "const": 5}),
        ({"enum": ["z", 2, None, 1]}, {"$schema": DRAFT202012, "enum": [None, 1, 2, "z"]}),
        ({"const": None}, {"$schema": DRAFT202012, "type": "null"}),
        ({"type": ["integer", "string"]}, {"$schema": DRAFT202012, "type": ["integer", "string"]}),
        ({"type": "boolean", "enum": [True]}, {"$schema": DRAFT202012, "const": True}),
        ({"type": "integer", "enum": [1, "x", 2]}, {"$schema": DRAFT202012, "enum": [1, 2]}),
        (
            {"allOf": [{"type": ["integer", "string"]}, {"enum": [1, "x", None]}]},
            {"$schema": DRAFT202012, "enum": [1, "x"]},
        ),
        (
            {"anyOf": [{"const": 5}, {"type": "string"}]},
            {"$schema": DRAFT202012, "anyOf": [{"type": "string"}, {"const": 5}]},
        ),
        (
            {"anyOf": [{"type": "integer"}, {"type": "string"}]},
            {"$schema": DRAFT202012, "type": ["integer", "string"]},
        ),
    ],
)
def test_valueset_canonical_forms(schema, expected):
    assert canonicalize(schema).to_json_schema() == expected


def test_view_const():
    match canonicalize({"enum": [5]}).view():
        case canonical.ConstView(value=value):
            assert value == 5
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_enum():
    match canonicalize({"enum": [2, 1]}).view():
        case canonical.EnumView(values=values):
            assert values == [1, 2]
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_multi_type():
    match canonicalize({"type": ["string", "integer"]}).view():
        case canonical.MultiTypeView(types=types):
            assert types == ["integer", "string"]
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_true_false():
    assert isinstance(canonicalize({}).view(), canonical.TrueView)
    assert isinstance(canonicalize(False).view(), canonical.FalseView)


def test_view_typed_group_draft4_integer():
    schema = {
        "$schema": "http://json-schema.org/draft-04/schema#",
        "type": "integer",
        "enum": [1, 2],
    }
    match canonicalize(schema).view():
        case canonical.TypedGroupView(type_name=type_name, body=body) if isinstance(body, CanonicalSchema):
            assert type_name == "integer"
            match body.view():
                case canonical.EnumView(values=values):
                    assert values == [1, 2]
                case other:
                    pytest.fail(f"unexpected body view: {other!r}")
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_any_of():
    match canonicalize({"anyOf": [{"const": 5}, {"type": "string"}]}).view():
        case canonical.AnyOfView(branches=branches):
            assert [branch.kind for branch in branches] == ["multi_type", "const"]
            assert all(isinstance(branch, CanonicalSchema) for branch in branches)
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_raw():
    match canonicalize({"not": {}}).view():
        case canonical.RawView(schema=payload):
            assert payload == {"not": {}}
        case other:
            pytest.fail(f"unexpected view: {other!r}")


@pytest.mark.parametrize(
    ("schema", "kind"),
    [
        ({"const": 5}, "const"),
        ({"enum": [1, 2]}, "enum"),
        ({"type": ["integer", "string"]}, "multi_type"),
        ({"anyOf": [{"const": 5}, {"type": "string"}]}, "any_of"),
        ({}, "true"),
        (False, "false"),
        ({"pattern": "a"}, "raw"),
    ],
)
def test_kind(schema, kind):
    assert canonicalize(schema).kind == kind


def test_is_satisfiable():
    assert canonicalize({"const": 5}).is_satisfiable()
    assert not canonicalize({"type": "integer", "enum": ["x"]}).is_satisfiable()


@pytest.mark.parametrize(
    ("left", "right"),
    [
        ({"enum": [5]}, {"const": 5}),
        ({"const": 1}, {"const": 1.0}),
    ],
)
def test_value_equivalence(left, right):
    assert canonicalize(left) == canonicalize(right)


def test_invalid_schema_raises_validation_error():
    with pytest.raises(ValidationError):
        canonicalize({"type": 123})


@pytest.mark.parametrize("schema", [42, "string", [1], None])
def test_invalid_schema_type(schema):
    with pytest.raises(canonical.InvalidSchemaType):
        canonicalize(schema)


def test_exception_hierarchy():
    assert issubclass(canonical.CanonicalizationError, ValueError)
    assert issubclass(canonical.InvalidSchemaType, canonical.CanonicalizationError)

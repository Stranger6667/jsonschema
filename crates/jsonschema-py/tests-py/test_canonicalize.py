from decimal import Decimal

import pytest

import jsonschema_rs
from jsonschema_rs import CanonicalSchema, ValidationError, canonical, canonicalize
from jsonschema_rs.canonical import CanonicalKind, Containment, Distinctness, Satisfiability

DRAFT202012 = "https://json-schema.org/draft/2020-12/schema"
# `anyOf` annotates whichever branch the instance matched, which no `additional*` twin spells,
# so this stays raw. Each construct canonicalization learns needs a still-unsupported stand-in here.
UNSUPPORTED = {"if": {}, "unevaluatedProperties": False}


@pytest.mark.parametrize(
    "schema",
    [
        UNSUPPORTED,
    ],
)
def test_unsupported_round_trips_verbatim(schema):
    result = canonicalize(schema)
    assert isinstance(result, CanonicalSchema)
    assert result.to_json_schema() == schema
    assert result.kind == CanonicalKind.RAW


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


def test_view_string():
    match canonicalize({"type": "string", "minLength": 2, "pattern": "^a"}).view():
        case canonical.StringView(min_length=min_length, max_length=max_length, patterns=patterns):
            assert min_length == 2
            assert max_length is None
            assert patterns == ["^a"]
        case other:
            pytest.fail(f"unexpected view: {other!r}")


# Bounds past `u64` stay exact under arbitrary precision.
@pytest.mark.parametrize("keyword, attribute", [("minLength", "min_length"), ("maxLength", "max_length")])
def test_view_string_bound_past_u64(keyword, attribute):
    huge = 10**23
    match canonicalize({"type": "string", keyword: huge}).view():
        case canonical.StringView() as view:
            assert getattr(view, attribute) == huge
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_string_formats():
    match canonicalize({"type": "string", "format": "email"}, validate_formats=True).view():
        case canonical.StringView(patterns=patterns, formats=formats):
            assert patterns == []
            assert formats == ["email"]
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_string_content():
    # Same-object contentEncoding+contentMediaType decode-then-check and stay raw; separate allOf
    # branches model independently, which the view then exposes.
    schema = {"allOf": [{"type": "string", "contentEncoding": "base64"}, {"contentMediaType": "application/json"}]}
    match canonicalize(schema, draft=7).view():
        case canonical.StringView(content_media_types=content_media_types, content_encodings=content_encodings):
            assert content_media_types == ["application/json"]
            assert content_encodings == ["base64"]
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_array_lengths():
    schema = {"type": "array", "minItems": 1, "maxItems": 3, "uniqueItems": True, "items": {"type": "integer"}}
    match canonicalize(schema).view():
        case canonical.ArrayView(
            min_items=min_items,
            max_items=max_items,
            distinctness=distinctness,
            prefix_items=prefix_items,
            items=items,
        ):
            assert min_items == 1
            assert max_items == 3
            assert distinctness == Distinctness.ALL_DISTINCT
            assert prefix_items == []
            assert items.to_json_schema() == {"$schema": DRAFT202012, "type": "integer"}
        case other:
            pytest.fail(f"unexpected view: {other!r}")


@pytest.mark.parametrize(
    ("schema", "expected"),
    (
        ({"type": "array", "minItems": 1}, Distinctness.UNCONSTRAINED),
        ({"type": "array", "uniqueItems": True}, Distinctness.ALL_DISTINCT),
        (
            {"type": "array", "allOf": [{"not": {"type": "array", "uniqueItems": True}}]},
            Distinctness.SOME_REPEATED,
        ),
    ),
)
def test_view_array_distinctness(schema, expected):
    match canonicalize(schema).view():
        case canonical.ArrayView(distinctness=distinctness):
            assert distinctness == expected
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_array_prefix_items():
    schema = {"type": "array", "prefixItems": [{"type": "integer"}, {"type": "string"}], "items": {"type": "boolean"}}
    match canonicalize(schema).view():
        case canonical.ArrayView(prefix_items=prefix_items, items=items):
            assert [p.to_json_schema() for p in prefix_items] == [
                {"$schema": DRAFT202012, "type": "integer"},
                {"$schema": DRAFT202012, "type": "string"},
            ]
            assert items.to_json_schema() == {"$schema": DRAFT202012, "type": "boolean"}
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_array_contains():
    schema = {"type": "array", "contains": {"type": "string"}, "minContains": 0, "maxContains": 2}
    match canonicalize(schema).view():
        case canonical.ArrayView(contains=[facet]):
            assert facet.schema.to_json_schema() == {"$schema": DRAFT202012, "type": "string"}
            assert facet.min_contains == 0
            assert facet.max_contains == 2
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_array_contains_default_minimum():
    schema = {"type": "array", "contains": {"type": "null"}}
    match canonicalize(schema).view():
        case canonical.ArrayView(contains=[facet]):
            assert facet.schema.to_json_schema() == {"$schema": DRAFT202012, "type": "null"}
            assert facet.min_contains is None
            assert facet.max_contains is None
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_object_sizes():
    schema = {
        "type": "object",
        "minProperties": 1,
        "maxProperties": 3,
        "required": ["a"],
        "propertyNames": {"maxLength": 4},
        "properties": {"a": {"type": "integer"}},
    }
    match canonicalize(schema).view():
        case canonical.ObjectView(
            min_properties=min_properties,
            max_properties=max_properties,
            required=required,
            property_names=property_names,
            properties=properties,
        ):
            assert min_properties is None
            assert max_properties == 3
            assert required == ["a"]
            assert property_names is not None
            assert property_names.to_json_schema() == {
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "string",
                "maxLength": 4,
            }
            assert list(properties) == ["a"]
            assert properties["a"].to_json_schema() == {
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "integer",
            }
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_object_pattern_properties():
    schema = {"type": "object", "patternProperties": {"^a": {"type": "integer"}}}
    match canonicalize(schema).view():
        case canonical.ObjectView(pattern_properties=pattern_properties):
            assert list(pattern_properties) == ["^a"]
            assert pattern_properties["^a"].to_json_schema() == {
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "integer",
            }
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_object_view_violations():
    schema = canonicalize(
        {
            "type": "object",
            "minProperties": 1,
            "properties": {"filter": {"type": "string"}},
            "not": {"additionalProperties": False, "properties": {"filter": {"type": "string"}}},
        }
    )
    view = schema.view()
    assert len(view.violations) == 1
    [violation] = view.violations
    assert isinstance(violation, canonical.NameFailsView)
    assert violation.schema.to_json_schema()["const"] == "filter"


def test_view_number_multiple_of():
    match canonicalize({"type": "number", "multipleOf": 0.5}).view():
        case canonical.NumberView(multiple_of=multiple_of):
            assert multiple_of == [0.5]
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_number_not_multiple_of():
    match canonicalize({"type": "number", "not": {"multipleOf": 0.5}}).view():
        case canonical.NumberView(multiple_of=multiple_of, not_multiple_of=not_multiple_of):
            assert multiple_of == []
            assert not_multiple_of == [0.5]
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_integer_not_multiple_of():
    match canonicalize({"type": "integer", "not": {"multipleOf": 3}}).view():
        case canonical.IntegerView(multiple_of=multiple_of, not_multiple_of=not_multiple_of):
            assert multiple_of == []
            assert not_multiple_of == [3]
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_number_excludes_integers():
    schema = {"$schema": "http://json-schema.org/draft-04/schema#", "type": "number", "not": {"type": "integer"}}
    match canonicalize(schema).view():
        case canonical.NumberView(excludes_integers=excludes_integers, not_multiple_of=not_multiple_of):
            assert excludes_integers is True
            assert not_multiple_of == []
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_number_interval():
    match canonicalize({"type": "number", "minimum": 2, "exclusiveMaximum": 5}).view():
        case canonical.NumberView(
            minimum=minimum,
            exclusive_minimum=exclusive_minimum,
            maximum=maximum,
            exclusive_maximum=exclusive_maximum,
        ):
            assert minimum == 2
            assert exclusive_minimum is False
            assert maximum == 5
            assert exclusive_maximum is True
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_number_bound_off_the_float_grid():
    # Folding `multipleOf` into an exclusive bound lands a fraction past a number no float can hold
    # apart from its neighbour, and rounding it there would admit the value the schema excludes.
    schema = {"type": "number", "multipleOf": 0.1, "exclusiveMinimum": 10**20}
    match canonicalize(schema).view():
        case canonical.NumberView(minimum=minimum, exclusive_minimum=exclusive_minimum):
            assert minimum == Decimal("100000000000000000000.1")
            assert exclusive_minimum is False
            assert not jsonschema_rs.is_valid(schema, float(minimum))
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_number_bound_on_the_float_grid():
    match canonicalize({"type": "number", "multipleOf": 0.5, "exclusiveMinimum": 1}).view():
        case canonical.NumberView(minimum=minimum):
            assert minimum == 1.5
            assert isinstance(minimum, float)
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_integer_multiple_of():
    match canonicalize({"type": "integer", "multipleOf": 3}).view():
        case canonical.IntegerView(minimum=minimum, multiple_of=multiple_of):
            assert minimum is None
            assert multiple_of == [3]
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_integer_bound_past_i64():
    huge = 10**23
    match canonicalize({"type": "integer", "minimum": huge}).view():
        case canonical.IntegerView(minimum=minimum):
            assert minimum == huge
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_integer():
    match canonicalize({"type": "integer", "minimum": 2, "maximum": 9}).view():
        case canonical.IntegerView(minimum=minimum, maximum=maximum):
            assert minimum == 2
            assert maximum == 9
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_any_of():
    match canonicalize({"anyOf": [{"const": 5}, {"type": "string"}]}).view():
        case canonical.AnyOfView(branches=branches):
            assert [branch.kind for branch in branches] == [
                CanonicalKind.MULTI_TYPE,
                CanonicalKind.CONST,
            ]
            assert all(isinstance(branch, CanonicalSchema) for branch in branches)
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_one_of():
    schema = {
        "oneOf": [{"$ref": "#/$defs/plain"}, {"$ref": "#/$defs/tight"}],
        "$defs": {"plain": {"type": "string"}, "tight": {"type": "string", "minLength": 3}},
    }
    match canonicalize(schema).view():
        case canonical.OneOfView(branches=branches):
            assert [branch.kind for branch in branches] == [
                CanonicalKind.REFERENCE,
                CanonicalKind.REFERENCE,
            ]
            assert all(isinstance(branch, CanonicalSchema) for branch in branches)
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_reference_and_definitions():
    result = canonicalize({"$ref": "#/$defs/value", "$defs": {"value": {"type": "string"}}})

    match result.view():
        case canonical.ReferenceView(uri=uri):
            assert uri == "#/$defs/value"
        case other:
            pytest.fail(f"unexpected view: {other!r}")
    assert result.definitions()["#/$defs/value"].kind == CanonicalKind.MULTI_TYPE


def test_view_all_of_with_symbolic_reference():
    # A cycle keeps the conjunction symbolic: such a document is not folded through its targets.
    schema = {
        "allOf": [
            {"$ref": "#/$defs/value"},
            {"type": "string"},
        ],
        "$defs": {"value": {"type": "object", "properties": {"self": {"$ref": "#/$defs/value"}}}},
    }
    match canonicalize(schema).view():
        case canonical.AllOfView(branches=branches):
            assert [branch.kind for branch in branches] == [
                CanonicalKind.MULTI_TYPE,
                CanonicalKind.REFERENCE,
            ]
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_not_with_symbolic_reference():
    match canonicalize(
        {
            "not": {"$ref": "#/$defs/other"},
            "$defs": {"other": {"type": "object", "properties": {"child": {"$ref": "#/$defs/other"}}}},
        }
    ).view():
        case canonical.NotView(schema=inner):
            assert inner.kind == CanonicalKind.REFERENCE
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_view_raw():
    match canonicalize(UNSUPPORTED).view():
        case canonical.RawView(schema=payload):
            assert payload == UNSUPPORTED
        case other:
            pytest.fail(f"unexpected view: {other!r}")


def test_contains_view_is_public():
    view = canonicalize({"type": "array", "contains": {"type": "integer"}}).view()
    assert isinstance(view, canonical.ArrayView)
    assert isinstance(view.contains[0], canonical.ContainsView)


@pytest.mark.parametrize(
    ("schema", "kind"),
    [
        ({"const": 5}, CanonicalKind.CONST),
        ({"enum": [1, 2]}, CanonicalKind.ENUM),
        ({"type": ["integer", "string"]}, CanonicalKind.MULTI_TYPE),
        ({"anyOf": [{"const": 5}, {"type": "string"}]}, CanonicalKind.ANY_OF),
        ({}, CanonicalKind.TRUE),
        (False, CanonicalKind.FALSE),
        ({"type": "string", "minLength": 3}, CanonicalKind.STRING),
        ({"type": "integer", "minimum": 0}, CanonicalKind.INTEGER),
        ({"pattern": "a"}, CanonicalKind.ANY_OF),
    ],
)
def test_kind(schema, kind):
    assert canonicalize(schema).kind == kind


def test_satisfiability():
    assert canonicalize({"const": 5}).satisfiability() != Satisfiability.NO
    assert canonicalize({"type": "integer", "enum": ["x"]}).satisfiability() == Satisfiability.NO


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


def test_invalid_pattern():
    with pytest.raises(canonical.InvalidPattern):
        canonicalize({"pattern": "["})


@pytest.mark.parametrize(
    ("schema", "expected"),
    [
        (
            {"type": "string", "minLength": 2, "maxLength": 4},
            {"$schema": DRAFT202012, "type": "string", "minLength": 2, "maxLength": 4},
        ),
        (
            {"pattern": "^a"},
            {
                "$schema": DRAFT202012,
                "anyOf": [
                    {"type": ["null", "boolean", "number", "array", "object"]},
                    {"type": "string", "pattern": "^a"},
                ],
            },
        ),
    ],
)
def test_string_canonical_forms(schema, expected):
    assert canonicalize(schema).to_json_schema() == expected


# A pattern whose compiled size exceeds the default regex limit; real AWS schemas carry these.
LARGE_PATTERN_SCHEMA = {"type": "string", "pattern": "^.{0,100000}$"}


def test_large_pattern_is_rejected_by_default():
    with pytest.raises(jsonschema_rs.canonical.InvalidPattern):
        canonicalize(LARGE_PATTERN_SCHEMA)


@pytest.mark.parametrize(
    "options",
    [
        jsonschema_rs.FancyRegexOptions(size_limit=150_000_000),
        jsonschema_rs.RegexOptions(size_limit=150_000_000),
    ],
)
def test_large_pattern_with_raised_size_limit(options):
    canonical_schema = canonicalize(LARGE_PATTERN_SCHEMA, pattern_options=options)
    assert canonical_schema.kind == CanonicalKind.STRING


def test_pattern_options_rejects_other_types():
    with pytest.raises(TypeError):
        canonicalize(LARGE_PATTERN_SCHEMA, pattern_options=object())


@pytest.mark.parametrize(
    ("left", "right", "expected"),
    [
        (
            {"type": "string"},
            {"minLength": 4},
            {"$schema": DRAFT202012, "type": "string", "minLength": 4},
        ),
        ({"const": "A"}, {"pattern": "^A$"}, {"$schema": DRAFT202012, "const": "A"}),
        ({"const": "A"}, {"const": "B"}, {"$schema": DRAFT202012, "not": {}}),
    ],
)
def test_intersect(left, right, expected):
    result = canonicalize(left).intersect(canonicalize(right))
    assert isinstance(result, CanonicalSchema)
    assert result.to_json_schema() == expected


@pytest.mark.parametrize(
    ("left", "right", "expected"),
    [
        (
            {"type": "string"},
            {"type": "integer"},
            {"$schema": DRAFT202012, "type": ["integer", "string"]},
        ),
        (
            {"const": "a"},
            {"enum": ["a", "b"]},
            {"$schema": DRAFT202012, "enum": ["a", "b"]},
        ),
        (
            {"type": "string", "minLength": 4},
            {"type": "string"},
            {"$schema": DRAFT202012, "type": "string"},
        ),
    ],
)
def test_union(left, right, expected):
    result = canonicalize(left).union(canonicalize(right))
    assert isinstance(result, CanonicalSchema)
    assert result.to_json_schema() == expected


@pytest.mark.parametrize(
    ("left", "right", "expected", "satisfiability"),
    [
        (
            {"type": "integer", "minimum": 10},
            {"type": "integer", "minimum": 15},
            {"$schema": DRAFT202012, "type": "integer", "minimum": 10, "maximum": 14},
            Satisfiability.YES,
        ),
        (
            {"enum": ["a", "b"]},
            {"enum": ["a"]},
            {"$schema": DRAFT202012, "const": "b"},
            Satisfiability.YES,
        ),
        (
            {"type": "integer"},
            {"type": ["integer", "string"]},
            {"$schema": DRAFT202012, "not": {}},
            Satisfiability.NO,
        ),
        # What a narrowed schema stopped accepting, which is the comparison workflow: the values
        # `old` took and `new` turns away.
        (
            {"type": "string"},
            {"type": "string", "maxLength": 50},
            {"$schema": DRAFT202012, "type": "string", "minLength": 51},
            Satisfiability.YES,
        ),
        # Empty the other way round, since `new` only narrowed - which is what proves nothing was
        # lost.
        (
            {"type": "string", "maxLength": 50},
            {"type": "string"},
            {"$schema": DRAFT202012, "not": {}},
            Satisfiability.NO,
        ),
    ],
)
def test_subtract(left, right, expected, satisfiability):
    result = canonicalize(left).subtract(canonicalize(right))
    assert isinstance(result, CanonicalSchema)
    assert result.to_json_schema() == expected
    assert result.satisfiability() == satisfiability


@pytest.mark.parametrize("operation", ["union", "subtract"])
def test_set_operations_reject_uncombinable_operands(operation):
    modeled = canonicalize({"type": "string"})
    with pytest.raises(canonical.UnsupportedOperand):
        getattr(modeled, operation)(canonicalize(UNSUPPORTED))
    with pytest.raises(canonical.IncompatibleOperands):
        getattr(canonicalize({"type": "string"}, draft=7), operation)(canonicalize({"type": "string"}, draft=20))


def test_result_carries_exactly_the_targets_it_still_names():
    left = canonicalize({"$defs": {"A": {"type": "string"}}, "$ref": "#/$defs/A"})
    right = canonicalize({"$defs": {"B": {"minLength": 4}}, "$ref": "#/$defs/B"})
    # Both pointers are read through and the meet folds into one leaf, naming neither.
    result = left.intersect(right)
    assert result.definitions() == {}
    assert result.to_json_schema() == {
        "$schema": DRAFT202012,
        "type": "string",
        "minLength": 4,
    }


@pytest.mark.parametrize("swap", [False, True])
def test_intersect_rejects_unsupported_operand(swap):
    raw = canonicalize(UNSUPPORTED)
    modeled = canonicalize({"type": "string"})
    left, right = (modeled, raw) if swap else (raw, modeled)
    with pytest.raises(canonical.UnsupportedOperand):
        left.intersect(right)


@pytest.mark.parametrize(
    ("left", "right"),
    [
        ({"type": "string"}, {"type": "string"}),
        ({"$defs": {"A": {"type": "string"}}, "$ref": "#/$defs/A"}, {"type": "string"}),
    ],
)
def test_intersect_rejects_draft_mismatch(left, right):
    with pytest.raises(canonical.IncompatibleOperands):
        canonicalize(left, draft=7).intersect(canonicalize(right, draft=20))


def test_intersect_renames_a_definition_the_two_documents_bind_differently():
    # A `$defs` name is private to its document, so two documents binding it to different bodies
    # rename apart rather than refusing to combine.
    left = canonicalize({"$defs": {"A": {"type": "string"}}, "$ref": "#/$defs/A"})
    right = canonicalize({"$defs": {"A": {"minLength": 4}}, "$ref": "#/$defs/A"})
    assert left.intersect(right).to_json_schema() == {
        "$schema": DRAFT202012,
        "type": "string",
        "minLength": 4,
    }


def test_intersect_rejects_documents_binding_the_root_differently():
    # `#` names the document itself, so unlike a private key it cannot be renamed apart.
    left = canonicalize({"type": "object", "properties": {"next": {"$ref": "#"}}})
    right = canonicalize({"type": "object", "properties": {"next": {"$ref": "#"}}, "minProperties": 1})
    with pytest.raises(canonical.IncompatibleOperands):
        left.intersect(right)


@pytest.mark.parametrize(
    ("outer", "inner", "expected"),
    [
        ({"type": "integer"}, {"type": "integer"}, Containment.YES),
        ({"type": "integer"}, {"const": 1}, Containment.YES),
        ({"type": "integer"}, {"enum": [1, 2]}, Containment.YES),
        ({"type": "integer"}, {"const": "x"}, Containment.NO),
        ({"type": "integer"}, {"enum": [1, "x"]}, Containment.NO),
        ({"type": "integer"}, {"type": "string"}, Containment.NO),
        ({"type": "integer", "minimum": 5}, {"type": "integer"}, Containment.NO),
        ({"type": "string", "pattern": "^a"}, {"type": "string"}, Containment.NO),
    ],
)
def test_covers(outer, inner, expected):
    assert canonicalize(outer).covers(canonicalize(inner)) == expected


@pytest.mark.parametrize("swap", [False, True])
def test_covers_rejects_unsupported_operand(swap):
    raw = canonicalize(UNSUPPORTED)
    modeled = canonicalize({"type": "string"})
    left, right = (modeled, raw) if swap else (raw, modeled)
    with pytest.raises(canonical.UnsupportedOperand):
        left.covers(right)


def test_covers_rejects_draft_mismatch():
    with pytest.raises(canonical.IncompatibleOperands):
        canonicalize({"type": "string"}, draft=7).covers(canonicalize({"type": "string"}, draft=20))


LABEL_MEMBERS = [
    (Containment.YES, "yes"),
    (Containment.NO, "no"),
    (Containment.UNKNOWN, "unknown"),
    (Satisfiability.YES, "yes"),
    (Satisfiability.NO, "no"),
    (Satisfiability.UNKNOWN, "unknown"),
    (Distinctness.UNCONSTRAINED, "unconstrained"),
    (Distinctness.ALL_DISTINCT, "all_distinct"),
    (Distinctness.SOME_REPEATED, "some_repeated"),
    (CanonicalKind.MULTI_TYPE, "multi_type"),
    (CanonicalKind.TYPED_GROUP, "typed_group"),
    (CanonicalKind.STRING, "string"),
    (CanonicalKind.INTEGER, "integer"),
    (CanonicalKind.NUMBER, "number"),
    (CanonicalKind.ARRAY, "array"),
    (CanonicalKind.OBJECT, "object"),
    (CanonicalKind.CONST, "const"),
    (CanonicalKind.ENUM, "enum"),
    (CanonicalKind.NOT, "not"),
    (CanonicalKind.ALL_OF, "all_of"),
    (CanonicalKind.ANY_OF, "any_of"),
    (CanonicalKind.ONE_OF, "one_of"),
    (CanonicalKind.REFERENCE, "reference"),
    (CanonicalKind.TRUE, "true"),
    (CanonicalKind.FALSE, "false"),
    (CanonicalKind.RAW, "raw"),
]


@pytest.mark.parametrize(("member", "label"), LABEL_MEMBERS)
def test_label_members_are_their_own_values(member, label):
    assert member not in (0, 1, True, False, label)


@pytest.mark.parametrize(("member", "label"), LABEL_MEMBERS)
def test_label_members_carry_their_label(member, label):
    assert member.value == label


@pytest.mark.parametrize(("member", "label"), LABEL_MEMBERS)
def test_label_members_read_as_their_label(member, label):
    assert str(member) == label
    assert f"{member}" == label


@pytest.mark.parametrize(("member", "label"), LABEL_MEMBERS)
def test_label_members_are_hashable(member, label):
    assert {member: "kept"}[member] == "kept"


def test_label_members_hash_apart():
    assert len({Containment.YES, Containment.NO, Containment.UNKNOWN}) == 3
    assert len({CanonicalKind.RAW, CanonicalKind.OBJECT}) == 2


def test_definition():
    schema = canonicalize({"$defs": {"A": {"type": "string"}}, "$ref": "#/$defs/A"})
    uri, target = next(iter(schema.definitions().items()))
    assert schema.definition(uri) == target
    assert schema.definition("#/$defs/absent") is None


@pytest.mark.parametrize(
    ("schema", "expected"),
    [
        (
            {"type": "string", "minLength": 5},
            {
                "$schema": DRAFT202012,
                "anyOf": [
                    {"type": ["null", "boolean", "number", "array", "object"]},
                    {"type": "string", "maxLength": 4},
                ],
            },
        ),
        (
            {"type": "number", "minimum": 5},
            {
                "$schema": DRAFT202012,
                "anyOf": [
                    {"type": ["null", "boolean", "string", "array", "object"]},
                    {"type": "number", "exclusiveMaximum": 5},
                ],
            },
        ),
    ],
)
def test_negate(schema, expected):
    result = canonicalize(schema).negate()
    assert isinstance(result, CanonicalSchema)
    assert result.to_json_schema() == expected


# A `Raw` operand raises `UnsupportedOperand`, not `UnsupportedResult`.
def test_negate_rejects_an_unsupported_schema():
    with pytest.raises(canonical.UnsupportedOperand):
        canonicalize(UNSUPPORTED).negate()


# The two declines are different answers, so each needs its own reachable case.
@pytest.mark.parametrize(
    "schema",
    [
        {"type": "object", "patternProperties": {"^a": {"type": "string"}}},
        {"type": "array", "contains": {"type": "string"}, "minContains": 2},
    ],
)
def test_negate_raises_unsupported_result(schema):
    with pytest.raises(canonical.UnsupportedResult) as exc:
        canonicalize(schema).negate()
    assert str(exc.value) == "result is not supported in canonical form"


def test_subtract_raises_unsupported_result():
    plain = canonicalize({"type": "object"})
    hard = canonicalize({"type": "object", "patternProperties": {"^a": {"type": "string"}}})
    with pytest.raises(canonical.UnsupportedResult):
        plain.subtract(hard)


def test_subtracting_a_schema_from_itself_needs_no_complement():
    schema = {"type": "object", "patternProperties": {"^a": {"type": "string"}}}
    # Negating this schema declines, so the self-difference must not go through a complement.
    with pytest.raises(canonical.UnsupportedResult):
        canonicalize(schema).negate()
    assert canonicalize(schema).subtract(canonicalize(schema)).satisfiability() == Satisfiability.NO


def test_one_document_canonicalized_twice_combines():
    source = {"$defs": {"Id": {"type": "string"}}, "type": "object", "properties": {"id": {"$ref": "#/$defs/Id"}}}
    left = canonicalize(source)
    right = canonicalize(source)
    assert left == right
    assert left.intersect(right) == left
    assert left.covers(right) == Containment.YES


@pytest.mark.parametrize(
    ("schema", "expected"),
    [
        (True, Satisfiability.YES),
        ({"const": 5}, Satisfiability.YES),
        ({"enum": [1, 2]}, Satisfiability.YES),
        (False, Satisfiability.NO),
        ({"type": "string"}, Satisfiability.YES),
        ({"type": "string", "minLength": 2}, Satisfiability.YES),
        ({"type": "string", "pattern": "^a"}, Satisfiability.YES),
        ({"type": "string", "pattern": "a{100}"}, Satisfiability.UNKNOWN),
    ],
)
def test_satisfiability_answers(schema, expected):
    assert canonicalize(schema).satisfiability() == expected


def test_negate_draft4_integer_type():
    schema = {"$schema": "http://json-schema.org/draft-04/schema#", "type": "integer"}
    result = canonicalize(schema).negate()
    assert isinstance(result, CanonicalSchema)
    assert result.to_json_schema() == {
        "$schema": "http://json-schema.org/draft-04/schema#",
        "anyOf": [
            {"type": ["null", "boolean", "string", "array", "object"]},
            {"type": "number", "not": {"type": "integer"}},
        ],
    }


def test_negate_draft4_typed_group():
    schema = {"$schema": "http://json-schema.org/draft-04/schema#", "type": "integer", "enum": [1, 2]}
    result = canonicalize(schema).negate()
    assert isinstance(result, CanonicalSchema)
    assert result.to_json_schema() == {
        "$schema": "http://json-schema.org/draft-04/schema#",
        "anyOf": [
            {"type": ["null", "boolean", "string", "array", "object"]},
            {"type": "integer", "maximum": 0},
            {"type": "integer", "minimum": 3},
            {"type": "number", "not": {"type": "integer"}},
        ],
    }


def test_negate_integer_leaf():
    result = canonicalize({"type": "integer", "minimum": 0}).negate()
    assert isinstance(result, CanonicalSchema)
    assert result.to_json_schema() == {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "anyOf": [
            {"type": ["null", "boolean", "string", "array", "object"]},
            {"type": "integer", "maximum": -1},
            {"type": "number", "not": {"multipleOf": 1}},
        ],
    }


def test_negate_resolves_a_reference():
    schema = canonicalize({"$defs": {"A": {"type": "string"}}, "$ref": "#/$defs/A"})
    complement = schema.negate()
    assert complement.to_json_schema() == {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": ["null", "boolean", "number", "array", "object"],
    }


def test_registry_resolves_an_external_reference():
    registry = jsonschema_rs.Registry([("https://example.com/external", {"type": "string"})])
    result = canonicalize({"$ref": "https://example.com/external"}, registry=registry)

    assert jsonschema_rs.is_valid(result.to_json_schema(), "value")
    assert not jsonschema_rs.is_valid(result.to_json_schema(), 1)


def test_retriever_fetches_a_reference_absent_from_the_registry():
    def retrieve(uri):
        assert uri == "https://example.com/remote"
        return {"type": "string"}

    result = canonicalize({"$ref": "https://example.com/remote"}, retriever=retrieve)

    assert jsonschema_rs.is_valid(result.to_json_schema(), "value")
    assert not jsonschema_rs.is_valid(result.to_json_schema(), 1)


def test_registry_retriever_is_reused():
    def retrieve(uri):
        assert uri == "https://example.com/remote"
        return {"type": "string"}

    registry = jsonschema_rs.Registry([], retriever=retrieve)
    result = canonicalize({"$ref": "https://example.com/remote"}, registry=registry)

    assert jsonschema_rs.is_valid(result.to_json_schema(), "value")
    assert not jsonschema_rs.is_valid(result.to_json_schema(), 1)


def test_base_uri_resolves_a_relative_reference():
    registry = jsonschema_rs.Registry([("https://example.com/external", {"type": "string"})])
    result = canonicalize({"$ref": "external"}, registry=registry, base_uri="https://example.com/root")

    assert jsonschema_rs.is_valid(result.to_json_schema(), "value")
    assert not jsonschema_rs.is_valid(result.to_json_schema(), 1)


def test_retriever_failure_surfaces():
    def retrieve(uri):
        raise KeyError(f"Schema not found: {uri}")

    with pytest.raises(canonical.CanonicalizationError) as exc:
        canonicalize({"$ref": "https://example.com/remote"}, retriever=retrieve)
    assert "Schema not found" in str(exc.value)


def test_offline_refuses_remote_reference():
    with pytest.raises(ValueError, match="Retrieval is disabled"):
        canonicalize({"$ref": "https://example.com/schema.json"}, offline=True)


def test_offline_rejects_a_retriever():
    def retrieve(uri: str):
        return {}

    with pytest.raises(ValueError, match="`offline` cannot be used together with `retriever`"):
        canonicalize({"$ref": "https://example.com/schema.json"}, offline=True, retriever=retrieve)

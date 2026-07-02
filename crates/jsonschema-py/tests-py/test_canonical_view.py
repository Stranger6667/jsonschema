import pytest

import jsonschema_rs


def test_kind_label():
    schema = jsonschema_rs.canonicalize({"type": "integer"})
    assert schema.kind == "integer"


def test_true_view():
    view = jsonschema_rs.canonicalize(True).view()
    assert isinstance(view, jsonschema_rs.canonical.TrueView)


def test_false_view():
    view = jsonschema_rs.canonicalize(False).view()
    assert isinstance(view, jsonschema_rs.canonical.FalseView)


def test_integer_view():
    # Integer exclusive bounds tighten to the inclusive neighbor: exclusiveMaximum 10 is maximum 9.
    view = jsonschema_rs.canonicalize({"type": "integer", "minimum": 0, "exclusiveMaximum": 10}).view()
    assert isinstance(view, jsonschema_rs.canonical.IntegerView)
    assert view.minimum == 0
    assert view.maximum == 9
    assert view.exclusive_maximum is None
    assert view.exclusive_minimum is None


def test_number_view():
    view = jsonschema_rs.canonicalize({"type": "number", "multipleOf": 0.5}).view()
    assert isinstance(view, jsonschema_rs.canonical.NumberView)
    assert view.multiple_of == 0.5


def test_number_view_keeps_exclusive_bounds():
    # Numbers are dense, so exclusive bounds stay exclusive (unlike integers).
    view = jsonschema_rs.canonicalize({"type": "number", "exclusiveMinimum": 2}).view()
    assert isinstance(view, jsonschema_rs.canonical.NumberView)
    assert view.exclusive_minimum == 2
    assert view.minimum is None


def test_integer_view_exposes_not_multiple_of():
    view = jsonschema_rs.canonicalize({"type": "integer", "not": {"type": "integer", "multipleOf": 3}}).view()
    assert isinstance(view, jsonschema_rs.canonical.IntegerView)
    assert view.not_multiple_of == [3]


def test_number_view_exposes_not_multiple_of():
    view = jsonschema_rs.canonicalize({"type": "number", "not": {"type": "number", "multipleOf": 0.5}}).view()
    assert isinstance(view, jsonschema_rs.canonical.NumberView)
    assert view.not_multiple_of == [0.5]


def test_string_view():
    view = jsonschema_rs.canonicalize({"type": "string", "minLength": 1, "pattern": "^a"}).view()
    assert isinstance(view, jsonschema_rs.canonical.StringView)
    assert view.min_length == 1
    assert view.patterns == ["^a"]


def test_string_view_exposes_not_patterns():
    view = jsonschema_rs.canonicalize({"type": "string", "not": {"type": "string", "pattern": "^a"}}).view()
    assert isinstance(view, jsonschema_rs.canonical.StringView)
    assert view.not_patterns == ["^a"]


@pytest.mark.parametrize(
    "schema",
    [
        {"type": "string", "not": {"pattern": "^a"}},
        {"not": {"pattern": "^a"}},
    ],
)
def test_string_view_exposes_not_patterns_from_untyped_pattern(schema):
    view = jsonschema_rs.canonicalize(schema).view()
    assert isinstance(view, jsonschema_rs.canonical.StringView)
    assert view.not_patterns == ["^a"]


@pytest.mark.parametrize(
    "schema",
    [
        {"type": "array", "not": {"type": "array", "uniqueItems": True}},
        {"not": {"uniqueItems": True}},
        {"type": "array", "not": {"uniqueItems": True}},
    ],
)
def test_array_view_exposes_repeated_items(schema):
    view = jsonschema_rs.canonicalize(schema).view()
    assert isinstance(view, jsonschema_rs.canonical.ArrayView)
    assert view.repeated_items is True
    assert view.unique_items is False
    assert view.min_items == 2


def test_object_view_exposes_additional_property_requirement():
    # `not(additionalProperties: S)` negates to the additional-property existential.
    negated = jsonschema_rs.canonicalize({"type": "object", "additionalProperties": {"type": "integer"}}).negate()
    found = []

    def walk(schema):
        view = schema.view()
        if isinstance(view, jsonschema_rs.canonical.ObjectView):
            found.extend(
                r for r in view.requirements if isinstance(r, jsonschema_rs.canonical.AdditionalPropertiesRequirement)
            )
        for child in getattr(view, "schemas", []):
            walk(child)

    walk(negated)
    assert found, f"no AdditionalPropertiesRequirement in {negated.to_json_schema()}"
    assert isinstance(found[0].schema, jsonschema_rs.CanonicalSchema)


def test_integer_view_keyword_match():
    match jsonschema_rs.canonicalize({"type": "integer", "minimum": 5}).view():
        case jsonschema_rs.canonical.IntegerView(minimum=lo):
            assert lo == 5
        case _:
            raise AssertionError("expected IntegerView")


def test_array_view():
    view = jsonschema_rs.canonicalize(
        {
            "type": "array",
            "prefixItems": [{"type": "integer"}],
            "items": {"type": "string"},
            "minItems": 1,
            "maxItems": 3,
            "uniqueItems": True,
            "contains": {"type": "string"},
            "minContains": 2,
        }
    ).view()
    assert isinstance(view, jsonschema_rs.canonical.ArrayView)
    assert len(view.prefix) == 1
    assert isinstance(view.prefix[0], jsonschema_rs.CanonicalSchema)
    assert isinstance(view.prefix[0].view(), jsonschema_rs.canonical.IntegerView)
    assert isinstance(view.tail.view(), jsonschema_rs.canonical.StringView)
    assert view.min_items == 2  # canonicalizer promotes to max(minItems=1, minContains=2)
    assert view.max_items == 3
    assert view.unique_items is True
    assert len(view.contains) == 1
    assert view.contains[0].min_contains == 2
    assert isinstance(view.contains[0].schema.view(), jsonschema_rs.canonical.StringView)


def test_object_view():
    view = jsonschema_rs.canonicalize(
        {
            "type": "object",
            "properties": {"a": {"type": "integer"}},
            "patternProperties": {"^x": {"type": "string"}},
            "additionalProperties": False,
            "required": ["a"],
            # Above the required count, so the leaf keeps it (minProperties at or below it is implied away)
            "minProperties": 2,
            "maxProperties": 3,
        }
    ).view()
    assert isinstance(view, jsonschema_rs.canonical.ObjectView)
    assert view.min_properties == 2
    assert view.max_properties == 3
    names = {type(c).__name__ for c in view.constraints}
    assert {"NamedPropertyConstraint", "PatternPropertyConstraint", "AdditionalPropertiesConstraint"} <= names
    assert any(isinstance(r, jsonschema_rs.canonical.RequiredProperty) and r.name == "a" for r in view.requirements)


def test_multi_type_view():
    view = jsonschema_rs.canonicalize({"type": ["integer", "string"]}).view()
    assert isinstance(view, jsonschema_rs.canonical.MultiTypeView)
    assert set(view.types) == {"integer", "string"}


def test_any_of_view():
    # Const-only branches fold to Enum; constrained branches produce AnyOf.
    view = jsonschema_rs.canonicalize(
        {"anyOf": [{"type": "integer", "minimum": 0}, {"type": "string", "minLength": 2}]}
    ).view()
    assert isinstance(view, jsonschema_rs.canonical.AnyOfView)
    assert len(view.schemas) == 2
    kinds = {type(s.view()).__name__ for s in view.schemas}
    assert kinds == {"IntegerView", "StringView"}


def test_not_view():
    view = jsonschema_rs.canonicalize({"not": {"type": "integer"}}).view()
    assert isinstance(view, jsonschema_rs.canonical.NotView)
    assert isinstance(view.schema.view(), jsonschema_rs.canonical.IntegerView)


def test_type_guard_view():
    view = jsonschema_rs.canonicalize({"minimum": 5}).view()
    assert isinstance(view, jsonschema_rs.canonical.TypeGuardView)
    assert view.type_name == "number"
    assert isinstance(view.body.view(), jsonschema_rs.canonical.NumberView)


def test_const_view():
    view = jsonschema_rs.canonicalize({"const": "x"}).view()
    assert isinstance(view, jsonschema_rs.canonical.ConstView)
    assert view.value == "x"


def test_enum_view():
    view = jsonschema_rs.canonicalize({"enum": ["a", "b"]}).view()
    assert isinstance(view, jsonschema_rs.canonical.EnumView)
    assert view.values == ["a", "b"]


def test_raw_view():
    # A `$dynamicRef` resolves against the runtime dynamic scope, which the
    # structural IR does not model, so the schema is preserved raw.
    schema = {
        "$dynamicRef": "#node",
        "$defs": {"node": {"$dynamicAnchor": "node", "type": "object"}},
    }
    view = jsonschema_rs.canonicalize(schema).view()
    assert isinstance(view, jsonschema_rs.canonical.RawView)
    assert view.schema == jsonschema_rs.canonicalize(schema).to_json_schema()


def test_stub_attributes_exist():
    schema = jsonschema_rs.canonicalize({"type": "integer", "minimum": 0})
    assert schema.kind == "integer"
    view = schema.view()
    assert isinstance(view, jsonschema_rs.canonical.IntegerView)
    assert view.minimum == 0


def test_inline_budget_emits_shared_ref_symbolic():
    schema = {
        "type": "object",
        "properties": {"x": {"$ref": "#/$defs/shared"}, "y": {"$ref": "#/$defs/shared"}},
        "$defs": {"shared": {"type": "string", "minLength": 3}},
    }
    # Finite budget: references are symbolic, with one definition entry.
    budgeted = jsonschema_rs.canonicalize(schema, inline_budget=0)
    view = budgeted.view()
    assert isinstance(view, jsonschema_rs.canonical.ObjectView)
    x = next(c for c in view.constraints if getattr(c, "name", None) == "x")
    assert isinstance(x.schema.view(), jsonschema_rs.canonical.ReferenceView)
    assert x.schema.view().uri == "#/$defs/shared"
    defs = budgeted.definitions()
    assert set(defs) == {"#/$defs/shared"}
    assert isinstance(defs["#/$defs/shared"], jsonschema_rs.CanonicalSchema)

    # Default (infinite) budget: fully inlined, no definitions.
    inlined = jsonschema_rs.canonicalize(schema)
    view = inlined.view()
    x_inlined = next(c for c in view.constraints if getattr(c, "name", None) == "x")
    assert isinstance(x_inlined.schema.view(), jsonschema_rs.canonical.StringView)
    assert inlined.definitions() == {}


def test_definitions_transitive_closure_and_recursive_uri():
    # Mutual recursion A.b -> B, B.a -> A, plus a dangling ref.
    schema = {
        "$ref": "#/$defs/A",
        "$defs": {
            "A": {"type": "object", "properties": {"b": {"$ref": "#/$defs/B"}, "x": {"$ref": "#/$defs/missing"}}},
            "B": {"type": "object", "properties": {"a": {"$ref": "#/$defs/A"}}},
        },
    }
    defs = jsonschema_rs.canonicalize(schema, inline_budget=0).definitions()
    # Both reachable uris are keys; the dangling one is absent.
    assert "#/$defs/A" in defs
    assert "#/$defs/B" in defs
    assert "#/$defs/missing" not in defs


def test_recursive_view_exposes_uri():
    # Static cycle survives to a RecursiveView keyed by its target uri.
    schema = jsonschema_rs.canonicalize(
        {"type": "object", "properties": {"next": {"$ref": "#"}}},
        inline_budget=0,
    )
    view = schema.view()
    assert isinstance(view, jsonschema_rs.canonical.ObjectView)
    next_c = next(c for c in view.constraints if getattr(c, "name", None) == "next")
    ref = next_c.schema.view()
    # `#` resolves to the document root; its uri keys into definitions().
    assert isinstance(ref, (jsonschema_rs.canonical.ReferenceView, jsonschema_rs.canonical.RecursiveView))
    assert ref.uri == "#"


def test_intersect_preserves_definitions_self_contained():
    left = jsonschema_rs.canonicalize(
        {
            "type": "object",
            "properties": {"kids": {"type": "array", "items": {"$ref": "#/definitions/Root"}}},
            "definitions": {"Root": {"type": "integer"}},
        },
        inline_budget=0,
    )
    right = jsonschema_rs.canonicalize({"type": "object"})
    merged = left.intersect(right)
    # The surviving Reference carries its definition, so emitted schemas are self-contained.
    assert "#/definitions/Root" in merged.definitions()
    # validator_for raises on a dangling pointer; this must not.
    jsonschema_rs.validator_for(merged.to_json_schema())


def test_negate_is_sound():
    schema = jsonschema_rs.canonicalize({"type": "integer"})
    negated = schema.negate()
    assert isinstance(negated, jsonschema_rs.CanonicalSchema)
    neg = negated.to_json_schema()
    # Integers were valid originally -> rejected by the negation.
    assert not jsonschema_rs.is_valid(neg, 5)
    # Non-integers were invalid originally -> accepted by the negation.
    assert jsonschema_rs.is_valid(neg, "hello")
    assert jsonschema_rs.is_valid(neg, True)


def test_subtract_is_set_difference():
    all_ints = jsonschema_rs.canonicalize({"type": "integer"})
    non_neg = jsonschema_rs.canonicalize({"type": "integer", "minimum": 0})
    # self \ self = empty
    assert not all_ints.subtract(all_ints).is_satisfiable()
    # all_ints \ non_neg = negative integers (non-empty)
    assert all_ints.subtract(non_neg).is_satisfiable()
    # non_neg \ all_ints = empty
    assert not non_neg.subtract(all_ints).is_satisfiable()


def test_union_is_set_union():
    integer = jsonschema_rs.canonicalize({"type": "integer"})
    string = jsonschema_rs.canonicalize({"type": "string"})
    union = integer.union(string)
    assert isinstance(union, jsonschema_rs.CanonicalSchema)
    emitted = union.to_json_schema()
    assert jsonschema_rs.is_valid(emitted, 5)
    assert jsonschema_rs.is_valid(emitted, "hello")
    assert not jsonschema_rs.is_valid(emitted, True)


def test_is_subschema_of():
    min5 = jsonschema_rs.canonicalize({"type": "integer", "minimum": 5})
    min0 = jsonschema_rs.canonicalize({"type": "integer", "minimum": 0})
    assert min5.is_subschema_of(min0) is True
    assert min0.is_subschema_of(min5) is False
    assert jsonschema_rs.canonicalize(False).is_subschema_of(min0) is True
    assert min0.is_subschema_of(jsonschema_rs.canonicalize(True)) is True


def test_string_view_positional_match():
    match jsonschema_rs.canonicalize({"type": "string", "minLength": 1, "maxLength": 5}).view():
        case jsonschema_rs.canonical.StringView(lo, hi):
            assert lo == 1
            assert hi == 5
        case _:
            raise AssertionError("expected StringView")


def test_multi_type_view_positional_match():
    match jsonschema_rs.canonicalize({"type": ["integer", "string"]}).view():
        case jsonschema_rs.canonical.MultiTypeView(types):
            assert set(types) == {"integer", "string"}
        case _:
            raise AssertionError("expected MultiTypeView")


def test_enum_view_positional_match():
    match jsonschema_rs.canonicalize({"enum": ["a", "b"]}).view():
        case jsonschema_rs.canonical.EnumView(values):
            assert values == ["a", "b"]
        case _:
            raise AssertionError("expected EnumView")


def test_not_view_positional_match():
    match jsonschema_rs.canonicalize({"not": {"type": "integer"}}).view():
        case jsonschema_rs.canonical.NotView(inner):
            assert isinstance(inner.view(), jsonschema_rs.canonical.IntegerView)
        case _:
            raise AssertionError("expected NotView")


def test_object_view_positional_match():
    schema = {"type": "object", "required": ["a"], "properties": {"a": {"type": "integer"}}}
    match jsonschema_rs.canonicalize(schema).view():
        case jsonschema_rs.canonical.ObjectView(requirements, constraints):
            assert any(isinstance(r, jsonschema_rs.canonical.RequiredProperty) for r in requirements)
            assert any(isinstance(c, jsonschema_rs.canonical.NamedPropertyConstraint) for c in constraints)
        case _:
            raise AssertionError("expected ObjectView")


@pytest.mark.parametrize("other", [42, "schema", None, {"type": "integer"}])
def test_equality_with_other_types(other):
    schema = jsonschema_rs.canonicalize({"type": "integer"})
    assert schema != other
    assert not (schema == other)  # noqa: SIM201

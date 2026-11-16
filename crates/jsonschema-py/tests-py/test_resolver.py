import pytest

from jsonschema_rs import Draft202012, ReferencingError, Registry

NESTED_RESOURCES = [
    (
        "https://example.com/address.json",
        {
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {"street": {"type": "string"}, "city": {"type": "string"}},
            "required": ["street", "city"],
        },
    ),
    (
        "https://example.com/person.json",
        {
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {"name": {"type": "string"}, "address": {"$ref": "address.json"}},
            "required": ["name", "address"],
        },
    ),
]

ANCHOR_RESOURCE = [
    (
        "https://example.com/anchors.json",
        {
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": {
                "thing": {
                    "$anchor": "thing",
                    "type": "string",
                }
            },
        },
    )
]


def test_registry_resolver_returns_base_uri_and_lookup_api():
    registry = Registry(NESTED_RESOURCES)

    resolver = registry.resolver("https://example.com/person.json")

    assert resolver.base_uri == "https://example.com/person.json"
    assert resolver.dynamic_scope == ()


def test_resolver_lookup_returns_contents_follow_up_resolver_and_draft():
    registry = Registry(NESTED_RESOURCES)

    root = registry.resolver("https://example.com/person.json")
    resolved_root = root.lookup("")

    assert resolved_root.contents["properties"]["address"]["$ref"] == "address.json"
    assert resolved_root.draft == Draft202012
    assert resolved_root.resolver.base_uri == "https://example.com/person.json"
    assert resolved_root.resolver.dynamic_scope == ("https://example.com/person.json",)

    ref = resolved_root.contents["properties"]["address"]["$ref"]
    resolved_address = resolved_root.resolver.lookup(ref)

    assert resolved_address.contents["required"] == ["street", "city"]
    assert resolved_address.resolver.base_uri == "https://example.com/address.json"
    assert resolved_address.resolver.dynamic_scope == (
        "https://example.com/person.json",
        "https://example.com/person.json",
    )


def test_resolver_lookup_supports_retriever_backed_resources():
    def retrieve(uri: str):
        if uri == "https://example.com/dynamic.json":
            return {"type": "number"}
        raise KeyError(f"Schema not found: {uri}")

    registry = Registry(
        [("https://example.com/inner.json", {"$ref": "https://example.com/dynamic.json"})],
        retriever=retrieve,
    )

    root = registry.resolver("https://example.com/inner.json")
    resolved_inner = root.lookup("")
    resolved_dynamic = resolved_inner.resolver.lookup(resolved_inner.contents["$ref"])

    assert resolved_dynamic.contents == {"type": "number"}
    assert resolved_dynamic.resolver.base_uri == "https://example.com/dynamic.json"


def test_resolver_lookup_supports_pointers_and_anchors():
    registry = Registry(ANCHOR_RESOURCE)
    resolver = registry.resolver("https://example.com/anchors.json")

    pointer = resolver.lookup("#/$defs/thing")
    anchor = resolver.lookup("#thing")

    assert pointer.contents == {"$anchor": "thing", "type": "string"}
    assert anchor.contents == {"$anchor": "thing", "type": "string"}
    assert anchor.resolver.base_uri == "https://example.com/anchors.json"


SAME_NAME_DYNAMIC_ANCHORS = [
    (
        "https://example.com/root",
        {
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": {
                "outer": {"$dynamicAnchor": "item", "type": "null"},
                "inner": {"$id": "inner", "$dynamicAnchor": "item", "type": "string"},
            },
        },
    )
]


@pytest.mark.parametrize(
    ("method", "expected", "base_uri"),
    [
        ("lookup", {"$id": "inner", "$dynamicAnchor": "item", "type": "string"}, "https://example.com/inner"),
        ("lookup_dynamic_ref", {"$dynamicAnchor": "item", "type": "null"}, "https://example.com/root"),
    ],
)
def test_resolver_lookup_of_dynamic_anchor(method, expected, base_uri):
    registry = Registry(SAME_NAME_DYNAMIC_ANCHORS)
    resolver = registry.resolver("https://example.com/root")

    resolved = getattr(resolver, method)("inner#item")

    assert resolved.contents == expected
    assert resolved.resolver.base_uri == base_uri


def test_resolver_lookup_raises_referencing_error_for_missing_resource():
    registry = Registry(NESTED_RESOURCES)
    resolver = registry.resolver("https://example.com/person.json")

    with pytest.raises(ReferencingError) as exc:
        resolver.lookup("https://example.com/missing.json")

    assert "missing.json" in str(exc.value)

//! Anchors identify sub-schemas within a document by name.
//!
//! JSON Schema defines two anchor flavors:
//! - [`Anchor::Default`]: `$anchor`, or `#name` in `id`/`$id` before Draft 2019-09; resolved
//!   against the current base URI.
//! - [`Anchor::Dynamic`]: a dynamic anchor (`$dynamicAnchor`), which `$dynamicRef` re-anchors to
//!   the outermost matching dynamic anchor found in the dynamic scope.
//!
//! A schema object has at most two anchors, so [`AnchorIter`] needs no heap allocation.

use std::sync::Arc;

use fluent_uri::Uri;
use serde_json::Value;

use crate::{Draft, Error, Resolved, Resolver, ResourceRef};

/// An anchor within a resource.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Anchor<'a> {
    Default {
        name: &'a str,
        resource: ResourceRef<'a>,
    },
    Dynamic {
        name: &'a str,
        resource: ResourceRef<'a>,
    },
}

impl<'a> Anchor<'a> {
    #[inline]
    pub(crate) fn name(&self) -> &'a str {
        match self {
            Anchor::Default { name, .. } | Anchor::Dynamic { name, .. } => name,
        }
    }
}

/// The outermost resource with a `$dynamicAnchor` called `name` among the dynamic scope and the
/// resource `caller` is based at; `resource`, in `document`, when none has one.
pub(crate) fn resolve_dynamic<'r>(
    name: &str,
    resource: ResourceRef<'r>,
    document: &Arc<Uri<String>>,
    caller: &Resolver<'r>,
) -> Result<Resolved<'r>, Error> {
    let base_uri = caller.base_uri();
    let scope = caller.dynamic_scope();
    let mut last = resource;
    let mut found_in = None;
    // Innermost first, so the last match is the outermost
    for uri in std::iter::once(&base_uri).chain(scope.iter_shared()) {
        match caller.lookup_anchor(uri, name) {
            Ok(anchor) => {
                if let Anchor::Dynamic { resource, .. } = anchor {
                    last = resource;
                    found_in = Some(uri);
                }
            }
            Err(Error::NoSuchAnchor { .. }) => {}
            Err(err) => return Err(err),
        }
    }
    // The anchor's own document is the base for references inside it
    let landed = Arc::clone(found_in.unwrap_or(document));
    Ok(Resolved::new(
        last.contents(),
        caller.dynamic_ref_lands_in(landed).in_subresource(last)?,
        last.draft(),
    ))
}

/// An iterator over the anchors of one schema object.
pub(crate) enum AnchorIter<'a> {
    Empty,
    One(Anchor<'a>),
    Two(Anchor<'a>, Anchor<'a>),
}

impl<'a> Iterator for AnchorIter<'a> {
    type Item = Anchor<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        match std::mem::replace(self, AnchorIter::Empty) {
            AnchorIter::Empty => None,
            AnchorIter::One(anchor) => Some(anchor),
            AnchorIter::Two(first, second) => {
                *self = AnchorIter::One(second);
                Some(first)
            }
        }
    }
}

pub(crate) fn anchor(draft: Draft, contents: &Value) -> AnchorIter<'_> {
    let Some(schema) = contents.as_object() else {
        return AnchorIter::Empty;
    };

    let default_anchor =
        schema
            .get("$anchor")
            .and_then(Value::as_str)
            .map(|name| Anchor::Default {
                name,
                resource: ResourceRef::new(contents, draft),
            });

    let dynamic_anchor = schema
        .get("$dynamicAnchor")
        .and_then(Value::as_str)
        .map(|name| Anchor::Dynamic {
            name,
            resource: ResourceRef::new(contents, draft),
        });

    match (default_anchor, dynamic_anchor) {
        (Some(default), Some(dynamic)) => AnchorIter::Two(default, dynamic),
        (Some(default), None) => AnchorIter::One(default),
        (None, Some(dynamic)) => AnchorIter::One(dynamic),
        (None, None) => AnchorIter::Empty,
    }
}

pub(crate) fn anchor_2019(draft: Draft, contents: &Value) -> AnchorIter<'_> {
    match contents
        .as_object()
        .and_then(|schema| schema.get("$anchor"))
        .and_then(Value::as_str)
    {
        Some(name) => AnchorIter::One(Anchor::Default {
            name,
            resource: ResourceRef::new(contents, draft),
        }),
        None => AnchorIter::Empty,
    }
}

pub(crate) fn legacy_anchor_in_dollar_id(draft: Draft, contents: &Value) -> AnchorIter<'_> {
    match contents
        .as_object()
        .and_then(|schema| schema.get("$id"))
        .and_then(Value::as_str)
        .and_then(|id| id.strip_prefix('#'))
    {
        Some(id) => AnchorIter::One(Anchor::Default {
            name: id,
            resource: ResourceRef::new(contents, draft),
        }),
        None => AnchorIter::Empty,
    }
}

pub(crate) fn legacy_anchor_in_id(draft: Draft, contents: &Value) -> AnchorIter<'_> {
    match contents
        .as_object()
        .and_then(|schema| schema.get("id"))
        .and_then(Value::as_str)
        .and_then(|id| id.strip_prefix('#'))
    {
        Some(id) => AnchorIter::One(Anchor::Default {
            name: id,
            resource: ResourceRef::new(contents, draft),
        }),
        None => AnchorIter::Empty,
    }
}

#[cfg(test)]
mod tests {
    use crate::{Draft, Registry};
    use fluent_uri::Uri;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test]
    fn test_lookup_trivial_dynamic_ref() {
        let one = Draft::Draft202012.create_resource(json!({"$dynamicAnchor": "foo"}));
        let registry = Registry::new()
            .add("http://example.com", &one)
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");
        let resolver = registry
            .resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));
        let resolved = resolver.lookup_dynamic_ref("#foo").expect("Lookup failed");
        assert_eq!(resolved.contents(), one.contents());
    }

    #[test]
    fn test_multiple_lookup_trivial_dynamic_ref() {
        let true_resource = Draft::Draft202012.create_resource(json!(true));
        let root = Draft::Draft202012.create_resource(json!({
            "$id": "http://example.com",
            "$dynamicAnchor": "fooAnchor",
            "$defs": {
                "foo": {
                    "$id": "foo",
                    "$dynamicAnchor": "fooAnchor",
                    "$defs": {
                        "bar": true,
                        "baz": {
                            "$dynamicAnchor": "fooAnchor",
                        },
                    },
                },
            },
        }));

        let registry = Registry::new()
            .extend([
                ("http://example.com", &root),
                ("http://example.com/foo/", &true_resource),
                ("http://example.com/foo/bar", &root),
            ])
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");
        let resolver = registry
            .resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

        let first = resolver.lookup("").expect("Lookup failed");
        let second = first.resolver().lookup("foo/").expect("Lookup failed");
        let third = second.resolver().lookup("bar").expect("Lookup failed");
        let fourth = third
            .resolver()
            .lookup_dynamic_ref("#fooAnchor")
            .expect("Lookup failed");
        assert_eq!(fourth.contents(), root.contents());
        assert_eq!(format!("{:?}", fourth.resolver()), "Resolver { base_uri: \"http://example.com\", scopes: \"[http://example.com/foo/, http://example.com, http://example.com]\" }");
    }

    #[test]
    fn dynamic_anchor_in_another_document_uses_that_document_as_base() {
        let root = Draft::Draft202012.create_resource(json!({
            "$id": "https://example.com/root.json",
            "$defs": {"node": {"$dynamicAnchor": "node"}}
        }));
        let tree = Draft::Draft202012.create_resource(json!({
            "$id": "https://example.com/tree.json",
            "$dynamicAnchor": "node"
        }));
        let registry = Registry::new()
            .extend([
                ("https://example.com/root.json", &root),
                ("https://example.com/tree.json", &tree),
            ])
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");
        let resolver = registry.resolver(
            crate::uri::from_str("https://example.com/root.json").expect("Invalid base URI"),
        );

        let tree = resolver.lookup("tree.json").expect("Lookup failed");
        let node = tree
            .resolver()
            .lookup_dynamic_ref("#node")
            .expect("Lookup failed");

        assert_eq!(node.contents(), &root.contents()["$defs"]["node"]);
        assert_eq!(
            node.resolver().base_uri().as_str(),
            "https://example.com/root.json"
        );
    }

    #[test]
    fn lookup_of_a_dynamic_anchor_ignores_the_dynamic_scope() {
        let root = Draft::Draft202012.create_resource(json!({
            "$id": "https://example.com/root",
            "$defs": {
                "outer": {"$dynamicAnchor": "item", "type": "null"},
                "inner": {"$id": "inner", "$dynamicAnchor": "item", "type": "string"}
            }
        }));
        let registry = Registry::new()
            .add("https://example.com/root", &root)
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");
        let resolver = registry
            .resolver(crate::uri::from_str("https://example.com/root").expect("Invalid base URI"));

        let resolved = resolver.lookup("inner#item").expect("Lookup failed");

        assert_eq!(resolved.contents(), &root.contents()["$defs"]["inner"]);
        assert_eq!(
            resolved.resolver().base_uri().as_str(),
            "https://example.com/inner"
        );

        let resolved = resolver
            .lookup_dynamic_ref("inner#item")
            .expect("Lookup failed");

        assert_eq!(resolved.contents(), &root.contents()["$defs"]["outer"]);
        assert_eq!(
            resolved.resolver().base_uri().as_str(),
            "https://example.com/root"
        );
    }

    // `caller`'s `$dynamicRef "#a"` lands in `root`. `caller` stays in the dynamic scope there when
    // it declares a `$dynamicAnchor` no resource in the scope declares, so `leaf`'s `"#b"` binds it.
    #[test_case(
        &json!({"$dynamicAnchor": "b", "type": "string"}),
        &["https://example.com/caller", "https://example.com/root"],
        &json!({"$dynamicAnchor": "b", "type": "string"});
        "caller declares a name the scope lacks"
    )]
    #[test_case(
        &json!({"type": "string"}),
        &["https://example.com/root"],
        &json!({"$dynamicAnchor": "b", "type": "integer"});
        "caller declares only names the scope has"
    )]
    fn dynamic_ref_landing_outside_its_resource(b: &Value, scope: &[&str], bound: &Value) {
        let root = Draft::Draft202012.create_resource(json!({
            "$id": "https://example.com/root",
            "$defs": {
                "a": {"$dynamicAnchor": "a", "$ref": "leaf"},
                "caller": {
                    "$id": "caller",
                    "$dynamicRef": "#a",
                    "$defs": {"a": {"$dynamicAnchor": "a", "type": "null"}, "b": b}
                },
                "leaf": {
                    "$id": "leaf",
                    "$dynamicRef": "#b",
                    "$defs": {"b": {"$dynamicAnchor": "b", "type": "integer"}}
                }
            }
        }));
        let registry = Registry::new()
            .add("https://example.com/root", &root)
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");
        let resolver = registry
            .resolver(crate::uri::from_str("https://example.com/root").expect("Invalid base URI"));

        let caller = resolver.lookup("caller").expect("Lookup failed");
        let landed = caller
            .resolver()
            .lookup_dynamic_ref("#a")
            .expect("Lookup failed");
        assert_eq!(landed.contents(), &root.contents()["$defs"]["a"]);
        assert_eq!(
            landed
                .resolver()
                .dynamic_scope()
                .iter()
                .map(Uri::as_str)
                .collect::<Vec<_>>(),
            scope
        );

        let leaf = landed.resolver().lookup("leaf").expect("Lookup failed");
        let bound_to = leaf
            .resolver()
            .lookup_dynamic_ref("#b")
            .expect("Lookup failed");
        assert_eq!(bound_to.contents(), bound);
    }

    #[test]
    fn test_multiple_lookup_dynamic_ref_to_nondynamic_ref() {
        let one = Draft::Draft202012.create_resource(json!({"$anchor": "fooAnchor"}));
        let two = Draft::Draft202012.create_resource(json!({
            "$id": "http://example.com",
            "$dynamicAnchor": "fooAnchor",
            "$defs": {
                "foo": {
                    "$id": "foo",
                    "$dynamicAnchor": "fooAnchor",
                    "$defs": {
                        "bar": true,
                        "baz": {
                            "$dynamicAnchor": "fooAnchor",
                        },
                    },
                },
            },
        }));

        let registry = Registry::new()
            .extend([
                ("http://example.com", &two),
                ("http://example.com/foo/", &one),
                ("http://example.com/foo/bar", &two),
            ])
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");
        let resolver = registry
            .resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

        let first = resolver.lookup("").expect("Lookup failed");
        let second = first.resolver().lookup("foo/").expect("Lookup failed");
        let third = second.resolver().lookup("bar").expect("Lookup failed");
        let fourth = third
            .resolver()
            .lookup_dynamic_ref("#fooAnchor")
            .expect("Lookup failed");
        assert_eq!(fourth.contents(), two.contents());
    }

    #[test]
    fn test_unknown_anchor() {
        let schema = Draft::Draft202012.create_resource(json!({
            "$defs": {
                "foo": { "$anchor": "knownAnchor" }
            }
        }));
        let registry = Registry::new()
            .add("http://example.com", schema)
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");
        let resolver = registry
            .resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

        let result = resolver.lookup("#unknownAnchor");
        assert_eq!(
            result.expect_err("Should fail").to_string(),
            "Anchor 'unknownAnchor' does not exist"
        );
    }

    #[test]
    fn test_invalid_anchor_with_slash() {
        let schema = Draft::Draft202012.create_resource(json!({
            "$defs": {
                "foo": { "$anchor": "knownAnchor" }
            }
        }));
        let registry = Registry::new()
            .add("http://example.com", schema)
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");
        let resolver = registry
            .resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

        let result = resolver.lookup("#invalid/anchor");
        assert_eq!(
            result.expect_err("Should fail").to_string(),
            "Anchor 'invalid/anchor' is invalid"
        );
    }

    #[test]
    fn test_lookup_trivial_recursive_ref() {
        let resource = Draft::Draft201909.create_resource(json!({"$recursiveAnchor": true}));
        let registry = Registry::new()
            .add("http://example.com", &resource)
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");
        let resolver = registry
            .resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));
        let first = resolver.lookup("").expect("Lookup failed");
        let resolved = first
            .resolver()
            .lookup_recursive_ref()
            .expect("Lookup failed");
        assert_eq!(resolved.contents(), resource.contents());
    }

    #[test]
    fn test_lookup_recursive_ref_to_bool() {
        let true_resource = Draft::Draft201909.create_resource(json!(true));
        let registry = Registry::new()
            .add("http://example.com", &true_resource)
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");
        let resolver = registry
            .resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));
        let resolved = resolver.lookup_recursive_ref().expect("Lookup failed");
        assert_eq!(resolved.contents(), true_resource.contents());
    }

    #[test]
    fn test_multiple_lookup_recursive_ref_to_bool() {
        let true_resource = Draft::Draft201909.create_resource(json!(true));
        let root = Draft::Draft201909.create_resource(json!({
            "$id": "http://example.com",
            "$recursiveAnchor": true,
            "$defs": {
                "foo": {
                    "$id": "foo",
                    "$recursiveAnchor": true,
                    "$defs": {
                        "bar": true,
                        "baz": {
                            "$recursiveAnchor": true,
                            "$anchor": "fooAnchor",
                        },
                    },
                },
            },
        }));

        let registry = Registry::new()
            .extend([
                ("http://example.com", &root),
                ("http://example.com/foo/", &true_resource),
                ("http://example.com/foo/bar", &root),
            ])
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");
        let resolver = registry
            .resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));
        let first = resolver.lookup("").expect("Lookup failed");
        let second = first.resolver().lookup("foo/").expect("Lookup failed");
        let third = second.resolver().lookup("bar").expect("Lookup failed");
        let fourth = third
            .resolver()
            .lookup_recursive_ref()
            .expect("Lookup failed");
        assert_eq!(fourth.contents(), root.contents());
    }

    #[test]
    fn test_multiple_lookup_recursive_ref_with_nonrecursive_ref() {
        let one = Draft::Draft201909.create_resource(json!({"$recursiveAnchor": true}));
        let two = Draft::Draft201909.create_resource(json!({
            "$recursiveAnchor": true,
            "$defs": {
                "foo": {
                    "$id": "foo",
                    "$recursiveAnchor": true,
                    "$defs": {
                        "bar": true,
                        "baz": {
                            "$recursiveAnchor": true,
                            "$anchor": "fooAnchor",
                        },
                    },
                },
            },
        }));
        let three = Draft::Draft201909.create_resource(json!({"$recursiveAnchor": false}));

        let registry = Registry::new()
            .extend([
                ("http://example.com", &three),
                ("http://example.com/foo/", &two),
                ("http://example.com/foo/bar", &one),
            ])
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");
        let resolver = registry
            .resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));
        let first = resolver.lookup("").expect("Lookup failed");
        let second = first.resolver().lookup("foo/").expect("Lookup failed");
        let third = second.resolver().lookup("bar").expect("Lookup failed");
        let fourth = third
            .resolver()
            .lookup_recursive_ref()
            .expect("Lookup failed");
        assert_eq!(fourth.contents(), two.contents());
    }

    // A resource without `$recursiveAnchor: true` between two that have it does not end the walk.
    #[test_case(json!({}) ; "middle without the anchor")]
    #[test_case(json!({"$recursiveAnchor": false}) ; "middle with the anchor false")]
    fn test_lookup_recursive_ref_past_unanchored_resource(middle: Value) {
        let outer = Draft::Draft201909
            .create_resource(json!({"$recursiveAnchor": true, "type": "boolean"}));
        let middle = Draft::Draft201909.create_resource(middle);
        let inner = Draft::Draft201909
            .create_resource(json!({"$recursiveAnchor": true, "type": "integer"}));

        let registry = Registry::new()
            .extend([
                ("http://example.com/outer", &outer),
                ("http://example.com/middle", &middle),
                ("http://example.com/inner", &inner),
            ])
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");
        let resolver = registry
            .resolver(crate::uri::from_str("http://example.com/outer").expect("Invalid base URI"));
        let first = resolver.lookup("").expect("Lookup failed");
        let second = first.resolver().lookup("middle").expect("Lookup failed");
        let third = second.resolver().lookup("inner").expect("Lookup failed");
        let resolved = third
            .resolver()
            .lookup_recursive_ref()
            .expect("Lookup failed");
        assert_eq!(resolved.contents(), outer.contents());
    }
}

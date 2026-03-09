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
    /// Anchor's name.
    pub(crate) fn name(&self) -> &'a str {
        match self {
            Anchor::Default { name, .. } | Anchor::Dynamic { name, .. } => name,
        }
    }
}

impl<'r> Anchor<'r> {
    /// Get the resource for this anchor.
    pub(crate) fn resolve(&self, resolver: Resolver<'r>) -> Result<Resolved<'r>, Error> {
        match self {
            Anchor::Default { resource, .. } => Ok(Resolved::new(
                resource.contents(),
                resolver,
                resource.draft(),
            )),
            Anchor::Dynamic { name, resource } => {
                let mut last = *resource;
                for uri in &resolver.dynamic_scope() {
                    match resolver.index.anchor(uri, name) {
                        Ok(anchor) => {
                            if let Anchor::Dynamic { resource, .. } = anchor {
                                last = *resource;
                            }
                        }
                        Err(Error::NoSuchAnchor { .. }) => {}
                        Err(err) => return Err(err),
                    }
                }
                Ok(Resolved::new(
                    last.contents(),
                    resolver.in_subresource(last)?,
                    last.draft(),
                ))
            }
        }
    }
}

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

    // First check for top-level anchors
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
    use serde_json::json;

    #[test]
    fn test_lookup_trivial_dynamic_ref() {
        let one = Draft::Draft202012.create_resource(json!({"$dynamicAnchor": "foo"}));
        let registry =
            Registry::try_new("http://example.com", one.clone()).expect("Invalid resources");
        let index = registry.build_index().expect("Invalid index");
        let resolver =
            index.resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));
        let resolved = resolver.lookup("#foo").expect("Lookup failed");
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

        let registry = Registry::try_from_resources([
            ("http://example.com".to_string(), root.clone()),
            ("http://example.com/foo/".to_string(), true_resource),
            ("http://example.com/foo/bar".to_string(), root.clone()),
        ])
        .expect("Invalid resources");
        let index = registry.build_index().expect("Invalid index");
        let resolver =
            index.resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

        let first = resolver.lookup("").expect("Lookup failed");
        let second = first.resolver().lookup("foo/").expect("Lookup failed");
        let third = second.resolver().lookup("bar").expect("Lookup failed");
        let fourth = third
            .resolver()
            .lookup("#fooAnchor")
            .expect("Lookup failed");
        assert_eq!(fourth.contents(), root.contents());
        assert_eq!(format!("{:?}", fourth.resolver()), "Resolver { base_uri: \"http://example.com\", scopes: \"[http://example.com/foo/, http://example.com, http://example.com]\" }");
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

        let registry = Registry::try_from_resources([
            ("http://example.com".to_string(), two.clone()),
            ("http://example.com/foo/".to_string(), one),
            ("http://example.com/foo/bar".to_string(), two.clone()),
        ])
        .expect("Invalid resources");
        let index = registry.build_index().expect("Invalid index");
        let resolver =
            index.resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

        let first = resolver.lookup("").expect("Lookup failed");
        let second = first.resolver().lookup("foo/").expect("Lookup failed");
        let third = second.resolver().lookup("bar").expect("Lookup failed");
        let fourth = third
            .resolver()
            .lookup("#fooAnchor")
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
        let registry = Registry::try_new("http://example.com", schema).expect("Invalid resources");
        let index = registry.build_index().expect("Invalid index");
        let resolver =
            index.resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

        let result = resolver.lookup("#unknownAnchor");
        assert_eq!(
            result.expect_err("Should fail").to_string(),
            "Anchor 'unknownAnchor' does not exist"
        );
    }
}

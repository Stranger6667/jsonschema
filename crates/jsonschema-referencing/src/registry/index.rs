//! Lookup structures produced by the build pass.
use std::sync::Arc;

use fluent_uri::Uri;

use serde_json::Value;

use crate::{
    anchor::Anchor, draft::Draft, pointer::ParsedPointer, small_map::SmallMap, ResourceRef,
};

use super::build::StoredResource;

/// Lookup tables mapping canonical URIs to resources and anchors.
#[derive(Debug, Clone, Default)]
pub(super) struct Index<'a> {
    pub(super) resources: SmallMap<Arc<Uri<String>>, IndexedResource<'a>>,
    pub(super) anchors: SmallMap<Arc<Uri<String>>, SmallMap<Box<str>, IndexedAnchor<'a>>>,
}

impl<'a> Index<'a> {
    /// Register a document: insert its resource entry and all its anchors.
    pub(super) fn register_document(
        &mut self,
        key: &Arc<Uri<String>>,
        document: &Arc<StoredResource<'a>>,
    ) {
        if let Some(contents) = document.borrowed_contents() {
            self.register_borrowed_subresource(key, document.draft(), true, contents);
        } else {
            let pointer = ParsedPointer::default();
            self.register_owned_subresource(
                key,
                document,
                &pointer,
                document.draft(),
                true,
                document.contents(),
            );
        }
    }

    /// Register a subresource discovered during BFS traversal of a borrowed document.
    /// If `has_id` is true, the subresource is also registered as a resource entry.
    pub(super) fn register_borrowed_subresource(
        &mut self,
        key: &Arc<Uri<String>>,
        draft: Draft,
        has_id: bool,
        contents: &'a Value,
    ) {
        if has_id {
            self.resources.insert(
                Arc::clone(key),
                IndexedResource::Borrowed(ResourceRef::new(contents, draft)),
            );
        }
        let anchors = self.anchors.get_or_insert_default(Arc::clone(key));
        for anchor in draft.anchors(contents) {
            anchors.insert(
                anchor.name().to_string().into_boxed_str(),
                IndexedAnchor::Borrowed(anchor),
            );
        }
    }

    /// Register a subresource discovered during BFS traversal of an owned document.
    /// If `has_id` is true, the subresource is also registered as a resource entry.
    pub(super) fn register_owned_subresource(
        &mut self,
        key: &Arc<Uri<String>>,
        document: &Arc<StoredResource<'a>>,
        pointer: &ParsedPointer,
        draft: Draft,
        has_id: bool,
        contents: &Value,
    ) {
        if has_id {
            self.resources.insert(
                Arc::clone(key),
                IndexedResource::Owned {
                    document: Arc::clone(document),
                    pointer: pointer.clone(),
                    draft,
                },
            );
        }
        let anchors = self.anchors.get_or_insert_default(Arc::clone(key));
        for anchor in draft.anchors(contents) {
            let (name, kind) = match anchor {
                Anchor::Default { name, .. } => (name, AnchorKind::Default),
                Anchor::Dynamic { name, .. } => (name, AnchorKind::Dynamic),
            };
            let name = name.to_string().into_boxed_str();
            anchors.insert(
                name.clone(),
                IndexedAnchor::Owned {
                    document: Arc::clone(document),
                    pointer: pointer.clone(),
                    draft,
                    kind,
                    name,
                },
            );
        }
    }
}

/// A schema resource in the index: either borrowed from the caller or owned by the registry.
#[derive(Debug, Clone)]
pub(super) enum IndexedResource<'a> {
    Borrowed(ResourceRef<'a>),
    Owned {
        document: Arc<StoredResource<'a>>,
        pointer: ParsedPointer,
        draft: Draft,
    },
}

impl IndexedResource<'_> {
    #[inline]
    pub(super) fn resolve(&self) -> Option<ResourceRef<'_>> {
        match self {
            IndexedResource::Borrowed(resource) => {
                Some(ResourceRef::new(resource.contents(), resource.draft()))
            }
            IndexedResource::Owned {
                document,
                pointer,
                draft,
            } => {
                let contents = pointer.lookup(document.contents())?;
                Some(ResourceRef::new(contents, *draft))
            }
        }
    }
}

/// An anchor in the index: either borrowed from the caller or owned by the registry.
#[derive(Debug, Clone)]
pub(super) enum IndexedAnchor<'a> {
    Borrowed(Anchor<'a>),
    Owned {
        document: Arc<StoredResource<'a>>,
        pointer: ParsedPointer,
        draft: Draft,
        kind: AnchorKind,
        name: Box<str>,
    },
}

impl IndexedAnchor<'_> {
    #[inline]
    pub(super) fn resolve(&self) -> Option<Anchor<'_>> {
        match self {
            IndexedAnchor::Borrowed(anchor) => Some(match anchor {
                Anchor::Default { name, resource } => Anchor::Default {
                    name,
                    resource: ResourceRef::new(resource.contents(), resource.draft()),
                },
                Anchor::Dynamic { name, resource } => Anchor::Dynamic {
                    name,
                    resource: ResourceRef::new(resource.contents(), resource.draft()),
                },
            }),
            IndexedAnchor::Owned {
                document,
                pointer,
                draft,
                kind,
                name,
            } => {
                let contents = pointer.lookup(document.contents())?;
                let resource = ResourceRef::new(contents, *draft);
                Some(match kind {
                    AnchorKind::Default => Anchor::Default { name, resource },
                    AnchorKind::Dynamic => Anchor::Dynamic { name, resource },
                })
            }
        }
    }
}

/// Whether an anchor is a plain anchor (`$anchor`) or a dynamic anchor (`$dynamicAnchor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AnchorKind {
    Default,
    Dynamic,
}

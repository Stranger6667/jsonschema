use core::fmt;
use std::sync::Arc;

use fluent_uri::Uri;
use serde_json::Value;

use crate::{list::List, Anchor, Draft, Error, Registry, ResourceRef, VocabularySet};

/// A reference resolver.
///
/// Resolves references against the base URI and looks up the result in the registry.
#[derive(Clone)]
pub struct Resolver<'r> {
    pub(crate) registry: &'r Registry<'r>,
    base_uri: Arc<Uri<String>>,
    scopes: List<Uri<String>>,
}

impl PartialEq for Resolver<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.base_uri == other.base_uri
    }
}
impl Eq for Resolver<'_> {}

impl fmt::Debug for Resolver<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Resolver")
            .field("base_uri", &self.base_uri.as_str())
            .field("scopes", &{
                let mut buf = String::from("[");
                let mut values = self.scopes.iter();
                if let Some(value) = values.next() {
                    buf.push_str(value.as_str());
                }
                for value in values {
                    buf.push_str(", ");
                    buf.push_str(value.as_str());
                }
                buf.push(']');
                buf
            })
            .finish()
    }
}

impl<'r> Resolver<'r> {
    #[inline]
    pub(crate) fn new(registry: &'r Registry<'r>, base_uri: Arc<Uri<String>>) -> Self {
        Self {
            registry,
            base_uri,
            scopes: List::new(),
        }
    }
    /// Returns the current base URI of this resolver.
    #[must_use]
    #[inline]
    pub fn base_uri(&self) -> Arc<Uri<String>> {
        self.base_uri.clone()
    }
    /// Resolve a reference to the resource it points to.
    ///
    /// # Errors
    ///
    /// If the reference cannot be resolved or is invalid.
    pub fn lookup(&self, reference: &str) -> Result<Resolved<'r>, Error> {
        let target = self.lookup_target(reference)?;
        Ok(self.enter(&target))
    }
    /// Resolve a `$dynamicRef` to the resource it points to.
    ///
    /// A fragment naming a `$dynamicAnchor` resolves to the outermost resource with a
    /// `$dynamicAnchor` of that name, in the dynamic scope or at this resolver's base URI. When
    /// that resource is another one, the resource at the base URI joins the dynamic scope of the
    /// returned resolver if it declares a `$dynamicAnchor` that no resource in the scope declares.
    /// Any other reference resolves as [`Resolver::lookup`] does.
    ///
    /// # Errors
    ///
    /// If the reference cannot be resolved or is invalid.
    pub fn lookup_dynamic_ref(&self, reference: &str) -> Result<Resolved<'r>, Error> {
        let target = self.lookup_target(reference)?;
        self.enter_dynamic_ref(&target)
    }
    /// Resolve a reference to a target that [`Resolver::enter`] turns into what
    /// [`Resolver::lookup`] returns, and [`Resolver::enter_dynamic_ref`] into what
    /// [`Resolver::lookup_dynamic_ref`] returns.
    #[doc(hidden)]
    ///
    /// The target depends only on the base URI and the reference, not on the dynamic scope, so
    /// any resolver with this base URI can enter it.
    ///
    /// # Errors
    ///
    /// If the reference cannot be resolved or is invalid.
    pub fn lookup_target(&self, reference: &str) -> Result<Target<'r>, Error> {
        let (uri, fragment) = if let Some(reference) = reference.strip_prefix('#') {
            (self.base_uri.clone(), reference)
        } else {
            let (uri, fragment) = if let Some((uri, fragment)) = reference.rsplit_once('#') {
                (uri, fragment)
            } else {
                (reference, "")
            };
            let uri = self.registry.resolve_uri(&self.base_uri.borrow(), uri)?;
            (uri, fragment)
        };

        let Some(retrieved) = self.registry.resource_by_uri(&uri) else {
            return Err(Error::unretrievable(
                uri.as_str(),
                "Retrieving external resources is not supported once the registry is populated"
                    .into(),
            ));
        };
        // Under its retrieval URI, a resource is based at its `$id`. Under its `$id` URI it already
        // is, and resolving a relative `$id` again would not lead back to the same resource.
        let uri = match retrieved.id() {
            Some(id) => {
                let canonical = self.registry.resolve_uri(&uri.borrow(), id)?;
                if self
                    .registry
                    .resource_by_uri(&canonical)
                    .is_some_and(|resource| std::ptr::eq(resource.contents(), retrieved.contents()))
                {
                    canonical
                } else {
                    uri
                }
            }
            None => uri,
        };

        if fragment.starts_with('/') {
            let resolved = retrieved.pointer(fragment, self.evolve(Arc::clone(&uri)))?;
            return Ok(Target {
                #[cfg(debug_assertions)]
                origin: Arc::clone(&self.base_uri),
                document: uri,
                kind: TargetKind::Resolved {
                    contents: resolved.contents,
                    base_uri: resolved.resolver.base_uri,
                    draft: resolved.draft,
                },
            });
        }

        if !fragment.is_empty() {
            let kind = match self.lookup_anchor(&uri, fragment)? {
                Anchor::Default { resource, .. } => TargetKind::Resolved {
                    contents: resource.contents(),
                    base_uri: Arc::clone(&uri),
                    draft: resource.draft(),
                },
                Anchor::Dynamic { name, resource } => TargetKind::DynamicAnchor { name, resource },
            };
            return Ok(Target {
                #[cfg(debug_assertions)]
                origin: Arc::clone(&self.base_uri),
                document: uri,
                kind,
            });
        }

        Ok(Target {
            #[cfg(debug_assertions)]
            origin: Arc::clone(&self.base_uri),
            kind: TargetKind::Resolved {
                contents: retrieved.contents(),
                base_uri: Arc::clone(&uri),
                draft: retrieved.draft(),
            },
            document: uri,
        })
    }
    /// Enter a target returned by [`Resolver::lookup_target`] on a resolver with this base URI.
    #[doc(hidden)]
    #[must_use]
    pub fn enter(&self, target: &Target<'r>) -> Resolved<'r> {
        #[cfg(debug_assertions)]
        assert_eq!(
            target.origin, self.base_uri,
            "a target is entered on a resolver with the base URI it was resolved from"
        );
        match &target.kind {
            TargetKind::Resolved {
                contents,
                base_uri,
                draft,
            } => {
                // A pointer that lands in an embedded resource also leaves the current one.
                let leaves = target.document != self.base_uri || *base_uri != target.document;
                let resolver = self.moved(Arc::clone(base_uri), leaves);
                Resolved::new(contents, resolver, *draft)
            }
            TargetKind::DynamicAnchor { resource, .. } => Resolved::new(
                resource.contents(),
                self.evolve(Arc::clone(&target.document)),
                resource.draft(),
            ),
        }
    }
    /// Enter a target as `$dynamicRef` does: a dynamic anchor resolves in this dynamic scope.
    #[doc(hidden)]
    ///
    /// # Errors
    ///
    /// If the target is a dynamic anchor that cannot be resolved in this dynamic scope.
    pub fn enter_dynamic_ref(&self, target: &Target<'r>) -> Result<Resolved<'r>, Error> {
        match &target.kind {
            TargetKind::Resolved { .. } => Ok(self.enter(target)),
            TargetKind::DynamicAnchor { name, resource } => {
                #[cfg(debug_assertions)]
                assert_eq!(
                    target.origin, self.base_uri,
                    "a target is entered on a resolver with the base URI it was resolved from"
                );
                crate::anchor::resolve_dynamic(name, *resource, &target.document, self)
            }
        }
    }
    /// Resolve a recursive reference.
    ///
    /// This method implements the recursive reference resolution algorithm
    /// as specified in JSON Schema Draft 2019-09.
    ///
    /// It resolves "#", and when that resource has `$recursiveAnchor: true`, lands on the
    /// outermost resource in the dynamic scope with `$recursiveAnchor: true` instead. Resources
    /// in between without it do not end the search.
    ///
    /// # Errors
    ///
    /// This method can return any error that [`Resolver::lookup`] can return.
    pub fn lookup_recursive_ref(&self) -> Result<Resolved<'r>, Error> {
        let mut resolved = self.lookup("#")?;
        if has_recursive_anchor(resolved.contents) {
            for uri in &self.dynamic_scope() {
                let next = self.lookup(uri.as_str())?;
                if has_recursive_anchor(next.contents) {
                    resolved = next;
                }
            }
        }
        Ok(resolved)
    }

    #[inline]
    pub(crate) fn lookup_anchor<'a>(
        &self,
        uri: &'a Uri<String>,
        name: &'a str,
    ) -> Result<Anchor<'r>, Error> {
        self.registry.anchor(uri, name)
    }

    /// Create a resolver for a subresource.
    ///
    /// # Errors
    ///
    /// Returns an error if the resource id cannot be resolved against the base URI of this resolver.
    #[inline]
    pub fn in_subresource(&self, subresource: ResourceRef<'_>) -> Result<Self, Error> {
        if let Some(id) = subresource.id() {
            let base_uri = self.registry.resolve_uri(&self.base_uri.borrow(), id)?;
            Ok(Resolver {
                registry: self.registry,
                base_uri,
                scopes: self.scopes.clone(),
            })
        } else {
            Ok(self.clone())
        }
    }
    /// The resolver for a subschema that evaluation descends into. A subschema whose `$id` names
    /// another resource starts it, and the resource it is embedded in joins the dynamic scope, as
    /// it does when a reference leaves it.
    #[doc(hidden)]
    ///
    /// # Errors
    ///
    /// Returns an error if the resource id cannot be resolved against the base URI of this resolver.
    pub fn descend_into(&self, subresource: ResourceRef<'_>) -> Result<Self, Error> {
        let Some(id) = subresource.id() else {
            return Ok(self.clone());
        };
        let base_uri = self.registry.resolve_uri(&self.base_uri.borrow(), id)?;
        // `moved` pushes this resource onto an empty scope for a reference that stays in it.
        let scopes = if base_uri == self.base_uri
            || self.scopes.iter().next() == Some(self.base_uri.as_ref())
        {
            self.scopes.clone()
        } else {
            self.scopes.push_front(Arc::clone(&self.base_uri))
        };
        Ok(Resolver {
            registry: self.registry,
            base_uri,
            scopes,
        })
    }
    /// The resolver a `$ref` to this resolver's own resource enters; the resource joins an empty
    /// dynamic scope.
    #[doc(hidden)]
    #[must_use]
    pub fn in_dynamic_scope(&self) -> Self {
        self.evolve(Arc::clone(&self.base_uri))
    }
    /// The resolver for the target of a `$dynamicRef` in this resolver's resource, which landed in
    /// the resource at `landed`. Landing in another resource keeps this one in the dynamic scope,
    /// which matters only when it declares a `$dynamicAnchor` no resource in the scope declares.
    pub(crate) fn dynamic_ref_lands_in(&self, landed: Arc<Uri<String>>) -> Self {
        let leaves = landed != self.base_uri && self.adds_dynamic_anchors();
        self.moved(landed, leaves)
    }
    /// Whether the resource at the base URI declares a `$dynamicAnchor` no resource in the dynamic
    /// scope declares.
    fn adds_dynamic_anchors(&self) -> bool {
        let mut adds = false;
        self.registry
            .for_each_dynamic_anchor(&self.base_uri, |name| {
                adds = adds || !self.declares_dynamic_anchor(&self.scopes, name);
            });
        adds
    }
    #[must_use]
    #[inline]
    pub fn dynamic_scope(&self) -> List<Uri<String>> {
        self.scopes.clone()
    }
    /// Whether every `$dynamicRef` resolves alike in the dynamic scope `outer` and in this one,
    /// also after both extend by the same resources; `false` unless this scope extends `outer`.
    ///
    /// A `$dynamicRef` resolves to the outermost resource in the scope with a `$dynamicAnchor` of
    /// its name, so the resources this scope adds change only names no resource in `outer`
    /// declares.
    #[doc(hidden)]
    #[must_use]
    pub fn binds_dynamic_anchors_like(&self, outer: &List<Uri<String>>) -> bool {
        let Some(mut added) = self.scopes.prefix_before(outer) else {
            return false;
        };
        added.all(|uri| {
            outer.iter().any(|resource| resource == uri) || {
                let mut declared = true;
                self.registry.for_each_dynamic_anchor(uri, |name| {
                    declared = declared && self.declares_dynamic_anchor(outer, name);
                });
                declared
            }
        })
    }
    /// Whether a resource in `scope` declares a `$dynamicAnchor` called `name`.
    fn declares_dynamic_anchor(&self, scope: &List<Uri<String>>, name: &str) -> bool {
        scope
            .iter()
            .any(|uri| matches!(self.registry.anchor(uri, name), Ok(Anchor::Dynamic { .. })))
    }
    #[inline]
    fn evolve(&self, base_uri: Arc<Uri<String>>) -> Resolver<'r> {
        let leaves = base_uri != self.base_uri;
        self.moved(base_uri, leaves)
    }
    /// This resolver based at `base_uri`; the current resource joins the dynamic scope when the
    /// scope is empty or `leaves` is set.
    #[inline]
    fn moved(&self, base_uri: Arc<Uri<String>>, leaves: bool) -> Resolver<'r> {
        if !self.base_uri.as_str().is_empty() && (self.scopes.is_empty() || leaves) {
            Resolver {
                registry: self.registry,
                base_uri,
                scopes: self.scopes.push_front(self.base_uri.clone()),
            }
        } else {
            Resolver {
                registry: self.registry,
                base_uri,
                scopes: self.scopes.clone(),
            }
        }
    }
    /// Resolve a reference against a base.
    ///
    /// # Errors
    ///
    /// If the reference is invalid.
    #[inline]
    pub fn resolve_uri(&self, base: &Uri<&str>, uri: &str) -> Result<Arc<Uri<String>>, Error> {
        self.registry.resolve_uri(base, uri)
    }

    /// Vocabulary of `contents` when it declares `$schema`; otherwise of the `$schema` above it
    /// that its resource root does not settle, or of the containing resource root (the resolver's
    /// base URI), so a `$schema`-less subschema inherits it.
    #[must_use]
    pub fn find_vocabularies(&self, draft: Draft, contents: &Value) -> VocabularySet {
        if contents.get("$schema").is_some() {
            return self.registry.find_vocabularies(draft, contents);
        }
        if let Some(declared) = self.registry.nested_dialect(contents) {
            return self.registry.vocabularies_declared(draft, Some(declared));
        }
        if let Some(resource) = self.registry.resource_by_uri(self.base_uri.as_ref()) {
            return self.registry.find_vocabularies(draft, resource.contents());
        }
        self.registry.find_vocabularies(draft, contents)
    }
}

fn has_recursive_anchor(contents: &Value) -> bool {
    contents.get("$recursiveAnchor").and_then(Value::as_bool) == Some(true)
}

/// A reference resolved by [`Resolver::lookup_target`], before entering the dynamic scope.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct Target<'r> {
    /// The base URI the reference was resolved from.
    #[cfg(debug_assertions)]
    origin: Arc<Uri<String>>,
    /// The document the reference names.
    document: Arc<Uri<String>>,
    kind: TargetKind<'r>,
}

impl Target<'_> {
    /// The document the reference names, before a JSON Pointer fragment enters any resource
    /// embedded in it.
    #[must_use]
    pub fn document(&self) -> &Arc<Uri<String>> {
        &self.document
    }
}

#[derive(Debug, Clone)]
enum TargetKind<'r> {
    Resolved {
        contents: &'r Value,
        base_uri: Arc<Uri<String>>,
        draft: Draft,
    },
    /// `$dynamicRef` resolves it in the dynamic scope it is entered in.
    DynamicAnchor {
        name: &'r str,
        resource: ResourceRef<'r>,
    },
}

/// A reference resolved to its contents by a [`Resolver`].
#[derive(Debug)]
pub struct Resolved<'r> {
    /// The contents of the resolved reference.
    contents: &'r Value,
    /// Resolver for references inside `contents`.
    resolver: Resolver<'r>,
    draft: Draft,
}

impl<'r> Resolved<'r> {
    pub(crate) fn new(contents: &'r Value, resolver: Resolver<'r>, draft: Draft) -> Self {
        Self {
            contents,
            resolver,
            draft,
        }
    }
    /// Resolved contents.
    #[must_use]
    pub fn contents(&self) -> &'r Value {
        self.contents
    }
    /// Resolver for references inside the resolved contents.
    #[must_use]
    pub fn resolver(&self) -> &Resolver<'r> {
        &self.resolver
    }
    #[must_use]
    pub fn into_inner(self) -> (&'r Value, Resolver<'r>, Draft) {
        (self.contents, self.resolver, self.draft)
    }
}

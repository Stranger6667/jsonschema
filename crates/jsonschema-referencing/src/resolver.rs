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
    /// Create a new `Resolver` with the given registry and base URI.
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
        self.enter(&target)
    }
    /// Resolve a reference to a target that [`Resolver::enter`] turns into what
    /// [`Resolver::lookup`] returns.
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
                anchor @ Anchor::Dynamic { .. } => TargetKind::DynamicAnchor(anchor),
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
    ///
    /// # Errors
    ///
    /// If the target is a dynamic anchor that cannot be resolved in this dynamic scope.
    pub fn enter(&self, target: &Target<'r>) -> Result<Resolved<'r>, Error> {
        #[cfg(debug_assertions)]
        assert_eq!(
            target.origin, self.base_uri,
            "a target is entered on a resolver with the base URI it was resolved from"
        );
        let mut resolver = self.evolve(Arc::clone(&target.document));
        match &target.kind {
            TargetKind::Resolved {
                contents,
                base_uri,
                draft,
            } => {
                resolver.base_uri = Arc::clone(base_uri);
                Ok(Resolved::new(contents, resolver, *draft))
            }
            TargetKind::DynamicAnchor(anchor) => anchor.resolve(resolver),
        }
    }
    /// Resolve a recursive reference.
    ///
    /// This method implements the recursive reference resolution algorithm
    /// as specified in JSON Schema Draft 2019-09.
    ///
    /// It starts by resolving "#" and then follows the dynamic scope,
    /// looking for resources with `$recursiveAnchor: true`.
    ///
    /// # Errors
    ///
    /// This method can return any error that [`Resolver::lookup`] can return.
    pub fn lookup_recursive_ref(&self) -> Result<Resolved<'r>, Error> {
        let mut resolved = self.lookup("#")?;

        if let Value::Object(obj) = resolved.contents {
            if obj
                .get("$recursiveAnchor")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                for uri in &self.dynamic_scope() {
                    let next_resolved = self.lookup(uri.as_str())?;

                    match next_resolved.contents {
                        Value::Object(next_obj) => {
                            if !next_obj
                                .get("$recursiveAnchor")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                            {
                                break;
                            }
                        }
                        _ => break,
                    }

                    resolved = next_resolved;
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
    /// The same resolver based at `uri`, keeping the dynamic scope.
    #[inline]
    pub(crate) fn at_uri(&self, uri: &Uri<String>) -> Result<Self, Error> {
        Ok(Resolver {
            registry: self.registry,
            base_uri: self
                .registry
                .resolve_uri(&self.base_uri.borrow(), uri.as_str())?,
            scopes: self.scopes.clone(),
        })
    }
    #[must_use]
    #[inline]
    pub fn dynamic_scope(&self) -> List<Uri<String>> {
        self.scopes.clone()
    }
    #[inline]
    fn evolve(&self, base_uri: Arc<Uri<String>>) -> Resolver<'r> {
        if !self.base_uri.as_str().is_empty()
            && (self.scopes.is_empty() || base_uri != self.base_uri)
        {
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

    /// Vocabulary of `contents` when it declares `$schema`; otherwise of the containing resource
    /// root (the resolver's base URI), so a `$schema`-less subschema inherits it.
    #[must_use]
    pub fn find_vocabularies(&self, draft: Draft, contents: &Value) -> VocabularySet {
        if contents.get("$schema").is_some() {
            return self.registry.find_vocabularies(draft, contents);
        }
        if let Some(resource) = self.registry.resource_by_uri(self.base_uri.as_ref()) {
            return self.registry.find_vocabularies(draft, resource.contents());
        }
        self.registry.find_vocabularies(draft, contents)
    }
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

#[derive(Debug, Clone)]
enum TargetKind<'r> {
    Resolved {
        contents: &'r Value,
        base_uri: Arc<Uri<String>>,
        draft: Draft,
    },
    /// Its target depends on the dynamic scope it is entered in.
    DynamicAnchor(Anchor<'r>),
}

/// A reference resolved to its contents by a [`Resolver`].
#[derive(Debug)]
pub struct Resolved<'r> {
    /// The contents of the resolved reference.
    contents: &'r Value,
    /// The resolver that resolved this reference, which can be used for further resolutions.
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
    /// Resolver used to resolve this contents.
    #[must_use]
    pub fn resolver(&self) -> &Resolver<'r> {
        &self.resolver
    }
    #[must_use]
    pub fn into_inner(self) -> (&'r Value, Resolver<'r>, Draft) {
        (self.contents, self.resolver, self.draft)
    }
}

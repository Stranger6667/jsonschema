//! Input normalization for resources entering the registry.
use std::sync::Arc;

use ahash::AHashMap;
use fluent_uri::Uri;
use serde_json::Value;

use crate::{Resource, ResourceRef, Retrieve};

/// A resource waiting to enter the registry.
#[derive(Clone)]
pub(crate) enum PendingResource<'a> {
    Value(Value),
    ValueRef(&'a Value),
    Resource(Resource),
    ResourceRef(ResourceRef<'a>),
}

pub(crate) mod private {
    use ahash::AHashMap;
    use fluent_uri::Uri;

    use super::PendingResource;

    pub(crate) trait Sealed<'a> {
        fn insert_into(
            self,
            pending: &mut AHashMap<Uri<String>, PendingResource<'a>>,
            uri: Uri<String>,
        );
    }
}

#[allow(private_bounds)]
pub trait IntoRegistryResource<'a>: private::Sealed<'a> {}

impl<'a, T> IntoRegistryResource<'a> for T where T: private::Sealed<'a> {}

impl<'a> private::Sealed<'a> for Resource {
    fn insert_into(
        self,
        pending: &mut AHashMap<Uri<String>, PendingResource<'a>>,
        uri: Uri<String>,
    ) {
        pending.insert(uri, PendingResource::Resource(self));
    }
}

impl<'a> private::Sealed<'a> for &'a Resource {
    fn insert_into(
        self,
        pending: &mut AHashMap<Uri<String>, PendingResource<'a>>,
        uri: Uri<String>,
    ) {
        pending.insert(
            uri,
            PendingResource::ResourceRef(ResourceRef::new(self.contents(), self.draft())),
        );
    }
}

impl<'a> private::Sealed<'a> for &'a Value {
    fn insert_into(
        self,
        pending: &mut AHashMap<Uri<String>, PendingResource<'a>>,
        uri: Uri<String>,
    ) {
        pending.insert(uri, PendingResource::ValueRef(self));
    }
}

impl<'a> private::Sealed<'a> for ResourceRef<'a> {
    fn insert_into(
        self,
        pending: &mut AHashMap<Uri<String>, PendingResource<'a>>,
        uri: Uri<String>,
    ) {
        pending.insert(uri, PendingResource::ResourceRef(self));
    }
}

impl<'a> private::Sealed<'a> for Value {
    fn insert_into(
        self,
        pending: &mut AHashMap<Uri<String>, PendingResource<'a>>,
        uri: Uri<String>,
    ) {
        pending.insert(uri, PendingResource::Value(self));
    }
}

pub trait IntoRetriever {
    fn into_retriever(self) -> Arc<dyn Retrieve>;
}

impl<T: Retrieve + 'static> IntoRetriever for T {
    fn into_retriever(self) -> Arc<dyn Retrieve> {
        Arc::new(self)
    }
}

impl IntoRetriever for Arc<dyn Retrieve> {
    fn into_retriever(self) -> Arc<dyn Retrieve> {
        self
    }
}

#[cfg(feature = "retrieve-async")]
pub trait IntoAsyncRetriever {
    fn into_retriever(self) -> Arc<dyn crate::AsyncRetrieve>;
}

#[cfg(feature = "retrieve-async")]
impl<T: crate::AsyncRetrieve + 'static> IntoAsyncRetriever for T {
    fn into_retriever(self) -> Arc<dyn crate::AsyncRetrieve> {
        Arc::new(self)
    }
}

#[cfg(feature = "retrieve-async")]
impl IntoAsyncRetriever for Arc<dyn crate::AsyncRetrieve> {
    fn into_retriever(self) -> Arc<dyn crate::AsyncRetrieve> {
        self
    }
}

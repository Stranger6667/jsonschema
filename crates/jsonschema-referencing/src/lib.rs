//! # referencing
//!
//! An implementation-agnostic JSON reference resolution library for Rust.
mod anchors;
mod builder;
mod cache;
mod context;
mod error;
mod list;
pub mod meta;
mod registry;
mod resolver;
mod resource;
mod retriever;
mod segments;
mod specification;
pub mod uri;
mod vocabularies;

pub(crate) use anchors::Anchor;
pub use builder::RegistryBuilder;
pub use context::ResolutionContext;
pub use error::{Error, UriError};
pub use fluent_uri::{Iri, IriRef, Uri, UriRef};
pub use list::List;
#[cfg(feature = "retrieve-async")]
pub use registry::IntoAsyncRetriever;
pub use registry::{
    parse_index, pointer, IntoRetriever, Registry, RegistryOptions, SPECIFICATIONS,
};
pub use resolver::{Resolved, Resolver};
pub use resource::{unescape_segment, IntoDocument, Resource, ResourceRef};
pub use retriever::{DefaultRetriever, Retrieve};
pub(crate) use segments::Segments;
pub use specification::Draft;
pub use vocabularies::{Vocabulary, VocabularySet};

#[cfg(feature = "retrieve-async")]
pub use retriever::AsyncRetrieve;

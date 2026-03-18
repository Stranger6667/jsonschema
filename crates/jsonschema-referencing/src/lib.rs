//! # referencing
//!
//! An implementation-agnostic JSON reference resolution library for Rust.
#[macro_export]
macro_rules! observe_registry {
    ($($arg:tt)*) => {{
        #[cfg(feature = "perf-observe-registry")]
        {
            println!($($arg)*);
        }
    }};
}

mod anchors;
mod cache;
mod error;
mod list;
pub mod meta;
mod path;
mod registry;
mod resolver;
mod resource;
mod retriever;
mod segments;
mod small_map;
mod specification;
pub mod uri;
mod vocabularies;

pub(crate) use anchors::Anchor;
pub use error::{Error, UriError};
pub use fluent_uri::{Iri, IriRef, Uri, UriRef};
pub use list::List;
#[doc(hidden)]
pub use path::{write_escaped_str, write_index};
pub use path::{JsonPointerNode, JsonPointerSegment, OwnedJsonPointer};
pub use registry::{
    parse_index, pointer, IntoRegistryResource, Registry, RegistryBuilder, SPECIFICATIONS,
};
pub use resolver::{Resolved, Resolver};
pub use resource::{unescape_segment, Resource, ResourceRef};
pub use retriever::{DefaultRetriever, Retrieve};
pub(crate) use segments::Segments;
pub use specification::Draft;
pub use vocabularies::{Vocabulary, VocabularySet};

#[cfg(feature = "retrieve-async")]
pub use retriever::AsyncRetrieve;

#[cfg(test)]
mod tests {
    use crate::{JsonPointerNode, OwnedJsonPointer};

    #[test]
    fn test_json_pointer_types_are_exported_from_crate_root() {
        let root = JsonPointerNode::new();
        let child = root.push(1usize);

        let pointer = OwnedJsonPointer::from(&child);

        assert_eq!(pointer.as_str(), "/1");
    }
}

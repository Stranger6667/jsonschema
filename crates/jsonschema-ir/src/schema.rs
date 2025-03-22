use core::fmt;
use std::ops::Range;

use ahash::HashMap;
use string_interner::{backend::BucketBackend, symbol::SymbolU32, StringInterner};

use crate::{
    blocks::SubSchema,
    metadata::{Location, Metadata},
    JsonValue, Keyword,
};

pub enum Schema {
    Boolean(bool),
    Object {
        root: SubSchema,
        nested: Vec<SubSchema>,
        metadata: Metadata,
    },
}

// TODO: Add extra wrapper that will have metainfo

struct ObjectSchema {
    /// Root level of an object schema.
    /// Points to a range of keywords.
    root: Range<u32>,
    keywords: Vec<Keyword>,
    subschemas: Vec<Range<u32>>,
    // Location for each keyword.
    locations: Vec<Location>,
    strings: StringInterner<BucketBackend>,
    values: Vec<JsonValue>,
    definitions: Vec<(SymbolU32, u32)>,
    metadata: Metadata,
}

enum SchemaEntry {
    ItemsArray(Range<u32>),
}

fn iter() {

    // 1. take root & keywords slice
    // 2. iter over this slice of keywords
    // 3. if any nested keyword appear, then push to the iteration stack, or just increase depth? /
    //    yield (Up, Keyword) pairs? or the consumer should know when scope changes?
    // TODO: How to actually build a validator from this?
    // -
}

//numbers: Vec<()>,
///// All `required` values merged into a single block.
//required: Vec<SymbolU32>,
//dependent_schemas: Vec<(SymbolU32, BlockId)>,
//dependent_required: Vec<(SymbolU32, Range<Idx>)>,
//properties: Vec<(SymbolU32, BlockId)>,
//pattern_properties: Vec<(SymbolU32, BlockId)>,

impl Schema {
    pub(crate) fn new_boolean(value: bool) -> Self {
        Self::Boolean(value)
    }
    pub(crate) fn new_object(root: SubSchema, nested: Vec<SubSchema>, metadata: Metadata) -> Self {
        Self::Object {
            root,
            nested,
            metadata,
        }
    }
}

impl fmt::Display for Schema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Schema::Boolean(true) => f.write_str("true"),
            Schema::Boolean(false) => f.write_str("false"),
            Schema::Object {
                root,
                nested,
                metadata,
            } => {
                // TODO:
                //  - Properly handle idents.
                //  - Schema should provide a convenient way to iterate over nodes.
                //  -
                f.write_str("{")?;
                for node in root.nodes() {
                    f.write_str("\"minimum\":10")?;
                }
                f.write_str("}")
            }
        }
    }
}

// TODO:
//   - Legacy keywords
//   - Other keys (for annotations / custom keywords)
//   - add serde / pyo3 / bigint features
//   - Fill all missing keyword inner fields
//   - Generate IDs for entities
//   - write disassembler (format a schema as string)
//   - Abstract over SizeId - with `bigint` feature it should be a reference into sizes (enum of
//   inline vs constand_id), without that feature it should be just inline value (usize).
//   - Flatten `nodes`
//   - Add `Error` with location & expectations
//   - Benchmarks
//   - Docs
//   - Restructure
//   - Calculate immediate block size + subtree size
//   - Try to keep the Keyword size <=16 bytes

use std::collections::HashMap;
use string_interner::{backend::BucketBackend, symbol::SymbolU32, StringInterner};

use crate::{blocks::Block, metadata::location::Locations};

pub type Size = u32;
pub struct SchemaId(u32);
// Id in sizes constants - indirection to support bug numbers
pub struct SizeId(u32);
pub struct PropertiesId(u32);
pub struct PatternPropertiesId(u32);
pub struct EnumId(u32);
pub struct ConstantId(u32);
pub struct ReferenceId(SymbolU32);
pub struct AnchorId(u32);
pub struct FormatId(u32);

pub struct Schema {
    root: Block,
    nested: Vec<Block>,
    constants: Constants,
    locations: Locations,
}

struct Vocabulary {
    name: String,
    enabled: bool,
}

// TODO:
//   - Legacy keywords
//   - Other keys (for annotations / custom keywords)
//   - Write basic tests for translation of `serde_json` to IR
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

use crate::{blocks::Block, metadata::Metadata};

pub struct Schema {
    root: Block,
    nested: Vec<Block>,
    metadata: Metadata,
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

use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use crate::canonical::ir::{Schema, SchemaNode, SharedSchema};

// Folds the variant's bit with each child's mask. Allocates a fresh `Arc`; equal subtrees are not deduplicated.
#[must_use]
pub(crate) fn shared(schema: Schema) -> SharedSchema {
    let mut mask = schema.variant_bit();
    let mut size: u32 = 1;
    schema.for_each_child(|child| {
        mask = mask.union(child.mask);
        size = size.saturating_add(child.size);
    });
    let hash = structural_hash(&schema);
    Arc::new(SchemaNode {
        schema,
        mask,
        hash,
        size,
    })
}

// Children already carry their cached hash, so hashing a `Schema` only folds in the variant and the children's hashes -
// O(direct children), not the subtree.
fn structural_hash(schema: &Schema) -> u64 {
    let mut hasher = ahash::AHasher::default();
    schema.hash(&mut hasher);
    hasher.finish()
}

// Two-branch `AllOf` fallback when a precise intersection can't be computed.
#[must_use]
pub(crate) fn allof_pair(left: &SharedSchema, right: &SharedSchema) -> SharedSchema {
    shared(Schema::AllOf(vec![Arc::clone(left), Arc::clone(right)]))
}

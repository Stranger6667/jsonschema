use std::{collections::HashMap, ops::Range};

use string_interner::{backend::BucketBackend, symbol::SymbolU32, StringInterner};

pub(crate) mod location;
pub(crate) mod vocabulary;

pub type Size = usize;

pub struct Metadata {
    pub(crate) constants: Constants,
}

pub(crate) struct Constants {
    strings: StringInterner<BucketBackend>,
    enums: HashMap<EnumId, Vec<ConstantId>>,
    /// All `required` values merged into a single block.
    required: Vec<SymbolU32>,
}

#[derive(Debug)]
pub(crate) struct RequiredId(Range<u32>);

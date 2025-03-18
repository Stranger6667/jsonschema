use std::ops::Range;

use string_interner::{backend::BucketBackend, symbol::SymbolU32, StringInterner};

use crate::{BlockId, JsonValue};

pub(crate) mod location;
pub(crate) mod vocabulary;

pub type Size = usize;
type Idx = u32;

pub struct Metadata {
    pub(crate) constants: Constants,
}

pub(crate) struct Constants {
    strings: StringInterner<BucketBackend>,
    values: Vec<JsonValue>,
    numbers: Vec<()>,
    /// All `required` values merged into a single block.
    required: Vec<SymbolU32>,
    definitions: Vec<(SymbolU32, BlockId)>,
    dependent_schemas: Vec<(SymbolU32, BlockId)>,
    dependent_required: Vec<(SymbolU32, Range<Idx>)>,
    properties: Vec<(SymbolU32, BlockId)>,
    pattern_properties: Vec<(SymbolU32, BlockId)>,
}

#[derive(Debug)]
pub(crate) struct ConstantId(Idx);

// TODO: Should be inline value if potential `bignum` feature is not enabled
//       otherwise enum with Inline / Idx variants to support big integers
#[derive(Debug)]
pub(crate) struct NumberId(Idx);

/// Identifies a value range where enum values are stored.
#[derive(Debug)]
pub(crate) struct EnumId(Range<Idx>);

#[derive(Debug)]
pub(crate) struct RequiredId(Range<Idx>);

#[derive(Debug)]
pub(crate) struct DependentSchemasId(Range<Idx>);

#[derive(Debug)]
pub(crate) struct DependentRequiredId(Range<Idx>);

#[derive(Debug)]
pub(crate) struct FormatId(SymbolU32);

#[derive(Debug)]
pub(crate) struct PropertiesId(Range<Idx>);

#[derive(Debug)]
pub(crate) struct PatternPropertiesId(Range<Idx>);

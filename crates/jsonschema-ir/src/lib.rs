use std::{collections::HashMap, ops::Range, sync::Arc};

use string_interner::{backend::BucketBackend, symbol::SymbolU32, StringInterner};

pub type Size = u32;
pub struct BlockId(u32);
pub struct SchemaId(u32);
// Id in sizes constants - indirection to support bug numbers
pub struct SizeId(u32);
pub struct PropertiesId(u32);
pub struct PatternPropertiesId(u32);
pub struct EnumId(u32);
pub struct ConstantId(u32);
pub struct VocabularyId(u32);
pub struct ReferenceId(SymbolU32);
pub struct AnchorId(u32);
pub struct FormatId(u32);
pub struct LocationId(u32);

pub struct Schema {
    root: Block,
    nested: Vec<Block>,
    constants: Constants,
    paths: Paths,
}

struct Location(Arc<String>);

pub struct Paths {
    items: Vec<Location>,
}

pub struct Block {
    id: BlockId,
    nodes: Vec<Node>,
}

pub struct Node {
    keyword: Keyword,
    location: LocationId,
}

pub struct Constants {
    strings: StringInterner<BucketBackend>,
    enums: HashMap<EnumId, Vec<ConstantId>>,
    // TODO: Abstract size so it is possible to support Bignum and without the flag to be more
    // efficient
    sizes: Vec<usize>,
}

struct Vocabulary {
    name: String,
    enabled: bool,
}

// TODO: Add sizes for immediate child + full subtrees? to jump over them

pub enum Keyword {
    /// Defines a JSON Schema dialect.
    Schema {
        id: SchemaId,
    },
    /// Identifies vocabularies available for use in schemas described by that meta-schema.
    Vocabulary {
        range: Range<VocabularyId>,
    },
    /// Identifies a schema resource.
    Id {
        id: SchemaId,
    },
    /// A reference to a statically identified schema.
    Ref {
        target: ReferenceId,
    },
    Anchor {
        value: AnchorId,
    },
    /// A reference resolved in runtime.
    DynamicRef {
        target: ReferenceId,
    },
    DynamicAnchor {
        value: AnchorId,
    },
    /// Location for re-usable schemas.
    Defs {
        range: Range<SchemaId>,
    },
    OneOf {
        range: Range<BlockId>,
    },
    AnyOf {
        range: Range<BlockId>,
    },
    AllOf {
        range: Range<BlockId>,
    },
    Not {
        block: BlockId,
    },
    If {
        block: BlockId,
    },
    Then {
        block: BlockId,
    },
    Else {
        block: BlockId,
    },
    /// Subschemas that are evaluated if the instance is an object and contains a certain property.
    DependentSchemas {
        // TODO: range of name -> BlockId
    },
    /// Validation succeeds if each element of the instance validates against the schema at the same position, if any.
    PrefixItems {
        range: Range<Size>,
    },
    Items {
        block: BlockId,
    },
    Contains,
    Properties(PropertiesId),
    PatternProperties(PatternPropertiesId),
    AdditionalProperties,
    PropertyNames,
    UnevaluatedItems,
    UnevaluatedProperties,
    True,
    False,
    Type,
    Enum(EnumId),
    Const(ConstantId),
    // Numeric
    MultipleOf,
    Maximum,
    ExclusiveMaximum,
    Minimum,
    ExclusiveMinimum,
    MinLength(SizeId),
    Maxlength(SizeId),
    Pattern(SymbolU32),
    MaxItems(SizeId),
    MinItems,
    UniqueItems,
    MaxContains,
    MinContains,
    MaxProperties(SizeId),
    MinProperties(SizeId),
    Required,
    DependentRequired,
    Format(FormatId),
}

const _: () = const {
    assert!(std::mem::size_of::<Keyword>() == 12);
};

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

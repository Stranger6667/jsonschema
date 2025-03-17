use std::collections::HashMap;

use string_interner::{backend::BucketBackend, symbol::SymbolU32, StringInterner};

pub type Size = u32;
pub struct SizeId(u32);
pub struct PropertiesId(u32);
pub struct PatternPropertiesId(u32);
pub struct EnumId(u32);
pub struct ConstantId(u32);
pub struct VocabularyId(u32);
pub struct ReferenceId(u32);
pub struct AnchorId(u32);
pub struct FormatId(u32);

pub struct Schema {
    instructions: Vec<JSIR>,
    constants: Constants,
}

pub struct Constants {
    strings: StringInterner<BucketBackend>,
    enums: HashMap<EnumId, Vec<ConstantId>>,
    // TODO: Abstract size so it is possible to support Bignum and without the flag to be more
    // efficient
    sizes: Vec<usize>,
}

// TODO: Add sizes for immediate child + full subtrees? to jump over them

pub enum JSIR {
    // Markers for schema boundaries within JSIR.
    StartSchema,
    EndSchema,
    // Core
    Schema(SymbolU32),
    Id(SymbolU32),
    Vocabulary(VocabularyId),
    // References
    Ref(ReferenceId),
    DynamicRef(ReferenceId),
    DynamicAnchor(AnchorId),
    Defs,
    // Logical
    OneOf { size: Size },
    AnyOf { size: Size },
    AllOf { size: Size },
    Not,
    // Conditional
    If,
    Then,
    Else,
    DependentSchemas,
    // Applying sub-schemas to child instances
    PrefixItems { size: Size },
    Items,
    Contains,
    Properties(PropertiesId),
    PatternProperties(PatternPropertiesId),
    AdditionalProperties,
    PropertyNames,
    // Unevaluated
    UnevaluatedItems,
    UnevaluatedProperties,

    // Any type
    Type, // TODO: Types
    Enum(EnumId),
    Const(ConstantId),
    // Numeric
    MultipleOf,
    Maximum,
    ExclusiveMaximum,
    Minimum,
    ExclusiveMinimum,
    // String
    MinLength(SizeId),
    Maxlength(SizeId),
    Pattern(SymbolU32),
    // Array
    MaxItems(SizeId),
    MinItems,
    UniqueItems,
    MaxContains,
    MinContains,
    // Object
    MaxProperties(SizeId),
    MinProperties(SizeId),
    Required,
    DependentRequired,
    // Format
    Format(FormatId),
    // TODO: Legacy keywords
}

pub struct Program {
    schemas: HashMap<String, Schema>,
}

const _: () = const {
    assert!(std::mem::size_of::<JSIR>() == 8);
};

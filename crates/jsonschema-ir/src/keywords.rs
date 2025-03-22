use std::ops::Range;

use string_interner::symbol::SymbolU32;

use crate::{
    blocks::BlockId,
    metadata::{
        vocabulary::VocabularyId, ConstantId, DependentRequiredId, DependentSchemasId, EnumId,
        FormatId, NumberId, PatternPropertiesId, PropertiesId, RequiredId, Size,
    },
};

// TODO: Add sizes for immediate child + full subtrees? to jump over them
// TODO: Note why `$defs` is not here
pub enum Keyword {
    /// Defines a JSON Schema dialect.
    Schema {
        id: SymbolU32,
    },
    /// Identifies vocabularies available for use in schemas described by that meta-schema.
    Vocabulary {
        range: Range<VocabularyId>,
    },
    /// Identifies a schema resource.
    Id {
        id: SymbolU32,
    },
    /// A reference to a statically identified schema.
    Ref {
        target: SymbolU32,
    },
    Anchor {
        value: SymbolU32,
    },
    /// A reference resolved in runtime.
    DynamicRef {
        target: SymbolU32,
    },
    DynamicAnchor {
        value: SymbolU32,
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
        id: DependentSchemasId,
    },
    /// Validation succeeds if each element of the instance validates against the schema at the same position, if any.
    PrefixItems {
        range: Range<BlockId>,
    },
    Items {
        block: BlockId,
    },
    Contains,
    Properties {
        id: PropertiesId,
    },
    PatternProperties {
        id: PatternPropertiesId,
    },
    AdditionalProperties,
    PropertyNames {
        block: BlockId,
    },
    UnevaluatedItems,
    UnevaluatedProperties,
    True,
    False,
    // TODO: Specify types
    Type,
    Enum {
        id: EnumId,
    },
    Const {
        id: ConstantId,
    },
    // TODO: Specify values
    MultipleOf {
        id: NumberId,
    },
    Maximum {
        limit: NumberId,
    },
    ExclusiveMaximum {
        limit: NumberId,
    },
    Minimum {
        limit: NumberId,
    },
    ExclusiveMinimum {
        limit: NumberId,
    },
    MinLength {
        size: Size,
    },
    Maxlength {
        size: Size,
    },
    Pattern {
        pattern: SymbolU32,
    },
    MaxItems {
        size: Size,
    },
    MinItems {
        size: Size,
    },
    UniqueItems,
    MaxContains {
        size: Size,
    },
    MinContains {
        size: Size,
    },
    MaxProperties {
        size: Size,
    },
    MinProperties {
        size: Size,
    },
    Required {
        id: RequiredId,
    },
    /// Conditionally requires that certain properties must be present if a given property is
    /// present in an object.
    DependentRequired {
        id: DependentRequiredId,
    },
    Format {
        id: FormatId,
    },
}

impl Keyword {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Keyword::Schema { .. } => "$schema",
            Keyword::Vocabulary { .. } => "$vocabulary",
            Keyword::Id { .. } => "$id",
            Keyword::Ref { .. } => "$ref",
            Keyword::Anchor { .. } => "$anchor",
            Keyword::DynamicRef { .. } => "$dynamicRef",
            Keyword::DynamicAnchor { .. } => "$dynamicAnchor",
            Keyword::OneOf { .. } => "oneOf",
            Keyword::AnyOf { .. } => "anyOf",
            Keyword::AllOf { .. } => "allOf",
            Keyword::Not { .. } => "not",
            Keyword::If { .. } => "if",
            Keyword::Then { .. } => "then",
            Keyword::Else { .. } => "else",
            Keyword::DependentSchemas { .. } => "dependentSchemas",
            Keyword::PrefixItems { .. } => "prefixItems",
            Keyword::Items { .. } => "items",
            Keyword::Contains => "contains",
            Keyword::Properties { .. } => "properties",
            Keyword::PatternProperties { .. } => "patternProperties",
            Keyword::AdditionalProperties => "additionalProperties",
            Keyword::PropertyNames { .. } => "propertyNames",
            Keyword::UnevaluatedItems => "unevaluatedItems",
            Keyword::UnevaluatedProperties => "unevaluatedProperties",
            Keyword::True => todo!(),
            Keyword::False => todo!(),
            Keyword::Type => "type",
            Keyword::Enum { .. } => "enum",
            Keyword::Const { .. } => "const",
            Keyword::MultipleOf { .. } => "multipleOf",
            Keyword::Maximum { .. } => "maximum",
            Keyword::ExclusiveMaximum { .. } => "exclusiveMaximum",
            Keyword::Minimum { .. } => "minimum",
            Keyword::ExclusiveMinimum { .. } => "exclusiveMinimum",
            Keyword::MinLength { .. } => "minLength",
            Keyword::Maxlength { .. } => "maxLength",
            Keyword::Pattern { .. } => "pattern",
            Keyword::MaxItems { .. } => "maxItems",
            Keyword::MinItems { .. } => "minItems",
            Keyword::UniqueItems => "uniqueItems",
            Keyword::MaxContains { .. } => "maxContains",
            Keyword::MinContains { .. } => "minContains",
            Keyword::MaxProperties { .. } => "maxProperties",
            Keyword::MinProperties { .. } => "minProperties",
            Keyword::Required { .. } => "required",
            Keyword::DependentRequired { .. } => "dependentRequired",
            Keyword::Format { .. } => "format",
        }
    }
}

const _: () = const {
    assert!(std::mem::size_of::<Keyword>() == 24);
};

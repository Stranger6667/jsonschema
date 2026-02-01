//! Enum-based validator dispatch for eliminating vtable overhead.
//!
//! Uses a two-level enum hierarchy to improve branch prediction:
//! - `ValidatorEnum`: Hot path validators (~50 variants, most commonly used)
//! - `ColdValidatorEnum`: Cold path validators (~70 variants, less frequently used)
//!
//! This reduces indirect branch misprediction from ~70% to ~30% for typical schemas.

use enum_dispatch::enum_dispatch;

use serde_json::Value;

use crate::{
    error::ErrorIterator,
    keywords::{
        additional_items::{AdditionalItemsBooleanValidator, AdditionalItemsObjectValidator},
        additional_properties::{
            AdditionalPropertiesFalseValidator, AdditionalPropertiesNotEmptyFalseValidator,
            AdditionalPropertiesNotEmptyValidator, AdditionalPropertiesValidator,
            AdditionalPropertiesWithPatternsFalseValidator,
            AdditionalPropertiesWithPatternsNotEmptyFalseValidator,
            AdditionalPropertiesWithPatternsNotEmptyValidator,
            AdditionalPropertiesWithPatternsValidator,
        },
        all_of::{AllOfValidator, SingleValueAllOfValidator},
        any_of::{AnyOfValidator, SingleAnyOfValidator},
        boolean::FalseValidator,
        const_::{
            ConstArrayValidator, ConstBooleanValidator, ConstNullValidator, ConstNumberValidator,
            ConstObjectValidator, ConstStringValidator,
        },
        contains::{
            ContainsValidator, MaxContainsValidator, MinContainsValidator, MinMaxContainsValidator,
        },
        content::{
            ContentEncodingValidator, ContentMediaTypeAndEncodingValidator,
            ContentMediaTypeValidator,
        },
        custom::CustomKeyword,
        dependencies::{
            DependenciesValidator, DependentRequiredValidator, DependentSchemasValidator,
        },
        enum_::{EnumValidator, SingleValueEnumValidator},
        format::{CustomFormatValidator, EmailValidator, IdnEmailValidator},
        if_::{IfElseValidator, IfThenElseValidator, IfThenValidator},
        items::{ItemsArrayValidator, ItemsObjectSkipPrefixValidator, ItemsObjectValidator},
        legacy::type_draft_4,
        max_items::MaxItemsValidator,
        max_length::MaxLengthValidator,
        max_properties::MaxPropertiesValidator,
        min_items::MinItemsValidator,
        min_length::MinLengthValidator,
        min_properties::MinPropertiesValidator,
        minmax::{ExclusiveMaximum, ExclusiveMinimum, Maximum, Minimum},
        multiple_of::{MultipleOfFloatValidator, MultipleOfIntegerValidator},
        not::NotValidator,
        one_of::{OneOfValidator, SingleOneOfValidator},
        pattern::{PatternValidator, PrefixPatternValidator},
        pattern_properties::{
            PatternPropertiesValidator, PrefixPatternPropertiesValidator,
            SinglePrefixPatternPropertiesValidator, SingleValuePatternPropertiesValidator,
        },
        prefix_items::PrefixItemsValidator,
        properties::PropertiesValidator,
        property_names::{PropertyNamesBooleanValidator, PropertyNamesObjectValidator},
        ref_::RefValidator,
        required::{RequiredValidator, SingleItemRequiredValidator},
        type_::{
            ArrayTypeValidator, BooleanTypeValidator, IntegerTypeValidator, MultipleTypesValidator,
            NullTypeValidator, NumberTypeValidator, ObjectTypeValidator, StringTypeValidator,
        },
        unevaluated_items::UnevaluatedItemsValidator,
        unevaluated_properties::UnevaluatedPropertiesValidator,
        unique_items::UniqueItemsValidator,
    },
    paths::{LazyLocation, Location, RefTracker},
    validator::{EvaluationResult, Validate, ValidationContext},
    ValidationError,
};

#[cfg(feature = "arbitrary-precision")]
use crate::keywords::{
    minmax::bigint_validators::{
        BigFracExclusiveMaximum, BigFracExclusiveMinimum, BigFracMaximum, BigFracMinimum,
        BigIntExclusiveMaximum, BigIntExclusiveMinimum, BigIntMaximum, BigIntMinimum,
    },
    multiple_of::{MultipleOfBigFracValidator, MultipleOfBigIntValidator},
};

/// Hot path validators - most commonly used in typical schemas.
/// Based on benchmark analysis: type (20%), items (15%), properties (3.5%),
/// additionalProperties (3.6%), required (2%), pattern (2%), enum (1.6%), const (0.7%)
#[enum_dispatch(Validate)]
pub(crate) enum ValidatorEnum {
    // Type validators (20% of usage - most critical)
    NullType(NullTypeValidator),
    BooleanType(BooleanTypeValidator),
    StringType(StringTypeValidator),
    ArrayType(ArrayTypeValidator),
    ObjectType(ObjectTypeValidator),
    NumberType(NumberTypeValidator),
    IntegerType(IntegerTypeValidator),
    MultipleTypes(MultipleTypesValidator),

    // Items validators (15% of usage)
    ItemsObject(ItemsObjectValidator),
    ItemsObjectSkipPrefix(ItemsObjectSkipPrefixValidator),
    ItemsArray(ItemsArrayValidator),
    PrefixItems(PrefixItemsValidator),

    // Properties validator (3.5% of usage)
    Properties(PropertiesValidator),

    // AdditionalProperties validators (3.6% of usage - core variants only)
    AdditionalPropertiesFalse(AdditionalPropertiesFalseValidator),
    AdditionalProperties(AdditionalPropertiesValidator),
    AdditionalPropertiesNotEmptyFalseHash(
        AdditionalPropertiesNotEmptyFalseValidator<
            ahash::AHashMap<String, crate::node::SchemaNode>,
        >,
    ),
    AdditionalPropertiesNotEmptyFalseVec(
        AdditionalPropertiesNotEmptyFalseValidator<Vec<(String, crate::node::SchemaNode)>>,
    ),
    AdditionalPropertiesNotEmptyHash(
        AdditionalPropertiesNotEmptyValidator<ahash::AHashMap<String, crate::node::SchemaNode>>,
    ),
    AdditionalPropertiesNotEmptyVec(
        AdditionalPropertiesNotEmptyValidator<Vec<(String, crate::node::SchemaNode)>>,
    ),

    // Required validators (2% of usage)
    Required(RequiredValidator),
    SingleItemRequired(SingleItemRequiredValidator),

    // Pattern validators (2% of usage)
    PrefixPattern(PrefixPatternValidator),
    PatternFancy(PatternValidator<fancy_regex::Regex>),
    PatternStd(PatternValidator<regex::Regex>),

    // Enum validators (1.6% of usage)
    Enum(EnumValidator),
    SingleValueEnum(SingleValueEnumValidator),

    // Const validators (0.7% of usage)
    ConstNull(ConstNullValidator),
    ConstBoolean(ConstBooleanValidator),
    ConstString(ConstStringValidator),
    ConstNumber(ConstNumberValidator),
    ConstArray(ConstArrayValidator),
    ConstObject(ConstObjectValidator),

    // Reference validator (critical for schema composition)
    Ref(RefValidator),

    // Composition validators (used in OpenAPI/Swagger heavily)
    AllOf(AllOfValidator),
    SingleValueAllOf(SingleValueAllOfValidator),
    AnyOf(AnyOfValidator),
    SingleAnyOf(SingleAnyOfValidator),
    OneOf(OneOfValidator),
    SingleOneOf(SingleOneOfValidator),
    Not(NotValidator),

    // Numeric validators (common in data validation)
    MinimumU64(Minimum<u64>),
    MaximumU64(Maximum<u64>),
    ExclusiveMinimumU64(ExclusiveMinimum<u64>),
    ExclusiveMaximumU64(ExclusiveMaximum<u64>),
    MinimumI64(Minimum<i64>),
    MaximumI64(Maximum<i64>),
    ExclusiveMinimumI64(ExclusiveMinimum<i64>),
    ExclusiveMaximumI64(ExclusiveMaximum<i64>),
    MinimumF64(Minimum<f64>),
    MaximumF64(Maximum<f64>),
    ExclusiveMinimumF64(ExclusiveMinimum<f64>),
    ExclusiveMaximumF64(ExclusiveMaximum<f64>),

    // MultipleOf validators
    MultipleOfInteger(MultipleOfIntegerValidator),
    MultipleOfFloat(MultipleOfFloatValidator),

    // Length validators
    MinLength(MinLengthValidator),
    MaxLength(MaxLengthValidator),
    MinItems(MinItemsValidator),
    MaxItems(MaxItemsValidator),
    MinProperties(MinPropertiesValidator),
    MaxProperties(MaxPropertiesValidator),

    // False validator (common for additionalProperties: false patterns)
    False(FalseValidator),

    // Cold path - wrapped in inner enum for less common validators
    Cold(ColdValidatorEnum),
}

/// Cold path validators - less frequently used in typical schemas.
/// Wrapped in a single variant to reduce hot path enum size.
#[enum_dispatch(Validate)]
pub(crate) enum ColdValidatorEnum {
    // Draft 4 legacy type validators
    IntegerTypeDraft4(type_draft_4::IntegerTypeValidator),
    MultipleTypesDraft4(type_draft_4::MultipleTypesValidator),

    // Additional items validators
    AdditionalItemsBoolean(AdditionalItemsBooleanValidator),
    AdditionalItemsObject(AdditionalItemsObjectValidator),

    // AdditionalProperties with patterns (less common)
    AdditionalPropertiesWithPatternsFalseFancy(
        AdditionalPropertiesWithPatternsFalseValidator<
            crate::properties::CompiledPattern<fancy_regex::Regex>,
        >,
    ),
    AdditionalPropertiesWithPatternsFalseStd(
        AdditionalPropertiesWithPatternsFalseValidator<
            crate::properties::CompiledPattern<regex::Regex>,
        >,
    ),
    AdditionalPropertiesWithPatternsFancy(
        AdditionalPropertiesWithPatternsValidator<
            crate::properties::CompiledPattern<fancy_regex::Regex>,
        >,
    ),
    AdditionalPropertiesWithPatternsStd(
        AdditionalPropertiesWithPatternsValidator<crate::properties::CompiledPattern<regex::Regex>>,
    ),
    AdditionalPropertiesWithPatternsNotEmptyFalseHashFancy(
        AdditionalPropertiesWithPatternsNotEmptyFalseValidator<
            ahash::AHashMap<String, crate::node::SchemaNode>,
            crate::properties::CompiledPattern<fancy_regex::Regex>,
        >,
    ),
    AdditionalPropertiesWithPatternsNotEmptyFalseHashStd(
        AdditionalPropertiesWithPatternsNotEmptyFalseValidator<
            ahash::AHashMap<String, crate::node::SchemaNode>,
            crate::properties::CompiledPattern<regex::Regex>,
        >,
    ),
    AdditionalPropertiesWithPatternsNotEmptyFalseVecFancy(
        AdditionalPropertiesWithPatternsNotEmptyFalseValidator<
            Vec<(String, crate::node::SchemaNode)>,
            crate::properties::CompiledPattern<fancy_regex::Regex>,
        >,
    ),
    AdditionalPropertiesWithPatternsNotEmptyFalseVecStd(
        AdditionalPropertiesWithPatternsNotEmptyFalseValidator<
            Vec<(String, crate::node::SchemaNode)>,
            crate::properties::CompiledPattern<regex::Regex>,
        >,
    ),
    AdditionalPropertiesWithPatternsNotEmptyHashFancy(
        AdditionalPropertiesWithPatternsNotEmptyValidator<
            ahash::AHashMap<String, crate::node::SchemaNode>,
            crate::properties::CompiledPattern<fancy_regex::Regex>,
        >,
    ),
    AdditionalPropertiesWithPatternsNotEmptyHashStd(
        AdditionalPropertiesWithPatternsNotEmptyValidator<
            ahash::AHashMap<String, crate::node::SchemaNode>,
            crate::properties::CompiledPattern<regex::Regex>,
        >,
    ),
    AdditionalPropertiesWithPatternsNotEmptyVecFancy(
        AdditionalPropertiesWithPatternsNotEmptyValidator<
            Vec<(String, crate::node::SchemaNode)>,
            crate::properties::CompiledPattern<fancy_regex::Regex>,
        >,
    ),
    AdditionalPropertiesWithPatternsNotEmptyVecStd(
        AdditionalPropertiesWithPatternsNotEmptyValidator<
            Vec<(String, crate::node::SchemaNode)>,
            crate::properties::CompiledPattern<regex::Regex>,
        >,
    ),

    // Pattern properties validators
    PatternPropertiesFancy(PatternPropertiesValidator<fancy_regex::Regex>),
    PatternPropertiesStd(PatternPropertiesValidator<regex::Regex>),
    PrefixPatternProperties(PrefixPatternPropertiesValidator),
    SinglePrefixPatternProperties(SinglePrefixPatternPropertiesValidator),
    SingleValuePatternPropertiesFancy(SingleValuePatternPropertiesValidator<fancy_regex::Regex>),
    SingleValuePatternPropertiesStd(SingleValuePatternPropertiesValidator<regex::Regex>),

    // Property names validators
    PropertyNamesBoolean(PropertyNamesBooleanValidator),
    PropertyNamesObject(PropertyNamesObjectValidator),

    // Contains validators
    Contains(ContainsValidator),
    MinContains(MinContainsValidator),
    MaxContains(MaxContainsValidator),
    MinMaxContains(MinMaxContainsValidator),

    // Unique items
    UniqueItems(UniqueItemsValidator),

    // Conditional validators
    IfThen(IfThenValidator),
    IfElse(IfElseValidator),
    IfThenElse(IfThenElseValidator),

    // Dependency validators
    Dependencies(DependenciesValidator),
    DependentRequired(DependentRequiredValidator),
    DependentSchemas(DependentSchemasValidator),

    // Unevaluated validators (boxed - these are large)
    UnevaluatedProperties(Box<UnevaluatedPropertiesValidator>),
    UnevaluatedItems(Box<UnevaluatedItemsValidator>),

    // Format validators
    Email(EmailValidator),
    IdnEmail(IdnEmailValidator),
    CustomFormat(CustomFormatValidator),
    Date(crate::keywords::format::DateValidator),
    DateTime(crate::keywords::format::DateTimeValidator),
    Duration(crate::keywords::format::DurationValidator),
    Hostname(crate::keywords::format::HostnameValidator),
    IdnHostname(crate::keywords::format::IdnHostnameValidator),
    IpV4(crate::keywords::format::IpV4Validator),
    IpV6(crate::keywords::format::IpV6Validator),
    Iri(crate::keywords::format::IriValidator),
    IriReference(crate::keywords::format::IriReferenceValidator),
    JsonPointer(crate::keywords::format::JsonPointerValidator),
    FormatRegex(crate::keywords::format::RegexValidator),
    RelativeJsonPointer(crate::keywords::format::RelativeJsonPointerValidator),
    Time(crate::keywords::format::TimeValidator),
    Uri(crate::keywords::format::UriValidator),
    UriReference(crate::keywords::format::UriReferenceValidator),
    UriTemplate(crate::keywords::format::UriTemplateValidator),
    Uuid(crate::keywords::format::UuidValidator),

    // Content validators
    ContentEncoding(ContentEncodingValidator),
    ContentMediaType(ContentMediaTypeValidator),
    ContentMediaTypeAndEncoding(ContentMediaTypeAndEncodingValidator),

    // Custom keyword
    Custom(CustomKeyword),

    // BigInt validators (arbitrary-precision feature)
    #[cfg(feature = "arbitrary-precision")]
    BigIntMinimum(BigIntMinimum),
    #[cfg(feature = "arbitrary-precision")]
    BigIntMaximum(BigIntMaximum),
    #[cfg(feature = "arbitrary-precision")]
    BigIntExclusiveMinimum(BigIntExclusiveMinimum),
    #[cfg(feature = "arbitrary-precision")]
    BigIntExclusiveMaximum(BigIntExclusiveMaximum),
    #[cfg(feature = "arbitrary-precision")]
    BigFracMinimum(BigFracMinimum),
    #[cfg(feature = "arbitrary-precision")]
    BigFracMaximum(BigFracMaximum),
    #[cfg(feature = "arbitrary-precision")]
    BigFracExclusiveMinimum(BigFracExclusiveMinimum),
    #[cfg(feature = "arbitrary-precision")]
    BigFracExclusiveMaximum(BigFracExclusiveMaximum),
    #[cfg(feature = "arbitrary-precision")]
    MultipleOfBigInt(MultipleOfBigIntValidator),
    #[cfg(feature = "arbitrary-precision")]
    MultipleOfBigFrac(MultipleOfBigFracValidator),
}

// Helper macro to convert cold validators to ValidatorEnum via ColdValidatorEnum
// This allows: ColdValidator.into() -> ColdValidatorEnum -> ValidatorEnum

macro_rules! impl_cold_into_validator_enum {
    ($($variant:ident($type:ty)),+ $(,)?) => {
        $(
            impl From<$type> for ValidatorEnum {
                fn from(v: $type) -> Self {
                    ValidatorEnum::Cold(ColdValidatorEnum::$variant(v))
                }
            }
        )+
    };
}

impl_cold_into_validator_enum! {
    // Draft 4 legacy
    IntegerTypeDraft4(type_draft_4::IntegerTypeValidator),
    MultipleTypesDraft4(type_draft_4::MultipleTypesValidator),

    // Additional items
    AdditionalItemsBoolean(AdditionalItemsBooleanValidator),
    AdditionalItemsObject(AdditionalItemsObjectValidator),

    // AdditionalProperties with patterns
    AdditionalPropertiesWithPatternsFalseFancy(AdditionalPropertiesWithPatternsFalseValidator<crate::properties::CompiledPattern<fancy_regex::Regex>>),
    AdditionalPropertiesWithPatternsFalseStd(AdditionalPropertiesWithPatternsFalseValidator<crate::properties::CompiledPattern<regex::Regex>>),
    AdditionalPropertiesWithPatternsFancy(AdditionalPropertiesWithPatternsValidator<crate::properties::CompiledPattern<fancy_regex::Regex>>),
    AdditionalPropertiesWithPatternsStd(AdditionalPropertiesWithPatternsValidator<crate::properties::CompiledPattern<regex::Regex>>),
    AdditionalPropertiesWithPatternsNotEmptyFalseHashFancy(AdditionalPropertiesWithPatternsNotEmptyFalseValidator<ahash::AHashMap<String, crate::node::SchemaNode>, crate::properties::CompiledPattern<fancy_regex::Regex>>),
    AdditionalPropertiesWithPatternsNotEmptyFalseHashStd(AdditionalPropertiesWithPatternsNotEmptyFalseValidator<ahash::AHashMap<String, crate::node::SchemaNode>, crate::properties::CompiledPattern<regex::Regex>>),
    AdditionalPropertiesWithPatternsNotEmptyFalseVecFancy(AdditionalPropertiesWithPatternsNotEmptyFalseValidator<Vec<(String, crate::node::SchemaNode)>, crate::properties::CompiledPattern<fancy_regex::Regex>>),
    AdditionalPropertiesWithPatternsNotEmptyFalseVecStd(AdditionalPropertiesWithPatternsNotEmptyFalseValidator<Vec<(String, crate::node::SchemaNode)>, crate::properties::CompiledPattern<regex::Regex>>),
    AdditionalPropertiesWithPatternsNotEmptyHashFancy(AdditionalPropertiesWithPatternsNotEmptyValidator<ahash::AHashMap<String, crate::node::SchemaNode>, crate::properties::CompiledPattern<fancy_regex::Regex>>),
    AdditionalPropertiesWithPatternsNotEmptyHashStd(AdditionalPropertiesWithPatternsNotEmptyValidator<ahash::AHashMap<String, crate::node::SchemaNode>, crate::properties::CompiledPattern<regex::Regex>>),
    AdditionalPropertiesWithPatternsNotEmptyVecFancy(AdditionalPropertiesWithPatternsNotEmptyValidator<Vec<(String, crate::node::SchemaNode)>, crate::properties::CompiledPattern<fancy_regex::Regex>>),
    AdditionalPropertiesWithPatternsNotEmptyVecStd(AdditionalPropertiesWithPatternsNotEmptyValidator<Vec<(String, crate::node::SchemaNode)>, crate::properties::CompiledPattern<regex::Regex>>),

    // Pattern properties
    PatternPropertiesFancy(PatternPropertiesValidator<fancy_regex::Regex>),
    PatternPropertiesStd(PatternPropertiesValidator<regex::Regex>),
    PrefixPatternProperties(PrefixPatternPropertiesValidator),
    SinglePrefixPatternProperties(SinglePrefixPatternPropertiesValidator),
    SingleValuePatternPropertiesFancy(SingleValuePatternPropertiesValidator<fancy_regex::Regex>),
    SingleValuePatternPropertiesStd(SingleValuePatternPropertiesValidator<regex::Regex>),

    // Property names
    PropertyNamesBoolean(PropertyNamesBooleanValidator),
    PropertyNamesObject(PropertyNamesObjectValidator),

    // Contains
    Contains(ContainsValidator),
    MinContains(MinContainsValidator),
    MaxContains(MaxContainsValidator),
    MinMaxContains(MinMaxContainsValidator),

    // Unique items
    UniqueItems(UniqueItemsValidator),

    // Conditional
    IfThen(IfThenValidator),
    IfElse(IfElseValidator),
    IfThenElse(IfThenElseValidator),

    // Dependencies
    Dependencies(DependenciesValidator),
    DependentRequired(DependentRequiredValidator),
    DependentSchemas(DependentSchemasValidator),

    // Format validators
    Email(EmailValidator),
    IdnEmail(IdnEmailValidator),
    CustomFormat(CustomFormatValidator),
    Date(crate::keywords::format::DateValidator),
    DateTime(crate::keywords::format::DateTimeValidator),
    Duration(crate::keywords::format::DurationValidator),
    Hostname(crate::keywords::format::HostnameValidator),
    IdnHostname(crate::keywords::format::IdnHostnameValidator),
    IpV4(crate::keywords::format::IpV4Validator),
    IpV6(crate::keywords::format::IpV6Validator),
    Iri(crate::keywords::format::IriValidator),
    IriReference(crate::keywords::format::IriReferenceValidator),
    JsonPointer(crate::keywords::format::JsonPointerValidator),
    FormatRegex(crate::keywords::format::RegexValidator),
    RelativeJsonPointer(crate::keywords::format::RelativeJsonPointerValidator),
    Time(crate::keywords::format::TimeValidator),
    Uri(crate::keywords::format::UriValidator),
    UriReference(crate::keywords::format::UriReferenceValidator),
    UriTemplate(crate::keywords::format::UriTemplateValidator),
    Uuid(crate::keywords::format::UuidValidator),

    // Content validators
    ContentEncoding(ContentEncodingValidator),
    ContentMediaType(ContentMediaTypeValidator),
    ContentMediaTypeAndEncoding(ContentMediaTypeAndEncodingValidator),

    // Custom keyword
    Custom(CustomKeyword),
}

// Unevaluated validators need special handling (boxed)
impl From<Box<UnevaluatedPropertiesValidator>> for ValidatorEnum {
    fn from(v: Box<UnevaluatedPropertiesValidator>) -> Self {
        ValidatorEnum::Cold(ColdValidatorEnum::UnevaluatedProperties(v))
    }
}

impl From<Box<UnevaluatedItemsValidator>> for ValidatorEnum {
    fn from(v: Box<UnevaluatedItemsValidator>) -> Self {
        ValidatorEnum::Cold(ColdValidatorEnum::UnevaluatedItems(v))
    }
}

// BigInt validators (arbitrary-precision feature)
#[cfg(feature = "arbitrary-precision")]
macro_rules! impl_bigint_cold_into_validator_enum {
    ($($variant:ident($type:ty)),+ $(,)?) => {
        $(
            impl From<$type> for ValidatorEnum {
                fn from(v: $type) -> Self {
                    ValidatorEnum::Cold(ColdValidatorEnum::$variant(v))
                }
            }
        )+
    };
}

#[cfg(feature = "arbitrary-precision")]
impl_bigint_cold_into_validator_enum! {
    BigIntMinimum(BigIntMinimum),
    BigIntMaximum(BigIntMaximum),
    BigIntExclusiveMinimum(BigIntExclusiveMinimum),
    BigIntExclusiveMaximum(BigIntExclusiveMaximum),
    BigFracMinimum(BigFracMinimum),
    BigFracMaximum(BigFracMaximum),
    BigFracExclusiveMinimum(BigFracExclusiveMinimum),
    BigFracExclusiveMaximum(BigFracExclusiveMaximum),
    MultipleOfBigInt(MultipleOfBigIntValidator),
    MultipleOfBigFrac(MultipleOfBigFracValidator),
}

impl ValidatorEnum {
    pub(crate) fn canonical_location(&self) -> Option<&Location> {
        match self {
            // Hot path validators
            ValidatorEnum::NullType(v) => v.canonical_location(),
            ValidatorEnum::BooleanType(v) => v.canonical_location(),
            ValidatorEnum::StringType(v) => v.canonical_location(),
            ValidatorEnum::ArrayType(v) => v.canonical_location(),
            ValidatorEnum::ObjectType(v) => v.canonical_location(),
            ValidatorEnum::NumberType(v) => v.canonical_location(),
            ValidatorEnum::IntegerType(v) => v.canonical_location(),
            ValidatorEnum::MultipleTypes(v) => v.canonical_location(),
            ValidatorEnum::ItemsObject(v) => v.canonical_location(),
            ValidatorEnum::ItemsObjectSkipPrefix(v) => v.canonical_location(),
            ValidatorEnum::ItemsArray(v) => v.canonical_location(),
            ValidatorEnum::PrefixItems(v) => v.canonical_location(),
            ValidatorEnum::Properties(v) => v.canonical_location(),
            ValidatorEnum::AdditionalPropertiesFalse(v) => v.canonical_location(),
            ValidatorEnum::AdditionalProperties(v) => v.canonical_location(),
            ValidatorEnum::AdditionalPropertiesNotEmptyFalseHash(v) => v.canonical_location(),
            ValidatorEnum::AdditionalPropertiesNotEmptyFalseVec(v) => v.canonical_location(),
            ValidatorEnum::AdditionalPropertiesNotEmptyHash(v) => v.canonical_location(),
            ValidatorEnum::AdditionalPropertiesNotEmptyVec(v) => v.canonical_location(),
            ValidatorEnum::Required(v) => v.canonical_location(),
            ValidatorEnum::SingleItemRequired(v) => v.canonical_location(),
            ValidatorEnum::PrefixPattern(v) => v.canonical_location(),
            ValidatorEnum::PatternFancy(v) => v.canonical_location(),
            ValidatorEnum::PatternStd(v) => v.canonical_location(),
            ValidatorEnum::Enum(v) => v.canonical_location(),
            ValidatorEnum::SingleValueEnum(v) => v.canonical_location(),
            ValidatorEnum::ConstNull(v) => v.canonical_location(),
            ValidatorEnum::ConstBoolean(v) => v.canonical_location(),
            ValidatorEnum::ConstString(v) => v.canonical_location(),
            ValidatorEnum::ConstNumber(v) => v.canonical_location(),
            ValidatorEnum::ConstArray(v) => v.canonical_location(),
            ValidatorEnum::ConstObject(v) => v.canonical_location(),
            ValidatorEnum::Ref(v) => v.canonical_location(),
            ValidatorEnum::AllOf(v) => v.canonical_location(),
            ValidatorEnum::SingleValueAllOf(v) => v.canonical_location(),
            ValidatorEnum::AnyOf(v) => v.canonical_location(),
            ValidatorEnum::SingleAnyOf(v) => v.canonical_location(),
            ValidatorEnum::OneOf(v) => v.canonical_location(),
            ValidatorEnum::SingleOneOf(v) => v.canonical_location(),
            ValidatorEnum::Not(v) => v.canonical_location(),
            ValidatorEnum::MinimumU64(v) => v.canonical_location(),
            ValidatorEnum::MaximumU64(v) => v.canonical_location(),
            ValidatorEnum::ExclusiveMinimumU64(v) => v.canonical_location(),
            ValidatorEnum::ExclusiveMaximumU64(v) => v.canonical_location(),
            ValidatorEnum::MinimumI64(v) => v.canonical_location(),
            ValidatorEnum::MaximumI64(v) => v.canonical_location(),
            ValidatorEnum::ExclusiveMinimumI64(v) => v.canonical_location(),
            ValidatorEnum::ExclusiveMaximumI64(v) => v.canonical_location(),
            ValidatorEnum::MinimumF64(v) => v.canonical_location(),
            ValidatorEnum::MaximumF64(v) => v.canonical_location(),
            ValidatorEnum::ExclusiveMinimumF64(v) => v.canonical_location(),
            ValidatorEnum::ExclusiveMaximumF64(v) => v.canonical_location(),
            ValidatorEnum::MultipleOfInteger(v) => v.canonical_location(),
            ValidatorEnum::MultipleOfFloat(v) => v.canonical_location(),
            ValidatorEnum::MinLength(v) => v.canonical_location(),
            ValidatorEnum::MaxLength(v) => v.canonical_location(),
            ValidatorEnum::MinItems(v) => v.canonical_location(),
            ValidatorEnum::MaxItems(v) => v.canonical_location(),
            ValidatorEnum::MinProperties(v) => v.canonical_location(),
            ValidatorEnum::MaxProperties(v) => v.canonical_location(),
            ValidatorEnum::False(v) => v.canonical_location(),
            ValidatorEnum::Cold(v) => v.canonical_location(),
        }
    }
}

impl ColdValidatorEnum {
    pub(crate) fn canonical_location(&self) -> Option<&Location> {
        match self {
            ColdValidatorEnum::IntegerTypeDraft4(v) => v.canonical_location(),
            ColdValidatorEnum::MultipleTypesDraft4(v) => v.canonical_location(),
            ColdValidatorEnum::AdditionalItemsBoolean(v) => v.canonical_location(),
            ColdValidatorEnum::AdditionalItemsObject(v) => v.canonical_location(),
            ColdValidatorEnum::AdditionalPropertiesWithPatternsFalseFancy(v) => {
                v.canonical_location()
            }
            ColdValidatorEnum::AdditionalPropertiesWithPatternsFalseStd(v) => {
                v.canonical_location()
            }
            ColdValidatorEnum::AdditionalPropertiesWithPatternsFancy(v) => v.canonical_location(),
            ColdValidatorEnum::AdditionalPropertiesWithPatternsStd(v) => v.canonical_location(),
            ColdValidatorEnum::AdditionalPropertiesWithPatternsNotEmptyFalseHashFancy(v) => {
                v.canonical_location()
            }
            ColdValidatorEnum::AdditionalPropertiesWithPatternsNotEmptyFalseHashStd(v) => {
                v.canonical_location()
            }
            ColdValidatorEnum::AdditionalPropertiesWithPatternsNotEmptyFalseVecFancy(v) => {
                v.canonical_location()
            }
            ColdValidatorEnum::AdditionalPropertiesWithPatternsNotEmptyFalseVecStd(v) => {
                v.canonical_location()
            }
            ColdValidatorEnum::AdditionalPropertiesWithPatternsNotEmptyHashFancy(v) => {
                v.canonical_location()
            }
            ColdValidatorEnum::AdditionalPropertiesWithPatternsNotEmptyHashStd(v) => {
                v.canonical_location()
            }
            ColdValidatorEnum::AdditionalPropertiesWithPatternsNotEmptyVecFancy(v) => {
                v.canonical_location()
            }
            ColdValidatorEnum::AdditionalPropertiesWithPatternsNotEmptyVecStd(v) => {
                v.canonical_location()
            }
            ColdValidatorEnum::PatternPropertiesFancy(v) => v.canonical_location(),
            ColdValidatorEnum::PatternPropertiesStd(v) => v.canonical_location(),
            ColdValidatorEnum::PrefixPatternProperties(v) => v.canonical_location(),
            ColdValidatorEnum::SinglePrefixPatternProperties(v) => v.canonical_location(),
            ColdValidatorEnum::SingleValuePatternPropertiesFancy(v) => v.canonical_location(),
            ColdValidatorEnum::SingleValuePatternPropertiesStd(v) => v.canonical_location(),
            ColdValidatorEnum::PropertyNamesBoolean(v) => v.canonical_location(),
            ColdValidatorEnum::PropertyNamesObject(v) => v.canonical_location(),
            ColdValidatorEnum::Contains(v) => v.canonical_location(),
            ColdValidatorEnum::MinContains(v) => v.canonical_location(),
            ColdValidatorEnum::MaxContains(v) => v.canonical_location(),
            ColdValidatorEnum::MinMaxContains(v) => v.canonical_location(),
            ColdValidatorEnum::UniqueItems(v) => v.canonical_location(),
            ColdValidatorEnum::IfThen(v) => v.canonical_location(),
            ColdValidatorEnum::IfElse(v) => v.canonical_location(),
            ColdValidatorEnum::IfThenElse(v) => v.canonical_location(),
            ColdValidatorEnum::Dependencies(v) => v.canonical_location(),
            ColdValidatorEnum::DependentRequired(v) => v.canonical_location(),
            ColdValidatorEnum::DependentSchemas(v) => v.canonical_location(),
            ColdValidatorEnum::UnevaluatedProperties(v) => v.canonical_location(),
            ColdValidatorEnum::UnevaluatedItems(v) => v.canonical_location(),
            ColdValidatorEnum::Email(v) => v.canonical_location(),
            ColdValidatorEnum::IdnEmail(v) => v.canonical_location(),
            ColdValidatorEnum::CustomFormat(v) => v.canonical_location(),
            ColdValidatorEnum::Date(v) => v.canonical_location(),
            ColdValidatorEnum::DateTime(v) => v.canonical_location(),
            ColdValidatorEnum::Duration(v) => v.canonical_location(),
            ColdValidatorEnum::Hostname(v) => v.canonical_location(),
            ColdValidatorEnum::IdnHostname(v) => v.canonical_location(),
            ColdValidatorEnum::IpV4(v) => v.canonical_location(),
            ColdValidatorEnum::IpV6(v) => v.canonical_location(),
            ColdValidatorEnum::Iri(v) => v.canonical_location(),
            ColdValidatorEnum::IriReference(v) => v.canonical_location(),
            ColdValidatorEnum::JsonPointer(v) => v.canonical_location(),
            ColdValidatorEnum::FormatRegex(v) => v.canonical_location(),
            ColdValidatorEnum::RelativeJsonPointer(v) => v.canonical_location(),
            ColdValidatorEnum::Time(v) => v.canonical_location(),
            ColdValidatorEnum::Uri(v) => v.canonical_location(),
            ColdValidatorEnum::UriReference(v) => v.canonical_location(),
            ColdValidatorEnum::UriTemplate(v) => v.canonical_location(),
            ColdValidatorEnum::Uuid(v) => v.canonical_location(),
            ColdValidatorEnum::ContentEncoding(v) => v.canonical_location(),
            ColdValidatorEnum::ContentMediaType(v) => v.canonical_location(),
            ColdValidatorEnum::ContentMediaTypeAndEncoding(v) => v.canonical_location(),
            ColdValidatorEnum::Custom(v) => v.canonical_location(),
            #[cfg(feature = "arbitrary-precision")]
            ColdValidatorEnum::BigIntMinimum(v) => v.canonical_location(),
            #[cfg(feature = "arbitrary-precision")]
            ColdValidatorEnum::BigIntMaximum(v) => v.canonical_location(),
            #[cfg(feature = "arbitrary-precision")]
            ColdValidatorEnum::BigIntExclusiveMinimum(v) => v.canonical_location(),
            #[cfg(feature = "arbitrary-precision")]
            ColdValidatorEnum::BigIntExclusiveMaximum(v) => v.canonical_location(),
            #[cfg(feature = "arbitrary-precision")]
            ColdValidatorEnum::BigFracMinimum(v) => v.canonical_location(),
            #[cfg(feature = "arbitrary-precision")]
            ColdValidatorEnum::BigFracMaximum(v) => v.canonical_location(),
            #[cfg(feature = "arbitrary-precision")]
            ColdValidatorEnum::BigFracExclusiveMinimum(v) => v.canonical_location(),
            #[cfg(feature = "arbitrary-precision")]
            ColdValidatorEnum::BigFracExclusiveMaximum(v) => v.canonical_location(),
            #[cfg(feature = "arbitrary-precision")]
            ColdValidatorEnum::MultipleOfBigInt(v) => v.canonical_location(),
            #[cfg(feature = "arbitrary-precision")]
            ColdValidatorEnum::MultipleOfBigFrac(v) => v.canonical_location(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_enum_sizes() {
        println!("\n=== Enum Sizes ===");
        println!(
            "ValidatorEnum: {} bytes",
            std::mem::size_of::<ValidatorEnum>()
        );
        println!(
            "ColdValidatorEnum: {} bytes",
            std::mem::size_of::<ColdValidatorEnum>()
        );
        println!("==================\n");
    }
}

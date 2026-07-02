//! Read-only inspection of a canonical schema's IR.

use serde_json::{Number, Value};

use crate::{
    canonical::{
        emit::strip_synthetic_root,
        ir::{
            ArrayLeaf, BooleanBounds, BoundCardinality, BoundFraction, BoundInteger, CanonicalJson,
            IntegerBounds, IntegerLeaf, NumberBounds, NumberLeaf, ObjectLeaf, ObjectRequirement,
            OneOf, PropertyNameMatcher, Schema, SharedSchema, StringLeaf,
        },
        CanonicalSchema,
    },
    JsonType, JsonTypeSet,
};

pub use crate::canonical::ir::CanonicalKind;

impl CanonicalKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// The single, total view of a canonical node: exactly one arm per IR variant, matched once to dispatch. Children
/// are [`CanonicalSchema`] handles, viewed lazily by the caller.
#[derive(Debug, Clone)]
pub enum CanonicalView {
    Null,
    Boolean(BooleanView),
    Integer(NumericView),
    Number(NumericView),
    String(StringView),
    Array(ArrayView),
    Object(ObjectView),
    MultiType(JsonTypeSet),
    AllOf(Vec<CanonicalSchema>),
    AnyOf(Vec<CanonicalSchema>),
    OneOf(Vec<CanonicalSchema>),
    Not(CanonicalSchema),
    IfThenElse(IfThenElseView),
    /// A value matches iff its JSON type is the view's `ty` *and* it satisfies `body`; other types do not match.
    TypedGroup(TypedGroupView),
    /// Constrains only values of JSON type `ty` (must satisfy `body`); any other type matches unconditionally.
    TypeGuard(TypedGroupView),
    Const(Value),
    Enum(Vec<Value>),
    True,
    False,
    Reference(String),
    Recursive(String),
    DynamicRef(String),
    Raw(Value),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BooleanVariant {
    Any,
    JustTrue,
    JustFalse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BooleanView {
    pub variant: BooleanVariant,
}

/// Numeric bounds for `Integer`/`Number`. `minimum`/`exclusive_minimum` are mutually exclusive (likewise maximum):
/// the set one carries the bound, the other is `None`.
#[derive(Debug, Clone, Default)]
pub struct NumericView {
    pub minimum: Option<Number>,
    pub maximum: Option<Number>,
    pub exclusive_minimum: Option<Number>,
    pub exclusive_maximum: Option<Number>,
    pub multiple_of: Option<Number>,
    pub not_multiple_of: Vec<Number>,
}

#[derive(Debug, Clone)]
pub struct StringView {
    pub min_length: Option<Number>,
    pub max_length: Option<Number>,
    pub patterns: Vec<String>,
    pub not_patterns: Vec<String>,
    pub format: Option<String>,
    pub content: Vec<ContentFacetView>,
    pub extended_regex: bool,
}

#[derive(Debug, Clone)]
pub struct ContentFacetView {
    pub content_encoding: Option<String>,
    pub content_media_type: Option<String>,
    pub content_schema: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct ArrayView {
    pub prefix: Vec<CanonicalSchema>,
    pub tail: Option<CanonicalSchema>,
    pub min_items: Number,
    pub max_items: Option<Number>,
    pub unique_items: bool,
    pub repeated_items: bool,
    pub contains: Vec<ContainsView>,
}

#[derive(Debug, Clone)]
pub struct ContainsView {
    pub schema: CanonicalSchema,
    pub min_contains: Number,
    pub max_contains: Option<Number>,
}

#[derive(Debug, Clone)]
pub struct ObjectView {
    pub requirements: Vec<ObjectRequirementView>,
    pub constraints: Vec<ObjectConstraintView>,
    pub property_names: Option<CanonicalSchema>,
    pub min_properties: Option<Number>,
    pub max_properties: Option<Number>,
}

#[derive(Debug, Clone)]
pub enum ObjectRequirementView {
    RequiredProperty {
        name: String,
    },
    PatternPropertyRequirement {
        pattern: String,
        schema: CanonicalSchema,
    },
    /// Existential over the additional-name set: some property not covered by `properties`/`patternProperties`
    /// has a value satisfying `schema`.
    AdditionalPropertiesRequirement {
        schema: CanonicalSchema,
    },
    DependentPropertiesRequirement {
        property: String,
        required_properties: Vec<String>,
    },
    DependentSchemaRequirement {
        property: String,
        schema: CanonicalSchema,
    },
}

#[derive(Debug, Clone)]
pub enum ObjectConstraintView {
    NamedProperty {
        name: String,
        schema: CanonicalSchema,
    },
    PatternProperty {
        pattern: String,
        schema: CanonicalSchema,
    },
    AdditionalProperties {
        schema: CanonicalSchema,
    },
}

#[derive(Debug, Clone)]
pub struct IfThenElseView {
    pub condition: CanonicalSchema,
    pub then_branch: Option<CanonicalSchema>,
    pub else_branch: Option<CanonicalSchema>,
}

/// Payload shared by [`CanonicalView::TypedGroup`] and [`CanonicalView::TypeGuard`]: JSON type `ty` and a `body`
/// schema, read differently per arm (conjunction vs. implication).
#[derive(Debug, Clone)]
pub struct TypedGroupView {
    pub ty: JsonType,
    pub body: CanonicalSchema,
}

impl CanonicalSchema {
    /// The single view for this node. Match it once to dispatch.
    #[must_use]
    pub fn view(&self) -> CanonicalView {
        match self.as_schema() {
            Schema::Null => CanonicalView::Null,
            Schema::True => CanonicalView::True,
            Schema::False => CanonicalView::False,
            Schema::Boolean(bounds) => CanonicalView::Boolean(boolean_view(bounds)),
            Schema::Integer(leaf) => CanonicalView::Integer(integer_view(leaf)),
            Schema::Number(leaf) => CanonicalView::Number(number_view(leaf)),
            Schema::String(leaf) => CanonicalView::String(string_view(leaf)),
            Schema::Array(leaf) => CanonicalView::Array(self.array_view(leaf)),
            Schema::Object(leaf) => CanonicalView::Object(self.object_view(leaf)),
            Schema::MultiType(set) => CanonicalView::MultiType(*set),
            Schema::AllOf(branches) => CanonicalView::AllOf(self.wrap_children(branches)),
            Schema::AnyOf(branches) => CanonicalView::AnyOf(self.wrap_children(branches)),
            Schema::OneOf(OneOf(branches)) => CanonicalView::OneOf(self.wrap_children(branches)),
            Schema::Not(inner) => CanonicalView::Not(self.wrap_child(inner)),
            // `if`/`then`/`else` exists only in pre-pipeline IR: `canonicalize_one` always desugars it and
            // `negate`/`intersect` never build one, so no `CanonicalSchema` can hold it.
            Schema::IfThenElse(_) => {
                unreachable!("if/then/else is desugared during canonicalization")
            }
            Schema::TypedGroup { ty, body } => CanonicalView::TypedGroup(TypedGroupView {
                ty: *ty,
                body: self.wrap_child(body),
            }),
            Schema::TypeGuard { ty, body } => CanonicalView::TypeGuard(TypedGroupView {
                ty: *ty,
                body: self.wrap_child(body),
            }),
            Schema::Const(value) => CanonicalView::Const(value.to_value()),
            Schema::Enum(values) => {
                CanonicalView::Enum(values.iter().map(CanonicalJson::to_value).collect())
            }
            Schema::Reference(uri) => {
                CanonicalView::Reference(strip_synthetic_root(uri.as_str()).to_string())
            }
            // Re-keyed to the full target uri at parse, so it keys into `definitions()` exactly like `Reference`.
            Schema::Recursive(uri) => {
                CanonicalView::Recursive(strip_synthetic_root(uri).to_string())
            }
            Schema::DynamicRef(name) => CanonicalView::DynamicRef(name.to_string()),
            Schema::Raw(_) => CanonicalView::Raw(self.to_json_schema()),
        }
    }

    fn array_view(&self, leaf: &ArrayLeaf) -> ArrayView {
        // The IR stores an always-present tail; `Schema::True` means "open".
        let tail = if matches!(leaf.tail.as_schema(), Schema::True) {
            None
        } else {
            Some(self.wrap_child(&leaf.tail))
        };
        ArrayView {
            prefix: leaf.prefix.iter().map(|c| self.wrap_child(c)).collect(),
            tail,
            min_items: leaf.length.minimum.to_number(),
            max_items: leaf
                .length
                .maximum
                .as_ref()
                .map(BoundCardinality::to_number),
            unique_items: leaf.unique_items,
            repeated_items: leaf.repeated_items,
            contains: leaf
                .contains
                .iter()
                .map(|clause| ContainsView {
                    schema: self.wrap_child(&clause.schema),
                    min_contains: clause.min_contains.to_number(),
                    max_contains: clause
                        .max_contains
                        .as_ref()
                        .map(BoundCardinality::to_number),
                })
                .collect(),
        }
    }

    fn object_view(&self, leaf: &ObjectLeaf) -> ObjectView {
        let mut requirements = Vec::new();
        let mut min_properties = None;
        let mut max_properties = None;
        for requirement in &leaf.requirements {
            match requirement {
                ObjectRequirement::RequiredProperty(name) => {
                    requirements.push(ObjectRequirementView::RequiredProperty {
                        name: name.to_string(),
                    });
                }
                ObjectRequirement::PatternPropertyRequirement { matcher, schema } => {
                    match matcher {
                        PropertyNameMatcher::PatternProperty(pattern) => {
                            requirements.push(ObjectRequirementView::PatternPropertyRequirement {
                                pattern: pattern.to_string(),
                                schema: self.wrap_child(schema),
                            });
                        }
                        PropertyNameMatcher::AdditionalProperties => {
                            requirements.push(
                                ObjectRequirementView::AdditionalPropertiesRequirement {
                                    schema: self.wrap_child(schema),
                                },
                            );
                        }
                        PropertyNameMatcher::NamedProperty(name) => {
                            requirements.push(ObjectRequirementView::PatternPropertyRequirement {
                                pattern: format!("^{}$", regex::escape(name)),
                                schema: self.wrap_child(schema),
                            });
                        }
                    }
                }
                ObjectRequirement::DependentPropertiesRequirement {
                    property,
                    required_properties,
                } => {
                    requirements.push(ObjectRequirementView::DependentPropertiesRequirement {
                        property: property.to_string(),
                        required_properties: required_properties
                            .iter()
                            .map(ToString::to_string)
                            .collect(),
                    });
                }
                ObjectRequirement::DependentSchemaRequirement { property, schema } => {
                    requirements.push(ObjectRequirementView::DependentSchemaRequirement {
                        property: property.to_string(),
                        schema: self.wrap_child(schema),
                    });
                }
                ObjectRequirement::MinProperties(value) => {
                    min_properties = Some(value.to_number());
                }
                ObjectRequirement::MaxProperties(value) => {
                    max_properties = Some(value.to_number());
                }
            }
        }
        let constraints = leaf
            .constraints
            .iter()
            .map(|constraint| {
                let schema = self.wrap_child(&constraint.schema);
                match &constraint.matcher {
                    PropertyNameMatcher::NamedProperty(name) => {
                        ObjectConstraintView::NamedProperty {
                            name: name.to_string(),
                            schema,
                        }
                    }
                    PropertyNameMatcher::PatternProperty(pattern) => {
                        ObjectConstraintView::PatternProperty {
                            pattern: pattern.to_string(),
                            schema,
                        }
                    }
                    PropertyNameMatcher::AdditionalProperties => {
                        ObjectConstraintView::AdditionalProperties { schema }
                    }
                }
            })
            .collect();
        ObjectView {
            requirements,
            constraints,
            property_names: leaf.property_names.as_ref().map(|c| self.wrap_child(c)),
            min_properties,
            max_properties,
        }
    }

    fn wrap_children(&self, children: &[SharedSchema]) -> Vec<CanonicalSchema> {
        children.iter().map(|c| self.wrap_child(c)).collect()
    }
}

fn boolean_view(bounds: &BooleanBounds) -> BooleanView {
    let variant = match bounds {
        BooleanBounds::Any => BooleanVariant::Any,
        BooleanBounds::JustTrue => BooleanVariant::JustTrue,
        BooleanBounds::JustFalse => BooleanVariant::JustFalse,
    };
    BooleanView { variant }
}

fn integer_view(leaf: &IntegerLeaf) -> NumericView {
    let mut view = split_integer_bounds(&leaf.bounds);
    view.multiple_of = leaf.multiple_of.as_ref().map(BoundInteger::to_number);
    view.not_multiple_of = leaf
        .not_multiple_of
        .iter()
        .map(BoundInteger::to_number)
        .collect();
    view
}

fn number_view(leaf: &NumberLeaf) -> NumericView {
    let mut view = split_number_bounds(&leaf.bounds);
    view.multiple_of = leaf
        .multiple_of
        .as_ref()
        .and_then(BoundFraction::to_json_number);
    view.not_multiple_of = leaf
        .not_multiple_of
        .iter()
        .filter_map(BoundFraction::to_json_number)
        .collect();
    view
}

fn split_integer_bounds(bounds: &IntegerBounds) -> NumericView {
    let minimum = bounds.minimum.as_ref().map(BoundInteger::to_number);
    let maximum = bounds.maximum.as_ref().map(BoundInteger::to_number);
    split(
        minimum,
        bounds.exclusive_minimum,
        maximum,
        bounds.exclusive_maximum,
    )
}

fn split_number_bounds(bounds: &NumberBounds) -> NumericView {
    let minimum = bounds
        .minimum
        .as_ref()
        .and_then(BoundFraction::to_json_number);
    let maximum = bounds
        .maximum
        .as_ref()
        .and_then(BoundFraction::to_json_number);
    split(
        minimum,
        bounds.exclusive_minimum,
        maximum,
        bounds.exclusive_maximum,
    )
}

fn split(
    bound_minimum: Option<Number>,
    minimum_exclusive: bool,
    bound_maximum: Option<Number>,
    maximum_exclusive: bool,
) -> NumericView {
    let (minimum, exclusive_minimum) = if minimum_exclusive {
        (None, bound_minimum)
    } else {
        (bound_minimum, None)
    };
    let (maximum, exclusive_maximum) = if maximum_exclusive {
        (None, bound_maximum)
    } else {
        (bound_maximum, None)
    };
    NumericView {
        minimum,
        maximum,
        exclusive_minimum,
        exclusive_maximum,
        multiple_of: None,
        not_multiple_of: Vec::new(),
    }
}

fn string_view(leaf: &StringLeaf) -> StringView {
    StringView {
        min_length: leaf.min_length.as_ref().map(BoundCardinality::to_number),
        max_length: leaf.max_length.as_ref().map(BoundCardinality::to_number),
        patterns: leaf.patterns.iter().map(ToString::to_string).collect(),
        not_patterns: leaf.not_patterns.iter().map(ToString::to_string).collect(),
        format: leaf.format.as_ref().map(ToString::to_string),
        content: leaf
            .content
            .iter()
            .map(|facet| ContentFacetView {
                content_encoding: facet.content_encoding.as_ref().map(ToString::to_string),
                content_media_type: facet.content_media_type.as_ref().map(ToString::to_string),
                content_schema: facet.content_schema.as_ref().map(CanonicalJson::to_value),
            })
            .collect(),
        extended_regex: leaf.extended_regex(),
    }
}

impl CanonicalSchema {
    /// Cheap structural label for this node (no allocation, no recursion).
    #[must_use]
    pub fn kind(&self) -> CanonicalKind {
        self.as_schema().into()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::needless_pass_by_value)]

    use serde_json::{json, Number, Value};
    use test_case::test_case;

    use crate::{
        canonical::{
            options,
            view::{
                CanonicalKind, CanonicalView, NumericView, ObjectConstraintView,
                ObjectRequirementView, StringView, TypedGroupView,
            },
            CanonicalSchema,
        },
        canonicalize, JsonType,
    };

    fn kind_of(schema: Value) -> CanonicalKind {
        canonicalize(&schema)
            .unwrap_or_else(|e| panic!("canonicalize({schema}) failed: {e}"))
            .kind()
    }

    // Only canonicalize-reachable variants (see "Variant reachability"). Note `{"type":"boolean"}` -> Enum and
    // `{"const":null}` -> Const, by design.
    #[test_case(json!(true), CanonicalKind::True ; "true_schema")]
    #[test_case(json!(false), CanonicalKind::False ; "false_schema")]
    #[test_case(json!({"type": "integer"}), CanonicalKind::Integer ; "integer")]
    #[test_case(json!({"type": "number"}), CanonicalKind::Number ; "number")]
    #[test_case(json!({"type": "string"}), CanonicalKind::String ; "string")]
    #[test_case(json!({"type": "boolean"}), CanonicalKind::Enum ; "boolean_becomes_enum")]
    #[test_case(json!({"type": "array"}), CanonicalKind::Array ; "array")]
    #[test_case(json!({"type": "object"}), CanonicalKind::Object ; "object")]
    #[test_case(json!({"const": "x"}), CanonicalKind::Const ; "const_val")]
    #[test_case(json!({"enum": ["a", "b", "c"]}), CanonicalKind::Enum ; "enum_val")]
    #[test_case(json!({"type": ["integer", "string"]}), CanonicalKind::MultiType ; "multi_type")]
    #[test_case(json!({"minimum": 5}), CanonicalKind::TypeGuard ; "type_guard")]
    #[test_case(json!({"not": {"type": "integer"}}), CanonicalKind::Not ; "not")]
    #[test_case(json!({"anyOf": [{"type": "integer", "minimum": 0}, {"type": "string", "minLength": 2}]}), CanonicalKind::AnyOf ; "any_of")]
    #[test_case(json!({"allOf": [{"type": "number"}, {"not": {"type": "integer", "minimum": 1}}]}), CanonicalKind::AllOf ; "all_of")]
    fn kind_matches_variant(schema: Value, expected: CanonicalKind) {
        assert_eq!(kind_of(schema), expected);
    }

    #[test_case(CanonicalKind::MultiType, "multi_type" ; "multi_type")]
    #[test_case(CanonicalKind::IfThenElse, "if_then_else" ; "if_then_else")]
    #[test_case(CanonicalKind::DynamicRef, "dynamic_ref" ; "dynamic_ref")]
    fn kind_as_str_is_snake_case(kind: CanonicalKind, expected: &'static str) {
        assert_eq!(kind.as_str(), expected);
    }

    fn view_of(schema: Value) -> CanonicalView {
        canonicalize(&schema)
            .unwrap_or_else(|e| panic!("canonicalize({schema}) failed: {e}"))
            .view()
    }

    fn json_number(value: Value) -> Number {
        match value {
            Value::Number(n) => n,
            other => panic!("not a number: {other}"),
        }
    }

    // `Null`/`Boolean` views are not exercised here: canonicalize promotes them to `Const`/`Enum` (see "Variant
    // reachability"). The arms are present for match totality.
    #[test]
    fn marker_views() {
        assert!(matches!(view_of(json!(true)), CanonicalView::True));
        assert!(matches!(view_of(json!(false)), CanonicalView::False));
    }

    #[test]
    fn integer_view_splits_inclusive_and_exclusive_bounds() {
        // Exclusive integer bounds canonicalize to the inclusive neighbor.
        let CanonicalView::Integer(NumericView {
            minimum,
            exclusive_maximum,
            maximum,
            exclusive_minimum,
            multiple_of,
            ..
        }) = view_of(json!({"type": "integer", "minimum": 0, "exclusiveMaximum": 10}))
        else {
            panic!("expected Integer view");
        };
        assert_eq!(minimum, Some(json_number(json!(0))));
        assert_eq!(exclusive_minimum, None);
        assert_eq!(maximum, Some(json_number(json!(9))));
        assert_eq!(exclusive_maximum, None);
        assert_eq!(multiple_of, None);
        // With multipleOf, snap_upper converts exclusive 10 to inclusive 8.
        let CanonicalView::Integer(NumericView {
            multiple_of,
            maximum,
            exclusive_maximum: ex_max,
            ..
        }) = view_of(json!({"type": "integer", "exclusiveMaximum": 10, "multipleOf": 2}))
        else {
            panic!("expected Integer view");
        };
        assert_eq!(multiple_of, Some(json_number(json!(2))));
        assert_eq!(maximum, Some(json_number(json!(8))));
        assert_eq!(ex_max, None);
    }

    #[test]
    fn number_view_carries_fractional_bound() {
        let CanonicalView::Number(NumericView { multiple_of, .. }) =
            view_of(json!({"type": "number", "multipleOf": 0.5}))
        else {
            panic!("expected Number view");
        };
        assert_eq!(multiple_of, Some(json_number(json!(0.5))));
    }

    #[test]
    fn numeric_view_carries_not_multiple_of() {
        let CanonicalView::Integer(NumericView {
            not_multiple_of, ..
        }) = view_of(json!({"type": "integer", "not": {"type": "integer", "multipleOf": 3}}))
        else {
            panic!("expected Integer view");
        };
        assert_eq!(not_multiple_of, vec![json_number(json!(3))]);

        let CanonicalView::Number(NumericView {
            not_multiple_of, ..
        }) = view_of(json!({"type": "number", "not": {"type": "number", "multipleOf": 0.5}}))
        else {
            panic!("expected Number view");
        };
        assert_eq!(not_multiple_of, vec![json_number(json!(0.5))]);
    }

    #[test]
    fn array_view_open_tail_and_contains() {
        let CanonicalView::Array(view) = view_of(json!({
            "type": "array",
            "prefixItems": [{"type": "integer"}],
            "minItems": 1,
            "maxItems": 4,
            "uniqueItems": true,
            "contains": {"type": "string"},
            "minContains": 2
        })) else {
            panic!("expected Array view");
        };
        assert_eq!(view.prefix.len(), 1);
        assert!(matches!(view.prefix[0].view(), CanonicalView::Integer(_)));
        // `items` absent -> open tail.
        assert!(view.tail.is_none());
        // min_items lifted from minContains: 2 -> 2
        assert_eq!(view.min_items, json_number(json!(2)));
        assert_eq!(view.max_items, Some(json_number(json!(4))));
        assert!(view.unique_items);
        assert_eq!(view.contains.len(), 1);
        assert_eq!(view.contains[0].min_contains, json_number(json!(2)));
        assert_eq!(view.contains[0].max_contains, None);
        assert!(matches!(
            view.contains[0].schema.view(),
            CanonicalView::String(_)
        ));
    }

    #[test_case(json!({"type": "array", "not": {"type": "array", "uniqueItems": true}}) ; "typed_inner")]
    #[test_case(json!({"not": {"uniqueItems": true}}) ; "standalone_untyped_inner")]
    #[test_case(json!({"type": "array", "not": {"uniqueItems": true}}) ; "typed_outer_untyped_inner")]
    fn array_view_carries_repeated_items(schema: Value) {
        let CanonicalView::Array(view) = view_of(schema) else {
            panic!("expected Array view");
        };
        assert!(view.repeated_items && !view.unique_items);
        assert_eq!(view.min_items, json_number(json!(2)));
    }

    #[test]
    fn array_view_closed_tail() {
        let CanonicalView::Array(view) =
            view_of(json!({"type": "array", "items": {"type": "integer"}}))
        else {
            panic!("expected Array view");
        };
        assert!(matches!(
            view.tail.as_ref().map(CanonicalSchema::view),
            Some(CanonicalView::Integer(_))
        ));
    }

    #[test]
    fn const_and_enum_views() {
        assert!(
            matches!(view_of(json!({"const": "x"})), CanonicalView::Const(Value::String(s)) if s == "x")
        );
        let CanonicalView::Enum(values) = view_of(json!({"enum": ["a", "b", "c"]})) else {
            panic!("expected Enum view");
        };
        assert_eq!(values, vec![json!("a"), json!("b"), json!("c")]);
    }

    #[test]
    fn raw_view_round_trips_value() {
        // A `$dynamicRef` resolves against the runtime dynamic scope, which the structural IR does not model, so the
        // schema is preserved raw.
        let schema = json!({
            "$dynamicRef": "#node",
            "$defs": {"node": {"$dynamicAnchor": "node", "type": "object"}}
        });
        let canonical = canonicalize(&schema).expect("canonicalize");
        let CanonicalView::Raw(value) = canonical.view() else {
            panic!("expected Raw view, got {:?}", canonical.kind());
        };
        assert_eq!(value, canonical.to_json_schema());
    }

    #[test]
    fn multi_type_view_lists_types() {
        let CanonicalView::MultiType(types) = view_of(json!({"type": ["integer", "string"]}))
        else {
            panic!("expected MultiType view");
        };
        assert!(types.contains(JsonType::Integer));
        assert!(types.contains(JsonType::String));
    }

    #[test]
    fn any_of_view_exposes_branches() {
        // Constrained, non-foldable branches produce AnyOf (const-only branches fold to Enum). Branches are an Integer
        // leaf and a String leaf.
        let CanonicalView::AnyOf(branches) = view_of(
            json!({"anyOf": [{"type": "integer", "minimum": 0}, {"type": "string", "minLength": 2}]}),
        ) else {
            panic!("expected AnyOf view");
        };
        assert_eq!(branches.len(), 2);
        assert!(branches
            .iter()
            .any(|b| matches!(b.view(), CanonicalView::Integer(_))));
        assert!(branches
            .iter()
            .any(|b| matches!(b.view(), CanonicalView::String(_))));
    }

    #[test]
    fn all_of_view_exposes_branches() {
        let CanonicalView::AllOf(branches) = view_of(
            json!({"allOf": [{"type": "number"}, {"not": {"type": "integer", "minimum": 1}}]}),
        ) else {
            panic!("expected AllOf view");
        };
        assert_eq!(branches.len(), 2);
        assert!(branches
            .iter()
            .any(|b| matches!(b.view(), CanonicalView::Not(_))));
    }

    #[test]
    fn not_view_wraps_inner() {
        let CanonicalView::Not(inner) = view_of(json!({"not": {"type": "integer"}})) else {
            panic!("expected Not view");
        };
        assert!(matches!(inner.view(), CanonicalView::Integer(_)));
    }

    // Negating an untyped guard pins the in-kind complement under its type; the wrapper survives when the `AnyOf`
    // body keeps a pinned-kind-less member (a symbolic `Not`) that `collapse_typed_group` cannot unwrap.
    #[test_case(json!({"contains": {"const": 1}}), JsonType::Array ; "array_contains")]
    fn typed_group_view_carries_type_and_body(schema: Value, expected_ty: JsonType) {
        let negated = canonicalize(&schema)
            .unwrap_or_else(|e| panic!("canonicalize({schema}) failed: {e}"))
            .negate();
        let CanonicalView::TypedGroup(TypedGroupView { ty, body }) = negated.view() else {
            panic!("expected TypedGroup view, got {:?}", negated.kind());
        };
        assert_eq!(ty, expected_ty);
        assert!(matches!(body.view(), CanonicalView::AnyOf(_)));
    }

    // Negating a bounded numeric guard resolves to a clean union of number leaves -- no surviving
    // symbolic `Not` (the `negate_numeric_bounds_type_guard` dual; P1).
    #[test]
    fn negated_numeric_guard_has_no_symbolic_not() {
        let negated = canonicalize(&json!({"minimum": 0, "maximum": 10}))
            .unwrap_or_else(|e| panic!("{e}"))
            .negate();
        let CanonicalView::AnyOf(branches) = negated.view() else {
            panic!("expected AnyOf view, got {:?}", negated.kind());
        };
        assert!(branches
            .iter()
            .all(|b| matches!(b.view(), CanonicalView::Number(_))));
    }

    #[test]
    fn type_guard_view_carries_type_and_body() {
        let CanonicalView::TypeGuard(TypedGroupView { ty, body }) = view_of(json!({"minimum": 5}))
        else {
            panic!("expected TypeGuard view");
        };
        assert_eq!(ty, JsonType::Number);
        assert!(matches!(body.view(), CanonicalView::Number(_)));
    }

    #[test]
    fn object_view_splits_requirements_and_constraints() {
        let CanonicalView::Object(view) = view_of(json!({
            "type": "object",
            "properties": {"a": {"type": "integer"}},
            "patternProperties": {"^x": {"type": "string"}},
            "additionalProperties": false,
            "required": ["a"],
            // Above the required count, so the leaf keeps it (`minProperties` at or below it is implied away).
            "minProperties": 2,
            "maxProperties": 3
        })) else {
            panic!("expected Object view");
        };
        assert_eq!(view.min_properties, Some(json_number(json!(2))));
        assert_eq!(view.max_properties, Some(json_number(json!(3))));
        // min/max properties are NOT in `requirements`.
        assert!(view
            .requirements
            .iter()
            .any(|r| matches!(r, ObjectRequirementView::RequiredProperty { name } if name == "a")));
        assert!(view
            .constraints
            .iter()
            .any(|c| matches!(c, ObjectConstraintView::NamedProperty { name, .. } if name == "a")));
        assert!(view
            .constraints
            .iter()
            .any(|c| matches!(c, ObjectConstraintView::PatternProperty { pattern, .. } if pattern == "^x")));
        assert!(view
            .constraints
            .iter()
            .any(|c| matches!(c, ObjectConstraintView::AdditionalProperties { .. })));
    }

    #[test]
    fn object_view_additional_property_requirement_via_negation() {
        // Negating an `additionalProperties` constraint yields an `AdditionalPropertiesRequirement`.
        let negated = canonicalize(&json!({
            "type": "object",
            "additionalProperties": {"type": "integer"}
        }))
        .expect("canonicalize")
        .negate();
        let CanonicalView::AnyOf(branches) = negated.view() else {
            panic!("expected AnyOf after negation, got {:?}", negated.kind());
        };
        let has_additional_req = branches.iter().any(|branch| {
            if let CanonicalView::Object(obj) = branch.view() {
                obj.requirements.iter().any(|r| {
                    matches!(
                        r,
                        ObjectRequirementView::AdditionalPropertiesRequirement { .. }
                    )
                })
            } else {
                false
            }
        });
        assert!(has_additional_req);
    }

    #[test]
    fn string_view_carries_all_facets() {
        let CanonicalView::String(StringView {
            min_length,
            max_length,
            patterns,
            not_patterns,
            format,
            content,
            ..
        }) = view_of(
            json!({"type": "string", "minLength": 1, "maxLength": 5, "pattern": "^a", "format": "email"}),
        )
        else {
            panic!("expected String view");
        };
        assert_eq!(min_length, Some(json_number(json!(1))));
        assert_eq!(max_length, Some(json_number(json!(5))));
        assert_eq!(patterns, vec!["^a".to_string()]);
        assert!(not_patterns.is_empty());
        assert_eq!(format, Some("email".to_string()));
        assert!(content.is_empty());
    }

    #[test]
    fn string_view_carries_not_patterns() {
        let CanonicalView::String(StringView { not_patterns, .. }) =
            view_of(json!({"type": "string", "not": {"type": "string", "pattern": "^a"}}))
        else {
            panic!("expected String view");
        };
        assert_eq!(not_patterns, vec!["^a".to_string()]);
    }

    #[test]
    fn exclusive_minimum_splits_into_exclusive_minimum_field() {
        // The `true` branch of split() when minimum_exclusive is set; integers tighten exclusive
        // bounds away, so only `number` reaches it.
        let CanonicalView::Number(NumericView {
            minimum,
            exclusive_minimum,
            maximum,
            ..
        }) = view_of(json!({"type": "number", "exclusiveMinimum": 5, "maximum": 10}))
        else {
            panic!("expected Number view");
        };
        assert_eq!(exclusive_minimum, Some(json_number(json!(5))));
        assert_eq!(minimum, None);
        assert_eq!(maximum, Some(json_number(json!(10))));
    }

    #[test]
    fn reference_view_and_kind() {
        let schema = json!({
            "type": "object",
            "properties": {"x": {"$ref": "#/$defs/shared"}},
            "$defs": {"shared": {"type": "integer"}},
        });
        let canonical = options()
            .with_inline_budget(0)
            .canonicalize(&schema)
            .expect("canonicalize");
        let CanonicalView::Object(obj) = canonical.view() else {
            panic!("expected Object view");
        };
        let prop_schema = obj
            .constraints
            .iter()
            .find_map(|c| match c {
                ObjectConstraintView::NamedProperty { name, schema } if name == "x" => Some(schema),
                _ => None,
            })
            .expect("x constraint");
        assert!(matches!(prop_schema.view(), CanonicalView::Reference(_)));
        assert_eq!(prop_schema.kind(), CanonicalKind::Reference);
    }

    #[test]
    fn recursive_view_and_kind() {
        // A self-referencing schema; with inline_budget(0) the cycle survives as Recursive.
        let schema = json!({
            "type": "object",
            "properties": {"children": {"type": "array", "items": {"$ref": "#"}}},
        });
        let canonical = options()
            .with_inline_budget(0)
            .canonicalize(&schema)
            .expect("canonicalize");
        let CanonicalView::Object(obj) = canonical.view() else {
            panic!("expected Object view");
        };
        let children_schema = obj
            .constraints
            .iter()
            .find_map(|c| match c {
                ObjectConstraintView::NamedProperty { name, schema } if name == "children" => {
                    Some(schema)
                }
                _ => None,
            })
            .expect("children constraint");
        let CanonicalView::Array(arr) = children_schema.view() else {
            panic!("expected Array view");
        };
        let tail = arr.tail.expect("closed tail");
        assert!(matches!(tail.view(), CanonicalView::Recursive(_)));
        assert_eq!(tail.kind(), CanonicalKind::Recursive);
    }

    #[test]
    fn raw_schema_kind_is_raw() {
        let schema = json!({
            "$dynamicRef": "#node",
            "$defs": {"node": {"$dynamicAnchor": "node", "type": "object"}}
        });
        assert_eq!(
            canonicalize(&schema).expect("canonicalize").kind(),
            CanonicalKind::Raw
        );
    }

    #[test]
    fn object_view_dependent_required() {
        let CanonicalView::Object(view) = view_of(json!({
            "type": "object",
            "dependentRequired": {"a": ["b", "c"]}
        })) else {
            panic!("expected Object view");
        };
        assert!(view.requirements.iter().any(|r| matches!(
            r,
            ObjectRequirementView::DependentPropertiesRequirement { property, required_properties }
            if property == "a" && required_properties.contains(&"b".to_string())
        )));
    }

    #[test]
    fn object_view_dependent_schemas() {
        let CanonicalView::Object(view) = view_of(json!({
            "type": "object",
            "dependentSchemas": {"a": {"type": "integer"}}
        })) else {
            panic!("expected Object view");
        };
        assert!(view.requirements.iter().any(|r| matches!(
            r,
            ObjectRequirementView::DependentSchemaRequirement { property, .. }
            if property == "a"
        )));
    }

    #[test]
    fn string_view_carries_content_facets() {
        let CanonicalView::String(StringView { content, .. }) = view_of(json!({
            "type": "string",
            "contentEncoding": "base64",
            "contentMediaType": "application/json"
        })) else {
            panic!("expected String view");
        };
        assert_eq!(content.len(), 1);
        assert_eq!(content[0].content_encoding, Some("base64".to_string()));
        assert_eq!(
            content[0].content_media_type,
            Some("application/json".to_string())
        );
    }

    #[test]
    fn object_view_pattern_property_requirement_via_negation() {
        // Negating a `patternProperties` constraint yields a `PatternPropertyRequirement` (PatternProperty matcher).
        let negated = canonicalize(&json!({
            "type": "object",
            "patternProperties": {"^x": {"type": "integer"}}
        }))
        .expect("canonicalize")
        .negate();
        let CanonicalView::AnyOf(branches) = negated.view() else {
            panic!("expected AnyOf after negation, got {:?}", negated.kind());
        };
        let has_pattern_req = branches.iter().any(|branch| {
            if let CanonicalView::Object(obj) = branch.view() {
                obj.requirements.iter().any(|r| {
                    matches!(
                        r,
                        ObjectRequirementView::PatternPropertyRequirement { pattern, .. }
                        if pattern == "^x"
                    )
                })
            } else {
                false
            }
        });
        assert!(has_pattern_req);
    }

    #[test]
    fn named_property_requirement_renders_as_anchored_pattern() {
        let negated = canonicalize(&json!({
            "type": "object",
            "properties": {"a": {"type": "integer"}}
        }))
        .expect("canonicalize")
        .negate();
        let CanonicalView::AnyOf(branches) = negated.view() else {
            panic!("expected AnyOf after negation, got {:?}", negated.kind());
        };
        let found = branches.iter().any(|branch| {
            let CanonicalView::Object(obj) = branch.view() else {
                return false;
            };
            obj.requirements.iter().any(|requirement| {
                matches!(
                    requirement,
                    ObjectRequirementView::PatternPropertyRequirement { pattern, .. }
                    if pattern == "^a$"
                )
            })
        });
        assert!(found, "expected an anchored-name pattern requirement");
    }
}

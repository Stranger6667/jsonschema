use std::collections::{BTreeMap, HashMap, HashSet};

use super::super::{
    compile_schema,
    draft::DraftExt,
    errors::{invalid_schema_non_empty_array_expression, invalid_schema_type_expression},
    refs::resolve_lone_top_level_ref,
    CompileContext, CompiledExpr,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::Value;

/// A typed constant value that can serve as a oneOf discriminator.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum DiscriminatorLiteral {
    // Variant order determines the derived Ord between kinds: Str < Bool < Int.
    Str(String),
    Bool(bool),
    Int(i64),
}

impl DiscriminatorLiteral {
    fn kind(&self) -> DiscriminatorKind {
        match self {
            Self::Str(_) => DiscriminatorKind::Str,
            Self::Bool(_) => DiscriminatorKind::Bool,
            Self::Int(_) => DiscriminatorKind::Int,
        }
    }

    /// The token for the inner value, used when building nested or-patterns.
    fn to_inner_token(&self) -> TokenStream {
        match self {
            Self::Str(value) => quote! { #value },
            Self::Bool(value) => quote! { #value },
            Self::Int(value) => quote! { #value },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DiscriminatorKind {
    Str,
    Bool,
    Int,
}

/// `per_branch_values[i]` is `None` when branch `i` has no discriminator entry.
struct DiscriminatorPlan {
    key: String,
    kind: DiscriminatorKind,
    per_branch_values: Vec<Option<Vec<DiscriminatorLiteral>>>,
    vacuous_guarded: usize,
}

/// Per-key stats accumulated while searching for the best discriminator.
/// `(coverage, distinct_values, total_cardinality, kind, invalid)`. `invalid`
/// latches when a key's `const`/`enum` kinds disagree across branches.
type KeyStats = (
    usize,
    HashSet<DiscriminatorLiteral>,
    usize,
    Option<DiscriminatorKind>,
    bool,
);

pub(crate) fn compile(ctx: &mut CompileContext<'_>, value: &Value) -> CompiledExpr {
    let Some(schemas) = value.as_array() else {
        return invalid_schema_type_expression(value, &["array"]);
    };
    if schemas.is_empty() {
        return invalid_schema_non_empty_array_expression();
    }

    let discriminator_plan = build_discriminator_plan(ctx, schemas);
    let allow_const = ctx.draft.supports_const_keyword();

    let mut branch_validations: Vec<TokenStream> = Vec::new();
    let mut branch_collectors = Vec::with_capacity(schemas.len());

    for (idx, schema) in schemas.iter().enumerate() {
        let discriminator_values = discriminator_plan
            .as_ref()
            .and_then(|plan| plan.per_branch_values[idx].as_ref());
        let reduced_property = match (&discriminator_plan, discriminator_values) {
            (Some(plan), Some(values)) => reduce_discriminator_property(
                schema,
                &plan.key,
                values,
                allow_const,
                &ctx.config.custom_keywords,
            ),
            _ => None,
        };
        let compiled = ctx.with_schema_path_segment("oneOf", |ctx| {
            ctx.with_schema_path_segment(&idx.to_string(), |ctx| {
                if let (Some(plan), Some(reduced_property)) =
                    (&discriminator_plan, reduced_property)
                {
                    ctx.with_discriminator_assumption(&plan.key, reduced_property, |ctx| {
                        compile_schema(ctx, schema)
                    })
                } else {
                    compile_schema(ctx, schema)
                }
            })
        });
        let branch_helper = ctx.register_branch_helper(
            compiled.is_valid_token_stream(),
            compiled.collect.as_token_stream(),
        );
        branch_collectors.push(format_ident!("collect_branch_errors_{}", branch_helper));
        let branch_is_valid = format_ident!("is_branch_valid_{}", branch_helper);
        let branch_validation = if let (Some(_), Some(discriminator_values)) =
            (&discriminator_plan, discriminator_values)
        {
            // Nested or-patterns `Some("a" | "b")` (not `Some("a") | Some("b")`) satisfy
            // the `unnested_or_patterns` lint in generated code.
            let inner: Vec<TokenStream> = discriminator_values
                .iter()
                .map(DiscriminatorLiteral::to_inner_token)
                .collect();
            quote! {
                (matches!(__one_of_discriminator, Some(#(#inner)|*))
                    && { #branch_is_valid(instance) })
            }
        } else {
            quote! { #branch_is_valid(instance) }
        };

        branch_validations.push(branch_validation);
    }

    let discriminator_init = if let Some(plan) = &discriminator_plan {
        let init_expr = match plan.kind {
            DiscriminatorKind::Str => {
                crate::codegen::emit_serde::instance_object_property_as_str(&plan.key)
            }
            DiscriminatorKind::Bool => {
                crate::codegen::emit_serde::instance_object_property_as_bool(&plan.key)
            }
            DiscriminatorKind::Int => {
                crate::codegen::emit_serde::instance_object_property_as_i64(&plan.key)
            }
        };
        quote! { let __one_of_discriminator = #init_expr; }
    } else {
        quote! {}
    };
    let validate_correction = match &discriminator_plan {
        Some(plan) if plan.vacuous_guarded > 0 => {
            let vacuous_guarded = plan.vacuous_guarded;
            let is_object = crate::codegen::emit_serde::instance_is_object();
            quote! { if !(#is_object) { __count += #vacuous_guarded; } }
        }
        _ => quote! {},
    };
    ctx.branch_gate_cache.insert(
        schemas.as_ptr() as usize,
        crate::context::BranchGates {
            init: discriminator_init.clone(),
            gates: branch_validations.clone(),
        },
    );

    let first_check = &branch_validations[0];
    let rest_checks: Vec<TokenStream> = branch_validations[1..]
        .iter()
        .map(|check| {
            quote! {
                if #check {
                    if matched { false } else { matched = true; true }
                } else {
                    true
                }
            }
        })
        .collect();
    let is_valid = if rest_checks.is_empty() {
        quote! {
            {
                #discriminator_init
                #first_check
            }
        }
    } else {
        quote! {
            {
                #discriminator_init
                let mut matched = #first_check;
                ( #(#rest_checks)&&* ) && matched
            }
        }
    };
    let schema_path = ctx.schema_path_for_keyword("oneOf");
    let branch_count = branch_collectors.len();

    CompiledExpr::with_validate_blocks(
        is_valid,
        quote! {
            {
                #discriminator_init
                let mut __count = 0usize;
                #(if #branch_validations { __count += 1; })*
                #validate_correction
                if __count != 1 {
                    let mut __context = Vec::with_capacity(#branch_count);
                    #({
                        let mut __branch_errors = Vec::new();
                        #branch_collectors(instance, __path, &mut __branch_errors);
                        __context.push(__branch_errors);
                    })*
                    return Some(if __count == 0 {
                        __err::one_of_not_valid(
                            #schema_path, __path.into(), instance, __context,
                        )
                    } else {
                        __err::one_of_multiple_valid(
                            #schema_path, __path.into(), instance, __context,
                        )
                    });
                }
            }
        },
    )
}

fn reduce_discriminator_property(
    schema: &Value,
    key: &str,
    values: &[DiscriminatorLiteral],
    allow_const: bool,
    custom_keywords: &HashMap<String, TokenStream>,
) -> Option<Value> {
    if extract_object(schema, allow_const, custom_keywords)
        .get(key)
        .map(Vec::as_slice)
        != Some(values)
    {
        return None;
    }
    let mut reduced = schema
        .as_object()?
        .get("properties")?
        .as_object()?
        .get(key)?
        .clone();
    let property = reduced.as_object_mut()?;
    if !(allow_const && property.remove("const").is_some()) {
        property.remove("enum");
    }
    let implied_type = match values.first()?.kind() {
        DiscriminatorKind::Str => Some("string"),
        DiscriminatorKind::Bool => Some("boolean"),
        DiscriminatorKind::Int => None,
    };
    if let Some(implied) = implied_type {
        if property.get("type").and_then(Value::as_str) == Some(implied) {
            property.remove("type");
        }
    }
    Some(reduced)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BranchObjectApplicability {
    Typed,
    Vacuous,
    Other,
}

const OBJECT_ONLY_OR_ANNOTATION_KEYWORDS: &[&str] = &[
    "properties",
    "patternProperties",
    "additionalProperties",
    "required",
    "minProperties",
    "maxProperties",
    "propertyNames",
    "dependencies",
    "dependentRequired",
    "dependentSchemas",
    "title",
    "description",
    "default",
    "examples",
    "$comment",
    "deprecated",
    "readOnly",
    "writeOnly",
    "definitions",
    "$defs",
];

fn classify_branch(
    schema: &Value,
    custom_keywords: &HashMap<String, TokenStream>,
) -> BranchObjectApplicability {
    let Value::Object(obj) = schema else {
        return BranchObjectApplicability::Other;
    };
    if obj
        .keys()
        .any(|key| custom_keywords.contains_key(key.as_str()))
    {
        return BranchObjectApplicability::Other;
    }
    if is_explicit_object_only_type(obj.get("type")) {
        return BranchObjectApplicability::Typed;
    }
    if obj
        .keys()
        .all(|key| OBJECT_ONLY_OR_ANNOTATION_KEYWORDS.contains(&key.as_str()))
    {
        BranchObjectApplicability::Vacuous
    } else {
        BranchObjectApplicability::Other
    }
}

fn vacuous_branches_safely_guarded(
    classes: &[BranchObjectApplicability],
    plan: &DiscriminatorPlan,
) -> bool {
    let vacuous_count = classes
        .iter()
        .filter(|class| **class == BranchObjectApplicability::Vacuous)
        .count();
    if vacuous_count == 0 {
        return true;
    }
    if vacuous_count < 2 || classes.contains(&BranchObjectApplicability::Other) {
        return false;
    }
    classes
        .iter()
        .zip(&plan.per_branch_values)
        .all(|(class, values)| *class != BranchObjectApplicability::Vacuous || values.is_some())
}

fn build_discriminator_plan(
    ctx: &mut CompileContext<'_>,
    schemas: &[Value],
) -> Option<DiscriminatorPlan> {
    if !ctx.supports_validation_vocabulary() || !ctx.supports_applicator_vocabulary() {
        return None;
    }
    let allow_const = ctx.draft.supports_const_keyword();
    let mut classes = Vec::with_capacity(schemas.len());
    let mut branch_discriminators = Vec::with_capacity(schemas.len());
    for schema in schemas {
        let (resolved, _hopped) = resolve_lone_top_level_ref(ctx, schema);
        classes.push(classify_branch(resolved, &ctx.config.custom_keywords));
        branch_discriminators.push(extract_object(
            resolved,
            allow_const,
            &ctx.config.custom_keywords,
        ));
    }

    if let Some(mut plan) = select_discriminator(&branch_discriminators) {
        if vacuous_branches_safely_guarded(&classes, &plan) {
            plan.vacuous_guarded = classes
                .iter()
                .filter(|class| **class == BranchObjectApplicability::Vacuous)
                .count();
            return Some(plan);
        }
    }
    let mut any_cleared = false;
    for (discriminators, class) in branch_discriminators.iter_mut().zip(&classes) {
        if *class == BranchObjectApplicability::Vacuous && !discriminators.is_empty() {
            discriminators.clear();
            any_cleared = true;
        }
    }
    if !any_cleared {
        return None;
    }
    select_discriminator(&branch_discriminators)
}

fn select_discriminator(
    branch_discriminators: &[HashMap<String, Vec<DiscriminatorLiteral>>],
) -> Option<DiscriminatorPlan> {
    // For each candidate key: track (coverage, distinct_value_set, total_cardinality, kind).
    // Keys whose values have inconsistent kinds across branches are rejected.
    // BTreeMap: iteration order below decides ties, so it must not depend on hashing.
    let mut stats: BTreeMap<String, KeyStats> = BTreeMap::new();
    for branch in branch_discriminators {
        for (key, values) in branch {
            let entry = stats.entry(key.clone()).or_default();
            let branch_kind = values.first().map(DiscriminatorLiteral::kind);
            match (entry.3, branch_kind) {
                (None, Some(kind)) => entry.3 = Some(kind),
                (Some(existing), Some(kind)) if existing != kind => {
                    // Conflicting kinds disqualify the key as a discriminator.
                    entry.4 = true;
                }
                _ => {}
            }
            entry.0 += 1;
            entry.2 += values.len();
            for value in values {
                entry.1.insert(value.clone());
            }
        }
    }

    let mut best: Option<(String, usize, usize, usize, DiscriminatorKind)> = None;
    for (key, (coverage, values, total_cardinality, kind, invalid)) in stats {
        if invalid {
            continue;
        }
        let Some(kind) = kind else { continue };
        let distinct_values = values.len();
        if coverage < 2 || distinct_values < 2 {
            continue;
        }
        match &best {
            None => best = Some((key, coverage, distinct_values, total_cardinality, kind)),
            Some((_, best_coverage, best_distinct, best_total_cardinality, _)) => {
                if coverage > *best_coverage
                    || (coverage == *best_coverage
                        && (distinct_values > *best_distinct
                            || (distinct_values == *best_distinct
                                && total_cardinality < *best_total_cardinality)))
                {
                    best = Some((key, coverage, distinct_values, total_cardinality, kind));
                }
            }
        }
    }

    let (key, _, _, _, kind) = best?;
    let per_branch_values = branch_discriminators
        .iter()
        .map(|branch| branch.get(&key).cloned())
        .collect();
    Some(DiscriminatorPlan {
        key,
        kind,
        per_branch_values,
        vacuous_guarded: 0,
    })
}

fn extract_object(
    schema: &Value,
    allow_const: bool,
    custom_keywords: &HashMap<String, TokenStream>,
) -> HashMap<String, Vec<DiscriminatorLiteral>> {
    let Value::Object(obj) = schema else {
        return HashMap::new();
    };
    if classify_branch(schema, custom_keywords) == BranchObjectApplicability::Other {
        return HashMap::new();
    }
    let required: Vec<&str> = obj
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let Some(properties) = obj.get("properties").and_then(Value::as_object) else {
        return HashMap::new();
    };
    let mut discriminators = HashMap::new();
    for name in required {
        let Some(Value::Object(prop_schema)) = properties.get(name) else {
            continue;
        };
        if allow_const {
            if let Some(literal) = const_as_literal(prop_schema.get("const")) {
                discriminators.insert(name.to_owned(), vec![literal]);
                continue;
            }
        }
        if let Some(literals) = enum_as_literals(prop_schema.get("enum")) {
            discriminators.insert(name.to_owned(), literals);
        }
    }
    discriminators
}

fn integer_literal(number: &serde_json::Number) -> Option<DiscriminatorLiteral> {
    number
        .as_i64()
        .filter(|value| value.unsigned_abs() <= 1 << 53)
        .map(DiscriminatorLiteral::Int)
}

fn const_as_literal(value: Option<&Value>) -> Option<DiscriminatorLiteral> {
    match value? {
        Value::String(string) => Some(DiscriminatorLiteral::Str(string.clone())),
        Value::Bool(boolean) => Some(DiscriminatorLiteral::Bool(*boolean)),
        Value::Number(number) => integer_literal(number),
        _ => None,
    }
}

fn enum_as_literals(value: Option<&Value>) -> Option<Vec<DiscriminatorLiteral>> {
    let items = value?.as_array()?;
    if items.is_empty() {
        return None;
    }
    let mut literals = Vec::with_capacity(items.len());
    for item in items {
        let literal = match item {
            Value::String(string) => DiscriminatorLiteral::Str(string.clone()),
            Value::Bool(boolean) => DiscriminatorLiteral::Bool(*boolean),
            Value::Number(number) => integer_literal(number)?,
            _ => return None,
        };
        if let Some(first) = literals.first() {
            if DiscriminatorLiteral::kind(first) != literal.kind() {
                return None;
            }
        }
        literals.push(literal);
    }
    literals.sort_unstable();
    literals.dedup();
    Some(literals)
}

fn is_explicit_object_only_type(type_value: Option<&Value>) -> bool {
    match type_value {
        Some(Value::String(single)) => single == "object",
        Some(Value::Array(types)) => {
            !types.is_empty() && types.iter().all(|item| item.as_str() == Some("object"))
        }
        _ => false,
    }
}

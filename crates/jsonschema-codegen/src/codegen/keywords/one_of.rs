use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

use super::super::{
    compile_schema,
    errors::{invalid_schema_non_empty_array_expression, invalid_schema_type_expression},
    refs::resolve_top_level_ref_for_one_of_analysis,
    CompileContext, CompiledExpr,
};
use proc_macro2::TokenStream;
use quote::quote;
use serde_json::Value;

/// A typed constant value that can serve as a oneOf discriminator.
///
/// The `'schema` lifetime lets `Str` borrow directly from inline branch schemas
/// (the common case) rather than cloning.  When a branch is reached via `$ref`,
/// the resolved `Value` is owned and the string falls back to `Cow::Owned`.
/// `Bool` and `Int` are `Copy` types and never allocate.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum DiscriminatorLiteral<'schema> {
    // Variant order determines the derived Ord between kinds: Str < Bool < Int.
    Str(Cow<'schema, str>),
    Bool(bool),
    Int(i64),
}

impl DiscriminatorLiteral<'_> {
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
            Self::Str(s) => {
                let s: &str = s.as_ref();
                quote! { #s }
            }
            Self::Bool(b) => quote! { #b },
            Self::Int(n) => quote! { #n },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DiscriminatorKind {
    Str,
    Bool,
    Int,
}

/// `(discriminator_key, kind, per_branch_values)`.
/// `per_branch_values[i]` is `None` when branch `i` has no discriminator entry.
type DiscriminatorPlan<'schema> = (
    String,
    DiscriminatorKind,
    Vec<Option<Vec<DiscriminatorLiteral<'schema>>>>,
);

/// Per-key stats accumulated while searching for the best discriminator.
/// `(coverage, distinct_values, total_cardinality, kind)`
type KeyStats<'schema> = (
    usize,
    HashSet<DiscriminatorLiteral<'schema>>,
    usize,
    Option<DiscriminatorKind>,
);

pub(crate) fn compile(ctx: &mut CompileContext<'_>, value: &Value) -> CompiledExpr {
    let Some(schemas) = value.as_array() else {
        return invalid_schema_type_expression(value, &["array"]);
    };
    if schemas.is_empty() {
        return invalid_schema_non_empty_array_expression();
    }

    let discriminator_plan = build_discriminator_plan(ctx, schemas);

    let mut checks: Vec<TokenStream> = Vec::new();
    let mut branch_is_valid_checks: Vec<TokenStream> = Vec::new();

    for (idx, schema) in schemas.iter().enumerate() {
        let validation = compile_schema(ctx, schema);
        let plain_is_valid = validation.is_valid_ts();
        branch_is_valid_checks.push(plain_is_valid.clone());

        let branch_validation = if let Some((_, _, branch_discriminators)) = &discriminator_plan {
            if let Some(discriminator_values) = &branch_discriminators[idx] {
                // Always use nested or-patterns: `Some("a" | "b")` rather than
                // `Some("a") | Some("b")` — the former satisfies the
                // `unnested_or_patterns` lint in generated code.
                let inner: Vec<TokenStream> = discriminator_values
                    .iter()
                    .map(DiscriminatorLiteral::to_inner_token)
                    .collect();
                quote! {
                    (matches!(__one_of_discriminator, Some(#(#inner)|*))
                        && { #plain_is_valid })
                }
            } else {
                plain_is_valid
            }
        } else {
            plain_is_valid
        };

        checks.push(quote! {
            if #branch_validation {
                if matched { false } else { matched = true; true }
            } else {
                true
            }
        });
    }

    let discriminator_init = if let Some((discriminator_key, kind, _)) = &discriminator_plan {
        let init_expr = match kind {
            DiscriminatorKind::Str => ctx
                .config
                .backend
                .instance_object_property_as_str(discriminator_key),
            DiscriminatorKind::Bool => ctx
                .config
                .backend
                .instance_object_property_as_bool(discriminator_key),
            DiscriminatorKind::Int => ctx
                .config
                .backend
                .instance_object_property_as_i64(discriminator_key),
        };
        quote! { let __one_of_discriminator = #init_expr; }
    } else {
        quote! {}
    };

    let is_valid_ts = quote! {
        {
            #discriminator_init
            let mut matched = false;
            ( #(#checks)&&* ) && matched
        }
    };
    let schema_path = ctx.schema_path_for_keyword("oneOf");

    CompiledExpr::with_validate_blocks(
        is_valid_ts,
        quote! {
            {
                let mut __count = 0usize;
                #(if #branch_is_valid_checks { __count += 1; })*
                if __count == 0 {
                    return Some(jsonschema::keywords_helpers::error::one_of_not_valid(
                        #schema_path, __path.clone(), instance,
                    ));
                }
                if __count > 1 {
                    return Some(jsonschema::keywords_helpers::error::one_of_multiple_valid(
                        #schema_path, __path.clone(), instance,
                    ));
                }
            }
        },
        quote! {
            {
                let mut __count = 0usize;
                #(if #branch_is_valid_checks { __count += 1; })*
                if __count == 0 {
                    __errors.push(jsonschema::keywords_helpers::error::one_of_not_valid(
                        #schema_path, __path.clone(), instance,
                    ));
                } else if __count > 1 {
                    __errors.push(jsonschema::keywords_helpers::error::one_of_multiple_valid(
                        #schema_path, __path.clone(), instance,
                    ));
                }
            }
        },
    )
}

fn build_discriminator_plan<'schema>(
    ctx: &mut CompileContext<'_>,
    schemas: &'schema [Value],
) -> Option<DiscriminatorPlan<'schema>> {
    let branch_discriminators: Vec<HashMap<Cow<'schema, str>, Vec<DiscriminatorLiteral<'schema>>>> =
        schemas
            .iter()
            .map(|schema| extract_discriminators(ctx, schema))
            .collect();

    // For each candidate key: track (coverage, distinct_value_set, total_cardinality, kind).
    // Keys whose values have inconsistent kinds across branches are rejected.
    let mut stats: HashMap<Cow<'schema, str>, KeyStats<'schema>> = HashMap::new();
    for branch in &branch_discriminators {
        for (key, values) in branch {
            let entry = stats.entry(key.clone()).or_default();
            let branch_kind = values.first().map(DiscriminatorLiteral::kind);
            match (entry.3, branch_kind) {
                (None, Some(k)) => entry.3 = Some(k),
                (Some(existing), Some(k)) if existing != k => {
                    // Conflicting kinds — mark invalid.
                    entry.0 = 0;
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

    let mut best: Option<(Cow<'schema, str>, usize, usize, usize, DiscriminatorKind)> = None;
    for (key, (coverage, values, total_cardinality, kind)) in stats {
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
    let key_str = key.into_owned();
    let per_branch_values = branch_discriminators
        .iter()
        .map(|branch| branch.get(key_str.as_str()).cloned())
        .collect();
    Some((key_str, kind, per_branch_values))
}

/// Extracts typed discriminator candidates from a branch schema.
///
/// Dispatches to `extract_object_borrowed` when the schema is inline (the common
/// case — zero string allocations) and to `extract_object_owned` when a `$ref`
/// was followed to produce an owned `Value`.
fn extract_discriminators<'schema>(
    ctx: &mut CompileContext<'_>,
    schema: &'schema Value,
) -> HashMap<Cow<'schema, str>, Vec<DiscriminatorLiteral<'schema>>> {
    match resolve_top_level_ref_for_one_of_analysis(ctx, schema) {
        Cow::Borrowed(v) => extract_object_borrowed(v),
        Cow::Owned(v) => extract_object_owned(&v),
    }
}

/// Zero-allocation path: all strings are borrowed from the schema with `'schema`.
fn extract_object_borrowed<'schema>(
    schema: &'schema Value,
) -> HashMap<Cow<'schema, str>, Vec<DiscriminatorLiteral<'schema>>> {
    let Value::Object(obj) = schema else {
        return HashMap::new();
    };
    if !is_explicit_object_only_type(obj.get("type")) {
        return HashMap::new();
    }
    let required: Vec<&'schema str> = obj
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let Some(properties) = obj.get("properties").and_then(Value::as_object) else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for name in required {
        let Some(Value::Object(prop_schema)) = properties.get(name) else {
            continue;
        };
        if let Some(lit) = const_as_literal_borrowed(prop_schema.get("const")) {
            out.insert(Cow::Borrowed(name), vec![lit]);
            continue;
        }
        if let Some(lits) = enum_as_literals_borrowed(prop_schema.get("enum")) {
            out.insert(Cow::Borrowed(name), lits);
        }
    }
    out
}

/// Fallback path for `$ref`-resolved schemas: strings are cloned into `Cow::Owned`.
fn extract_object_owned<'schema>(
    schema: &Value,
) -> HashMap<Cow<'schema, str>, Vec<DiscriminatorLiteral<'schema>>> {
    let Value::Object(obj) = schema else {
        return HashMap::new();
    };
    if !is_explicit_object_only_type(obj.get("type")) {
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
    let mut out = HashMap::new();
    for name in required {
        let Some(Value::Object(prop_schema)) = properties.get(name) else {
            continue;
        };
        if let Some(lit) = const_as_literal_owned(prop_schema.get("const")) {
            out.insert(Cow::Owned(name.to_owned()), vec![lit]);
            continue;
        }
        if let Some(lits) = enum_as_literals_owned(prop_schema.get("enum")) {
            out.insert(Cow::Owned(name.to_owned()), lits);
        }
    }
    out
}

fn const_as_literal_borrowed(value: Option<&Value>) -> Option<DiscriminatorLiteral<'_>> {
    match value? {
        Value::String(s) => Some(DiscriminatorLiteral::Str(Cow::Borrowed(s.as_str()))),
        Value::Bool(b) => Some(DiscriminatorLiteral::Bool(*b)),
        Value::Number(n) => n.as_i64().map(DiscriminatorLiteral::Int),
        _ => None,
    }
}

fn const_as_literal_owned<'schema>(value: Option<&Value>) -> Option<DiscriminatorLiteral<'schema>> {
    match value? {
        Value::String(s) => Some(DiscriminatorLiteral::Str(Cow::Owned(s.clone()))),
        Value::Bool(b) => Some(DiscriminatorLiteral::Bool(*b)),
        Value::Number(n) => n.as_i64().map(DiscriminatorLiteral::Int),
        _ => None,
    }
}

fn enum_as_literals_borrowed(value: Option<&Value>) -> Option<Vec<DiscriminatorLiteral<'_>>> {
    let items = value?.as_array()?;
    if items.is_empty() {
        return None;
    }
    let mut literals = Vec::with_capacity(items.len());
    for item in items {
        let lit = match item {
            Value::String(s) => DiscriminatorLiteral::Str(Cow::Borrowed(s.as_str())),
            Value::Bool(b) => DiscriminatorLiteral::Bool(*b),
            Value::Number(n) => DiscriminatorLiteral::Int(n.as_i64()?),
            _ => return None,
        };
        if let Some(first) = literals.first() {
            if DiscriminatorLiteral::kind(first) != lit.kind() {
                return None;
            }
        }
        literals.push(lit);
    }
    literals.sort_unstable();
    literals.dedup();
    Some(literals)
}

fn enum_as_literals_owned<'schema>(
    value: Option<&Value>,
) -> Option<Vec<DiscriminatorLiteral<'schema>>> {
    let items = value?.as_array()?;
    if items.is_empty() {
        return None;
    }
    let mut literals = Vec::with_capacity(items.len());
    for item in items {
        let lit = match item {
            Value::String(s) => DiscriminatorLiteral::Str(Cow::Owned(s.clone())),
            Value::Bool(b) => DiscriminatorLiteral::Bool(*b),
            Value::Number(n) => DiscriminatorLiteral::Int(n.as_i64()?),
            _ => return None,
        };
        if let Some(first) = literals.first() {
            if DiscriminatorLiteral::kind(first) != lit.kind() {
                return None;
            }
        }
        literals.push(lit);
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

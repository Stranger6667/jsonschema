use std::collections::{BTreeMap, HashMap, HashSet};

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
type DiscriminatorPlan = (
    String,
    DiscriminatorKind,
    Vec<Option<Vec<DiscriminatorLiteral>>>,
);

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
            DiscriminatorKind::Str => {
                crate::codegen::emit_serde::instance_object_property_as_str(discriminator_key)
            }
            DiscriminatorKind::Bool => {
                crate::codegen::emit_serde::instance_object_property_as_bool(discriminator_key)
            }
            DiscriminatorKind::Int => {
                crate::codegen::emit_serde::instance_object_property_as_i64(discriminator_key)
            }
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
                    return Some(jsonschema::__private::error::one_of_not_valid(
                        #schema_path, __path.clone(), instance,
                    ));
                }
                if __count > 1 {
                    return Some(jsonschema::__private::error::one_of_multiple_valid(
                        #schema_path, __path.clone(), instance,
                    ));
                }
            }
        },
    )
}

fn build_discriminator_plan(
    ctx: &mut CompileContext<'_>,
    schemas: &[Value],
) -> Option<DiscriminatorPlan> {
    let branch_discriminators: Vec<HashMap<String, Vec<DiscriminatorLiteral>>> = schemas
        .iter()
        .map(|schema| extract_discriminators(ctx, schema))
        .collect();

    // For each candidate key: track (coverage, distinct_value_set, total_cardinality, kind).
    // Keys whose values have inconsistent kinds across branches are rejected.
    // BTreeMap: iteration order below decides ties, so it must not depend on hashing.
    let mut stats: BTreeMap<String, KeyStats> = BTreeMap::new();
    for branch in &branch_discriminators {
        for (key, values) in branch {
            let entry = stats.entry(key.clone()).or_default();
            let branch_kind = values.first().map(DiscriminatorLiteral::kind);
            match (entry.3, branch_kind) {
                (None, Some(k)) => entry.3 = Some(k),
                (Some(existing), Some(k)) if existing != k => {
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
    Some((key, kind, per_branch_values))
}

/// Extracts typed discriminator candidates from a branch schema, following a
/// short top-level `$ref` chain first.
fn extract_discriminators(
    ctx: &mut CompileContext<'_>,
    schema: &Value,
) -> HashMap<String, Vec<DiscriminatorLiteral>> {
    extract_object(resolve_top_level_ref_for_one_of_analysis(ctx, schema).as_ref())
}

fn extract_object(schema: &Value) -> HashMap<String, Vec<DiscriminatorLiteral>> {
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
        if let Some(lit) = const_as_literal(prop_schema.get("const")) {
            out.insert(name.to_owned(), vec![lit]);
            continue;
        }
        if let Some(literals) = enum_as_literals(prop_schema.get("enum")) {
            out.insert(name.to_owned(), literals);
        }
    }
    out
}

fn const_as_literal(value: Option<&Value>) -> Option<DiscriminatorLiteral> {
    match value? {
        Value::String(s) => Some(DiscriminatorLiteral::Str(s.clone())),
        Value::Bool(b) => Some(DiscriminatorLiteral::Bool(*b)),
        Value::Number(n) => n.as_i64().map(DiscriminatorLiteral::Int),
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
        let lit = match item {
            Value::String(s) => DiscriminatorLiteral::Str(s.clone()),
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

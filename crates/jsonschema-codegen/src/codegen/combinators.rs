use std::collections::{HashMap, HashSet};

use proc_macro2::TokenStream;
use quote::quote;
use serde_json::{Map, Value};

use crate::context::CompileContext;

use super::{
    compile_schema, errors::invalid_schema_non_empty_array_expression,
    invalid_schema_type_expression, is_trivially_true,
    refs::resolve_top_level_ref_for_one_of_analysis,
};

/// Compile the "allOf" keyword.
pub(super) fn compile_all_of(ctx: &mut CompileContext<'_>, value: &Value) -> TokenStream {
    if let Some(schemas) = value.as_array() {
        if schemas.is_empty() {
            return invalid_schema_non_empty_array_expression();
        }
        let compiled: Vec<_> = schemas
            .iter()
            .map(|schema| compile_schema(ctx, schema))
            .filter(|t| !is_trivially_true(t))
            .collect();
        if compiled.is_empty() {
            quote! { true }
        } else {
            quote! { (#(#compiled)&&*) }
        }
    } else {
        invalid_schema_type_expression(value, &["array"])
    }
}

/// Compile the "anyOf" keyword.
pub(super) fn compile_any_of(ctx: &mut CompileContext<'_>, value: &Value) -> TokenStream {
    if let Some(schemas) = value.as_array() {
        if schemas.is_empty() {
            return invalid_schema_non_empty_array_expression();
        }
        let compiled: Vec<_> = schemas
            .iter()
            .map(|schema| compile_schema(ctx, schema))
            .collect();
        // If any branch is trivially true, anyOf is trivially true.
        if compiled.iter().any(is_trivially_true) {
            return quote! { true };
        }
        quote! { (#(#compiled)||*) }
    } else {
        invalid_schema_type_expression(value, &["array"])
    }
}

/// Compile the "oneOf" keyword.
pub(super) fn compile_one_of(ctx: &mut CompileContext<'_>, value: &Value) -> TokenStream {
    if let Some(schemas) = value.as_array() {
        if schemas.is_empty() {
            return invalid_schema_non_empty_array_expression();
        }

        // Try to detect a discriminator-like key (required string const/enum) shared by branches.
        // When present, branch validation is gated by a cheap string equality/set-membership check.
        let discriminator_plan = one_of_discriminator_plan(ctx, schemas);

        let checks: Vec<_> = schemas
            .iter()
            .enumerate()
            .map(|(idx, schema)| {
                let validation = compile_schema(ctx, schema);
                let branch_validation = match &discriminator_plan {
                    Some((_, branch_discriminators)) => {
                        if let Some(discriminator_values) = &branch_discriminators[idx] {
                            // The discriminator key is always `required` in these branches
                            // (that's how it was detected), so an absent key means this
                            // branch cannot match.
                            if discriminator_values.len() == 1 {
                                let discriminator_value = &discriminator_values[0];
                                quote! {
                                    (matches!(__one_of_discriminator, Some(#discriminator_value))
                                        && { #validation })
                                }
                            } else {
                                let allowed_values = discriminator_values.iter();
                                quote! {
                                    (matches!(__one_of_discriminator, Some(#(#allowed_values)|*))
                                        && { #validation })
                                }
                            }
                        } else {
                            validation
                        }
                    }
                    None => validation,
                };

                quote! {
                    if #branch_validation {
                        if matched {
                            false
                        } else {
                            matched = true;
                            true
                        }
                    } else {
                        true
                    }
                }
            })
            .collect();

        let discriminator_init = if let Some((discriminator_key, _)) = &discriminator_plan {
            let discriminator_value = ctx
                .config
                .backend
                .instance_object_property_as_str(discriminator_key);
            quote! {
                let __one_of_discriminator = #discriminator_value;
            }
        } else {
            quote! {}
        };

        // We short-circuit as soon as a second branch validates.
        quote! {
            {
                #discriminator_init
                let mut matched = false;
                ( #(#checks)&&* ) && matched
            }
        }
    } else {
        invalid_schema_type_expression(value, &["array"])
    }
}

/// Build a discriminator plan for oneOf branches when they share a required
/// string const/enum property (for example, `resourceType` in FHIR).
fn one_of_discriminator_plan(
    ctx: &mut CompileContext<'_>,
    schemas: &[Value],
) -> Option<(String, Vec<Option<Vec<String>>>)> {
    let branch_discriminators: Vec<HashMap<String, Vec<String>>> = schemas
        .iter()
        .map(|schema| extract_required_string_discriminators_for_one_of(ctx, schema))
        .collect();

    // key -> (coverage_count, distinct_values, total_branch_cardinality)
    let mut stats: HashMap<String, (usize, HashSet<String>, usize)> = HashMap::new();
    for branch in &branch_discriminators {
        for (key, values) in branch {
            let entry = stats.entry(key.clone()).or_default();
            entry.0 += 1;
            entry.2 += values.len();
            for value in values {
                entry.1.insert(value.clone());
            }
        }
    }

    let mut best: Option<(String, usize, usize, usize)> = None;

    for (key, (coverage, values, total_cardinality)) in stats {
        let distinct_values = values.len();
        // Need at least two covered branches and two distinct values to be useful.
        if coverage < 2 || distinct_values < 2 {
            continue;
        }
        match &best {
            None => best = Some((key, coverage, distinct_values, total_cardinality)),
            Some((_, best_coverage, best_distinct, best_total_cardinality)) => {
                if coverage > *best_coverage
                    || (coverage == *best_coverage
                        && (distinct_values > *best_distinct
                            || (distinct_values == *best_distinct
                                && total_cardinality < *best_total_cardinality)))
                {
                    best = Some((key, coverage, distinct_values, total_cardinality));
                }
            }
        }
    }

    let (key, _, _, _) = best?;
    let per_branch_values = branch_discriminators
        .iter()
        .map(|branch| branch.get(&key).cloned())
        .collect();
    Some((key, per_branch_values))
}

/// Extract `required` string const/enum discriminator properties from a branch schema.
/// Resolves top-level $ref chains to make oneOf branch analysis effective.
fn extract_required_string_discriminators_for_one_of(
    ctx: &mut CompileContext<'_>,
    schema: &Value,
) -> HashMap<String, Vec<String>> {
    let resolved = resolve_top_level_ref_for_one_of_analysis(ctx, schema);
    let Value::Object(obj) = resolved.as_ref() else {
        return HashMap::new();
    };
    if !is_explicit_object_only_type(obj.get("type")) {
        return HashMap::new();
    }

    let required: HashSet<&str> = obj
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let Some(properties) = obj.get("properties").and_then(Value::as_object) else {
        return HashMap::new();
    };

    let mut out = HashMap::new();
    for name in required {
        let Some(Value::Object(property_schema)) = properties.get(name) else {
            continue;
        };
        if let Some(Value::String(const_value)) = property_schema.get("const") {
            out.insert(name.to_string(), vec![const_value.clone()]);
            continue;
        }
        if let Some(Value::Array(enum_values)) = property_schema.get("enum") {
            let mut values: Vec<String> = enum_values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            // Only use this discriminator when all enum members are strings.
            if values.len() == enum_values.len() && !values.is_empty() {
                values.sort_unstable();
                values.dedup();
                out.insert(name.to_string(), values);
            }
        }
    }
    out
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

/// Compile the "not" keyword.
pub(super) fn compile_not(ctx: &mut CompileContext<'_>, value: &Value) -> TokenStream {
    let compiled = compile_schema(ctx, value);
    quote! { !(#compiled) }
}

/// Compile "if", "then", "else" keywords.
pub(super) fn compile_if_then_else(
    ctx: &mut CompileContext<'_>,
    parent: &Map<String, Value>,
    if_schema: &Value,
) -> Option<TokenStream> {
    let then_schema = parent.get("then");
    let else_schema = parent.get("else");

    match (then_schema, else_schema) {
        (Some(then_val), Some(else_val)) => {
            // if/then/else: if condition is true, validate with then, else validate with else
            let if_check = compile_schema(ctx, if_schema);
            let then_check = compile_schema(ctx, then_val);
            let else_check = compile_schema(ctx, else_val);
            Some(quote! {
                if #if_check {
                    #then_check
                } else {
                    #else_check
                }
            })
        }
        (Some(then_val), None) => {
            // if/then: if condition is true, validate with then, else true
            let if_check = compile_schema(ctx, if_schema);
            let then_check = compile_schema(ctx, then_val);
            Some(quote! {
                if #if_check {
                    #then_check
                } else {
                    true
                }
            })
        }
        (None, Some(else_val)) => {
            // if/else: if condition is true, return true, else validate with else
            let if_check = compile_schema(ctx, if_schema);
            let else_check = compile_schema(ctx, else_val);
            Some(quote! {
                if #if_check {
                    true
                } else {
                    #else_check
                }
            })
        }
        (None, None) => None,
    }
}

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::Draft;
use serde_json::{Map, Value};
use std::borrow::Cow;

use self::{
    draft::{
        has_vocabulary, supports_applicator_vocabulary, supports_contains_bounds_keyword,
        supports_contains_keyword, supports_content_validation_keywords,
        supports_dependent_required_keyword, supports_dependent_schemas_keyword,
        supports_draft201909_plus_formats, supports_draft6_plus_formats,
        supports_draft7_plus_formats, supports_dynamic_ref_keyword, supports_if_then_else_keyword,
        supports_prefix_items_keyword, supports_property_names_keyword,
        supports_recursive_ref_keyword, supports_unevaluated_items_keyword_for_context,
        supports_unevaluated_properties_keyword_for_context, supports_validation_vocabulary,
    },
    emit_root::emit_root_module,
    helpers::{
        collect_dynamic_anchor_bindings, dynamic_ref_anchor_name, get_or_create_eval_function,
        get_or_create_item_eval_function,
    },
    numeric::value_as_u64,
    refs::resolve_ref,
    regex::{compile_regex_match, translate_and_validate_regex},
};
use crate::context::CompileContext;
use errors::{
    invalid_schema_exclusive_minimum_expression, invalid_schema_expression,
    invalid_schema_minimum_expression, invalid_schema_type_expression,
};

pub(crate) mod backend;
mod combinators;
mod dispatch;
mod draft;
mod emit_root;
mod errors;
mod eval;
mod helpers;
mod keywords;
mod numeric;
mod refs;
mod regex;
mod schema_compile;
mod stack_emit;
pub(crate) mod symbols;
use self::eval::{
    items::compile_unevaluated_items,
    keys::compile_unevaluated_properties,
    shared::{compile_guarded_eval, compile_if_then_else_evaluated, compile_one_of_evaluated},
};

#[derive(Clone, Copy)]
enum ComparisonOp {
    Lt,
    Lte,
    Gt,
    Gte,
}

/// Entry point: generate validator impl methods from a `CodegenConfig`.
pub(crate) fn generate_from_config(
    config: &crate::context::CodegenConfig,
    recompile_trigger: &TokenStream,
    name: &proc_macro2::Ident,
    impl_mod_name: &proc_macro2::Ident,
) -> TokenStream {
    let _backend_id = config.backend.id();
    let _compile_only_stub_variants =
        crate::codegen::backend::BackendKind::compile_only_stub_variants();
    let mut ctx = CompileContext::new(config);
    let runtime_crate_alias = config
        .runtime_crate_alias
        .clone()
        .map(|path| quote! { use #path as jsonschema; });
    let validation_expr = compile_schema(&mut ctx, &config.schema);
    let recursive_stack_needed = ctx.uses_recursive_ref;
    let dynamic_stack_needed = ctx.uses_dynamic_ref;
    let root_recursive_anchor = recursive_stack_needed
        && supports_recursive_ref_keyword(config.draft)
        && config
            .schema
            .as_object()
            .and_then(|obj| obj.get("$recursiveAnchor"))
            .and_then(Value::as_bool)
            == Some(true);
    let root_key_eval_ident = if recursive_stack_needed {
        let name = get_or_create_eval_function(
            &mut ctx,
            "__root_key_eval",
            &config.schema,
            config.base_uri.clone(),
        );
        Some(format_ident!("{}", name))
    } else {
        None
    };
    let root_item_eval_ident = if recursive_stack_needed {
        let name = get_or_create_item_eval_function(
            &mut ctx,
            "__root_item_eval",
            &config.schema,
            config.base_uri.clone(),
        );
        Some(format_ident!("{}", name))
    } else {
        None
    };
    let root_dynamic_bindings = if dynamic_stack_needed {
        collect_dynamic_anchor_bindings(&mut ctx, config.base_uri.clone())
    } else {
        Vec::new()
    };
    emit_root_module(
        &ctx,
        runtime_crate_alias.as_ref(),
        recompile_trigger,
        name,
        impl_mod_name,
        &validation_expr,
        recursive_stack_needed,
        dynamic_stack_needed,
        root_recursive_anchor,
        root_key_eval_ident.as_ref(),
        root_item_eval_ident.as_ref(),
        &root_dynamic_bindings,
    )
}

fn is_trivially_true(tokens: &TokenStream) -> bool {
    tokens.to_string().trim() == "true"
}

fn is_negative_integer_valued_number(draft: Draft, value: &Value) -> bool {
    if value.as_i64().is_some_and(|n| n < 0) {
        return true;
    }
    if matches!(draft, Draft::Draft4) {
        return false;
    }
    value
        .as_f64()
        .is_some_and(|n| n.is_finite() && n < 0.0 && n.fract() == 0.0)
}

fn parse_nonnegative_integer_keyword(draft: Draft, value: &Value) -> Result<u64, TokenStream> {
    if let Some(parsed) = value_as_u64(draft, value) {
        Ok(parsed)
    } else if is_negative_integer_valued_number(draft, value) {
        Err(invalid_schema_minimum_expression(value, "0"))
    } else {
        Err(invalid_schema_type_expression(value, &["integer"]))
    }
}

/// Compile a schema into validation code.
pub(in crate::codegen) fn compile_schema(
    ctx: &mut CompileContext<'_>,
    schema: &Value,
) -> TokenStream {
    ctx.with_schema_scope(|ctx| match schema {
        Value::Bool(true) => quote! { true },
        Value::Bool(false) => quote! { false },
        Value::Object(obj) => schema_compile::compile_object_schema(ctx, obj),
        _ => invalid_schema_type_expression(schema, &["boolean", "object"]),
    })
}

/// Generate numeric comparison for extracted Number value.
fn generate_numeric_check(op: ComparisonOp, limit: &Value) -> TokenStream {
    if !limit.is_number() {
        return invalid_schema_type_expression(limit, &["number"]);
    }

    #[cfg(feature = "arbitrary-precision")]
    if let Value::Number(number) = limit {
        if number.as_u64().is_none() && number.as_i64().is_none() {
            let op_tag: u8 = match op {
                ComparisonOp::Lt => 0,
                ComparisonOp::Lte => 1,
                ComparisonOp::Gt => 2,
                ComparisonOp::Gte => 3,
            };
            let limit_literal = number.to_string();
            return quote! {
                jsonschema::keywords_helpers::numeric::check_compiled_bound(
                    n,
                    #op_tag,
                    #limit_literal
                )
            };
        }
    }

    let cmp_fn = match op {
        ComparisonOp::Lt => quote! { lt },
        ComparisonOp::Lte => quote! { le },
        ComparisonOp::Gt => quote! { gt },
        ComparisonOp::Gte => quote! { ge },
    };

    if let Some(u) = limit.as_u64() {
        quote! {
            jsonschema::keywords_helpers::numeric::#cmp_fn(n, #u as u64)
        }
    } else if let Some(i) = limit.as_i64() {
        quote! {
            jsonschema::keywords_helpers::numeric::#cmp_fn(n, #i as i64)
        }
    } else if let Some(f) = limit.as_f64() {
        quote! {
            jsonschema::keywords_helpers::numeric::#cmp_fn(n, #f as f64)
        }
    } else {
        // Keep current behavior for numeric forms that cannot be represented in this path.
        quote! { true }
    }
}

/// Generate multipleOf check for extracted Number value.
fn generate_multiple_of_check(value: &Value) -> TokenStream {
    if !value.is_number() {
        return invalid_schema_type_expression(value, &["number"]);
    }
    if !is_strictly_positive_number(value) {
        return invalid_schema_exclusive_minimum_expression(value, "0");
    }

    #[cfg(feature = "arbitrary-precision")]
    if let Value::Number(number) = value {
        if requires_ap_multiple_of_path(number) {
            let limit_literal = number.to_string();
            return quote! {
                jsonschema::keywords_helpers::numeric::check_compiled_multiple_of(
                    n,
                    #limit_literal
                )
            };
        }
    }

    if let Some(multiple) = value.as_f64() {
        if multiple.fract() == 0.0 {
            quote! {
                jsonschema::keywords_helpers::numeric::is_multiple_of_integer(n, #multiple)
            }
        } else {
            quote! {
                jsonschema::keywords_helpers::numeric::is_multiple_of_float(n, #multiple)
            }
        }
    } else {
        quote! { true }
    }
}

fn is_strictly_positive_number(value: &Value) -> bool {
    let Some(number) = value.as_number() else {
        return false;
    };

    if let Some(v) = number.as_u64() {
        return v > 0;
    }
    if let Some(v) = number.as_i64() {
        return v > 0;
    }
    if let Some(v) = number.as_f64() {
        return v > 0.0;
    }

    let raw = number.to_string();
    if raw.starts_with('-') {
        return false;
    }
    // Arbitrary-precision numbers may not round-trip through primitive accessors.
    // Determine non-zero by looking only at the significand (before exponent).
    raw.split(['e', 'E'])
        .next()
        .is_some_and(|significand| significand.bytes().any(|b| b.is_ascii_digit() && b != b'0'))
}

#[cfg(feature = "arbitrary-precision")]
fn requires_ap_multiple_of_path(number: &serde_json::Number) -> bool {
    const MAX_SAFE_INTEGER: u64 = 1u64 << 53;

    if let Some(value) = number.as_u64() {
        return value > MAX_SAFE_INTEGER;
    }
    if let Some(value) = number.as_i64() {
        return value.unsigned_abs() > MAX_SAFE_INTEGER;
    }

    // Any remaining representation (decimal, scientific notation, or very large integer)
    // can trigger BigInt/BigFraction paths in dynamic validators.
    true
}

/// Generate items check for extracted array value.
fn generate_items_check(ctx: &mut CompileContext<'_>, value: &Value) -> TokenStream {
    if let Value::Array(schemas) = value {
        // Tuple validation - check each position
        let compiled: Vec<_> = schemas
            .iter()
            .enumerate()
            .map(|(idx, schema)| {
                let validation = compile_schema(ctx, schema);
                quote! {
                    arr.get(#idx).map_or(true, |instance| #validation)
                }
            })
            .collect();
        if compiled.is_empty() {
            quote! { true }
        } else {
            quote! { ( #(#compiled)&&* ) }
        }
    } else {
        // Single schema (object or boolean) applies to all items.
        let compiled = compile_schema(ctx, value);
        quote! {
            arr.iter().all(|instance| #compiled)
        }
    }
}

/// Generate items check for extracted array value, skipping the `prefixItems` prefix.
fn generate_items_check_with_prefix(
    ctx: &mut CompileContext<'_>,
    value: &Value,
    prefix_len: usize,
) -> TokenStream {
    match value {
        Value::Bool(true) => quote! { true },
        Value::Bool(false) => quote! { arr.len() <= #prefix_len },
        _ => {
            let compiled = compile_schema(ctx, value);
            quote! {
                arr.iter().skip(#prefix_len).all(|instance| #compiled)
            }
        }
    }
}

/// Compile the "prefixItems" keyword (draft 2020-12+).
/// Returns the compiled check and the prefix length.
fn compile_prefix_items(
    ctx: &mut CompileContext<'_>,
    value: &Value,
) -> Option<(TokenStream, usize)> {
    let Value::Array(schemas) = value else {
        return None;
    };
    let prefix_len = schemas.len();
    if prefix_len == 0 {
        return None;
    }
    let compiled = schemas.iter().enumerate().map(|(idx, schema)| {
        let validation = compile_schema(ctx, schema);
        quote! {
            arr.get(#idx).map_or(true, |instance| #validation)
        }
    });
    Some((quote! { ( #(#compiled)&&* ) }, prefix_len))
}

/// Compile the "uniqueItems" keyword.
fn compile_unique_items(value: &Value) -> Option<TokenStream> {
    match value.as_bool() {
        Some(true) => {
            // Call the runtime's optimized is_unique helper
            Some(quote! {
                jsonschema::keywords_helpers::unique_items::is_unique(arr)
            })
        }
        Some(false) => None,
        None => Some(invalid_schema_type_expression(value, &["boolean"])),
    }
}

/// Compile the "additionalItems" keyword.
fn compile_additional_items(
    ctx: &mut CompileContext<'_>,
    additional_items: Option<&Value>,
    items_schema: Option<&Value>,
) -> Option<TokenStream> {
    let additional_items_val = additional_items?;

    // Determine the tuple length from items schema
    let tuple_len = if let Some(Value::Array(items)) = items_schema {
        items.len()
    } else {
        // If items is not an array (tuple validation), additionalItems has no effect
        return None;
    };

    match additional_items_val {
        Value::Bool(false) => {
            // No additional items allowed beyond tuple length
            Some(quote! {
                arr.len() <= #tuple_len
            })
        }
        Value::Bool(true) => {
            // All additional items are allowed
            None
        }
        schema => {
            // Additional items must match schema
            let schema_check = compile_schema(ctx, schema);
            Some(quote! {
                arr.iter().skip(#tuple_len).all(|instance| #schema_check)
            })
        }
    }
}

/// Compile the "patternProperties" keyword.
fn compile_pattern_properties(ctx: &mut CompileContext<'_>, value: &Value) -> Option<TokenStream> {
    let Value::Object(patterns) = value else {
        return Some(invalid_schema_type_expression(value, &["object"]));
    };

    if patterns.is_empty() {
        return None;
    }

    let mut pattern_checks = Vec::new();
    for (pattern, schema) in patterns {
        let analysis = jsonschema_regex::analyze_pattern(pattern);
        let translated_regex = match analysis {
            Some(_) => None,
            None => match translate_and_validate_regex(ctx, "patternProperties", pattern) {
                Ok(translated) => Some(translated),
                Err(error_expr) => {
                    pattern_checks.push(error_expr);
                    continue;
                }
            },
        };

        let schema_check = compile_schema(ctx, schema);
        // If the schema is trivially valid (always true), no check is needed.
        // Regex validity is still checked above to avoid silently accepting invalid schemas.
        if schema_check.to_string() == "true" {
            continue;
        }

        let check = match analysis {
            Some(jsonschema_regex::PatternAnalysis::Prefix(prefix)) => {
                let prefix: &str = prefix.as_ref();
                quote! {
                    obj.iter()
                        .filter(|(key, _)| key.starts_with(#prefix))
                        .all(|(_, instance)| { #schema_check })
                }
            }
            Some(jsonschema_regex::PatternAnalysis::Exact(exact)) => {
                let exact: &str = exact.as_ref();
                quote! {
                    obj.iter()
                        .filter(|(key, _)| key.as_str() == #exact)
                        .all(|(_, instance)| { #schema_check })
                }
            }
            Some(jsonschema_regex::PatternAnalysis::Alternation(alts)) => {
                let alts: Vec<&str> = alts.iter().map(String::as_str).collect();
                quote! {
                    obj.iter()
                        .filter(|(key, _)| matches!(key.as_str(), #(#alts)|*))
                        .all(|(_, instance)| { #schema_check })
                }
            }
            None => {
                let pattern = translated_regex.expect("Regex translation must be present");
                let regex_check = compile_regex_match(ctx, &pattern, &quote! { key.as_str() });
                quote! {
                    obj.iter()
                        .filter(|(key, _)| { #regex_check })
                        .all(|(_, instance)| { #schema_check })
                }
            }
        };
        pattern_checks.push(check);
    }

    if pattern_checks.is_empty() {
        None
    } else {
        Some(quote! {
            ( #(#pattern_checks)&&* )
        })
    }
}

fn collect_pattern_coverage_parts<'a>(
    ctx: &mut CompileContext<'_>,
    pattern_properties: Option<&'a Value>,
) -> (
    Vec<Cow<'a, str>>,
    Vec<String>,
    Vec<String>,
    Vec<TokenStream>,
) {
    pattern_properties
        .and_then(|v| v.as_object())
        .map(|obj| {
            let mut prefixes = Vec::new();
            let mut literals = Vec::new();
            let mut regex_patterns = Vec::new();
            let mut regex_errors = Vec::new();
            for p in obj.keys() {
                match jsonschema_regex::analyze_pattern(p) {
                    Some(jsonschema_regex::PatternAnalysis::Prefix(s)) => prefixes.push(s),
                    Some(jsonschema_regex::PatternAnalysis::Exact(s)) => {
                        literals.push(s.into_owned());
                    }
                    Some(jsonschema_regex::PatternAnalysis::Alternation(alts)) => {
                        literals.extend(alts);
                    }
                    None => match translate_and_validate_regex(ctx, "patternProperties", p) {
                        Ok(regex) => regex_patterns.push(regex),
                        Err(error_expr) => regex_errors.push(error_expr),
                    },
                }
            }
            (prefixes, literals, regex_patterns, regex_errors)
        })
        .unwrap_or_default()
}

/// Build the `_ =>` arm body for a properties match, merging `additionalProperties`
/// coverage into a single iteration.
///
/// Returns `(statics_to_emit, wildcard_arm_body)`.  Both assume `key_str: &str` and
/// `instance: &Value` are already in scope at the call-site.
fn compile_wildcard_arm(
    ctx: &mut CompileContext<'_>,
    additional_properties: Option<&Value>,
    pattern_properties: Option<&Value>,
) -> (Vec<TokenStream>, TokenStream) {
    // Split patternProperties into prefix-optimizable, exact/alternation literals, and
    // regex-requiring patterns.  Used to check whether a key (not matched by any named
    // arm) is covered by a pattern property — if so it is not considered "additional".
    let (prefixes, literals, regex_patterns, regex_errors) =
        collect_pattern_coverage_parts(ctx, pattern_properties);

    if let Some(error_expr) = regex_errors.into_iter().next() {
        return (Vec::new(), error_expr);
    }

    let mut statics: Vec<TokenStream> = Vec::new();

    let prefix_check: Option<TokenStream> = match prefixes.as_slice() {
        [] => None,
        [p] => {
            let p: &str = p.as_ref();
            Some(quote! { key_str.starts_with(#p) })
        }
        _ => {
            let prefix_strs: Vec<&str> = prefixes.iter().map(Cow::as_ref).collect();
            statics.push(quote! {
                static PATTERN_PREFIXES: &[&str] = &[#(#prefix_strs),*];
            });
            Some(quote! { PATTERN_PREFIXES.iter().any(|p| key_str.starts_with(p)) })
        }
    };
    let literal_check: Option<TokenStream> = match literals.as_slice() {
        [] => None,
        [s] => Some(quote! { key_str == #s }),
        _ => {
            let lit_strs: Vec<&str> = literals.iter().map(String::as_str).collect();
            statics.push(quote! {
                static PATTERN_LITERALS: &[&str] = &[#(#lit_strs),*];
            });
            Some(quote! { PATTERN_LITERALS.contains(&key_str) })
        }
    };
    let regex_check: Option<TokenStream> = if regex_patterns.is_empty() {
        None
    } else {
        let checks: Vec<TokenStream> = regex_patterns
            .iter()
            .map(|pattern| compile_regex_match(ctx, pattern, &quote! { key_str }))
            .collect();
        Some(combine_or(checks))
    };

    // Combine all checks into a single pattern coverage expression.
    // Named match arms already act as the "known properties" filter, so KNOWN.contains()
    // is NOT needed here — any key reaching _ is by definition not in properties.
    let checks: Vec<TokenStream> = [prefix_check, literal_check, regex_check]
        .into_iter()
        .flatten()
        .collect();
    let pattern_cover_check: Option<TokenStream> = match checks.as_slice() {
        [] => None,
        [single] => Some(single.clone()),
        _ => Some(quote! { #(#checks)||* }),
    };

    let arm_body = match additional_properties {
        // absent or true: any additional key is allowed
        None | Some(Value::Bool(true)) => quote! { true },
        Some(Value::Bool(false)) => match pattern_cover_check {
            // No patterns: any key reaching _ is disallowed
            None => quote! { false },
            // Covered by a pattern: allowed; otherwise disallowed
            Some(check) => check,
        },
        Some(schema) => {
            let schema_check = compile_schema(ctx, schema);
            if schema_check.to_string() == "true" {
                // Schema is trivially valid — same as absent/true
                quote! { true }
            } else {
                match pattern_cover_check {
                    None => quote! { { #schema_check } },
                    Some(check) => quote! { (#check) || { #schema_check } },
                }
            }
        }
    };

    (statics, arm_body)
}

/// Build a fast key-only precheck for strict objects (`additionalProperties: false`)
/// with explicit `properties` and no `patternProperties`.
fn compile_known_keys_precheck(properties: &Map<String, Value>) -> TokenStream {
    let known_props: Vec<&str> = properties.keys().map(String::as_str).collect();
    if known_props.is_empty() {
        quote! { obj.is_empty() }
    } else {
        quote! {
            obj.keys().all(|key| {
                matches!(key.as_str(), #(#known_props)|*)
            })
        }
    }
}

/// Compile the "additionalProperties" keyword.
fn compile_additional_properties(
    ctx: &mut CompileContext<'_>,
    additional_properties: Option<&Value>,
    properties: Option<&Value>,
    pattern_properties: Option<&Value>,
) -> Option<TokenStream> {
    let additional_properties_val = additional_properties?;

    // Extract known property names from properties
    let known_props: Vec<&str> = properties
        .and_then(|v| v.as_object())
        .map(|obj| obj.keys().map(String::as_str).collect())
        .unwrap_or_default();

    // Split patternProperties into prefix-optimizable, exact/alternation literals,
    // and regex-requiring patterns.
    let (prefixes, literals, regex_patterns, regex_errors) =
        collect_pattern_coverage_parts(ctx, pattern_properties);

    if let Some(error_expr) = regex_errors.into_iter().next() {
        return Some(error_expr);
    }

    // Build statics and per-condition fragments that will be assembled into the final
    // expression.  Prefix patterns with a single entry are inlined directly as
    // `starts_with` calls so no static slice or iterator overhead is emitted.
    let mut statics: Vec<TokenStream> = Vec::new();

    if !known_props.is_empty() {
        statics.push(quote! {
            static KNOWN: &[&str] = &[#(#known_props),*];
        });
    }
    let prefix_check: Option<TokenStream> = match prefixes.as_slice() {
        [] => None,
        [p] => {
            let p: &str = p.as_ref();
            Some(quote! { key_str.starts_with(#p) })
        }
        _ => {
            let prefix_strs: Vec<&str> = prefixes.iter().map(Cow::as_ref).collect();
            statics.push(quote! {
                static PATTERN_PREFIXES: &[&str] = &[#(#prefix_strs),*];
            });
            Some(quote! { PATTERN_PREFIXES.iter().any(|p| key_str.starts_with(p)) })
        }
    };
    let literal_check: Option<TokenStream> = match literals.as_slice() {
        [] => None,
        [s] => Some(quote! { key_str == #s }),
        _ => {
            let lit_strs: Vec<&str> = literals.iter().map(String::as_str).collect();
            statics.push(quote! {
                static PATTERN_LITERALS: &[&str] = &[#(#lit_strs),*];
            });
            Some(quote! { PATTERN_LITERALS.contains(&key_str) })
        }
    };
    let regex_check: Option<TokenStream> = if regex_patterns.is_empty() {
        None
    } else {
        let checks: Vec<TokenStream> = regex_patterns
            .iter()
            .map(|pattern| compile_regex_match(ctx, pattern, &quote! { key_str }))
            .collect();
        Some(combine_or(checks))
    };

    match additional_properties_val {
        Value::Bool(false) => {
            // No additional properties allowed.
            // Build an OR of all "this key is covered" sub-expressions.
            let mut covered: Vec<TokenStream> = Vec::new();
            if let Some(check) = &prefix_check {
                covered.push(check.clone());
            }
            if let Some(check) = &literal_check {
                covered.push(check.clone());
            }
            if !known_props.is_empty() {
                covered.push(quote! { KNOWN.contains(&key_str) });
            }
            if let Some(check) = &regex_check {
                covered.push(check.clone());
            }

            if covered.is_empty() {
                Some(quote! { obj.is_empty() })
            } else {
                Some(quote! {
                    {
                        #(#statics)*
                        obj.keys().all(|key| {
                            let key_str = key.as_str();
                            #(#covered)||*
                        })
                    }
                })
            }
        }
        Value::Bool(true) => None,
        schema => {
            // Additional properties must match schema.
            // Build an AND of all "this key is NOT covered" sub-expressions for the filter.
            let schema_check = compile_schema(ctx, schema);

            let mut excluded: Vec<TokenStream> = Vec::new();
            if let Some(check) = &prefix_check {
                excluded.push(quote! { !(#check) });
            }
            if let Some(check) = &literal_check {
                excluded.push(quote! { !(#check) });
            }
            if !known_props.is_empty() {
                excluded.push(quote! { !KNOWN.contains(&key_str) });
            }
            if let Some(check) = &regex_check {
                excluded.push(quote! { !(#check) });
            }

            if excluded.is_empty() {
                Some(quote! {
                    obj.values().all(|instance| #schema_check)
                })
            } else {
                Some(quote! {
                    {
                        #(#statics)*
                        obj.iter()
                            .filter(|(key, _)| {
                                let key_str = key.as_str();
                                #(#excluded)&&*
                            })
                            .all(|(_, instance)| {
                                #schema_check
                            })
                    }
                })
            }
        }
    }
}

/// Compile the "dependencies" keyword.
fn compile_dependencies(ctx: &mut CompileContext<'_>, value: &Value) -> Option<TokenStream> {
    let Value::Object(deps) = value else {
        return Some(invalid_schema_type_expression(value, &["object"]));
    };

    if deps.is_empty() {
        return None;
    }

    let checks: Vec<_> = deps
        .iter()
        .map(|(prop, dep)| {
            match dep {
                Value::Array(required_props) => {
                    // Property dependencies: if prop exists, all required_props must exist
                    let mut props = Vec::with_capacity(required_props.len());
                    for required_prop in required_props {
                        let Some(prop_name) = required_prop.as_str() else {
                            return invalid_schema_type_expression(required_prop, &["string"]);
                        };
                        props.push(prop_name);
                    }

                    if props.is_empty() {
                        // Empty array means no additional requirements
                        quote! { true }
                    } else {
                        quote! {
                            if obj.contains_key(#prop) {
                                #(obj.contains_key(#props))&&*
                            } else {
                                true
                            }
                        }
                    }
                }
                schema => {
                    // Schema dependencies: if prop exists, instance must validate against schema
                    // Handles both object schemas and boolean schemas (Draft 6+)
                    let schema_check = compile_schema(ctx, schema);
                    quote! {
                        if obj.contains_key(#prop) {
                            #schema_check
                        } else {
                            true
                        }
                    }
                }
            }
        })
        .collect();

    if checks.is_empty() {
        None
    } else {
        Some(quote! {
            ( #(#checks)&&* )
        })
    }
}

/// Compile the "dependentRequired" keyword (draft 2019-09+).
/// For each key in the map: if the object has that key, all listed dependent keys must be present.
fn compile_dependent_required(value: &Value) -> Option<TokenStream> {
    let Value::Object(deps) = value else {
        return Some(invalid_schema_type_expression(value, &["object"]));
    };
    if deps.is_empty() {
        return None;
    }
    let checks: Vec<_> = deps
        .iter()
        .map(|(prop, required)| {
            let Value::Array(required_array) = required else {
                return invalid_schema_type_expression(required, &["array"]);
            };
            let mut seen = std::collections::HashSet::with_capacity(required_array.len());
            let mut required_props = Vec::with_capacity(required_array.len());
            for required_prop in required_array {
                let Some(required_name) = required_prop.as_str() else {
                    return invalid_schema_type_expression(required_prop, &["string"]);
                };
                if !seen.insert(required_name) {
                    return invalid_schema_expression(&format!(
                        "{required} has non-unique elements"
                    ));
                }
                required_props.push(required_name);
            }
            if required_props.is_empty() {
                return quote! { true };
            }
            quote! {
                if obj.contains_key(#prop) {
                    #(obj.contains_key(#required_props))&&*
                } else {
                    true
                }
            }
        })
        .collect();

    if checks.is_empty() {
        None
    } else {
        Some(quote! { (#(#checks)&&*) })
    }
}

/// Compile the "dependentSchemas" keyword (draft 2019-09+).
/// For each property in the map: if the object has that property,
/// validate the whole object against the mapped subschema.
fn compile_dependent_schemas(ctx: &mut CompileContext<'_>, value: &Value) -> Option<TokenStream> {
    let Value::Object(deps) = value else {
        return Some(invalid_schema_type_expression(value, &["object"]));
    };
    if deps.is_empty() {
        return None;
    }
    let checks: Vec<_> = deps
        .iter()
        .map(|(prop, subschema)| {
            let schema_check = compile_schema(ctx, subschema);
            quote! {
                if obj.contains_key(#prop) {
                    #schema_check
                } else {
                    true
                }
            }
        })
        .collect();
    Some(quote! { (#(#checks)&&*) })
}

fn combine_or(parts: Vec<TokenStream>) -> TokenStream {
    match parts.len() {
        0 => quote! { false },
        1 => parts.into_iter().next().unwrap_or_else(|| quote! { false }),
        _ => quote! { (#(#parts)||*) },
    }
}

fn compile_pattern_coverage_for_key(
    ctx: &mut CompileContext<'_>,
    patterns: &Map<String, Value>,
) -> Option<TokenStream> {
    let mut checks = Vec::new();
    for pattern in patterns.keys() {
        let check = match jsonschema_regex::analyze_pattern(pattern) {
            Some(jsonschema_regex::PatternAnalysis::Prefix(prefix)) => {
                let prefix: &str = prefix.as_ref();
                quote! { key_str.starts_with(#prefix) }
            }
            Some(jsonschema_regex::PatternAnalysis::Exact(exact)) => {
                let exact: &str = exact.as_ref();
                quote! { key_str == #exact }
            }
            Some(jsonschema_regex::PatternAnalysis::Alternation(alts)) => {
                let alts: Vec<&str> = alts.iter().map(String::as_str).collect();
                quote! { matches!(key_str, #(#alts)|*) }
            }
            None => match translate_and_validate_regex(ctx, "patternProperties", pattern) {
                Ok(translated) => compile_regex_match(ctx, &translated, &quote! { key_str }),
                Err(error_expr) => error_expr,
            },
        };
        checks.push(check);
    }
    (!checks.is_empty()).then(|| combine_or(checks))
}

pub(in crate::codegen) fn compile_key_evaluated_expr(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> TokenStream {
    let mut parts = Vec::new();
    let applicator_vocab_enabled = supports_applicator_vocabulary(ctx);
    let value_ty = ctx.config.backend.emit_symbols().value_ty();

    let properties_obj = if applicator_vocab_enabled {
        schema.get("properties").and_then(Value::as_object)
    } else {
        None
    };
    if applicator_vocab_enabled {
        if let Some(properties) = properties_obj {
            if !properties.is_empty() {
                let names: Vec<&str> = properties.keys().map(String::as_str).collect();
                parts.push(quote! { matches!(key_str, #(#names)|*) });
            }
        }
    }

    let pattern_obj = if applicator_vocab_enabled {
        schema.get("patternProperties").and_then(Value::as_object)
    } else {
        None
    };
    let pattern_coverage =
        pattern_obj.and_then(|patterns| compile_pattern_coverage_for_key(ctx, patterns));
    if applicator_vocab_enabled {
        if let Some(pattern_expr) = pattern_coverage.clone() {
            parts.push(pattern_expr);
        }
    }

    if applicator_vocab_enabled {
        if let Some(additional) = schema.get("additionalProperties") {
            if additional.as_bool() != Some(false) {
                let mut covered_parts = Vec::new();
                if let Some(properties) = properties_obj {
                    if !properties.is_empty() {
                        let names: Vec<&str> = properties.keys().map(String::as_str).collect();
                        covered_parts.push(quote! { matches!(key_str, #(#names)|*) });
                    }
                }
                if let Some(pattern_expr) = pattern_coverage {
                    covered_parts.push(pattern_expr);
                }
                let covered = combine_or(covered_parts);
                parts.push(quote! { !(#covered) });
            }
        }
    }

    if applicator_vocab_enabled && supports_dependent_schemas_keyword(ctx.draft) {
        if let Some(dependent_schemas) = schema.get("dependentSchemas").and_then(Value::as_object) {
            for (property, subschema) in dependent_schemas {
                if let Value::Object(subschema_obj) = subschema {
                    let sub_eval = compile_key_evaluated_expr(ctx, subschema_obj);
                    parts.push(quote! { obj.contains_key(#property) && (#sub_eval) });
                }
            }
        }
    }

    if applicator_vocab_enabled {
        if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
            for subschema in all_of {
                if let Value::Object(subschema_obj) = subschema {
                    let sub_eval = compile_key_evaluated_expr(ctx, subschema_obj);
                    parts.push(sub_eval);
                }
            }
        }

        if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array) {
            for subschema in any_of {
                if let Value::Object(subschema_obj) = subschema {
                    let sub_eval = compile_key_evaluated_expr(ctx, subschema_obj);
                    let sub_valid = compile_schema(ctx, subschema);
                    parts.push(compile_guarded_eval(&value_ty, &sub_valid, &sub_eval));
                }
            }
        }

        if let Some(one_of) = schema.get("oneOf").and_then(Value::as_array) {
            let cases: Vec<_> = one_of
                .iter()
                .filter_map(|subschema| {
                    let Value::Object(subschema_obj) = subschema else {
                        return None;
                    };
                    let sub_eval = compile_key_evaluated_expr(ctx, subschema_obj);
                    let sub_valid = compile_schema(ctx, subschema);
                    Some((sub_valid, sub_eval))
                })
                .collect();
            if let Some(one_of_eval) = compile_one_of_evaluated(&value_ty, &cases) {
                parts.push(one_of_eval);
            }
        }

        if supports_if_then_else_keyword(ctx.draft) {
            if let Some(if_schema) = schema.get("if") {
                let if_valid = compile_schema(ctx, if_schema);
                let if_eval = if let Value::Object(if_obj) = if_schema {
                    compile_key_evaluated_expr(ctx, if_obj)
                } else {
                    quote! { false }
                };
                let then_eval = schema.get("then").and_then(Value::as_object).map_or_else(
                    || quote! { false },
                    |then_obj| compile_key_evaluated_expr(ctx, then_obj),
                );
                let else_eval = schema.get("else").and_then(Value::as_object).map_or_else(
                    || quote! { false },
                    |else_obj| compile_key_evaluated_expr(ctx, else_obj),
                );
                parts.push(compile_if_then_else_evaluated(
                    &value_ty, &if_valid, &if_eval, &then_eval, &else_eval,
                ));
            }
        }
    }

    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        if let Ok(resolved) = resolve_ref(ctx, reference) {
            if !ctx.eval_seen.contains(&resolved.location) {
                let func_name = get_or_create_eval_function(
                    ctx,
                    &resolved.location,
                    &resolved.schema,
                    resolved.base_uri,
                );
                let func_ident = format_ident!("{}", func_name);
                parts.push(quote! { #func_ident(instance, obj, key_str) });
            }
        }
    }
    if supports_recursive_ref_keyword(ctx.draft) {
        if let Some(reference) = schema.get("$recursiveRef").and_then(Value::as_str) {
            if let Ok(resolved) = resolve_ref(ctx, reference) {
                let target_has_recursive_anchor = resolved
                    .schema
                    .as_object()
                    .and_then(|obj| obj.get("$recursiveAnchor"))
                    .and_then(Value::as_bool)
                    == Some(true);
                let fallback = if ctx.eval_seen.contains(&resolved.location) {
                    quote! { false }
                } else {
                    let func_name = get_or_create_eval_function(
                        ctx,
                        &resolved.location,
                        &resolved.schema,
                        resolved.base_uri,
                    );
                    let func_ident = format_ident!("{}", func_name);
                    quote! { #func_ident(instance, obj, key_str) }
                };
                if ctx.uses_recursive_ref && target_has_recursive_anchor {
                    parts.push(quote! {
                        {
                            let __recursive_target = __JSONSCHEMA_RECURSIVE_KEY_EVAL_STACK.with(|stack| {
                                let stack = stack.borrow();
                                let mut selected = None;
                                for (validate, is_anchor) in stack.iter().rev() {
                                    if *is_anchor {
                                        selected = Some(*validate);
                                    } else {
                                        break;
                                    }
                                }
                                selected
                            });
                            if let Some(target) = __recursive_target {
                                target(instance, obj, key_str)
                            } else {
                                #fallback
                            }
                        }
                    });
                } else {
                    parts.push(fallback);
                }
            }
        }
    }
    if supports_dynamic_ref_keyword(ctx.draft) {
        if let Some(reference) = schema.get("$dynamicRef").and_then(Value::as_str) {
            if let Ok(resolved) = resolve_ref(ctx, reference) {
                let fallback = if ctx.eval_seen.contains(&resolved.location) {
                    quote! { false }
                } else {
                    let func_name = get_or_create_eval_function(
                        ctx,
                        &resolved.location,
                        &resolved.schema,
                        resolved.base_uri,
                    );
                    let func_ident = format_ident!("{}", func_name);
                    quote! { #func_ident(instance, obj, key_str) }
                };
                if let Some(anchor_name) = dynamic_ref_anchor_name(reference, &resolved.schema) {
                    ctx.uses_dynamic_ref = true;
                    parts.push(quote! {
                        {
                            let __dynamic_target = __JSONSCHEMA_DYNAMIC_KEY_EVAL_STACK.with(|stack| {
                                let stack = stack.borrow();
                                let mut selected = None;
                                for (dynamic_anchor, validate) in stack.iter().rev() {
                                    if *dynamic_anchor == #anchor_name {
                                        selected = Some(*validate);
                                    }
                                }
                                selected
                            });
                            if let Some(target) = __dynamic_target {
                                target(instance, obj, key_str)
                            } else {
                                #fallback
                            }
                        }
                    });
                } else {
                    parts.push(fallback);
                }
            }
        }
    }
    let evaluated_without_unevaluated = combine_or(parts);

    if supports_unevaluated_properties_keyword_for_context(ctx) {
        if let Some(unevaluated) = schema.get("unevaluatedProperties") {
            if unevaluated.as_bool() == Some(true) {
                return quote! { true };
            }
            if unevaluated.as_bool() != Some(false) {
                let schema_check = compile_schema(ctx, unevaluated);
                return quote! {
                    (#evaluated_without_unevaluated) || {
                        obj.get(key_str).is_some_and(|instance| {
                            #schema_check
                        })
                    }
                };
            }
        }
    }

    evaluated_without_unevaluated
}

pub(in crate::codegen) fn compile_index_evaluated_expr(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> TokenStream {
    let mut parts = Vec::new();
    let applicator_vocab_enabled = supports_applicator_vocabulary(ctx);
    let value_ty = ctx.config.backend.emit_symbols().value_ty();

    if applicator_vocab_enabled {
        if let Some(items_schema) = schema.get("items") {
            match (ctx.draft, items_schema) {
                (Draft::Draft202012 | Draft::Unknown, _) => {
                    // In 2020-12+, `items` applies to the remaining tail and therefore
                    // collectively evaluates every index when the schema is valid.
                    parts.push(quote! { true });
                }
                (_, Value::Array(tuple)) => {
                    if schema.contains_key("additionalItems") {
                        // In older drafts, when `additionalItems` is present together with
                        // tuple `items`, runtime tracking treats all indexes as covered.
                        parts.push(quote! { true });
                    } else {
                        let tuple_len = tuple.len();
                        parts.push(quote! { idx < #tuple_len });
                    }
                }
                _ => {
                    // Draft 2019-09 and earlier: schema-form `items` evaluates all indexes.
                    parts.push(quote! { true });
                }
            }
        }

        if supports_prefix_items_keyword(ctx.draft) {
            if let Some(prefix_items) = schema.get("prefixItems").and_then(Value::as_array) {
                let prefix_len = prefix_items.len();
                parts.push(quote! { idx < #prefix_len });
            }
        }

        if let Some(contains_schema) = schema.get("contains") {
            let contains_check = compile_schema(ctx, contains_schema);
            parts.push(quote! {
                (|instance: &#value_ty| #contains_check)(item)
            });
        }

        if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
            for subschema in all_of {
                if let Value::Object(subschema_obj) = subschema {
                    parts.push(compile_index_evaluated_expr(ctx, subschema_obj));
                }
            }
        }

        if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array) {
            for subschema in any_of {
                if let Value::Object(subschema_obj) = subschema {
                    let sub_eval = compile_index_evaluated_expr(ctx, subschema_obj);
                    let sub_valid = compile_schema(ctx, subschema);
                    parts.push(compile_guarded_eval(&value_ty, &sub_valid, &sub_eval));
                }
            }
        }

        if let Some(one_of) = schema.get("oneOf").and_then(Value::as_array) {
            let cases: Vec<_> = one_of
                .iter()
                .filter_map(|subschema| {
                    let Value::Object(subschema_obj) = subschema else {
                        return None;
                    };
                    let sub_eval = compile_index_evaluated_expr(ctx, subschema_obj);
                    let sub_valid = compile_schema(ctx, subschema);
                    Some((sub_valid, sub_eval))
                })
                .collect();
            if let Some(one_of_eval) = compile_one_of_evaluated(&value_ty, &cases) {
                parts.push(one_of_eval);
            }
        }

        if supports_if_then_else_keyword(ctx.draft) {
            if let Some(if_schema) = schema.get("if") {
                let if_valid = compile_schema(ctx, if_schema);
                let if_eval = if let Value::Object(if_obj) = if_schema {
                    compile_index_evaluated_expr(ctx, if_obj)
                } else {
                    quote! { false }
                };
                let then_eval = schema.get("then").and_then(Value::as_object).map_or_else(
                    || quote! { false },
                    |then_obj| compile_index_evaluated_expr(ctx, then_obj),
                );
                let else_eval = schema.get("else").and_then(Value::as_object).map_or_else(
                    || quote! { false },
                    |else_obj| compile_index_evaluated_expr(ctx, else_obj),
                );
                parts.push(compile_if_then_else_evaluated(
                    &value_ty, &if_valid, &if_eval, &then_eval, &else_eval,
                ));
            }
        }
    }

    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        if let Ok(resolved) = resolve_ref(ctx, reference) {
            if !ctx.item_eval_seen.contains(&resolved.location) {
                let func_name = get_or_create_item_eval_function(
                    ctx,
                    &resolved.location,
                    &resolved.schema,
                    resolved.base_uri,
                );
                let func_ident = format_ident!("{}", func_name);
                parts.push(quote! { #func_ident(instance, arr, idx, item) });
            }
        }
    }
    if supports_recursive_ref_keyword(ctx.draft) {
        if let Some(reference) = schema.get("$recursiveRef").and_then(Value::as_str) {
            if let Ok(resolved) = resolve_ref(ctx, reference) {
                let target_has_recursive_anchor = resolved
                    .schema
                    .as_object()
                    .and_then(|obj| obj.get("$recursiveAnchor"))
                    .and_then(Value::as_bool)
                    == Some(true);
                let fallback = if ctx.item_eval_seen.contains(&resolved.location) {
                    quote! { false }
                } else {
                    let func_name = get_or_create_item_eval_function(
                        ctx,
                        &resolved.location,
                        &resolved.schema,
                        resolved.base_uri,
                    );
                    let func_ident = format_ident!("{}", func_name);
                    quote! { #func_ident(instance, arr, idx, item) }
                };
                if ctx.uses_recursive_ref && target_has_recursive_anchor {
                    parts.push(quote! {
                        {
                            let __recursive_target = __JSONSCHEMA_RECURSIVE_ITEM_EVAL_STACK.with(|stack| {
                                let stack = stack.borrow();
                                let mut selected = None;
                                for (validate, is_anchor) in stack.iter().rev() {
                                    if *is_anchor {
                                        selected = Some(*validate);
                                    } else {
                                        break;
                                    }
                                }
                                selected
                            });
                            if let Some(target) = __recursive_target {
                                target(instance, arr, idx, item)
                            } else {
                                #fallback
                            }
                        }
                    });
                } else {
                    parts.push(fallback);
                }
            }
        }
    }
    if supports_dynamic_ref_keyword(ctx.draft) {
        if let Some(reference) = schema.get("$dynamicRef").and_then(Value::as_str) {
            if let Ok(resolved) = resolve_ref(ctx, reference) {
                let fallback = if ctx.item_eval_seen.contains(&resolved.location) {
                    quote! { false }
                } else {
                    let func_name = get_or_create_item_eval_function(
                        ctx,
                        &resolved.location,
                        &resolved.schema,
                        resolved.base_uri,
                    );
                    let func_ident = format_ident!("{}", func_name);
                    quote! { #func_ident(instance, arr, idx, item) }
                };
                if let Some(anchor_name) = dynamic_ref_anchor_name(reference, &resolved.schema) {
                    ctx.uses_dynamic_ref = true;
                    parts.push(quote! {
                        {
                            let __dynamic_target = __JSONSCHEMA_DYNAMIC_ITEM_EVAL_STACK.with(|stack| {
                                let stack = stack.borrow();
                                let mut selected = None;
                                for (dynamic_anchor, validate) in stack.iter().rev() {
                                    if *dynamic_anchor == #anchor_name {
                                        selected = Some(*validate);
                                    }
                                }
                                selected
                            });
                            if let Some(target) = __dynamic_target {
                                target(instance, arr, idx, item)
                            } else {
                                #fallback
                            }
                        }
                    });
                } else {
                    parts.push(fallback);
                }
            }
        }
    }

    let evaluated_without_unevaluated = combine_or(parts);

    if supports_unevaluated_items_keyword_for_context(ctx) {
        if let Some(unevaluated) = schema.get("unevaluatedItems") {
            if unevaluated.as_bool() == Some(true) {
                return quote! { true };
            }
            if unevaluated.as_bool() != Some(false) {
                let schema_check = compile_schema(ctx, unevaluated);
                let value_ty = ctx.config.backend.emit_symbols().value_ty();
                return quote! {
                    (#evaluated_without_unevaluated)
                        || (|instance: &#value_ty| #schema_check)(item)
                };
            }
        }
    }

    evaluated_without_unevaluated
}

/// Compile the "contains" keyword with `minContains`/`maxContains` bounds.
fn compile_contains(
    ctx: &mut CompileContext<'_>,
    value: &Value,
    min_contains: Option<u64>,
    max_contains: Option<u64>,
) -> TokenStream {
    let schema_check = compile_schema(ctx, value);
    let min_contains = min_contains.unwrap_or(1);
    let max_check = if let Some(max) = max_contains {
        quote! { && __contains_count <= #max as usize }
    } else {
        quote! {}
    };
    quote! {
        {
            let mut __contains_count = 0usize;
            for instance in arr {
                if #schema_check {
                    __contains_count += 1;
                }
            }
            __contains_count >= #min_contains as usize #max_check
        }
    }
}

/// Compile the "propertyNames" keyword
/// All property names must validate against the schema
fn compile_property_names(ctx: &mut CompileContext<'_>, value: &Value) -> TokenStream {
    // Property names are always strings, so when the schema only checks
    // string-specific keywords (optionally with `type: "string"`), we can
    // bind `s = key` directly and skip the Value::String(key.clone()) wrapping.
    if let Value::Object(schema) = value {
        let only_string_keywords = schema.iter().all(|(k, v)| {
            matches!(k.as_str(), "minLength" | "maxLength" | "pattern" | "format")
                || (k == "type" && v.as_str() == Some("string"))
        });
        let has_string_keywords = schema.contains_key("minLength")
            || schema.contains_key("maxLength")
            || schema.contains_key("pattern")
            || schema.contains_key("format");
        if only_string_keywords && has_string_keywords {
            let string_check = keywords::string::compile(ctx, schema);
            return quote! {
                obj.keys().all(|s| { #string_check })
            };
        }
    }
    let schema_check = compile_schema(ctx, value);
    let value_ty = ctx.config.backend.emit_symbols().value_ty();
    let key_as_value_ref = ctx.config.backend.key_as_value_ref(quote! { key });
    quote! {
        obj.keys().all(|key| {
            (|instance: &#value_ty| #schema_check)(#key_as_value_ref)
        })
    }
}

/// Check if formats should be validated by default for this draft.
fn validates_formats_by_default(draft: Draft) -> bool {
    // Match runtime behavior exactly
    matches!(draft, Draft::Draft4 | Draft::Draft6 | Draft::Draft7)
}

fn compile_email_options_argument(ctx: &CompileContext<'_>) -> TokenStream {
    let Some(options) = ctx.config.email_options else {
        return quote! { None };
    };

    let mut expr = quote! { jsonschema::EmailOptions::default() };
    if let Some(minimum_sub_domains) = options.minimum_sub_domains {
        expr = quote! { #expr.with_minimum_sub_domains(#minimum_sub_domains) };
    }
    if options.no_minimum_sub_domains {
        expr = quote! { #expr.with_no_minimum_sub_domains() };
    }
    if options.required_tld {
        expr = quote! { #expr.with_required_tld() };
    }
    if let Some(allow_domain_literal) = options.allow_domain_literal {
        expr = if allow_domain_literal {
            quote! { #expr.with_domain_literal() }
        } else {
            quote! { #expr.without_domain_literal() }
        };
    }
    if let Some(allow_display_text) = options.allow_display_text {
        expr = if allow_display_text {
            quote! { #expr.with_display_text() }
        } else {
            quote! { #expr.without_display_text() }
        };
    }

    quote! { Some(&(#expr)) }
}

fn compile_builtin_format_check(
    ctx: &CompileContext<'_>,
    format_name: &str,
) -> Option<TokenStream> {
    let draft = ctx.draft;
    match format_name {
        "date" => Some(quote! { jsonschema::keywords_helpers::format::is_valid_date(s) }),
        "date-time" => Some(quote! { jsonschema::keywords_helpers::format::is_valid_datetime(s) }),
        "duration" if supports_draft201909_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_duration(s) })
        }
        "email" => {
            let options = compile_email_options_argument(ctx);
            Some(quote! {
                jsonschema::keywords_helpers::format::is_valid_email_with_options(s, #options)
            })
        }
        "hostname" => Some(quote! { jsonschema::keywords_helpers::format::is_valid_hostname(s) }),
        "idn-email" => {
            let options = compile_email_options_argument(ctx);
            Some(quote! {
                jsonschema::keywords_helpers::format::is_valid_idn_email_with_options(s, #options)
            })
        }
        "idn-hostname" if supports_draft7_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_idn_hostname(s) })
        }
        "ipv4" => Some(quote! { jsonschema::keywords_helpers::format::is_valid_ipv4(s) }),
        "ipv6" => Some(quote! { jsonschema::keywords_helpers::format::is_valid_ipv6(s) }),
        "iri" if supports_draft7_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_iri(s) })
        }
        "iri-reference" if supports_draft7_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_iri_reference(s) })
        }
        "json-pointer" if supports_draft6_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_json_pointer(s) })
        }
        "regex" => Some(quote! { jsonschema::keywords_helpers::format::is_valid_regex(s) }),
        "relative-json-pointer" if supports_draft7_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_relative_json_pointer(s) })
        }
        "time" => Some(quote! { jsonschema::keywords_helpers::format::is_valid_time(s) }),
        "uri" => Some(quote! { jsonschema::keywords_helpers::format::is_valid_uri(s) }),
        "uri-reference" if supports_draft6_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_uri_reference(s) })
        }
        "uri-template" if supports_draft6_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_uri_template(s) })
        }
        "uuid" if supports_draft201909_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_uuid(s) })
        }
        _ => None,
    }
}

fn format_emits_assertion(ctx: &CompileContext<'_>, value: &Value) -> bool {
    let Some(format_name) = value.as_str() else {
        // Non-string `format` is a schema error and compiles to `compile_error!`.
        return true;
    };

    let should_validate = ctx
        .config
        .validate_formats
        .unwrap_or_else(|| validates_formats_by_default(ctx.draft));
    if !should_validate {
        return false;
    }

    if ctx.config.custom_formats.contains_key(format_name) {
        return true;
    }
    if compile_builtin_format_check(ctx, format_name).is_some() {
        return true;
    }
    !ctx.config.ignore_unknown_formats
}

/// Compile the "format" keyword.
fn compile_format(ctx: &mut CompileContext<'_>, value: &Value) -> Option<TokenStream> {
    let Some(format_name) = value.as_str() else {
        return Some(invalid_schema_type_expression(value, &["string"]));
    };

    // Use explicit macro override when present, otherwise follow draft defaults.
    let should_validate = ctx
        .config
        .validate_formats
        .unwrap_or_else(|| validates_formats_by_default(ctx.draft));
    if !should_validate {
        return None;
    }

    if let Some(custom_call_path) = ctx.config.custom_formats.get(format_name) {
        return Some(quote! { #custom_call_path(s) });
    }

    if let Some(validation_call) = compile_builtin_format_check(ctx, format_name) {
        return Some(validation_call);
    }

    if ctx.config.ignore_unknown_formats {
        None
    } else {
        let message = format!(
            "Unknown format: '{format_name}'. Adjust configuration to ignore unrecognized formats"
        );
        Some(quote! {{
            compile_error!(#message);
            false
        }})
    }
}

#[cfg(test)]
mod schema_compile_phase_tests {
    use super::*;
    use serde_json::json;
    use test_case::test_case;

    #[test_case(json!({"type":"number","const":1}), Draft::Draft7, true; "const_integer_implies_number")]
    #[test_case(json!({"type":"string","const":"x"}), Draft::Draft7, true; "const_string_matches")]
    #[test_case(json!({"type":"integer","const":1.5}), Draft::Draft7, false; "const_mismatch")]
    #[test_case(json!({"type":"number","enum":[1, 2]}), Draft::Draft7, true; "enum_integer_implies_number")]
    #[test_case(json!({"type":"string","enum":["a", "b"]}), Draft::Draft7, true; "enum_string_matches")]
    #[test_case(json!({"type":"integer","enum":[1.5]}), Draft::Draft7, false; "enum_mismatch")]
    #[allow(clippy::needless_pass_by_value)]
    fn type_redundancy_detection(schema: Value, draft: Draft, expected: bool) {
        assert_eq!(
            schema_compile::type_check_is_redundant(&schema, draft),
            expected
        );
    }
}

use super::{
    super::{
        compile_schema,
        draft::DraftExt,
        errors::{invalid_schema_expression, invalid_schema_type_expression},
        expr::IsValidExpr,
        CompileContext, CompiledExpr,
    },
    additional_properties, compile_count_range, dependencies, object_pass, pattern_properties,
    properties, property_names, required, unevaluated_properties,
};
use crate::codegen::emit::ValueEmitter;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::{Map, Value};

fn property_names_implied_by_coverage(schema: &Map<String, Value>) -> bool {
    let sole_pattern = schema
        .get("patternProperties")
        .and_then(Value::as_object)
        .filter(|patterns| patterns.len() == 1)
        .and_then(|patterns| patterns.keys().next());
    let name_pattern = schema
        .get("propertyNames")
        .and_then(Value::as_object)
        .filter(|names| names.len() == 1)
        .and_then(|names| names.get("pattern"))
        .and_then(Value::as_str);
    schema.get("additionalProperties") == Some(&Value::Bool(false))
        && schema
            .get("properties")
            .and_then(Value::as_object)
            .is_none_or(serde_json::Map::is_empty)
        && matches!((sole_pattern, name_pattern), (Some(pattern), Some(name)) if pattern == name)
}

/// Compile all object-specific keywords; with `split_unevaluated`, the `unevaluatedProperties`
/// errors are returned separately.
pub(in super::super) fn compile<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    schema: &Map<String, Value>,
    split_unevaluated: bool,
) -> (CompiledExpr, Option<CompiledExpr>) {
    // Each check carries a flag: `true` marks the object cluster
    // (properties/patternProperties/additionalProperties/required) whose `is_valid` is
    // replaced by the unified single pass; errors still come from every check in order.
    let mut checks: Vec<(CompiledExpr, bool)> = Vec::new();
    let validation_vocab_enabled = ctx.supports_validation_vocabulary();
    let applicator_vocab_enabled = ctx.supports_applicator_vocabulary();
    let unevaluated_properties_enabled = ctx.supports_unevaluated_properties();
    let discriminator_assumption =
        ctx.discriminator_assumption()
            .map(|(property_name, reduced_property)| {
                (property_name.to_owned(), reduced_property.clone())
            });

    let object_len = E::object_len(format_ident!("obj"));
    let mut count_checks: Vec<CompiledExpr> = Vec::new();
    compile_count_range(
        ctx,
        schema
            .get("minProperties")
            .filter(|_| validation_vocab_enabled),
        schema
            .get("maxProperties")
            .filter(|_| validation_vocab_enabled),
        &object_len,
        "minProperties",
        "min_properties",
        "maxProperties",
        "max_properties",
        &mut count_checks,
    );
    for check in count_checks {
        checks.push((check, false));
    }

    // Collect required field names (needed for partitioning with properties below).
    let required_fields: Vec<&str> = if validation_vocab_enabled {
        match schema.get("required") {
            None => Vec::new(),
            Some(value @ Value::Array(arr)) => {
                let mut required = Vec::with_capacity(arr.len());
                for item in arr {
                    if let Some(name) = item.as_str() {
                        if required.contains(&name) {
                            checks.push((
                                invalid_schema_expression(&format!(
                                    "{value} has non-unique elements"
                                )),
                                false,
                            ));
                            break;
                        }
                        required.push(name);
                    } else {
                        checks.push((invalid_schema_type_expression(item, &["string"]), false));
                        break;
                    }
                }
                required
            }
            Some(other) => {
                checks.push((invalid_schema_type_expression(other, &["array"]), false));
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    let properties_map = if applicator_vocab_enabled {
        match schema.get("properties") {
            Some(Value::Object(map)) => Some(map),
            Some(other) => {
                checks.push((invalid_schema_type_expression(other, &["object"]), false));
                None
            }
            None => None,
        }
    } else {
        None
    };

    // `required` is validated before property values, as the runtime orders keywords. It goes here unless the properties pass takes it over, which needs nothing checked
    // in between.
    let required_slot = checks.len();

    // The runtime checks these after `properties`, `patternProperties` and
    // `additionalProperties`: `propertyNames`, then `dependencies`, then `dependentSchemas`.
    let dependencies_check = schema
        .get("dependencies")
        .filter(|_| applicator_vocab_enabled)
        .and_then(|value| dependencies::compile(ctx, value));

    if validation_vocab_enabled && ctx.draft.supports_dependent_required_keyword() {
        if let Some(value) = schema.get("dependentRequired") {
            if let Some(compiled) = dependencies::compile_dependent_required(ctx, value) {
                checks.push((compiled, false));
            }
        }
    }

    let dependent_schemas_check = schema
        .get("dependentSchemas")
        .filter(|_| applicator_vocab_enabled && ctx.draft.supports_dependent_schemas_keyword())
        .and_then(|value| dependencies::compile_dependent_schemas(ctx, value));

    // Names that `patternProperties` and `additionalProperties: false` already reject leave
    // `is_valid` to the unified pass, like the cluster; their errors still report.
    let property_names_check = schema
        .get("propertyNames")
        .filter(|_| applicator_vocab_enabled && ctx.draft.supports_property_names_keyword())
        .map(|value| {
            (
                property_names::compile(ctx, value),
                property_names_implied_by_coverage(schema),
            )
        });

    let additional_properties_schema = schema
        .get("additionalProperties")
        .filter(|_| applicator_vocab_enabled);
    let pattern_properties_schema = schema
        .get("patternProperties")
        .filter(|_| applicator_vocab_enabled);

    // `patternProperties` as a non-empty object, when present.
    let pattern_properties_value = pattern_properties_schema
        .filter(|value| value.as_object().is_some_and(|map| !map.is_empty()));
    // When patternProperties and a false/schema additionalProperties are both present, the runtime
    // fuses property, pattern, and additionalProperties validation into one instance-order pass.
    let additional_properties_fused = matches!(
        additional_properties_schema,
        Some(Value::Bool(false) | Value::Object(_))
    );

    let required_in_properties_pass = !required_fields.is_empty()
        && checks.len() == required_slot
        && properties_map.is_some()
        && !(pattern_properties_value.is_some() && additional_properties_fused);
    if !required_in_properties_pass {
        let required_checks: Vec<(CompiledExpr, bool)> = required_fields
            .iter()
            .map(|name| (required::compile_single(ctx, name), true))
            .collect();
        checks.splice(required_slot..required_slot, required_checks);
    }
    let properties_required: &[&str] = if required_in_properties_pass {
        &required_fields
    } else {
        &[]
    };

    // Every cluster subschema is compiled exactly once and shared by the
    // per-keyword and unified-pass emitters below.
    let cluster = object_pass::compile_cluster_subschemas(
        ctx,
        properties_map.filter(|m| !m.is_empty()),
        pattern_properties_value,
        additional_properties_schema,
    );
    let reduced_discriminator_property =
        discriminator_assumption
            .as_ref()
            .map(|(property_name, reduced_property)| {
                ctx.with_schema_path_segment("properties", |ctx| {
                    ctx.with_schema_path_segment(property_name, |ctx| {
                        compile_schema(ctx, reduced_property)
                    })
                })
            });

    if let Some(pattern_properties_value) =
        pattern_properties_value.filter(|_| additional_properties_fused)
    {
        if let Some(errors) = object_pass::compile_errors(
            ctx,
            &cluster,
            matches!(additional_properties_schema, Some(Value::Bool(false))),
        ) {
            checks.push((CompiledExpr::with_errors(quote! { true }, errors), true));
        } else {
            // Invalid pattern regex: fall back to per-keyword checks so the diagnostic surfaces.
            if let Some(props) = properties_map {
                checks.push((
                    properties::compile(ctx, props, additional_properties_schema, &cluster, &[]),
                    true,
                ));
            }
            checks.push((
                pattern_properties::compile::<E>(pattern_properties_value, &cluster)
                    .expect("an invalid pattern is reported"),
                true,
            ));
            if properties_map.is_none() {
                checks.push((
                    additional_properties::compile(
                        ctx,
                        additional_properties_schema,
                        pattern_properties_schema,
                        cluster.additional.as_ref(),
                    )
                    .expect("an invalid pattern is reported"),
                    true,
                ));
            }
        }
    } else if let Some(props) = properties_map {
        // Sequential: property values, then pattern values (additionalProperties true/absent).
        checks.push((
            properties::compile(
                ctx,
                props,
                additional_properties_schema,
                &cluster,
                properties_required,
            ),
            true,
        ));
        if let Some(value) = pattern_properties_schema {
            if let Some(compiled) = pattern_properties::compile::<E>(value, &cluster) {
                checks.push((compiled, true));
            }
        }
    } else if applicator_vocab_enabled {
        if let Some(value) = pattern_properties_schema {
            if let Some(compiled) = pattern_properties::compile::<E>(value, &cluster) {
                checks.push((compiled, true));
            }
        }
        if let Some(compiled) = additional_properties::compile(
            ctx,
            additional_properties_schema,
            pattern_properties_schema,
            cluster.additional.as_ref(),
        ) {
            checks.push((compiled, true));
        }
    }

    checks.extend(property_names_check);
    for check in [dependencies_check, dependent_schemas_check]
        .into_iter()
        .flatten()
    {
        checks.push((check, false));
    }

    let mut unevaluated_errors = None;
    if unevaluated_properties_enabled {
        if let Some(compiled) = unevaluated_properties::compile(ctx, schema) {
            let compiled = if split_unevaluated {
                let (check, errors) = compiled.split_errors();
                unevaluated_errors = Some(errors);
                check
            } else {
                compiled
            };
            checks.push((compiled, false));
        }
    }

    let is_valid_required_fields: Vec<_> = required_fields
        .iter()
        .copied()
        .filter(|name| {
            discriminator_assumption
                .as_ref()
                .is_none_or(|(property_name, _)| *name != property_name)
        })
        .collect();
    let reduced_discriminator_property = discriminator_assumption
        .as_ref()
        .zip(reduced_discriminator_property.as_ref())
        .map(|((property_name, _), compiled)| (property_name.as_str(), compiled));
    let unified = object_pass::compile_is_valid::<E>(
        &cluster,
        additional_properties_schema,
        &is_valid_required_fields,
        reduced_discriminator_property,
    );

    let checks = match unified {
        Some(pass) => {
            // Cluster `is_valid` is replaced by the unified pass; non-cluster keywords still gate.
            let extra: Vec<TokenStream> = checks
                .iter()
                .filter(|(_, cluster)| !*cluster)
                .map(|(check, _)| check.is_valid_token_stream())
                .collect();
            let combined = CompiledExpr::combine_and(checks.into_iter().map(|(check, _)| check));
            let is_valid = if extra.is_empty() {
                pass
            } else {
                quote! { (#pass) #(&& (#extra))* }
            };
            CompiledExpr {
                is_valid: IsValidExpr::Expr(is_valid),
                errors: combined.errors,
                compile_error: combined.compile_error,
                skips_valid: combined.skips_valid,
                ref_errors: combined.ref_errors,
            }
        }
        None => CompiledExpr::combine_and(checks.into_iter().map(|(check, _)| check)),
    };
    (checks, unevaluated_errors)
}

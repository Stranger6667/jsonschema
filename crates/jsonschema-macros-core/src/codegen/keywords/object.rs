use super::{
    super::{
        draft::DraftExt,
        errors::{invalid_schema_expression, invalid_schema_type_expression},
        expr::IsValidExpr,
        CompileContext, CompiledExpr,
    },
    additional_properties, compile_count_range, dependencies, object_pass, pattern_properties,
    properties, property_names, required, unevaluated_properties,
};
use proc_macro2::TokenStream;
use quote::quote;
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

/// Compile all object-specific keywords.
pub(in super::super) fn compile(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> CompiledExpr {
    // Each check carries a flag: `true` marks the object cluster
    // (properties/patternProperties/additionalProperties/required) whose `is_valid` is
    // replaced by the unified single pass; `validate` still uses every check in order.
    let mut checks: Vec<(CompiledExpr, bool)> = Vec::new();
    let validation_vocab_enabled = ctx.supports_validation_vocabulary();
    let applicator_vocab_enabled = ctx.supports_applicator_vocabulary();
    let unevaluated_properties_enabled = ctx.supports_unevaluated_properties();

    let object_len = crate::codegen::emit_serde::object_len(quote! { obj });
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

    // The runtime folds `required` into the property loop (checked after property values) only for
    // the additionalProperties:false single-required fusion; every other shape validates `required`
    // before property values. Emit the checks before `properties` here, or after it (below) for the
    // fused shape.
    let has_pattern_properties = schema
        .get("patternProperties")
        .and_then(Value::as_object)
        .is_some_and(|m| !m.is_empty());
    let required_after_properties =
        matches!(schema.get("additionalProperties"), Some(Value::Bool(false)))
            && required_fields.len() == 1
            && properties_map.is_some()
            && !has_pattern_properties;

    let required_checks = required_fields
        .iter()
        .map(|name| required::compile_single(ctx, name));
    let required_after_checks: Vec<CompiledExpr> = if required_after_properties {
        required_checks.collect()
    } else {
        for check in required_checks {
            checks.push((check, true));
        }
        Vec::new()
    };

    if applicator_vocab_enabled {
        if let Some(value) = schema.get("dependencies") {
            if let Some(compiled) = dependencies::compile(ctx, value) {
                checks.push((compiled, false));
            }
        }
    }

    if validation_vocab_enabled && ctx.draft.supports_dependent_required_keyword() {
        if let Some(value) = schema.get("dependentRequired") {
            if let Some(compiled) = dependencies::compile_dependent_required(ctx, value) {
                checks.push((compiled, false));
            }
        }
    }

    if applicator_vocab_enabled && ctx.draft.supports_dependent_schemas_keyword() {
        if let Some(value) = schema.get("dependentSchemas") {
            if let Some(compiled) = dependencies::compile_dependent_schemas(ctx, value) {
                checks.push((compiled, false));
            }
        }
    }

    if applicator_vocab_enabled && ctx.draft.supports_property_names_keyword() {
        if let Some(value) = schema.get("propertyNames") {
            if !property_names_implied_by_coverage(schema) {
                checks.push((property_names::compile(ctx, value), false));
            }
        }
    }

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

    // Every cluster subschema is compiled exactly once and shared by the
    // per-keyword and unified-pass emitters below.
    let cluster = object_pass::compile_cluster_subschemas(
        ctx,
        properties_map.filter(|m| !m.is_empty()),
        pattern_properties_value,
        additional_properties_schema,
    );

    if let Some(pattern_properties_value) =
        pattern_properties_value.filter(|_| additional_properties_fused)
    {
        if let Some(validate) =
            object_pass::compile_validate(ctx, &cluster, additional_properties_schema)
        {
            checks.push((
                CompiledExpr::with_validate_blocks(quote! { true }, validate),
                true,
            ));
        } else {
            // Invalid pattern regex: fall back to per-keyword checks so the diagnostic surfaces.
            if let Some(props) = properties_map {
                checks.push((
                    properties::compile(ctx, props, additional_properties_schema, &cluster),
                    true,
                ));
            }
            if let Some(compiled) = pattern_properties::compile(pattern_properties_value, &cluster)
            {
                checks.push((compiled, true));
            }
            if properties_map.is_none() {
                if let Some(compiled) = additional_properties::compile(
                    ctx,
                    additional_properties_schema,
                    pattern_properties_schema,
                    cluster.additional.as_ref(),
                ) {
                    checks.push((compiled, true));
                }
            }
        }
    } else if let Some(props) = properties_map {
        // Sequential: property values, then pattern values (additionalProperties true/absent).
        checks.push((
            properties::compile(ctx, props, additional_properties_schema, &cluster),
            true,
        ));
        if let Some(value) = pattern_properties_schema {
            if let Some(compiled) = pattern_properties::compile(value, &cluster) {
                checks.push((compiled, true));
            }
        }
    } else if applicator_vocab_enabled {
        if let Some(value) = pattern_properties_schema {
            if let Some(compiled) = pattern_properties::compile(value, &cluster) {
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

    for check in required_after_checks {
        checks.push((check, true));
    }

    if unevaluated_properties_enabled {
        if let Some(compiled) = unevaluated_properties::compile(ctx, schema) {
            checks.push((compiled, false));
        }
    }

    let unified =
        object_pass::compile_is_valid(&cluster, additional_properties_schema, &required_fields);

    match unified {
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
                validate: combined.validate,
                compile_error: combined.compile_error,
            }
        }
        None => CompiledExpr::combine_and(checks.into_iter().map(|(check, _)| check)),
    }
}

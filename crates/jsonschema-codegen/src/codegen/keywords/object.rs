use super::{
    super::{
        draft::DraftExt, errors::invalid_schema_type_expression, CompileContext, CompiledExpr,
    },
    additional_properties, compile_count_limit, dependencies, pattern_properties, properties,
    property_names, required, unevaluated_properties, Limit,
};
use quote::quote;
use serde_json::{Map, Value};

/// Compile all object-specific keywords.
pub(in super::super) fn compile(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> CompiledExpr {
    let mut checks: Vec<CompiledExpr> = Vec::new();
    let validation_vocab_enabled = ctx.supports_validation_vocabulary();
    let applicator_vocab_enabled = ctx.supports_applicator_vocabulary();
    let unevaluated_properties_enabled = ctx.supports_unevaluated_properties();

    let object_len = crate::codegen::emit_serde::object_len(quote! { obj });
    if let Some(v) = schema
        .get("minProperties")
        .filter(|_| validation_vocab_enabled)
    {
        checks.push(compile_count_limit(
            ctx,
            v,
            &object_len,
            "minProperties",
            "min_properties",
            &Limit::Min,
        ));
    }
    if let Some(v) = schema
        .get("maxProperties")
        .filter(|_| validation_vocab_enabled)
    {
        checks.push(compile_count_limit(
            ctx,
            v,
            &object_len,
            "maxProperties",
            "max_properties",
            &Limit::Max,
        ));
    }

    // Collect required field names (needed for partitioning with properties below).
    let required_fields: Vec<&str> = if validation_vocab_enabled {
        match schema.get("required") {
            None => Vec::new(),
            Some(Value::Array(arr)) => {
                let mut required = Vec::with_capacity(arr.len());
                for item in arr {
                    if let Some(name) = item.as_str() {
                        required.push(name);
                    } else {
                        checks.push(invalid_schema_type_expression(item, &["string"]));
                        break;
                    }
                }
                required
            }
            Some(other) => {
                checks.push(invalid_schema_type_expression(other, &["array"]));
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
                checks.push(invalid_schema_type_expression(other, &["object"]));
                None
            }
            None => None,
        }
    } else {
        None
    };

    // Partition required fields: those in `properties` are tracked inside the match;
    // the rest get standalone contains_key checks.
    let (required_in_props, required_only): (Vec<&str>, Vec<&str>) =
        if let Some(props) = properties_map {
            required_fields
                .iter()
                .copied()
                .partition(|name| props.contains_key(*name))
        } else {
            (Vec::new(), required_fields.clone())
        };

    for name in &required_only {
        checks.push(required::compile_single(ctx, name));
    }

    // patternProperties
    if applicator_vocab_enabled {
        if let Some(v) = schema.get("patternProperties") {
            if let Some(compiled) = pattern_properties::compile(ctx, v) {
                checks.push(compiled);
            }
        }
    }

    // dependencies
    if applicator_vocab_enabled {
        if let Some(v) = schema.get("dependencies") {
            if let Some(compiled) = dependencies::compile(ctx, v) {
                checks.push(compiled);
            }
        }
    }

    // dependentRequired
    if validation_vocab_enabled && ctx.draft.supports_dependent_required_keyword() {
        if let Some(v) = schema.get("dependentRequired") {
            if let Some(compiled) = dependencies::compile_dependent_required(ctx, v) {
                checks.push(compiled);
            }
        }
    }

    // dependentSchemas
    if applicator_vocab_enabled && ctx.draft.supports_dependent_schemas_keyword() {
        if let Some(v) = schema.get("dependentSchemas") {
            if let Some(compiled) = dependencies::compile_dependent_schemas(ctx, v) {
                checks.push(compiled);
            }
        }
    }

    // propertyNames
    if applicator_vocab_enabled && ctx.draft.supports_property_names_keyword() {
        if let Some(v) = schema.get("propertyNames") {
            checks.push(property_names::compile(ctx, v));
        }
    }

    let ap = if applicator_vocab_enabled {
        schema.get("additionalProperties")
    } else {
        None
    };
    let pp = if applicator_vocab_enabled {
        schema.get("patternProperties")
    } else {
        None
    };

    if let Some(props) = properties_map {
        checks.push(properties::compile(ctx, props, ap, pp, &required_in_props));
    } else if applicator_vocab_enabled {
        if let Some(compiled) =
            additional_properties::compile(ctx, ap, schema.get("properties"), pp)
        {
            checks.push(compiled);
        }
    }

    if unevaluated_properties_enabled {
        if let Some(compiled) = unevaluated_properties::compile(ctx, schema) {
            checks.push(compiled);
        }
    }

    CompiledExpr::combine_and(checks)
}

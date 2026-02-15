use super::{
    super::{
        draft::{
            supports_applicator_vocabulary, supports_dependent_required_keyword,
            supports_dependent_schemas_keyword, supports_property_names_keyword,
            supports_unevaluated_properties_keyword_for_context, supports_validation_vocabulary,
        },
        errors::invalid_schema_type_expression,
        parse_nonnegative_integer_keyword, CompileContext, CompiledExpr,
    },
    additional_properties, dependencies, max_properties, min_properties, pattern_properties,
    properties, property_names, required, unevaluated_properties,
};
use quote::quote;
use serde_json::{Map, Value};

/// Compile all object-specific keywords.
pub(in super::super) fn compile(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> CompiledExpr {
    let mut checks: Vec<CompiledExpr> = Vec::new();
    let validation_vocab_enabled = supports_validation_vocabulary(ctx);
    let applicator_vocab_enabled = supports_applicator_vocabulary(ctx);
    let unevaluated_properties_enabled = supports_unevaluated_properties_keyword_for_context(ctx);

    // minProperties / maxProperties with shared-length optimization.
    let min = if validation_vocab_enabled {
        schema
            .get("minProperties")
            .map(|v| parse_nonnegative_integer_keyword(ctx.draft, v))
    } else {
        None
    };
    let max = if validation_vocab_enabled {
        schema
            .get("maxProperties")
            .map(|v| parse_nonnegative_integer_keyword(ctx.draft, v))
    } else {
        None
    };
    let object_len = ctx.config.backend.object_len(quote! { obj });
    match (&min, &max) {
        (Some(Ok(mn)), Some(Ok(mx))) if mn == mx => {
            let sp_min = ctx.schema_path_for_keyword("minProperties");
            let sp_max = ctx.schema_path_for_keyword("maxProperties");
            let sp = if schema.contains_key("minProperties") {
                sp_min
            } else {
                sp_max
            };
            checks.push(CompiledExpr::from_bool_expr(
                quote! { #object_len == #mn as usize },
                &sp,
            ));
        }
        (Some(Ok(mn)), Some(Ok(mx))) => {
            let sp_min = ctx.schema_path_for_keyword("minProperties");
            let sp_max = ctx.schema_path_for_keyword("maxProperties");
            checks.push(CompiledExpr::from_bool_expr(
                quote! { #object_len >= #mn as usize },
                &sp_min,
            ));
            checks.push(CompiledExpr::from_bool_expr(
                quote! { #object_len <= #mx as usize },
                &sp_max,
            ));
        }
        (Some(Ok(mn)), None) => {
            let sp = ctx.schema_path_for_keyword("minProperties");
            checks.push(CompiledExpr::from_bool_expr(
                quote! { #object_len >= #mn as usize },
                &sp,
            ));
        }
        (None, Some(Ok(mx))) => {
            let sp = ctx.schema_path_for_keyword("maxProperties");
            checks.push(CompiledExpr::from_bool_expr(
                quote! { #object_len <= #mx as usize },
                &sp,
            ));
        }
        _ => {
            if let Some(v) = schema
                .get("minProperties")
                .filter(|_| validation_vocab_enabled)
            {
                checks.push(min_properties::compile(ctx, v));
            }
            if let Some(v) = schema
                .get("maxProperties")
                .filter(|_| validation_vocab_enabled)
            {
                checks.push(max_properties::compile(ctx, v));
            }
        }
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
    if validation_vocab_enabled && supports_dependent_required_keyword(ctx.draft) {
        if let Some(v) = schema.get("dependentRequired") {
            if let Some(compiled) = dependencies::compile_dependent_required(ctx, v) {
                checks.push(compiled);
            }
        }
    }

    // dependentSchemas
    if applicator_vocab_enabled && supports_dependent_schemas_keyword(ctx.draft) {
        if let Some(v) = schema.get("dependentSchemas") {
            if let Some(compiled) = dependencies::compile_dependent_schemas(ctx, v) {
                checks.push(compiled);
            }
        }
    }

    // propertyNames
    if applicator_vocab_enabled && supports_property_names_keyword(ctx.draft) {
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

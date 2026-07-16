use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::write_escaped_str;
use serde_json::{Map, Value};

use crate::context::CompileContext;

use super::{
    compile_schema,
    draft::DraftExt,
    helpers::{
        collect_dynamic_anchor_bindings, dynamic_ref_anchor_name, get_or_create_item_eval_fn,
        get_or_create_key_eval_fn,
    },
    keywords::{
        custom, format,
        pattern_properties::key_match_expr,
        unevaluated::{compile_index_evaluated_expr, compile_key_evaluated_expr, GuardHoist},
    },
    numeric::value_as_u64,
    object_schema,
    refs::resolve_ref,
    stack_emit::{stack_scoped_body, EVALUATION_FAMILY, ITEM_EVAL_FAMILY, KEY_EVAL_FAMILY},
    CompiledExpr,
};

const LEAF_KEYWORDS: &[&str] = &[
    "type",
    "const",
    "enum",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "minProperties",
    "maxProperties",
    "required",
    "pattern",
    "uniqueItems",
];

#[allow(clippy::unnecessary_wraps)]
pub(super) fn compile(ctx: &mut CompileContext<'_>, schema: &Value) -> Option<TokenStream> {
    let base_uri = ctx.current_base_uri.clone();
    let identifier = schema
        .as_object()
        .and_then(|schema| schema.get(ctx.draft.id_keyword()).and_then(Value::as_str));
    let location = identifier
        .and_then(|identifier| {
            ctx.config
                .registry
                .resolver((*base_uri).clone())
                .lookup(identifier)
                .ok()
                .map(|resolved| resolved.resolver().base_uri().to_string())
        })
        .unwrap_or_else(|| base_uri.to_string());
    let helper =
        get_or_create_evaluation_fn_with(ctx, &location, &location, schema, base_uri, true, true);
    let helper = format_ident!("{helper}");
    let root = quote! { #helper(instance, __path, __il, "") };
    let root_baked = ctx.baked_root_evaluation_helpers.contains(&location);
    let root = identifier
        .filter(|_| !location.starts_with("json-schema:///") && !root_baked)
        .map_or_else(
            || quote! { #root },
            |_| quote! { __eval::absolute(#root, #location) },
        );
    let clear_cache = evaluation_memo_needed(ctx).then(|| {
        quote! {
            __JSONSCHEMA_EVALUATION_CACHE.with(|__cache| __cache.borrow_mut().clear());
        }
    });
    Some(quote! {
        let __root_path = __paths::LazyLocation::new();
        let __path = &__root_path;
        let __root_il = __paths::Location::new();
        let __il = &__root_il;
        let __schema_path = "";
        #clear_cache
        __eval::finish(#root)
    })
}

pub(super) fn evaluation_memo_needed(ctx: &CompileContext<'_>) -> bool {
    !ctx.cyclic_evaluation_helpers.is_empty()
        && !ctx.draft.supports_recursive_ref_keyword()
        && !ctx.draft.supports_dynamic_ref_keyword()
}

fn compile_schema_node(ctx: &mut CompileContext<'_>, schema: &Value) -> TokenStream {
    ctx.with_schema_scope(|ctx| match schema {
        Value::Bool(valid) => compile_boolean(ctx, *valid),
        Value::Object(object) => {
            let identifier = (ctx.draft.supports_adjacent_validation()
                || !object.contains_key("$ref"))
            .then(|| object.get(ctx.draft.id_keyword()).and_then(Value::as_str))
            .flatten();
            let base_uri = identifier.and_then(|identifier| {
                let resolver = ctx
                    .config
                    .registry
                    .resolver((*ctx.current_base_uri).clone());
                resolver
                    .lookup(identifier)
                    .ok()
                    .map(|resolved| resolved.resolver().base_uri().clone())
            });
            if let Some(base_uri) = base_uri {
                let absolute_base = base_uri.to_string();
                let nested = !ctx.current_schema_path().is_empty();
                ctx.with_base_uri_scope(base_uri, |ctx| {
                    let compiled = compile_object(ctx, object);
                    let compiled = if nested && !ctx.bakes_absolute_locations(&absolute_base) {
                        quote! { __eval::absolute(#compiled, #absolute_base) }
                    } else {
                        compiled
                    };
                    let recursive_anchor = ctx.draft.supports_recursive_ref_keyword()
                        && object.get("$recursiveAnchor").and_then(Value::as_bool) == Some(true);
                    let (recursive_push, recursive_pop) = if nested
                        && recursive_anchor
                        && ctx.uses_recursive_ref
                    {
                        let location =
                            format!("{}#", ctx.current_base_uri.as_str().trim_end_matches('#'));
                        let current_base_uri = ctx.current_base_uri.clone();
                        let helper = get_or_create_evaluation_fn(
                            ctx,
                            &location,
                            schema,
                            current_base_uri.clone(),
                        );
                        let helper = format_ident!("{helper}");
                        let key_helper = format_ident!(
                            "{}",
                            get_or_create_key_eval_fn(
                                ctx,
                                &location,
                                schema,
                                current_base_uri.clone(),
                            )
                        );
                        let item_helper = format_ident!(
                            "{}",
                            get_or_create_item_eval_fn(ctx, &location, schema, current_base_uri,)
                        );
                        let evaluation_push = (EVALUATION_FAMILY.push_recursive)(&helper, true);
                        let key_push = (KEY_EVAL_FAMILY.push_recursive)(&key_helper, true);
                        let item_push = (ITEM_EVAL_FAMILY.push_recursive)(&item_helper, true);
                        let item_pop = (ITEM_EVAL_FAMILY.pop_recursive)();
                        let key_pop = (KEY_EVAL_FAMILY.pop_recursive)();
                        let evaluation_pop = (EVALUATION_FAMILY.pop_recursive)();
                        (
                            quote! {
                                #evaluation_push
                                #key_push
                                #item_push
                            },
                            quote! {
                                #item_pop
                                #key_pop
                                #evaluation_pop
                            },
                        )
                    } else {
                        (TokenStream::new(), TokenStream::new())
                    };
                    if nested && (ctx.uses_dynamic_ref || recursive_anchor) {
                        let bindings = if ctx.uses_dynamic_ref {
                            let current_base_uri = ctx.current_base_uri.clone();
                            collect_dynamic_anchor_bindings(ctx, current_base_uri)
                        } else {
                            Vec::new()
                        };
                        let evaluation_pushes = EVALUATION_FAMILY.dynamic_pushes(&bindings);
                        let key_pushes = KEY_EVAL_FAMILY.dynamic_pushes(&bindings);
                        let item_pushes = ITEM_EVAL_FAMILY.dynamic_pushes(&bindings);
                        let evaluation_pop = (EVALUATION_FAMILY.pop_dynamic_n)(bindings.len());
                        let key_pop = (KEY_EVAL_FAMILY.pop_dynamic_n)(bindings.len());
                        let item_pop = (ITEM_EVAL_FAMILY.pop_dynamic_n)(bindings.len());
                        quote! {
                            {
                                #recursive_push
                                #(#evaluation_pushes)*
                                #(#key_pushes)*
                                #(#item_pushes)*
                                let __result = { #compiled };
                                #item_pop
                                #key_pop
                                #evaluation_pop
                                #recursive_pop
                                __result
                            }
                        }
                    } else {
                        compiled
                    }
                })
            } else {
                compile_object(ctx, object)
            }
        }
        // A non-boolean, non-object subschema is invalid; `compile_schema` (is_valid) reports it
        // via `compile_error!`, so this diverging placeholder only keeps the module well-typed.
        _ => quote! {{
            let __invalid: __eval::Node = unreachable!();
            __invalid
        }},
    })
}

fn compile_boolean(ctx: &mut CompileContext<'_>, valid: bool) -> TokenStream {
    let path = ctx.current_schema_path().to_owned();
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    let errors = if valid {
        quote! { Vec::new() }
    } else {
        quote! {
            vec![__err::false_schema(
                #path,
                __il.clone(),
                instance,
            )]
        }
    };
    quote! {
        {
            __eval::node_at(__eval_path,
                &#sp,
                __il.clone(),
                #errors,
                None,
                Vec::new(),
            )
        }
    }
}

fn keyword_priority(keyword: &str) -> u8 {
    match keyword {
        "type" => 1,
        "const" => 5,
        "enum" => 6,
        "minimum" => 10,
        "maximum" => 11,
        "exclusiveMinimum" => 12,
        "exclusiveMaximum" => 13,
        "multipleOf" => 14,
        "minLength" => 20,
        "maxLength" => 21,
        "minItems" => 22,
        "maxItems" => 23,
        "minProperties" => 24,
        "maxProperties" => 25,
        "required" => 26,
        "dependentRequired" => 27,
        "pattern" => 30,
        "format" => 31,
        "contentEncoding" => 32,
        "contentMediaType" => 33,
        "contentSchema" => 34,
        "uniqueItems" => 35,
        "properties" => 40,
        "patternProperties" => 41,
        "additionalProperties" => 42,
        "propertyNames" => 43,
        "items" => 44,
        "prefixItems" => 45,
        "additionalItems" => 46,
        "contains" => 47,
        "dependencies" => 48,
        "dependentSchemas" => 49,
        "allOf" => 50,
        "anyOf" => 51,
        "oneOf" => 52,
        "not" => 53,
        "if" => 54,
        "unevaluatedProperties" => 60,
        "unevaluatedItems" => 61,
        "$ref" => 70,
        "$recursiveRef" => 71,
        "$dynamicRef" => 72,
        _ => 80,
    }
}

fn compile_object(ctx: &mut CompileContext<'_>, schema: &Map<String, Value>) -> TokenStream {
    let node_path = ctx.current_schema_path().to_owned();
    let node_sp = format_ident!("{}", ctx.intern_schema_path(&node_path));
    if !ctx.draft.supports_adjacent_validation() {
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            let evaluation = compile_reference(ctx, "$ref", reference);
            let ignored_siblings: Map<String, Value> = schema
                .iter()
                .filter(|(keyword, _)| !matches!(keyword.as_str(), "$ref" | "$schema"))
                .map(|(keyword, value)| (keyword.clone(), value.clone()))
                .collect();
            let annotations = if ignored_siblings.is_empty() {
                quote! { None }
            } else {
                let serialized = serde_json::to_string(&Value::Object(ignored_siblings))
                    .expect("ignored reference siblings serialize");
                let annotation = cached_annotation(&serialized);
                quote! { Some(#annotation) }
            };
            return quote! {
                {
                    let mut __children = Vec::with_capacity(1usize);
                    #evaluation
                    __eval::node_at(__eval_path,
                        &#node_sp,
                        __il.clone(),
                        Vec::new(),
                        #annotations,
                        __children,
                    )
                }
            };
        }
    }
    let mut evaluations = compile_leaf_assertions(ctx, schema);
    evaluations.extend(compile_annotation_keywords(ctx, schema));
    if ctx.supports_applicator_vocabulary() {
        let additional_properties = schema.get("additionalProperties");
        let fused_object = matches!(
            additional_properties,
            Some(Value::Bool(false) | Value::Object(_))
        );
        if fused_object {
            evaluations.push((
                keyword_priority("additionalProperties"),
                compile_additional_properties(ctx, schema),
            ));
        } else {
            if let Some(Value::Object(properties)) = schema.get("properties") {
                evaluations.push((
                    keyword_priority("properties"),
                    compile_properties(ctx, schema, properties),
                ));
            }
            if let Some(Value::Object(patterns)) = schema.get("patternProperties") {
                evaluations.push((
                    keyword_priority("patternProperties"),
                    compile_pattern_properties(ctx, patterns),
                ));
            }
        }
        if ctx.draft.supports_property_names_keyword() {
            if let Some(property_names) = schema
                .get("propertyNames")
                .filter(|property_names| property_names.as_bool() != Some(true))
            {
                evaluations.push((
                    keyword_priority("propertyNames"),
                    compile_property_names(ctx, property_names),
                ));
            }
        }
        if let Some(items) = schema
            .get("items")
            .filter(|items| items.as_bool() != Some(true))
        {
            evaluations.extend(
                compile_items(ctx, schema, items)
                    .into_iter()
                    .map(|evaluation| (keyword_priority("items"), evaluation)),
            );
        }
        if ctx.draft.supports_prefix_items_keyword() {
            if let Some(Value::Array(prefix_items)) = schema.get("prefixItems") {
                evaluations.push((
                    keyword_priority("prefixItems"),
                    compile_prefix_items(ctx, schema, prefix_items),
                ));
            }
        }
        if !ctx.draft.supports_prefix_items_keyword() {
            if let (Some(Value::Array(items)), Some(additional_items)) =
                (schema.get("items"), schema.get("additionalItems"))
            {
                evaluations.push((
                    keyword_priority("additionalItems"),
                    compile_additional_items(ctx, additional_items, items.len()),
                ));
            }
        }
        if let Some(contains) = schema.get("contains") {
            let evaluation =
                if schema.contains_key("minContains") || schema.contains_key("maxContains") {
                    compile_bounded_contains(ctx, schema, contains)
                } else {
                    compile_contains(ctx, contains)
                };
            evaluations.push((keyword_priority("contains"), evaluation));
        }
        if let Some(Value::Object(dependencies)) = schema.get("dependencies") {
            evaluations.push((
                keyword_priority("dependencies"),
                compile_dependencies(ctx, "dependencies", dependencies),
            ));
        }
        if ctx.draft.supports_dependent_schemas_keyword() {
            if let Some(Value::Object(dependencies)) = schema.get("dependentRequired") {
                evaluations.push((
                    keyword_priority("dependentRequired"),
                    compile_dependencies(ctx, "dependentRequired", dependencies),
                ));
            }
            if let Some(Value::Object(dependencies)) = schema.get("dependentSchemas") {
                evaluations.push((
                    keyword_priority("dependentSchemas"),
                    compile_dependencies(ctx, "dependentSchemas", dependencies),
                ));
            }
        }
        for keyword in ["allOf", "anyOf", "oneOf"] {
            if let Some(Value::Array(branches)) = schema.get(keyword) {
                evaluations.push((
                    keyword_priority(keyword),
                    compile_branch(ctx, keyword, branches),
                ));
            }
        }
        if ctx.draft.supports_if_then_else_keyword()
            && schema.contains_key("if")
            && (schema.contains_key("then") || schema.contains_key("else"))
        {
            evaluations.push((keyword_priority("if"), compile_conditional(ctx, schema)));
        }
    }
    if let Some(value) = schema.get("not") {
        let mut keyword_schema = Map::new();
        keyword_schema.insert("not".to_owned(), value.clone());
        let compiled = compile_cached_keyword_expr(ctx, "not", &keyword_schema);
        evaluations.push((keyword_priority("not"), compile_leaf(ctx, "not", &compiled)));
    }
    if ctx.supports_unevaluated_properties() {
        if let Some(unevaluated) = schema
            .get("unevaluatedProperties")
            .filter(|value| value.as_bool() != Some(true))
        {
            evaluations.push((
                keyword_priority("unevaluatedProperties"),
                compile_unevaluated_properties(ctx, schema, unevaluated),
            ));
        }
    }
    if ctx.supports_unevaluated_items() {
        if let Some(unevaluated) = schema
            .get("unevaluatedItems")
            .filter(|value| value.as_bool() != Some(true))
        {
            evaluations.push((
                keyword_priority("unevaluatedItems"),
                compile_unevaluated_items(ctx, schema, unevaluated),
            ));
        }
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        if let Some(evaluation) = compile_reference(ctx, "$ref", reference) {
            evaluations.push((keyword_priority("$ref"), evaluation));
        }
    }
    if ctx.draft.supports_recursive_ref_keyword() {
        if let Some(reference) = schema.get("$recursiveRef").and_then(Value::as_str) {
            if let Some(evaluation) = compile_reference(ctx, "$recursiveRef", reference) {
                evaluations.push((keyword_priority("$recursiveRef"), evaluation));
            }
        }
    }
    if ctx.draft.supports_dynamic_ref_keyword() {
        if let Some(reference) = schema.get("$dynamicRef").and_then(Value::as_str) {
            if let Some(evaluation) = compile_reference(ctx, "$dynamicRef", reference) {
                evaluations.push((keyword_priority("$dynamicRef"), evaluation));
            }
        }
    }
    evaluations.sort_by_key(|(priority, _)| *priority);
    let evaluations: Vec<TokenStream> = evaluations
        .into_iter()
        .map(|(_, evaluation)| evaluation)
        .collect();
    let capacity = evaluations.len();
    let schema_annotations: Map<String, Value> = schema
        .iter()
        .filter(|(keyword, _)| {
            !ctx.draft.is_known_keyword(keyword)
                && !ctx.config.custom_keywords.contains_key(keyword.as_str())
        })
        .map(|(keyword, value)| (keyword.clone(), value.clone()))
        .collect();
    let schema_annotations = if schema_annotations.is_empty() {
        quote! { None }
    } else {
        let serialized = serde_json::to_string(&Value::Object(schema_annotations))
            .expect("schema annotations serialize");
        let annotation = cached_annotation(&serialized);
        quote! { Some(#annotation) }
    };
    quote! {
        {
            let mut __children = Vec::with_capacity(#capacity);
            #(#evaluations)*
            __eval::node_at(__eval_path,
                &#node_sp,
                __il.clone(),
                Vec::new(),
                #schema_annotations,
                __children,
            )
        }
    }
}

fn compile_annotation_keywords(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> Vec<(u8, TokenStream)> {
    let mut evaluations = Vec::new();
    if matches!(
        ctx.draft,
        referencing::Draft::Draft201909 | referencing::Draft::Draft202012
    ) {
        for keyword in ["format", "contentEncoding", "contentMediaType"] {
            if let Some(Value::String(value)) = schema.get(keyword) {
                if keyword == "format" && !format::is_known(ctx, value) {
                    continue;
                }
                let mut keyword_schema = Map::new();
                keyword_schema.insert(keyword.to_owned(), Value::String(value.clone()));
                let compiled = compile_cached_keyword_expr(ctx, keyword, &keyword_schema);
                let evaluation = if compiled.is_trivially_true() {
                    compile_annotation_keyword(ctx, keyword, value)
                } else {
                    let serialized = serde_json::to_string(value).expect("annotation serializes");
                    compile_leaf_with_annotation(ctx, keyword, &compiled, &serialized)
                };
                evaluations.push((keyword_priority(keyword), evaluation));
            }
        }
        if schema.contains_key("contentMediaType") {
            if let Some(value) = schema.get("contentSchema") {
                let serialized = serde_json::to_string(value).expect("contentSchema serializes");
                evaluations.push((
                    keyword_priority("contentSchema"),
                    compile_serialized_annotation_keyword(ctx, "contentSchema", &serialized),
                ));
            }
        }
    }
    for (name, value) in schema {
        if ctx.config.custom_keywords.contains_key(name) {
            let compiled = custom::compile(ctx, name, schema, value);
            evaluations.push((keyword_priority(name), compile_leaf(ctx, name, &compiled)));
        }
    }
    evaluations
}

fn compile_annotation_keyword(
    ctx: &mut CompileContext<'_>,
    keyword: &str,
    value: &str,
) -> TokenStream {
    let serialized = serde_json::to_string(value).expect("annotation serializes");
    compile_serialized_annotation_keyword(ctx, keyword, &serialized)
}

/// Emit a compile-time-constant annotation parsed once into a `static` and cloned (an `Arc` bump)
/// per node, instead of re-parsing the JSON on every `evaluate` call.
fn cached_annotation(serialized: &str) -> TokenStream {
    quote! {
        {
            static __ANNOTATION: __Lazy<
                __eval::Annotations,
            > = __Lazy::new(|| {
                __eval::annotation(#serialized)
            });
            __ANNOTATION.clone()
        }
    }
}

fn compile_serialized_annotation_keyword(
    ctx: &mut CompileContext<'_>,
    keyword: &str,
    serialized: &str,
) -> TokenStream {
    let path = ctx.schema_path_for_keyword(keyword);
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    let annotation = cached_annotation(serialized);
    quote! {
        {
            let __annotations = instance.is_string().then(|| #annotation);
            __children.push(__eval::node_at(
                __eval_path,
                &#sp,
                __il.clone(),
                Vec::new(),
                __annotations,
                Vec::new(),
            ));
        }
    }
}

/// Compiles a synthetic single-keyword schema, sharing the result across identical keyword
/// values. Sound because leaf compiles are path-independent: error constructors receive the
/// runtime `__schema_path` binding, and helper registries are append-only and deduplicated.
fn compile_cached_keyword_expr(
    ctx: &mut CompileContext<'_>,
    keyword: &str,
    keyword_schema: &Map<String, Value>,
) -> CompiledExpr {
    let cache_key = format!(
        "{keyword}\u{0}{}\u{0}{:?}\u{0}{:?}",
        serde_json::to_string(keyword_schema).expect("keyword schema serializes"),
        ctx.draft,
        ctx.vocabularies,
    );
    if let Some(cached) = ctx.leaf_expr_cache.get(&cache_key) {
        return cached.clone();
    }
    let compiled = ctx.with_schema_path_swap(String::new(), |ctx| {
        ctx.with_schema_scope(|ctx| object_schema::compile_object_schema(ctx, keyword_schema))
    });
    ctx.leaf_expr_cache.insert(cache_key, compiled.clone());
    compiled
}

fn compile_leaf_assertions(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> Vec<(u8, TokenStream)> {
    let mut evaluations = Vec::new();
    for keyword in LEAF_KEYWORDS {
        if matches!(ctx.draft, referencing::Draft::Draft4)
            && matches!(*keyword, "exclusiveMinimum" | "exclusiveMaximum")
        {
            continue;
        }
        let Some(value) = schema.get(*keyword) else {
            continue;
        };
        if *keyword == "required"
            && value.as_array().is_some_and(Vec::is_empty)
            && matches!(ctx.draft, referencing::Draft::Draft4)
        {
            continue;
        }
        if *keyword == "required" && value.as_array().is_some_and(Vec::is_empty) {
            evaluations.push((
                keyword_priority(keyword),
                compile_leaf(ctx, keyword, &CompiledExpr::always_true()),
            ));
            continue;
        }
        if *keyword == "required"
            && matches!(schema.get("additionalProperties"), Some(Value::Bool(false)))
            && value.as_array().is_some_and(|items| items.len() == 1)
        {
            continue;
        }
        if *keyword == "required"
            && fused_required_properties(schema).is_some()
            && !matches!(
                schema.get("additionalProperties"),
                Some(Value::Bool(false) | Value::Object(_))
            )
        {
            continue;
        }
        let mut keyword_schema = Map::new();
        keyword_schema.insert((*keyword).to_owned(), value.clone());
        if matches!(ctx.draft, referencing::Draft::Draft4) {
            let modifier = match *keyword {
                "minimum" => "exclusiveMinimum",
                "maximum" => "exclusiveMaximum",
                _ => "",
            };
            if let Some(value) = schema.get(modifier) {
                keyword_schema.insert(modifier.to_owned(), value.clone());
            }
        }
        let compiled = compile_cached_keyword_expr(ctx, keyword, &keyword_schema);
        if !compiled.is_trivially_true() {
            if *keyword == "type" && !matches!(ctx.draft, referencing::Draft::Draft4) {
                if let Value::Array(types) = value {
                    if types.len() == 1 {
                        evaluations.push((
                            keyword_priority(keyword),
                            compile_leaf(ctx, keyword, &compiled),
                        ));
                        continue;
                    }
                    let mut expected_types =
                        types.iter().filter_map(Value::as_str).collect::<Vec<_>>();
                    expected_types.sort_by_key(|item_type| match *item_type {
                        "null" => 0,
                        "boolean" => 1,
                        "integer" => 2,
                        "number" => 3,
                        "string" => 4,
                        "array" => 5,
                        "object" => 6,
                        _ => 7,
                    });
                    let expected = expected_types.join(", ");
                    let is_valid = compiled.is_valid_token_stream();
                    let type_path = ctx.schema_path_for_keyword("type");
                    let type_sp = format_ident!("{}", ctx.intern_schema_path(&type_path));
                    evaluations.push((keyword_priority(keyword), quote! {
                        {
                            let __errors = if #is_valid {
                                Vec::new()
                            } else {
                                vec![("type", format!("{} is not of types ({})", instance, #expected))]
                            };
                            __children.push(__eval::node_with_descriptions_at(
                                __eval_path,
                                &#type_sp,
                                __il.clone(),
                                __errors,
                                None,
                                Vec::new(),
                            ));
                        }
                    }));
                    continue;
                }
            }
            evaluations.push((
                keyword_priority(keyword),
                compile_leaf(ctx, keyword, &compiled),
            ));
        }
    }
    if matches!(
        ctx.draft,
        referencing::Draft::Draft4 | referencing::Draft::Draft6 | referencing::Draft::Draft7
    ) {
        for keyword in ["format", "contentEncoding", "contentMediaType"] {
            let Some(value) = schema.get(keyword) else {
                continue;
            };
            if keyword == "contentEncoding" && schema.contains_key("contentMediaType") {
                continue;
            }
            let mut keyword_schema = Map::new();
            keyword_schema.insert(keyword.to_owned(), value.clone());
            if keyword == "contentMediaType" {
                if let Some(content_encoding) = schema.get("contentEncoding") {
                    keyword_schema.insert("contentEncoding".to_owned(), content_encoding.clone());
                }
            }
            let compiled = compile_cached_keyword_expr(ctx, keyword, &keyword_schema);
            if compiled.is_trivially_true() {
                if keyword == "format" {
                    if let Value::String(value) = value {
                        if format::is_known(ctx, value) {
                            evaluations.push((
                                keyword_priority(keyword),
                                compile_annotation_keyword(ctx, keyword, value),
                            ));
                        }
                    }
                }
            } else if keyword == "format" {
                let serialized = serde_json::to_string(value).expect("annotation serializes");
                let leaf = compile_leaf_with_annotation(ctx, keyword, &compiled, &serialized);
                evaluations.push((keyword_priority(keyword), leaf));
            } else {
                evaluations.push((
                    keyword_priority(keyword),
                    compile_leaf(ctx, keyword, &compiled),
                ));
            }
        }
    }
    evaluations
}

fn compile_leaf(
    ctx: &mut CompileContext<'_>,
    keyword: &str,
    compiled: &CompiledExpr,
) -> TokenStream {
    let path = ctx.schema_path_for_keyword(keyword);
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    let collect = compiled.collect.as_token_stream();
    quote! {
        {
            let __keyword_schema_path = #path;
            let mut __collected_errors = Vec::new();
            let __errors = &mut __collected_errors;
            #collect
            __children.push(__eval::node_at(__eval_path,
                &#sp,
                __il.clone(),
                __collected_errors,
                None,
                Vec::new(),
            ));
        }
    }
}

fn compile_leaf_with_annotation(
    ctx: &mut CompileContext<'_>,
    keyword: &str,
    compiled: &CompiledExpr,
    serialized: &str,
) -> TokenStream {
    let path = ctx.schema_path_for_keyword(keyword);
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    let collect = compiled.collect.as_token_stream();
    let annotation = cached_annotation(serialized);
    quote! {
        {
            let __keyword_schema_path = #path;
            let mut __collected_errors = Vec::new();
            let __errors = &mut __collected_errors;
            #collect
            let __annotations = instance.is_string().then(|| #annotation);
            __children.push(__eval::node_at(__eval_path,
                &#sp,
                __il.clone(),
                __collected_errors,
                __annotations,
                Vec::new(),
            ));
        }
    }
}

fn fused_required_properties(schema: &Map<String, Value>) -> Option<(&str, &str)> {
    if schema.contains_key("patternProperties")
        || schema
            .get("properties")
            .and_then(Value::as_object)
            .is_none_or(|properties| properties.len() >= 15)
    {
        return None;
    }
    let required = schema.get("required")?.as_array()?;
    if required.len() != 2 {
        return None;
    }
    Some((required[0].as_str()?, required[1].as_str()?))
}

fn compile_properties(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
    properties: &Map<String, Value>,
) -> TokenStream {
    let capacity = properties.len();
    let mut property_evaluations = Vec::with_capacity(properties.len());
    for (name, subschema) in properties {
        let mut child_path = String::new();
        let child = ctx.with_schema_path_segment("properties", |ctx| {
            ctx.with_schema_path_segment(name, |ctx| {
                ctx.current_schema_path().clone_into(&mut child_path);
                ctx.with_instance_scope(|ctx| compile_schema_node(ctx, subschema))
            })
        });
        let mut name_suffix = String::with_capacity(name.len() + 1);
        name_suffix.push('/');
        write_escaped_str(&mut name_suffix, name);
        property_evaluations.push(quote! {
            if let Some(__property_instance) = __object.get(#name) {
                __matched_properties.push(#name);
                let __child_path = __path.push(#name);
                let __path = &__child_path;
                let __child_il = __eval::location_join_raw(
                    __il,
                    #name_suffix,
                );
                let __il = &__child_il;
                let __schema_path = #child_path;
                let instance = __property_instance;
                __keyword_children.push(#child);
            }
        });
    }
    let required_check = fused_required_properties(schema).map(|(first, second)| {
        quote! {
            if !__object.contains_key(#first) || !__object.contains_key(#second) {
                __fused_required_valid = false;
            }
        }
    });
    let path = ctx.schema_path_for_keyword("properties");
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    quote! {
        {
            let mut __keyword_children = Vec::with_capacity(#capacity);
            let mut __errors = Vec::new();
            let mut __annotations = None;
            let mut __fused_required_valid = true;
            if let __eval::Value::Object(__object) = instance {
                #required_check
                if __fused_required_valid {
                    let mut __matched_properties: Vec<&str> = Vec::new();
                    #(#property_evaluations)*
                    __annotations = Some(__eval::dynamic_annotation(
                        __eval::Value::from(__matched_properties),
                    ));
                }
            }
            if __fused_required_valid {
                __children.push(__eval::node_at(__eval_path,
                    &#sp,
                    __il.clone(),
                    __errors,
                    __annotations,
                    __keyword_children,
                ));
            } else {
                __children.push(__eval::invalid_node_at(
                    __eval_path,
                    &#sp,
                    __il.clone(),
                ));
            }
        }
    }
}

fn compile_pattern_properties(
    ctx: &mut CompileContext<'_>,
    patterns: &Map<String, Value>,
) -> TokenStream {
    let mut pattern_evaluations = Vec::with_capacity(patterns.len());
    for (pattern, subschema) in patterns {
        let Ok(condition) = key_match_expr(ctx, pattern) else {
            continue;
        };
        let mut child_path = String::new();
        let child = ctx.with_schema_path_segment("patternProperties", |ctx| {
            ctx.with_schema_path_segment(pattern, |ctx| {
                ctx.current_schema_path().clone_into(&mut child_path);
                ctx.with_instance_scope(|ctx| compile_schema_node(ctx, subschema))
            })
        });
        pattern_evaluations.push(quote! {
            for (__property, __property_instance) in __object {
                let key = __property;
                if #condition {
                    __matched_properties.push(__property.clone());
                    let __child_path = __path.push(__property.as_str());
                    let __path = &__child_path;
                    let __child_il = __il.join(__property.as_str());
                    let __il = &__child_il;
                    let __schema_path = #child_path;
                    let instance = __property_instance;
                    __keyword_children.push(#child);
                }
            }
        });
    }
    let path = ctx.schema_path_for_keyword("patternProperties");
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    quote! {
        {
            let mut __keyword_children = Vec::new();
            let mut __annotations = None;
            if let __eval::Value::Object(__object) = instance {
                let mut __matched_properties: Vec<String> = Vec::new();
                #(#pattern_evaluations)*
                __annotations = Some(__eval::dynamic_annotation(
                    __eval::Value::from(__matched_properties),
                ));
            }
            __children.push(__eval::node_at(__eval_path,
                &#sp,
                __il.clone(),
                Vec::new(),
                __annotations,
                __keyword_children,
            ));
        }
    }
}

fn compile_additional_properties(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> TokenStream {
    let properties = schema.get("properties").and_then(Value::as_object);
    let patterns = schema.get("patternProperties").and_then(Value::as_object);
    let additional = schema
        .get("additionalProperties")
        .expect("checked by caller");
    let required = schema.get("required").and_then(Value::as_array);
    let fuse_required =
        matches!(additional, Value::Bool(false)) && required.is_some_and(|items| items.len() == 1);
    let required_checks: Vec<TokenStream> = if fuse_required {
        required
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|required| {
                quote! {
                    if !__object.contains_key(#required) {
                        let __required_schema_path = __eval::join_schema_path(
                            __schema_path,
                            "required",
                        );
                        __errors.push(__err::required(
                            &__required_schema_path,
                            __il.clone(),
                            instance,
                            #required,
                        ));
                    }
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    let mut property_arms = Vec::new();
    if let Some(properties) = properties {
        for (name, subschema) in properties {
            let mut child_path = String::new();
            let child = ctx.with_schema_path_segment("properties", |ctx| {
                ctx.with_schema_path_segment(name, |ctx| {
                    ctx.current_schema_path().clone_into(&mut child_path);
                    ctx.with_instance_scope(|ctx| compile_schema_node(ctx, subschema))
                })
            });
            property_arms.push(quote! {
                #name => {
                    __covered = true;
                    let __schema_path = #child_path;
                    let instance = __property_instance;
                    __keyword_children.push(#child);
                }
            });
        }
    }

    let mut pattern_evaluations = Vec::new();
    let annotate_patterns = patterns.is_some() && properties.is_none();
    if let Some(patterns) = patterns {
        for (pattern, subschema) in patterns {
            let Ok(condition) = key_match_expr(ctx, pattern) else {
                continue;
            };
            let record_match =
                annotate_patterns.then(|| quote! { __pattern_matches.push(__property.clone()); });
            let mut child_path = String::new();
            let child = ctx.with_schema_path_segment("patternProperties", |ctx| {
                ctx.with_schema_path_segment(pattern, |ctx| {
                    ctx.current_schema_path().clone_into(&mut child_path);
                    ctx.with_instance_scope(|ctx| compile_schema_node(ctx, subschema))
                })
            });
            pattern_evaluations.push(quote! {
                if #condition {
                    __covered = true;
                    #record_match
                    let __schema_path = #child_path;
                    let instance = __property_instance;
                    __keyword_children.push(#child);
                }
            });
        }
    }

    let fallback = match additional {
        Value::Bool(false) => quote! {
            if !__covered {
                __unexpected.push(__property.clone());
            }
        },
        subschema => {
            let child = ctx.with_schema_path_segment("additionalProperties", |ctx| {
                ctx.with_instance_scope(|ctx| compile_schema_node(ctx, subschema))
            });
            quote! {
                if !__covered {
                    __additional_matches.push(__property.clone());
                    let __schema_path = &__keyword_schema_path;
                    let instance = __property_instance;
                    __keyword_children.push(#child);
                }
            }
        }
    };
    let finish_errors = matches!(additional, Value::Bool(false)).then(|| {
        quote! {
            if !__unexpected.is_empty() {
                __errors.push(__err::additional_properties(
                    &__keyword_schema_path,
                    __il.clone(),
                    instance,
                    __unexpected,
                ));
            }
        }
    });
    let annotate_empty_additional = patterns.is_some() && properties.is_some();
    let finish_annotations = matches!(additional, Value::Object(_)).then(|| {
        let assign = quote! {
            __annotations = Some(__eval::dynamic_annotation(
                __eval::Value::from(__additional_matches),
            ));
        };
        if annotate_empty_additional {
            assign
        } else {
            quote! {
                if !__additional_matches.is_empty() {
                    #assign
                }
            }
        }
    });
    let pattern_path = ctx.schema_path_for_keyword("patternProperties");
    let pattern_sp = format_ident!("{}", ctx.intern_schema_path(&pattern_path));
    let pattern_annotation = annotate_patterns.then(|| {
        quote! {
            if !__pattern_matches.is_empty() {
                __keyword_children.push(__eval::node_at(__eval_path,
                    &#pattern_sp,
                    __il.clone(),
                    Vec::new(),
                    Some(__eval::dynamic_annotation(
                        __eval::Value::from(__pattern_matches),
                    )),
                    Vec::new(),
                ));
            }
        }
    });
    let path = ctx.schema_path_for_keyword("additionalProperties");
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    quote! {
        {
            let __keyword_schema_path = #path;
            let mut __keyword_children = Vec::new();
            let mut __errors = Vec::new();
            let mut __annotations = None;
            if let __eval::Value::Object(__object) = instance {
                let mut __unexpected: Vec<String> = Vec::new();
                let mut __additional_matches: Vec<String> = Vec::new();
                let mut __pattern_matches: Vec<String> = Vec::new();
                #(#required_checks)*
                for (__property, __property_instance) in __object {
                    let key = __property;
                    let mut __covered = false;
                    let __child_path = __path.push(__property.as_str());
                    let __path = &__child_path;
                    let __child_il = __il.join(__property.as_str());
                    let __il = &__child_il;
                    match __property.as_str() {
                        #(#property_arms,)*
                        _ => {}
                    }
                    #(#pattern_evaluations)*
                    #fallback
                }
                #pattern_annotation
                #finish_errors
                #finish_annotations
            }
            __children.push(__eval::node_at(__eval_path,
                &#sp,
                __il.clone(),
                __errors,
                __annotations,
                __keyword_children,
            ));
        }
    }
}

fn compile_property_names(ctx: &mut CompileContext<'_>, subschema: &Value) -> TokenStream {
    let path = ctx.schema_path_for_keyword("propertyNames");
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    if subschema.as_bool() == Some(false) {
        return quote! {
            {
                let __keyword_schema_path = __eval::join_schema_path(
                    __schema_path,
                    "propertyNames",
                );
                let __errors = if instance.as_object().is_some_and(|object| !object.is_empty()) {
                    vec![__err::false_schema(
                        &__keyword_schema_path,
                        __il.clone(),
                        instance,
                    )]
                } else {
                    Vec::new()
                };
                __children.push(__eval::node_at(
                    __eval_path,
                    &#sp,
                    __il.clone(),
                    __errors,
                    None,
                    Vec::new(),
                ));
            }
        };
    }
    let child =
        ctx.with_schema_path_segment("propertyNames", |ctx| compile_schema_node(ctx, subschema));
    quote! {
        {
            let __keyword_schema_path = __eval::join_schema_path(
                __schema_path,
                "propertyNames",
            );
            let mut __keyword_children = Vec::new();
            if let __eval::Value::Object(__object) = instance {
                __keyword_children.reserve_exact(__object.len());
                for __property in __object.keys() {
                    let __property_instance = __eval::Value::String(
                        __property.clone(),
                    );
                    let instance = &__property_instance;
                    let __schema_path = &__keyword_schema_path;
                    __keyword_children.push(#child);
                }
            }
            __children.push(__eval::node_at(
                __eval_path,
                &#sp,
                __il.clone(),
                Vec::new(),
                None,
                __keyword_children,
            ));
        }
    }
}

fn compile_items(
    ctx: &mut CompileContext<'_>,
    parent: &Map<String, Value>,
    items: &Value,
) -> Vec<TokenStream> {
    match items {
        Value::Array(schemas) if !ctx.draft.supports_prefix_items_keyword() => {
            vec![compile_tuple_items(ctx, "items", schemas, false, false)]
        }
        Value::Bool(_) | Value::Object(_) => {
            let skip = parent
                .get("prefixItems")
                .and_then(Value::as_array)
                .filter(|_| ctx.draft.supports_prefix_items_keyword())
                .map_or(0, Vec::len);
            vec![compile_schema_items(ctx, "items", items, skip)]
        }
        _ => Vec::new(),
    }
}

fn compile_schema_items(
    ctx: &mut CompileContext<'_>,
    keyword: &str,
    subschema: &Value,
    skip: usize,
) -> TokenStream {
    if skip == 0 {
        if let Some(item_type) = subschema.as_object().and_then(|schema| {
            (schema.len() == 1)
                .then(|| schema.get("type").and_then(Value::as_str))
                .flatten()
                .filter(|item_type| {
                    matches!(*item_type, "number" | "string" | "integer" | "boolean")
                })
        }) {
            let compiled = ctx.with_schema_path_segment(keyword, |ctx| {
                ctx.with_instance_scope(|ctx| compile_schema(ctx, subschema))
            });
            let is_valid = compiled.is_valid_token_stream();
            let path = ctx.schema_path_for_keyword(keyword);
            let sp = format_ident!("{}", ctx.intern_schema_path(&path));
            return quote! {
                {
                    let mut __errors = Vec::new();
                    let mut __annotations = None;
                    if let __eval::Value::Array(__items) = instance {
                        for (__index, __item) in __items.iter().enumerate() {
                            let instance = __item;
                            if !(#is_valid) {
                                __errors.push((
                                    "type",
                                    format!(
                                        r#"{} at index {} is not of type "{}""#,
                                        __item,
                                        __index,
                                        #item_type,
                                    ),
                                ));
                            }
                        }
                        __annotations = Some(__eval::dynamic_annotation(
                            __eval::Value::Bool(!__items.is_empty()),
                        ));
                    }
                    __children.push(__eval::node_with_descriptions_at(__eval_path,
                        &#sp,
                        __il.clone(),
                        __errors,
                        __annotations,
                        Vec::new(),
                    ));
                }
            };
        }
    }
    let child = ctx.with_schema_path_segment(keyword, |ctx| {
        ctx.with_instance_scope(|ctx| compile_schema_node(ctx, subschema))
    });
    let path = ctx.schema_path_for_keyword(keyword);
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    quote! {
        {
            let __keyword_schema_path = #path;
            let mut __keyword_children = Vec::new();
            let mut __annotations = None;
            if let __eval::Value::Array(__items) = instance {
                __keyword_children.reserve_exact(__items.len().saturating_sub(#skip));
                for (__index, __item) in __items.iter().enumerate().skip(#skip) {
                    let __child_path = __path.push(__index);
                    let __path = &__child_path;
                    let __child_il = __il.join(__index);
                    let __il = &__child_il;
                    let __schema_path = __keyword_schema_path;
                    let instance = __item;
                    __keyword_children.push(#child);
                }
                __annotations = Some(__eval::dynamic_annotation(
                    __eval::Value::Bool(__items.len() > #skip),
                ));
            }
            __children.push(__eval::node_at(__eval_path,
                &#sp,
                __il.clone(),
                Vec::new(),
                __annotations,
                __keyword_children,
            ));
        }
    }
}

fn compile_tuple_items(
    ctx: &mut CompileContext<'_>,
    keyword: &str,
    schemas: &[Value],
    annotations: bool,
    collapse_single_validators: bool,
) -> TokenStream {
    let capacity = schemas.len();
    let mut item_evaluations = Vec::with_capacity(schemas.len());
    for (index, subschema) in schemas.iter().enumerate() {
        let mut child_path = String::new();
        let child = ctx.with_schema_path_segment(keyword, |ctx| {
            ctx.with_schema_path_segment(&index.to_string(), |ctx| {
                ctx.current_schema_path().clone_into(&mut child_path);
                ctx.with_instance_scope(|ctx| compile_schema_node(ctx, subschema))
            })
        });
        // In the `prefixItems` + `unevaluatedItems` + `$dynamicRef`/`$recursiveRef` combination the
        // runtime keeps a valid single-keyword element as a leaf node (no keyword child) and only
        // materializes the child level on failure.
        let collapse = collapse_single_validators
            && subschema
                .as_object()
                .is_some_and(|schema| schema.len() == 1);
        let push = if collapse {
            let child_sp = format_ident!("{}", ctx.intern_schema_path(&child_path));
            quote! {
                let __node = #child;
                __keyword_children.push(if __node.is_valid() {
                    __eval::node_at(
                        __eval_path,
                        &#child_sp,
                        __il.clone(),
                        Vec::new(),
                        None,
                        Vec::new(),
                    )
                } else {
                    __node
                });
            }
        } else {
            quote! { __keyword_children.push(#child); }
        };
        item_evaluations.push(quote! {
            if let Some(__item) = __items.get(#index) {
                let __child_path = __path.push(#index);
                let __path = &__child_path;
                let __child_il = __il.join(#index);
                let __il = &__child_il;
                let __schema_path = #child_path;
                let instance = __item;
                #push
            }
        });
    }
    let annotation = annotations.then(|| {
        quote! {
            if !__items.is_empty() {
                __annotations = Some(__eval::dynamic_annotation(
                    if __keyword_children.len() == __items.len() {
                        __eval::Value::Bool(true)
                    } else {
                        __eval::Value::from(
                            __keyword_children.len().saturating_sub(1),
                        )
                    },
                ));
            }
        }
    });
    let path = ctx.schema_path_for_keyword(keyword);
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    quote! {
        {
            let mut __keyword_children = Vec::with_capacity(#capacity);
            let mut __annotations = None;
            if let __eval::Value::Array(__items) = instance {
                #(#item_evaluations)*
                #annotation
            }
            __children.push(__eval::node_at(__eval_path,
                &#sp,
                __il.clone(),
                Vec::new(),
                __annotations,
                __keyword_children,
            ));
        }
    }
}

fn compile_prefix_items(
    ctx: &mut CompileContext<'_>,
    parent: &Map<String, Value>,
    schemas: &[Value],
) -> TokenStream {
    let collapse = parent.contains_key("unevaluatedItems")
        && (parent.contains_key("$dynamicRef") || parent.contains_key("$recursiveRef"));
    compile_tuple_items(ctx, "prefixItems", schemas, true, collapse)
}

fn compile_additional_items(
    ctx: &mut CompileContext<'_>,
    subschema: &Value,
    item_count: usize,
) -> TokenStream {
    let keyword = "additionalItems";
    if matches!(subschema, Value::Bool(true)) {
        return quote! {};
    }
    let collect = if matches!(subschema, Value::Bool(false)) {
        quote! {
            if __items.len() > #item_count {
                __errors.push(__err::additional_items(
                    &__keyword_schema_path,
                    __il.clone(),
                    instance,
                    #item_count,
                ));
            }
        }
    } else {
        let compiled =
            ctx.with_schema_path_swap(String::new(), |ctx| compile_schema(ctx, subschema));
        let collect = compiled.collect.as_token_stream();
        quote! {
            for (__index, __item) in __items.iter().enumerate().skip(#item_count) {
                let instance = __item;
                let __child_path = __path.push(__index);
                let __path = &__child_path;
                #collect
            }
        }
    };
    let path = ctx.schema_path_for_keyword(keyword);
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    quote! {
        {
            let __keyword_schema_path = __eval::join_schema_path(
                __schema_path,
                #keyword,
            );
            let mut __errors = Vec::new();
            if let __eval::Value::Array(__items) = instance {
                #collect
            }
            __children.push(__eval::node_at(
                __eval_path,
                &#sp,
                __il.clone(),
                __errors,
                None,
                Vec::new(),
            ));
        }
    }
}

fn compile_dependencies(
    ctx: &mut CompileContext<'_>,
    keyword: &str,
    dependencies: &Map<String, Value>,
) -> TokenStream {
    let keyword_path = ctx.schema_path_for_keyword(keyword);
    let sp_kw = format_ident!("{}", ctx.intern_schema_path(&keyword_path));
    let required_path = format!("{keyword_path}/0");
    let sp_req = format_ident!("{}", ctx.intern_schema_path(&required_path));
    let mut dependency_evaluations = Vec::new();
    for (property, subschema) in dependencies {
        if let Value::Array(required_properties) = subschema {
            let required_evaluations = required_properties.iter().filter_map(|required_property| {
                let required_property = required_property.as_str()?;
                Some(quote! {
                    if !__object.contains_key(#required_property) {
                        __required_errors.push(__err::required(
                            &__required_schema_path,
                            __il.clone(),
                            instance,
                            #required_property,
                        ));
                    }
                })
            });
            dependency_evaluations.push(quote! {
                if __object.contains_key(#property) {
                    let __required_schema_path = __eval::join_schema_path(
                        &__keyword_schema_path,
                        "0",
                    );
                    let mut __required_errors = Vec::new();
                    #(#required_evaluations)*
                    let __dependency_children = vec![__eval::node_at(
                        __eval_path,
                        &#sp_req,
                        __il.clone(),
                        __required_errors,
                        None,
                        Vec::new(),
                    )];
                    __keyword_children.push(__eval::node_at(
                        __eval_path,
                        &#sp_kw,
                        __il.clone(),
                        Vec::new(),
                        None,
                        __dependency_children,
                    ));
                }
            });
            continue;
        }
        if !matches!(subschema, Value::Bool(_) | Value::Object(_)) {
            continue;
        }
        let escaped_property = escaped_segment(property);
        let child = ctx.with_schema_path_segment(keyword, |ctx| {
            ctx.with_schema_path_segment(property, |ctx| compile_schema_node(ctx, subschema))
        });
        dependency_evaluations.push(quote! {
            if __object.contains_key(#property) {
                let __child_schema_path = __eval::join_schema_path(
                    &__keyword_schema_path,
                    #escaped_property,
                );
                let __schema_path = &__child_schema_path;
                __keyword_children.push(#child);
            }
        });
    }
    let capacity = dependency_evaluations.len();
    quote! {
        {
            let __keyword_schema_path = __eval::join_schema_path(
                __schema_path,
                #keyword,
            );
            let mut __keyword_children = Vec::with_capacity(#capacity);
            if let __eval::Value::Object(__object) = instance {
                #(#dependency_evaluations)*
            }
            __children.push(__eval::node_at(
                __eval_path,
                &#sp_kw,
                __il.clone(),
                Vec::new(),
                None,
                __keyword_children,
            ));
        }
    }
}

fn compile_contains(ctx: &mut CompileContext<'_>, subschema: &Value) -> TokenStream {
    let child = ctx.with_schema_path_segment("contains", |ctx| {
        ctx.with_instance_scope(|ctx| compile_schema_node(ctx, subschema))
    });
    let path = ctx.schema_path_for_keyword("contains");
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    quote! {
        {
            let __keyword_schema_path = __eval::join_schema_path(
                __schema_path,
                "contains",
            );
            let mut __keyword_children = Vec::new();
            let mut __errors = Vec::new();
            let mut __annotations = None;
            if let __eval::Value::Array(__items) = instance {
                let mut __matching_indices = Vec::new();
                for (__index, __item) in __items.iter().enumerate() {
                    let __child_path = __path.push(__index);
                    let __path = &__child_path;
                    let __child_il = __il.join(__index);
                    let __il = &__child_il;
                    let __schema_path = &__keyword_schema_path;
                    let instance = __item;
                    let __result = #child;
                    if __result.is_valid() {
                        __matching_indices.push(__index);
                        __keyword_children.push(__result);
                    }
                }
                if __matching_indices.is_empty() {
                    __errors.push(__err::contains(
                        &__keyword_schema_path,
                        __il.clone(),
                        instance,
                    ));
                } else {
                    __annotations = Some(__eval::dynamic_annotation(
                        __eval::Value::from(__matching_indices),
                    ));
                }
            } else {
                __annotations = Some(__eval::dynamic_annotation(
                    __eval::Value::Array(Vec::new()),
                ));
            }
            __children.push(__eval::node_at(
                __eval_path,
                &#sp,
                __il.clone(),
                __errors,
                __annotations,
                __keyword_children,
            ));
        }
    }
}

fn compile_bounded_contains(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
    subschema: &Value,
) -> TokenStream {
    let compiled = ctx.with_schema_path_segment("contains", |ctx| {
        ctx.with_instance_scope(|ctx| compile_schema(ctx, subschema))
    });
    let is_valid = compiled.is_valid_token_stream();
    let minimum = schema
        .get("minContains")
        .and_then(|value| value_as_u64(ctx.draft, value))
        .unwrap_or(1);
    let maximum_check = schema
        .get("maxContains")
        .and_then(|value| value_as_u64(ctx.draft, value))
        .map_or_else(
            || quote! { false },
            |maximum| quote! { __matches > #maximum },
        );
    let path = ctx.schema_path_for_keyword("contains");
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    quote! {
        {
            let __keyword_schema_path = __eval::join_schema_path(
                __schema_path,
                "contains",
            );
            let mut __errors = Vec::new();
            if let __eval::Value::Array(__items) = instance {
                let mut __matches = 0u64;
                for __item in __items {
                    let instance = __item;
                    if #is_valid {
                        __matches += 1;
                    }
                }
                if __matches < #minimum || #maximum_check {
                    __errors.push(__err::contains(
                        &__keyword_schema_path,
                        __il.clone(),
                        instance,
                    ));
                }
            }
            __children.push(__eval::node_at(
                __eval_path,
                &#sp,
                __il.clone(),
                __errors,
                None,
                Vec::new(),
            ));
        }
    }
}

fn compile_unevaluated_properties(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
    subschema: &Value,
) -> TokenStream {
    let evaluated = compile_key_evaluated_expr(ctx, schema, false);
    let child = ctx.with_schema_path_segment("unevaluatedProperties", |ctx| {
        ctx.with_instance_scope(|ctx| compile_schema_node(ctx, subschema))
    });
    let path = ctx.schema_path_for_keyword("unevaluatedProperties");
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    quote! {
        {
            let __keyword_schema_path = __eval::join_schema_path(
                __schema_path,
                "unevaluatedProperties",
            );
            let mut __keyword_children = Vec::new();
            let mut __errors = Vec::new();
            if let __eval::Value::Object(obj) = instance {
                let mut __unevaluated = Vec::new();
                for (key, __property_instance) in obj {
                    let key_str = key.as_str();
                    if !(#evaluated) {
                        let __child_path = __path.push(key_str);
                        let __path = &__child_path;
                        let __child_il = __il.join(key_str);
                        let __il = &__child_il;
                        let __schema_path = &__keyword_schema_path;
                        let instance = __property_instance;
                        let __result = #child;
                        if !__result.is_valid() {
                            __unevaluated.push(key.clone());
                        }
                        __keyword_children.push(__result);
                    }
                }
                if !__unevaluated.is_empty() {
                    __errors.push(__err::unevaluated_properties(
                        &__keyword_schema_path,
                        __il.clone(),
                        instance,
                        __unevaluated,
                    ));
                }
            }
            __children.push(__eval::node_at(
                __eval_path,
                &#sp,
                __il.clone(),
                __errors,
                None,
                __keyword_children,
            ));
        }
    }
}

fn compile_unevaluated_items(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
    subschema: &Value,
) -> TokenStream {
    let mut siblings = schema.clone();
    siblings.remove("unevaluatedItems");
    let mut hoist = GuardHoist::inline();
    let evaluated = compile_index_evaluated_expr(ctx, &siblings, &mut hoist);
    let child = ctx.with_schema_path_segment("unevaluatedItems", |ctx| {
        ctx.with_instance_scope(|ctx| compile_schema_node(ctx, subschema))
    });
    let path = ctx.schema_path_for_keyword("unevaluatedItems");
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    quote! {
        {
            let __keyword_schema_path = __eval::join_schema_path(
                __schema_path,
                "unevaluatedItems",
            );
            let mut __keyword_children = Vec::new();
            let mut __errors = Vec::new();
            if let __eval::Value::Array(__items) = instance {
                let arr = __items.as_slice();
                let mut __unevaluated = Vec::new();
                for (__index, __item) in __items.iter().enumerate() {
                    let idx = __index;
                    let item = __item;
                    if !(#evaluated) {
                        let __child_path = __path.push(__index);
                        let __path = &__child_path;
                        let __child_il = __il.join(__index);
                        let __il = &__child_il;
                        let __schema_path = &__keyword_schema_path;
                        let instance = __item;
                        let __result = #child;
                        if !__result.is_valid() {
                            __unevaluated.push(__item.to_string());
                        }
                        __keyword_children.push(__result);
                    }
                }
                if !__unevaluated.is_empty() {
                    __errors.push(__err::unevaluated_items(
                        &__keyword_schema_path,
                        __il.clone(),
                        instance,
                        __unevaluated,
                    ));
                }
            }
            __children.push(__eval::node_at(
                __eval_path,
                &#sp,
                __il.clone(),
                __errors,
                None,
                __keyword_children,
            ));
        }
    }
}

fn compile_reference(
    ctx: &mut CompileContext<'_>,
    keyword: &str,
    reference: &str,
) -> Option<TokenStream> {
    if keyword == "$ref" && reference == "#" && ctx.current_schema_path().is_empty() {
        return None;
    }
    let resolved = resolve_ref(ctx, reference).ok()?;
    let evaluation_base_uri =
        base_uri_at_pointer(ctx, reference).unwrap_or_else(|| resolved.base_uri.clone());
    let reference_location =
        referencing::uri::resolve_against(&ctx.current_base_uri.as_ref().borrow(), reference)
            .ok()
            .map_or_else(
                || resolved.location.clone(),
                |location| location.to_string(),
            );
    let dynamic_anchor = (keyword == "$dynamicRef")
        .then(|| dynamic_ref_anchor_name(reference, resolved.schema))
        .flatten();
    let recursive_anchor = keyword == "$recursiveRef"
        && resolved
            .schema
            .as_object()
            .and_then(|schema| schema.get("$recursiveAnchor"))
            .and_then(Value::as_bool)
            == Some(true);
    if dynamic_anchor.is_some() {
        ctx.uses_dynamic_ref = true;
    }
    if recursive_anchor {
        ctx.uses_recursive_ref = true;
    }
    let fallback_base = resolved.base_uri.to_string();
    let reference_base = reference_location
        .split_once('#')
        .map_or_else(|| reference_location.clone(), |(base, _)| base.to_owned());
    let external_reference = keyword == "$ref" && !reference.starts_with('#');
    let rebase_lexical_resource = (!reference.contains('#') && reference_base != fallback_base)
        .then(|| {
            quote! {
                let __target = __eval::rebase_children_from(
                    __target,
                    #fallback_base,
                    #reference_base,
                );
            }
        });
    let rebase_retrieved_resource = resolved
        .schema
        .as_object()
        .and_then(|schema| schema.get(ctx.draft.id_keyword()))
        .and_then(Value::as_str)
        .and_then(|identifier| {
            referencing::uri::resolve_against(&resolved.base_uri.as_ref().borrow(), identifier).ok()
        })
        .filter(|declared_base| declared_base.as_str() != fallback_base)
        .map(|declared_base| {
            let declared_base = declared_base.to_string();
            quote! {
                let __target = __eval::rebase_children_from(
                    __target,
                    #declared_base,
                    #fallback_base,
                );
            }
        });
    let config_has_identifier = ctx
        .config
        .schema
        .as_object()
        .and_then(|schema| schema.get(ctx.draft.id_keyword()))
        .and_then(Value::as_str)
        .is_some();
    let absolute_dynamic_target = (dynamic_anchor.is_some()
        && (config_has_identifier || !fallback_base.starts_with("json-schema:///")))
    .then(|| {
        quote! {
            let __target = __eval::absolute_children(
                __target,
                #fallback_base,
            );
        }
    });
    let recursive = ctx.evaluation_fns.is_compiling(&reference_location);
    if recursive {
        ctx.recursive_evaluation_helpers
            .insert(reference_location.clone());
    }
    if keyword == "$ref" && ctx.applicator_branch_depth > 0 && recursive {
        ctx.cyclic_evaluation_helpers
            .insert(reference_location.clone());
    }
    let helper_name = get_or_create_evaluation_fn(
        ctx,
        &reference_location,
        resolved.schema,
        evaluation_base_uri,
    );
    let helper = format_ident!("{helper_name}");
    let target_baked = ctx
        .baked_root_evaluation_helpers
        .contains(&reference_location);
    let absolute_reference_target = (external_reference && !target_baked).then(|| {
        quote! {
            let __target = __eval::absolute_children(
                __target,
                #reference_base,
            );
        }
    });
    let site_root_location = (keyword == "$ref" && target_baked).then(|| {
        let fragment = canonical_reference_path(&reference_location);
        let referrer_base = ctx.current_base_uri.as_str();
        let (schema_location, absolute) = if ctx.bakes_absolute_locations(referrer_base) {
            (
                format!("{}#{fragment}", referrer_base.trim_end_matches('#')),
                true,
            )
        } else {
            (fragment, false)
        };
        let site_sp = format_ident!(
            "{}",
            ctx.intern_node_location(String::new(), schema_location, absolute)
        );
        quote! {
            let __target =
                __eval::root_location_at(__target, &#site_sp);
        }
    });
    let reference_relative = {
        let path = ctx.schema_path_for_keyword(keyword);
        path.strip_prefix(ctx.helper_root_schema_path.as_str())
            .unwrap_or(&path)
            .to_owned()
    };
    let target = if let Some(anchor) = dynamic_anchor {
        quote! {
            if let Some(evaluate) = __JSONSCHEMA_DYNAMIC_EVALUATION_STACK.with(|stack| {
                stack
                    .borrow()
                    .iter()
                    .find(|(dynamic_anchor, _)| *dynamic_anchor == #anchor)
                    .map(|(_, evaluate)| *evaluate)
            }) {
                evaluate(instance, __path, __il, &__reference_path)
            } else {
                #helper(instance, __path, __il, &__reference_path)
            }
        }
    } else if recursive_anchor {
        quote! {
            if let Some(evaluate) = __JSONSCHEMA_RECURSIVE_EVALUATION_STACK.with(|stack| {
                let stack = stack.borrow();
                let mut selected = None;
                for (evaluate, is_anchor) in stack.iter() {
                    if *is_anchor {
                        selected = Some(*evaluate);
                        break;
                    }
                }
                selected
            }) {
                evaluate(instance, __path, __il, &__reference_path)
            } else {
                #helper(instance, __path, __il, &__reference_path)
            }
        }
    } else {
        quote! { #helper(instance, __path, __il, &__reference_path) }
    };
    Some(quote! {
        {
            let mut __reference_path =
                String::with_capacity(__eval_path.len() + #reference_relative.len());
            __reference_path.push_str(__eval_path);
            __reference_path.push_str(#reference_relative);
            let __target = #target;
            #absolute_dynamic_target
            #absolute_reference_target
            #rebase_lexical_resource
            #rebase_retrieved_resource
            #site_root_location
            __children.push(__target);
        }
    })
}

fn base_uri_at_pointer(
    ctx: &CompileContext<'_>,
    reference: &str,
) -> Option<std::sync::Arc<referencing::Uri<String>>> {
    let (resource_reference, pointer) = reference.rsplit_once("#/")?;
    let resolver = ctx
        .config
        .registry
        .resolver((*ctx.current_base_uri).clone());
    let resource = resolver.lookup(resource_reference).ok()?;
    let mut schema = resource.contents();
    let mut base_uri = resource.resolver().base_uri().clone();
    for segment in pointer.split('/') {
        if let Value::Object(object) = schema {
            let identifier = object.get(ctx.draft.id_keyword()).and_then(Value::as_str);
            if let Some(identifier) = identifier {
                let resolver = ctx.config.registry.resolver((*base_uri).clone());
                base_uri = resolver
                    .lookup(identifier)
                    .ok()?
                    .resolver()
                    .base_uri()
                    .clone();
            }
        }
        let segment = segment.replace("~1", "/").replace("~0", "~");
        schema = match schema {
            Value::Object(object) => object.get(&segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(base_uri)
}

pub(crate) fn get_or_create_evaluation_fn(
    ctx: &mut CompileContext<'_>,
    location: &str,
    schema: &Value,
    base_uri: std::sync::Arc<referencing::Uri<String>>,
) -> String {
    get_or_create_evaluation_fn_with(ctx, location, location, schema, base_uri, true, false)
}

pub(crate) fn get_or_create_dynamic_evaluation_fn(
    ctx: &mut CompileContext<'_>,
    location: &str,
    schema: &Value,
    base_uri: std::sync::Arc<referencing::Uri<String>>,
) -> String {
    let cache_key = format!("{location}::dynamic");
    get_or_create_evaluation_fn_with(ctx, &cache_key, location, schema, base_uri, false, false)
}

fn get_or_create_evaluation_fn_with(
    ctx: &mut CompileContext<'_>,
    cache_key: &str,
    location: &str,
    schema: &Value,
    base_uri: std::sync::Arc<referencing::Uri<String>>,
    absolutize_anchor: bool,
    preserve_environment: bool,
) -> String {
    if let Some(name) = ctx.evaluation_fns.get_name(cache_key) {
        return name.clone();
    }
    let name = ctx.evaluation_fns.alloc_name(cache_key);
    let helper = format_ident!("{name}");
    let canonical_path = canonical_reference_path(location);
    let has_identifier = schema
        .as_object()
        .and_then(|schema| schema.get(ctx.draft.id_keyword()))
        .and_then(Value::as_str)
        .is_some();
    let resolved_base = location
        .split_once('#')
        .map_or_else(|| location.to_owned(), |(base, _)| base.to_owned());
    let pointer_location = location
        .split_once('#')
        .is_some_and(|(_, fragment)| fragment.starts_with('/'));
    let schema_base_uri = if pointer_location || preserve_environment {
        schema
            .as_object()
            .and_then(|schema| schema.get(ctx.draft.id_keyword()))
            .and_then(Value::as_str)
            .and_then(|identifier| {
                ctx.config
                    .registry
                    .resolver((*base_uri).clone())
                    .lookup(identifier)
                    .ok()
                    .map(|resolved| resolved.resolver().base_uri().clone())
            })
            .unwrap_or_else(|| base_uri.clone())
    } else {
        base_uri.clone()
    };
    let output_base =
        if pointer_location && base_uri.as_str().trim_end_matches('#') != resolved_base {
            base_uri.to_string()
        } else {
            resolved_base.clone()
        };
    let anchor_location = location
        .split_once('#')
        .is_some_and(|(_, fragment)| !fragment.is_empty() && !fragment.starts_with('/'));
    let is_real_base = !resolved_base.starts_with("json-schema:///");
    let absolute_base = (is_real_base
        && (has_identifier
            || !location.contains('#')
            || (base_uri != ctx.config.base_uri && (!anchor_location || absolutize_anchor))))
        .then_some(output_base);
    let mut reentry_static = String::new();
    let mut compile_body = |ctx: &mut CompileContext<'_>| {
        let previous_helper_root =
            std::mem::replace(&mut ctx.helper_root_schema_path, canonical_path.clone());
        let result = ctx.with_schema_path_swap(canonical_path.clone(), |ctx| {
            reentry_static = ctx.intern_schema_path(&canonical_path);
            let baked = ctx.bakes_absolute_locations(ctx.current_base_uri.as_str());
            if baked {
                ctx.baked_root_evaluation_helpers
                    .insert(location.to_owned());
            }
            let compiled = compile_schema_node(ctx, schema);
            let compiled = if baked {
                compiled
            } else {
                quote! {
                    __eval::root_schema_location(
                        #compiled,
                        #canonical_path,
                    )
                }
            };
            let compiled = absolute_base.as_ref().filter(|_| !baked).map_or_else(
                || quote! { #compiled },
                |base| quote! { __eval::absolute_children(#compiled, #base) },
            );
            let track_recursive_scope =
                ctx.uses_recursive_ref && (!has_identifier || canonical_path.is_empty());
            let is_recursive_anchor = ctx.draft.supports_recursive_ref_keyword()
                && schema.get("$recursiveAnchor").and_then(Value::as_bool) == Some(true);
            let dynamic_bindings = if ctx.uses_dynamic_ref {
                collect_dynamic_anchor_bindings(ctx, schema_base_uri.clone())
            } else {
                Vec::new()
            };
            // `key_eval`/`item_eval` helpers are only ever called from `unevaluatedProperties`/
            // `unevaluatedItems` compilation, so skip building them when the schema document
            // (and every registry resource reachable from it) contains neither keyword.
            let compiled = if ctx.uses_unevaluated_items {
                let item_helper = format_ident!(
                    "{}",
                    get_or_create_item_eval_fn(ctx, location, schema, schema_base_uri.clone())
                );
                stack_scoped_body(
                    &ITEM_EVAL_FAMILY,
                    &item_helper,
                    is_recursive_anchor,
                    track_recursive_scope,
                    ctx.uses_dynamic_ref,
                    &dynamic_bindings,
                    compiled,
                )
            } else {
                compiled
            };
            let compiled = if ctx.uses_unevaluated_properties {
                let key_helper = format_ident!(
                    "{}",
                    get_or_create_key_eval_fn(ctx, location, schema, schema_base_uri.clone())
                );
                stack_scoped_body(
                    &KEY_EVAL_FAMILY,
                    &key_helper,
                    is_recursive_anchor,
                    track_recursive_scope,
                    ctx.uses_dynamic_ref,
                    &dynamic_bindings,
                    compiled,
                )
            } else {
                compiled
            };
            stack_scoped_body(
                &EVALUATION_FAMILY,
                &helper,
                is_recursive_anchor,
                track_recursive_scope,
                ctx.uses_dynamic_ref,
                &dynamic_bindings,
                compiled,
            )
        });
        ctx.helper_root_schema_path = previous_helper_root;
        result
    };
    ctx.evaluation_fns.begin_compiling(location);
    let body = if preserve_environment {
        compile_body(ctx)
    } else {
        ctx.with_schema_env(schema, base_uri, compile_body)
    };
    let reentry_sp = format_ident!("{reentry_static}");
    ctx.evaluation_fns.finish_compiling(location);
    let memoize = ctx.cyclic_evaluation_helpers.contains(location)
        && !ctx.draft.supports_recursive_ref_keyword()
        && !ctx.draft.supports_dynamic_ref_keyword();
    let guard = ctx.recursive_evaluation_helpers.contains(location);
    let memo_load = memoize.then(|| {
        quote! {
            let __memo_repeat = match __JSONSCHEMA_EVALUATION_CACHE
                .with(|__cache| __cache.borrow().get(&__mark).cloned())
            {
                Some(Some(__cached)) => {
                    return __eval::reference(__eval_path, __cached);
                }
                Some(None) => true,
                None => false,
            };
        }
    });
    let memo_store = memoize.then(|| {
        quote! {
            __JSONSCHEMA_EVALUATION_CACHE.with(|__cache| {
                __cache.borrow_mut().insert(
                    __mark,
                    __memo_repeat.then(|| __result.clone()),
                )
            });
        }
    });
    let guard_enter = guard.then(|| {
        quote! {
            let __mark = (
                (#helper as fn(
                    &__Value,
                    &__paths::LazyLocation,
                    &__paths::Location,
                    &str,
                ) -> __eval::Node) as usize,
                std::ptr::from_ref(instance) as usize,
            );
            #memo_load
            let __reentered = __JSONSCHEMA_EVALUATION_MARK.with(|__marks| {
                let mut __marks = __marks.borrow_mut();
                if __marks.contains(&__mark) {
                    true
                } else {
                    __marks.push(__mark);
                    false
                }
            });
            if __reentered {
                return __eval::node_at(
                    __eval_path,
                    &#reentry_sp,
                    __il.clone(),
                    Vec::new(),
                    None,
                    Vec::new(),
                );
            }
        }
    });
    let guard_exit = guard.then(|| {
        quote! {
            __JSONSCHEMA_EVALUATION_MARK.with(|__marks| __marks.borrow_mut().pop());
        }
    });
    ctx.evaluation_fns.set_body(
        &name,
        quote! {
            #guard_enter
            let __schema_path = #canonical_path;
            let __result = #body;
            #guard_exit
            #memo_store
            __result
        },
    );
    name
}

fn canonical_reference_path(location: &str) -> String {
    location
        .rsplit_once('#')
        .map_or_else(String::new, |(_, fragment)| {
            if fragment.starts_with('/') {
                percent_encoding::percent_decode_str(fragment)
                    .decode_utf8_lossy()
                    .into_owned()
            } else {
                String::new()
            }
        })
}

fn compile_branch(ctx: &mut CompileContext<'_>, keyword: &str, branches: &[Value]) -> TokenStream {
    let skip_invalid = matches!(keyword, "anyOf" | "oneOf")
        && !ctx.draft.supports_recursive_ref_keyword()
        && !ctx.draft.supports_dynamic_ref_keyword();
    let cached_gates = if skip_invalid {
        ctx.branch_gate_cache
            .get(&(branches.as_ptr() as usize))
            .filter(|cached| cached.gates.len() == branches.len())
            .cloned()
    } else {
        None
    };
    let mut branch_evaluations = Vec::with_capacity(branches.len());
    let mut branch_valids = Vec::with_capacity(branches.len());
    for (index, subschema) in branches.iter().enumerate() {
        ctx.applicator_branch_depth += 1;
        let mut child_path = String::new();
        let child = ctx.with_schema_path_segment(keyword, |ctx| {
            ctx.with_schema_path_segment(&index.to_string(), |ctx| {
                ctx.current_schema_path().clone_into(&mut child_path);
                compile_schema_node(ctx, subschema)
            })
        });
        ctx.applicator_branch_depth -= 1;
        let push = quote! {
            {
                let __schema_path = #child_path;
                __keyword_children.push(#child);
            }
        };
        if skip_invalid {
            let is_valid = if let Some(cached) = &cached_gates {
                cached.gates[index].clone()
            } else {
                ctx.with_schema_path_swap(String::new(), |ctx| compile_schema(ctx, subschema))
                    .is_valid_token_stream()
            };
            branch_valids.push(is_valid);
            branch_evaluations.push(quote! {
                if !__any_branch_valid || __branch_valid[#index] {
                    #push
                }
            });
        } else {
            branch_evaluations.push(push);
        }
    }
    let constructor = match keyword {
        "allOf" => quote! { all_of },
        "anyOf" => quote! { any_of },
        "oneOf" => quote! { one_of },
        _ => unreachable!(),
    };
    let valid_setup = skip_invalid.then(|| {
        let gate_init = cached_gates.map(|cached| cached.init);
        quote! {
            #gate_init
            let __branch_valid = [#(#branch_valids),*];
            let __any_branch_valid = __branch_valid.iter().any(|__valid| *__valid);
        }
    });
    let capacity = branches.len();
    let path = ctx.schema_path_for_keyword(keyword);
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    quote! {
        {
            let mut __keyword_children = Vec::with_capacity(#capacity);
            #valid_setup
            #(#branch_evaluations)*
            __children.push(__eval::#constructor(
                __eval_path,
                &#sp,
                __il.clone(),
                __keyword_children,
            ));
        }
    }
}

fn compile_conditional(ctx: &mut CompileContext<'_>, schema: &Map<String, Value>) -> TokenStream {
    let condition = ctx.with_schema_path_segment("if", |ctx| {
        compile_schema_node(ctx, schema.get("if").expect("checked by caller"))
    });
    let then_branch = schema.get("then").map(|subschema| {
        ctx.with_schema_path_segment("then", |ctx| compile_schema_node(ctx, subschema))
    });
    let else_branch = schema.get("else").map(|subschema| {
        ctx.with_schema_path_segment("else", |ctx| compile_schema_node(ctx, subschema))
    });
    let capacity = 1 + usize::from(then_branch.is_some() || else_branch.is_some());
    let when_valid = then_branch.map_or_else(
        || quote! { __keyword_children.push(__condition); },
        |then_branch| {
            quote! {
                __keyword_children.push(__condition);
                let __then_schema_path = __eval::join_schema_path(
                    __schema_path,
                    "then",
                );
                let __schema_path = &__then_schema_path;
                __keyword_children.push(#then_branch);
            }
        },
    );
    let when_invalid = else_branch.map_or_else(
        || quote! {},
        |else_branch| {
            quote! {
                let __else_schema_path = __eval::join_schema_path(
                    __schema_path,
                    "else",
                );
                let __schema_path = &__else_schema_path;
                __keyword_children.push(#else_branch);
            }
        },
    );
    let path = ctx.schema_path_for_keyword("if");
    let sp = format_ident!("{}", ctx.intern_schema_path(&path));
    quote! {
        {
            let mut __keyword_children = Vec::with_capacity(#capacity);
            let __keyword_schema_path = __eval::join_schema_path(
                __schema_path,
                "if",
            );
            let __condition = {
                let __schema_path = &__keyword_schema_path;
                #condition
            };
            if __condition.is_valid() {
                #when_valid
            } else {
                #when_invalid
            }
            __children.push(__eval::node_at(
                __eval_path,
                &#sp,
                __il.clone(),
                Vec::new(),
                None,
                __keyword_children,
            ));
        }
    }
}

fn escaped_segment(segment: &str) -> String {
    let mut escaped = String::new();
    write_escaped_str(&mut escaped, segment);
    escaped
}

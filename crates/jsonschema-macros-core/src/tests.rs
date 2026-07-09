use std::{collections::HashMap, path::PathBuf, sync::Arc};

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::{Draft, Registry};
use serde_json::{json, Value};
use test_case::test_case;

use crate::{codegen::generate_from_config, context::CodegenConfig};

pub(crate) fn is_valid_body(schema: Value) -> String {
    extract_is_valid_body(&schema_to_code(schema))
}

fn validate_body(schema: Value) -> String {
    extract_fn_body(&schema_to_code(schema), "pub(super) fn validate")
}

fn collect_body(schema: Value) -> String {
    extract_fn_body(&schema_to_code(schema), "pub(super) fn collect_errors")
}

fn extract_is_valid_body(code: &str) -> String {
    extract_fn_body(code, "pub(super) fn is_valid")
}

fn extract_fn_body(code: &str, signature: &str) -> String {
    let fn_start = code
        .find(signature)
        .expect("function not found in generated code");
    let after_sig = &code[fn_start..];
    let brace_pos = after_sig.find('{').expect("opening brace not found");
    let rest = &after_sig[brace_pos + 1..];
    let mut depth = 1usize;
    let mut end = 0;
    for (i, ch) in rest.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }
    rest[..end].trim().to_string()
}

pub(crate) fn schema_to_code(schema: Value) -> String {
    schema_to_code_with_options(schema, None, HashMap::new(), true)
}

pub(crate) fn schema_to_code_with_options(
    schema: Value,
    validate_formats: Option<bool>,
    custom_formats: HashMap<String, TokenStream>,
    ignore_unknown_formats: bool,
) -> String {
    schema_to_code_with_runtime_alias(
        schema,
        validate_formats,
        custom_formats,
        ignore_unknown_formats,
        None,
    )
}

pub(crate) fn test_config(schema: Value) -> CodegenConfig {
    let draft = Draft::default().detect(&schema);
    test_config_with_draft(schema, draft)
}

pub(crate) fn test_config_with_draft(schema: Value, draft: Draft) -> CodegenConfig {
    let resource = draft.create_resource(schema.clone());
    let base_uri_str = "json-schema:///test";
    let registry = Registry::new()
        .draft(draft)
        .extend([(base_uri_str, resource)])
        .and_then(referencing::RegistryBuilder::prepare)
        .expect("registry build failed");
    let base_uri = referencing::uri::from_str(base_uri_str)
        .map(Arc::new)
        .expect("valid uri");

    CodegenConfig {
        schema,
        registry,
        base_uri,
        draft,
        runtime_crate_alias: None,
        validate_formats: None,
        custom_formats: HashMap::new(),
        custom_keywords: HashMap::new(),
        content_media_types: HashMap::new(),
        content_encodings: HashMap::new(),
        ignore_unknown_formats: true,
        email_options: None,
        pattern_options: crate::context::PatternEngineConfig::default(),
    }
}

fn schema_to_code_with_draft(schema: Value, draft: Draft) -> String {
    render_config(&test_config_with_draft(schema, draft))
}

fn schema_to_code_with_runtime_alias(
    schema: Value,
    validate_formats: Option<bool>,
    custom_formats: HashMap<String, TokenStream>,
    ignore_unknown_formats: bool,
    runtime_crate_alias: Option<TokenStream>,
) -> String {
    let mut config = test_config(schema);
    config.runtime_crate_alias = runtime_crate_alias;
    config.validate_formats = validate_formats;
    config.custom_formats = custom_formats;
    config.ignore_unknown_formats = ignore_unknown_formats;
    render_config(&config)
}

fn render_config(config: &CodegenConfig) -> String {
    let name = format_ident!("Validator");
    let impl_mod_name = format_ident!("__validator_impl");
    let recompile_trigger: TokenStream = quote! {};
    let tokens = generate_from_config(config, &recompile_trigger, &name, &impl_mod_name);

    // Wrap in a struct declaration so syn can parse as a complete file
    let wrapped: TokenStream = quote! {
        struct #name;
        #tokens
    };
    let file: syn::File = syn::parse2(wrapped).expect("valid token stream");
    prettyplease::unparse(&file)
}

#[test_case(json!({}), "empty_schema" ; "empty_schema")]
#[test_case(json!({"type": "string", "minLength": 3}), "min_length" ; "min_length")]
#[test_case(json!({"type": "string", "maxLength": 10}), "max_length" ; "max_length")]
#[test_case(json!({"type": "string", "minLength": 5, "maxLength": 5}), "min_max_length_equal" ; "min_max_length_equal")]
#[test_case(json!({"type": "string", "pattern": "^[a-z]+$"}), "pattern" ; "pattern")]
#[test_case(json!({"patternProperties": {"^x-": {"type": "string"}}}), "pattern_x_prefix" ; "pattern_x_prefix")]
#[test_case(json!({"patternProperties": {"^\\$ref": {"type": "string"}}}), "pattern_dollar_ref" ; "pattern_dollar_ref")]
#[test_case(json!({"patternProperties": {"^\\$ref$": {"type": "string"}}}), "pattern_dollar_ref_exact" ; "pattern_dollar_ref_exact")]
#[test_case(json!({"patternProperties": {"^(get|put|post)$": {"type": "object"}}}), "pattern_alternation" ; "pattern_alternation")]
#[test_case(json!({"type": "string", "pattern": "^(a|b)$"}), "pattern_string_alternation" ; "pattern_string_alternation")]
#[test_case(json!({"type": "string", "pattern": "^\\S*$"}), "pattern_no_whitespace" ; "pattern_no_whitespace")]
#[test_case(json!({"type": "number", "minimum": 0}), "minimum" ; "minimum")]
#[test_case(json!({"type": "number", "maximum": 100}), "maximum" ; "maximum")]
#[test_case(json!({"type": "number", "exclusiveMinimum": 0}), "exclusive_minimum" ; "exclusive_minimum")]
#[test_case(json!({"type": "number", "exclusiveMaximum": 100}), "exclusive_maximum" ; "exclusive_maximum")]
#[test_case(json!({"type": "number", "multipleOf": 3}), "multiple_of" ; "multiple_of")]
#[test_case(json!({"type": "number", "minimum": 5, "maximum": 5}), "min_max_equal" ; "min_max_equal")]
#[test_case(json!({"type": "number", "minimum": -3, "maximum": -3}), "min_max_equal_negative" ; "min_max_equal_negative")]
#[test_case(json!({"type": "number", "minimum": 1, "maximum": 10}), "min_max_unequal" ; "min_max_unequal")]
#[cfg_attr(not(feature = "arbitrary-precision"), test_case(json!({"type": "number", "minimum": 1.5, "maximum": 1.5}), "min_max_equal_float" ; "min_max_equal_float"))]
#[cfg_attr(feature = "arbitrary-precision", test_case(json!({"type": "number", "minimum": 1.5, "maximum": 1.5}), "min_max_equal_float_arbitrary" ; "min_max_equal_float"))]
#[test_case(json!({"type": "array", "minItems": 1}), "min_items" ; "min_items")]
#[test_case(json!({"type": "array", "maxItems": 10}), "max_items" ; "max_items")]
#[test_case(json!({"type": "array", "minItems": 3, "maxItems": 3}), "min_max_items_equal" ; "min_max_items_equal")]
#[test_case(json!({"type": "array", "items": {"type": "string"}}), "items_schema" ; "items_schema")]
#[test_case(json!({"type": "array", "uniqueItems": true}), "unique_items" ; "unique_items")]
#[test_case(json!({"type": "object", "required": ["name", "age"]}), "required" ; "required")]
#[test_case(json!({"type": "object", "properties": {"name": {"type": "string"}}}), "properties" ; "properties")]
#[test_case(json!({"properties": {"a": {}}, "additionalProperties": false}), "additional_false" ; "additional_false")]
#[test_case(json!({"properties": {"a": {}}, "additionalProperties": {"type": "string"}}), "additional_schema" ; "additional_schema")]
#[test_case(json!({"patternProperties": {"^str_": {"type": "string"}}}), "pattern_properties" ; "pattern_properties")]
#[test_case(json!({"patternProperties": {"^\\S*$": {"type": "string"}}, "additionalProperties": false}), "pattern_properties_no_whitespace" ; "pattern_properties_no_whitespace")]
#[test_case(json!({"type": "object", "additionalProperties": false, "patternProperties": {"^a": {"type": "string"}, "^b": {"type": "string"}}, "propertyNames": {"pattern": "^[ab]"}}), "property_names_multi_pattern_not_subsumed" ; "property_names_multi_pattern_not_subsumed")]
#[test_case(json!({"minProperties": 2, "maxProperties": 2}), "min_max_properties_equal" ; "min_max_properties_equal")]
#[test_case(json!({"allOf": [{"type": "string"}, {"minLength": 1}]}), "all_of" ; "all_of")]
#[test_case(json!({"anyOf": [{"type": "string"}, {"type": "number"}]}), "any_of" ; "any_of")]
#[test_case(json!({"oneOf": [{"type": "string"}, {"type": "number"}]}), "one_of" ; "one_of")]
#[test_case(
        json!({"oneOf": [
            {"type": "object", "required": ["kind"], "properties": {"kind": {"const": "circle"}, "radius": {"type": "number"}}},
            {"type": "object", "required": ["kind"], "properties": {"kind": {"const": "square"}, "side": {"type": "number"}}}
        ]}),
        "one_of_discriminator_string" ; "one_of_discriminator_string"
    )]
#[test_case(
        json!({"oneOf": [
            {"type": "object", "required": ["active"], "properties": {"active": {"const": true}, "name": {"type": "string"}}},
            {"type": "object", "required": ["active"], "properties": {"active": {"const": false}, "archived_at": {"type": "string"}}}
        ]}),
        "one_of_discriminator_bool" ; "one_of_discriminator_bool"
    )]
#[test_case(
        json!({"oneOf": [
            {"type": "object", "required": ["code"], "properties": {"code": {"const": 1}, "data": {"type": "string"}}},
            {"type": "object", "required": ["code"], "properties": {"code": {"const": 2}, "payload": {"type": "number"}}}
        ]}),
        "one_of_discriminator_int" ; "one_of_discriminator_int"
    )]
#[test_case(json!({"not": {"type": "string"}}), "not" ; "not")]
#[test_case(json!({"if": {"type": "string"}, "then": {"minLength": 1}}), "if_then_else" ; "if_then_else")]
#[test_case(json!({"const": "hello"}), "const_str" ; "const_str")]
#[test_case(json!({"type": "string", "const": "hello"}), "const_str_with_type" ; "const_str_with_type")]
#[test_case(json!({"enum": ["a", "b", "c"]}), "enum_str" ; "enum_str")]
#[test_case(json!({"type": "string", "enum": ["Point", "Feature"]}), "enum_str_with_type" ; "enum_str_with_type")]
#[test_case(json!({"type": "integer", "enum": [1.5]}), "enum_integer_not_redundant" ; "enum_integer_not_redundant")]
#[test_case(json!({"type": ["string", "null"]}), "multi_type_simple" ; "multi_type_simple")]
#[test_case(json!({"type": ["integer", "object"]}), "multi_type_with_integer" ; "multi_type_with_integer")]
#[test_case(json!({"type": ["string", "integer"], "minLength": 2}), "typed_fallback_mixed_types" ; "typed_fallback_mixed_types")]
#[test_case(json!({"$ref": "#"}), "self_ref_cycle" ; "self_ref_cycle")]
#[test_case(json!({"type":"object","additionalProperties":false}), "additional_false_no_siblings" ; "additional_false_no_siblings")]
fn test_codegen_snapshot(schema: Value, snap_name: &str) {
    let description = serde_json::to_string(&schema).expect("schema serialization");
    insta::with_settings!({ description => &description }, {
        insta::assert_snapshot!(snap_name, is_valid_body(schema));
    });
}

#[test]
fn dynamic_schema_full_module_emission() {
    let schema = json!({"$defs":{"node":{"$dynamicAnchor":"node","type":["object","boolean"]},"plain":{"type":"integer"}},"type":"object","properties":{"child":{"$dynamicRef":"#node"},"other":{"$ref":"#/$defs/plain"}}});
    let description = serde_json::to_string(&schema).expect("schema serialization");
    insta::with_settings!({ description => &description }, {
        insta::assert_snapshot!("dynamic_schema_full_module", schema_to_code(schema));
    });
}

#[test_case(json!({"$schema":"http://json-schema.org/draft-07/schema#","type":"string","format":"uri"}), "uri_format_per_call_cache" ; "uri")]
#[test_case(json!({"$schema":"http://json-schema.org/draft-07/schema#","type":"string","format":"iri-reference"}), "iri_reference_format_per_call_cache" ; "iri_reference")]
fn uri_format_emits_per_call_cache(schema: Value, snap_name: &str) {
    let description = serde_json::to_string(&schema).expect("schema serialization");
    let code = schema_to_code_with_options(schema, Some(true), HashMap::new(), true);
    insta::with_settings!({ description => &description }, {
        insta::assert_snapshot!(snap_name, code);
    });
}

#[test]
fn config_parses_with_trailing_commas() {
    let input = r#"schema = "{}", resources = { "json-schema:///a" => { schema = "{}", }, }, pattern_options = { size_limit = 1, }, email_options = { required_tld = true, }"#;
    assert!(
        syn::parse_str::<crate::Config>(input).is_ok(),
        "trailing commas after the final entry should parse"
    );
}

#[test]
fn custom_format_emits_direct_function_call() {
    let mut custom_formats = HashMap::new();
    custom_formats.insert(
        "currency".to_string(),
        quote! { crate::formats::is_currency },
    );
    let schema = json!({"$schema":"http://json-schema.org/draft-07/schema#","type":"string","format":"currency"});
    let description = serde_json::to_string(&schema).expect("schema serialization");
    let code = schema_to_code_with_options(schema, Some(true), custom_formats, true);
    insta::with_settings!({ description => &description }, {
        insta::assert_snapshot!("custom_format_direct_call", code);
    });
}

#[test_case(9_007_199_254_740_993_i64, "one_of_discriminator_beyond_f64_positive" ; "positive")]
#[test_case(-9_007_199_254_740_993_i64, "one_of_discriminator_beyond_f64_negative" ; "negative")]
fn one_of_int_discriminator_beyond_f64_exact_range_not_used(tag: i64, snap_name: &str) {
    let schema = json!({"oneOf":[
        {"type":"object","required":["k"],"properties":{"k":{"const":tag}}},
        {"type":"object","required":["k"],"properties":{"k":{"const":2}}}
    ]});
    let description = serde_json::to_string(&schema).expect("schema serialization");
    insta::with_settings!({ description => &description }, {
        insta::assert_snapshot!(snap_name, is_valid_body(schema));
    });
}

#[test]
fn one_of_const_discriminator_not_used_for_draft4() {
    let schema = json!({"oneOf":[
        {"type":"object","required":["k"],"properties":{"k":{"const":"a"}}},
        {"type":"object","required":["k"],"properties":{"k":{"const":"b"}}}
    ]});
    let description = serde_json::to_string(&schema).expect("schema serialization");
    let code = extract_is_valid_body(&schema_to_code_with_draft(
        schema,
        referencing::Draft::Draft4,
    ));
    insta::with_settings!({ description => &description }, {
        insta::assert_snapshot!("one_of_const_discriminator_draft4", code);
    });
}

#[test]
fn runtime_crate_alias_is_injected_into_generated_module() {
    let schema = json!({"type":"string","minLength":1});
    let description = serde_json::to_string(&schema).expect("schema serialization");
    let code = schema_to_code_with_runtime_alias(
        schema,
        None,
        HashMap::new(),
        true,
        Some(quote! { ::js }),
    );
    insta::with_settings!({ description => &description }, {
        insta::assert_snapshot!("runtime_crate_alias", code);
    });
}

#[test]
fn duplicate_regex_literals_are_deduplicated() {
    let schema = json!({
        "type": "object",
        "propertyNames": {"pattern": "^[a-z]{2,}$"},
        "patternProperties": {"^[a-z]{2,}$": {"type": "string"}},
        "additionalProperties": false
    });
    let description = serde_json::to_string(&schema).expect("schema serialization");
    let code = schema_to_code(schema);
    insta::with_settings!({ description => &description }, {
        insta::assert_snapshot!("duplicate_regex_literals_deduplicated", code);
    });
}

#[test_case(json!(true), false, "boolean_true_root" ; "boolean_true_root")]
#[test_case(json!(false), true, "boolean_false_root" ; "boolean_false_root")]
fn boolean_root_schema_matches_dynamic(
    schema: Value,
    rejects_every_instance: bool,
    snap_name: &str,
) {
    let validator = jsonschema::validator_for(&schema).expect("boolean root schema is valid");
    assert_eq!(validator.is_valid(&json!(1)), !rejects_every_instance);
    let description = serde_json::to_string(&schema).expect("schema serialization");
    insta::with_settings!({ description => &description }, {
        insta::assert_snapshot!(snap_name, is_valid_body(schema));
    });
}

#[cfg(feature = "arbitrary-precision")]
#[test]
fn invalid_multiple_of_scientific_zero_matches_dynamic() {
    let schema: Value =
        serde_json::from_str(r#"{"multipleOf":0e-10}"#).expect("valid json schema literal");
    let description = serde_json::to_string(&schema).expect("schema serialization");
    insta::with_settings!({ description => &description }, {
        insta::assert_snapshot!("multiple_of_scientific_zero_error", schema_to_code(schema));
    });
}

#[test]
fn resource_paths_are_tracked_as_recompile_inputs() {
    let config: crate::Config = syn::parse2(quote! {
        schema = r#"{}"#,
        resources = {
            "json-schema:///ext" => { path = "../benchmark/data/recursive_schema.json" },
        }
    })
    .expect("Config should parse");
    let item: syn::ItemStruct = syn::parse2(quote! {
        struct ResourcePathValidator;
    })
    .expect("Item should parse");

    let tokens = crate::validator_impl(&config, &item).expect("Macro should expand");
    let rendered = tokens.to_string();
    let resource_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../benchmark/data/recursive_schema.json")
        .to_string_lossy()
        .to_string();

    assert!(
        rendered.contains("include_str"),
        "generated code should include include_str trigger for resource paths, got:\n{rendered}"
    );
    let resource_path_literal = format!("{resource_path:?}");
    assert!(
            rendered.contains(&resource_path_literal),
            "generated code should include resolved resource path `{resource_path_literal}`, got:\n{rendered}"
        );
}

#[test]
fn emission_is_deterministic_across_runs() {
    let schema = json!({
        "type": "object",
        "$defs": {
            "a": {"type": "string", "minLength": 1},
            "b": {"type": "integer", "minimum": 0},
            "c": {"type": "boolean"},
            "d": {"type": "array", "items": {"type": "number"}}
        },
        "properties": {
            "w": {"$ref": "#/$defs/a"},
            "x": {"$ref": "#/$defs/b"},
            "y": {"$ref": "#/$defs/c"},
            "z": {"$ref": "#/$defs/d"}
        }
    });

    let first = schema_to_code(schema.clone());
    let second = schema_to_code(schema.clone());
    let third = schema_to_code(schema);

    assert_eq!(
        first, second,
        "emitted source must be byte-identical across runs (first vs second)"
    );
    assert_eq!(
        second, third,
        "emitted source must be byte-identical across runs (second vs third)"
    );
}

#[test]
fn discriminator_key_selection_is_deterministic() {
    let schema = json!({
        "oneOf": [
            {"type":"object","required":["aa","bb"],"properties":{"aa":{"const":"x"},"bb":{"const":"p"}}},
            {"type":"object","required":["aa","bb"],"properties":{"aa":{"const":"y"},"bb":{"const":"q"}}}
        ]
    });

    let first = schema_to_code(schema.clone());
    for _ in 0..23 {
        assert_eq!(
            first,
            schema_to_code(schema.clone()),
            "discriminator key choice must be identical across compilations"
        );
    }
}

fn nested_any_of(depth: usize) -> Value {
    let mut schema =
        json!({"type":"object","properties":{"leaf":{"type":"string"}},"required":["leaf"]});
    for i in 0..depth {
        let key = format!("p{i}");
        schema = json!({
            "anyOf": [schema, {"type":"object","properties":{key: {"type":"integer"}}}]
        });
    }
    json!({"type":"object","allOf":[schema],"unevaluatedProperties":false})
}

#[test]
fn unevaluated_guards_do_not_reinline_subschemas() {
    let shallow = schema_to_code(nested_any_of(4)).len();
    let deep = schema_to_code(nested_any_of(8)).len();
    // Doubling nesting depth must not blow up emitted size superlinearly.
    assert!(
        deep < shallow * 2,
        "emitted size grows superlinearly: depth4={shallow} bytes, depth8={deep} bytes"
    );
}

#[cfg(feature = "bench")]
#[test]
fn bench_helpers_emit_validator() {
    let schema = json!({"type": "integer", "minimum": 0});
    let description = serde_json::to_string(&schema).expect("schema serialization");
    let input = crate::bench::prepare(schema);
    let tokens = crate::bench::generate(&input);
    let wrapped: TokenStream = quote! { struct Validator; #tokens };
    let code = prettyplease::unparse(&syn::parse2(wrapped).expect("valid token stream"));
    insta::with_settings!({ description => &description }, {
        insta::assert_snapshot!("bench_helpers_validator", extract_is_valid_body(&code));
    });
}

#[test]
fn empty_prefix_items_generates_no_op() {
    let schema = json!({"type":"array","prefixItems":[]});
    let description = serde_json::to_string(&schema).expect("schema serialization");
    insta::with_settings!({ description => &description }, {
        insta::assert_snapshot!("empty_prefix_items", is_valid_body(schema));
    });
}

#[test_case(
        r#"{"$schema":"http://json-schema.org/draft-06/schema#","type":"array","minItems":-0.0}"#,
        "signed_zero_min_items_decimal" ; "draft6_min_items_negative_zero_decimal"
    )]
#[test_case(
        r#"{"$schema":"http://json-schema.org/draft-06/schema#","type":"array","maxItems":-0e0}"#,
        "signed_zero_max_items_scientific" ; "draft6_max_items_negative_zero_scientific"
    )]
#[test_case(
        r#"{"$schema":"http://json-schema.org/draft-06/schema#","type":"string","maxLength":-0.0}"#,
        "signed_zero_max_length_decimal" ; "draft6_max_length_negative_zero_decimal"
    )]
#[test_case(
        r#"{"$schema":"http://json-schema.org/draft-06/schema#","type":"object","minProperties":-0e0}"#,
        "signed_zero_min_properties_scientific" ; "draft6_min_properties_negative_zero_scientific"
    )]
#[test_case(
        r#"{"$schema":"http://json-schema.org/draft-06/schema#","type":"array","minItems":2.0}"#,
        "min_items_integer_valued_decimal" ; "min_items_integer_valued_decimal"
    )]
#[cfg_attr(not(feature = "arbitrary-precision"), test_case(
        r#"{"$schema":"http://json-schema.org/draft-06/schema#","type":"array","minItems":1e1}"#,
        "min_items_scientific_f64" ; "min_items_scientific"
    ))]
#[cfg_attr(not(feature = "arbitrary-precision"), test_case(
        r#"{"$schema":"http://json-schema.org/draft-06/schema#","type":"array","maxItems":1.5e2}"#,
        "max_items_scientific_fraction_f64" ; "max_items_scientific_with_fraction"
    ))]
#[cfg_attr(feature = "arbitrary-precision", test_case(
        r#"{"$schema":"http://json-schema.org/draft-06/schema#","type":"array","minItems":1e1}"#,
        "min_items_scientific_arbitrary" ; "scientific_no_fraction"
    ))]
#[cfg_attr(feature = "arbitrary-precision", test_case(
        r#"{"$schema":"http://json-schema.org/draft-06/schema#","type":"array","maxItems":1.5e2}"#,
        "max_items_scientific_fraction_arbitrary" ; "scientific_with_fraction"
    ))]
#[test_case(r#"{"type":"object","properties":{"a":{"type":"integer"}},"patternProperties":{"^x_":{"type":"string"}},"additionalProperties":{"type":"boolean"},"unevaluatedProperties":false}"#, "props_siblings" ; "props_siblings")]
#[test_case(r#"{"type":"object","allOf":[{"properties":{"a":{}}}],"unevaluatedProperties":false}"#, "props_all_of" ; "props_all_of")]
#[test_case(r#"{"type":"object","anyOf":[{"properties":{"a":{}},"required":["a"]},{"properties":{"b":{}},"required":["b"]}],"unevaluatedProperties":false}"#, "props_any_of" ; "props_any_of")]
#[test_case(r#"{"type":"object","oneOf":[{"properties":{"a":{}},"required":["a"]},{"properties":{"b":{}},"required":["b"]}],"unevaluatedProperties":false}"#, "props_one_of" ; "props_one_of")]
#[test_case(r#"{"type":"object","if":{"properties":{"kind":{"const":"x"}},"required":["kind"]},"then":{"properties":{"x_val":{}}},"else":{"properties":{"y_val":{}}},"unevaluatedProperties":false}"#, "props_if_then_else" ; "props_if_then_else")]
#[test_case(r#"{"type":"object","dependentSchemas":{"a":{"properties":{"b":{}}}},"unevaluatedProperties":false}"#, "props_dependent_schemas" ; "props_dependent_schemas")]
#[test_case(r##"{"type":"object","$ref":"#/$defs/base","$defs":{"base":{"properties":{"a":{}}}},"unevaluatedProperties":false}"##, "props_ref" ; "props_ref")]
#[test_case(r#"{"type":"object","anyOf":[{"allOf":[{"properties":{"a":{}}}],"not":{"required":["z"]}}],"unevaluatedProperties":false}"#, "props_nested_guard" ; "props_nested_guard")]
#[test_case(r#"{"type":"object","oneOf":[{"const":1},{"not":{"const":2}},{"if":{"type":"string"},"then":{}}],"unevaluatedProperties":false}"#, "props_guard_oneof_not_if" ; "props_guard_oneof_not_if")]
#[test_case(r#"{"type":"object","properties":{"a":{}},"unevaluatedProperties":{"type":"integer"}}"#, "props_schema_form" ; "props_schema_form")]
#[test_case(r#"{"type":"array","prefixItems":[{"type":"integer"}],"contains":{"type":"string"},"unevaluatedItems":false}"#, "items_prefix_contains" ; "items_prefix_contains")]
#[test_case(r#"{"type":"array","allOf":[{"prefixItems":[{"type":"integer"}]}],"unevaluatedItems":false}"#, "items_all_of" ; "items_all_of")]
#[test_case(r#"{"type":"array","anyOf":[{"prefixItems":[{}]},{"prefixItems":[{},{}]}],"unevaluatedItems":false}"#, "items_any_of" ; "items_any_of")]
#[test_case(r#"{"type":"array","oneOf":[{"prefixItems":[{"const":1}]},{"prefixItems":[{"const":2},{}]}],"unevaluatedItems":false}"#, "items_one_of" ; "items_one_of")]
#[test_case(r#"{"type":"array","if":{"prefixItems":[{"const":0}]},"then":{"prefixItems":[{},{}]},"else":{"prefixItems":[{}]},"unevaluatedItems":false}"#, "items_if_then_else" ; "items_if_then_else")]
#[test_case(r#"{"$schema":"https://json-schema.org/draft/2019-09/schema","type":"array","items":[{"type":"integer"}],"unevaluatedItems":false}"#, "items_tuple_2019" ; "items_tuple_2019")]
#[test_case(r#"{"$schema":"https://json-schema.org/draft/2019-09/schema","type":"array","items":[{"type":"integer"}],"additionalItems":{"type":"string"},"unevaluatedItems":false}"#, "items_tuple_additional_2019" ; "items_tuple_additional_2019")]
#[test_case(r#"{"type":"array","prefixItems":[{}],"unevaluatedItems":{"type":"integer"}}"#, "items_schema_form" ; "items_schema_form")]
#[test_case(r#"{"type":"object","oneOf":[true,{"properties":{"a":{}},"required":["a"]}],"unevaluatedProperties":false}"#, "props_one_of_bool_member" ; "props_one_of_bool_member")]
#[test_case(r#"{"type":"object","if":true,"then":{"properties":{"a":{}}},"unevaluatedProperties":false}"#, "props_if_bool" ; "props_if_bool")]
#[test_case(r#"{"type":"array","oneOf":[true,{"prefixItems":[{}]}],"unevaluatedItems":false}"#, "items_one_of_bool_member" ; "items_one_of_bool_member")]
#[test_case(r#"{"type":"array","if":true,"then":{"prefixItems":[{}]},"unevaluatedItems":false}"#, "items_if_bool" ; "items_if_bool")]
#[cfg_attr(feature = "arbitrary-precision", test_case(r#"{"type":"number","multipleOf":18446744073709551616}"#, "multiple_of_bigint" ; "multiple_of_bigint"))]
#[cfg_attr(feature = "arbitrary-precision", test_case(r#"{"type":"number","multipleOf":0.1}"#, "multiple_of_bigfrac" ; "multiple_of_bigfrac"))]
#[test_case(r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"string","contentEncoding":"base64"}"#, "content_base64" ; "content_base64")]
#[test_case(r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"string","contentEncoding":"base64url"}"#, "content_base64url" ; "content_base64url")]
#[test_case(r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"string","contentEncoding":"base32"}"#, "content_base32" ; "content_base32")]
#[test_case(r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"string","contentEncoding":"base32hex"}"#, "content_base32hex" ; "content_base32hex")]
#[test_case(r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"string","contentEncoding":"base16"}"#, "content_base16" ; "content_base16")]
#[test_case(r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"string","contentMediaType":"application/json","contentEncoding":"base64"}"#, "content_media_and_encoding" ; "content_media_and_encoding")]
#[test_case(r#"{"type":"string","contentEncoding":"base64"}"#, "content_ignored_on_modern_draft" ; "content_ignored_on_modern_draft")]
#[test_case(r#"{"allOf":[{},{"type":"string"}]}"#, "all_of_leading_empty" ; "all_of_leading_empty")]
#[test_case(r#"{"allOf":[{"type":"string"},{}]}"#, "all_of_trailing_empty" ; "all_of_trailing_empty")]
#[test_case(r#"{"allOf":[false,{"type":"string"}]}"#, "all_of_leading_false" ; "all_of_leading_false")]
#[test_case(r#"{"allOf":[{"type":"string"},false]}"#, "all_of_trailing_false" ; "all_of_trailing_false")]
#[test_case(r#"{"anyOf":[false,{"type":"string"}]}"#, "any_of_false_member" ; "any_of_false_member")]
#[test_case(r#"{"not":{}}"#, "not_empty" ; "not_empty")]
#[test_case(r#"{"not":true}"#, "not_true" ; "not_true")]
#[test_case(r#"{"type":"object","properties":{"a":{}},"patternProperties":{"^x":{"type":"integer"}},"additionalProperties":false}"#, "additional_false_with_props_and_patterns" ; "additional_false_with_props_and_patterns")]
#[test_case(r#"{"type":"object","properties":{"a":{}},"additionalProperties":{"type":"integer"}}"#, "additional_schema_with_props" ; "additional_schema_with_props")]
#[test_case(r#"{"type":"object","patternProperties":{"^x":{"type":"integer"}},"additionalProperties":{"type":"string"}}"#, "additional_schema_with_patterns" ; "additional_schema_with_patterns")]
#[test_case(r#"{"type":"object","additionalProperties":{"type":"integer"}}"#, "additional_schema_only" ; "additional_schema_only")]
#[test_case(r#"{"oneOf":[{"type":"object","properties":{"k":{"const":true}},"required":["k"]},{"type":"object","properties":{"k":{"const":false}},"required":["k"]}]}"#, "one_of_bool_discriminator" ; "one_of_bool_discriminator")]
#[test_case(r#"{"oneOf":[{"type":"object","properties":{"k":{"const":1}},"required":["k"]},{"type":"object","properties":{"k":{"const":2}},"required":["k"]}]}"#, "one_of_int_discriminator" ; "one_of_int_discriminator")]
#[test_case(r#"{"oneOf":[{"type":"object","properties":{"k":{"enum":["a","b"]}},"required":["k"]},{"type":"object","properties":{"k":{"enum":["c","d"]}},"required":["k"]}]}"#, "one_of_enum_discriminator" ; "one_of_enum_discriminator")]
#[test_case(r#"{"type":["string","number","integer","boolean","null","array","object"],"minimum":0}"#, "multi_type_number_with_all_fallbacks" ; "multi_type_number_with_all_fallbacks")]
#[test_case(r#"{"type":["boolean","null","array","object"]}"#, "multi_type_non_number" ; "multi_type_non_number")]
#[test_case(r#"{"type":["string","integer"]}"#, "multi_type_string_integer" ; "multi_type_string_integer")]
#[test_case(r#"{"type":"object","properties":{"a":{}},"patternProperties":{"^x":{"type":"integer"}},"dependencies":{"a":["b"]},"additionalProperties":false}"#, "object_props_patterns_dependencies" ; "object_props_patterns_dependencies")]
#[test_case(r#"{"type":"object","dependentRequired":{"a":["b"]},"properties":{"a":{"type":"integer"}}}"#, "object_dependent_required" ; "object_dependent_required")]
#[test_case(r#"{"type":"array","items":{}}"#, "items_empty_schema" ; "items_empty_schema")]
#[test_case(r#"{"type":"array","prefixItems":[{"type":"integer"}],"items":{"type":"string"}}"#, "prefix_items_then_items_schema" ; "prefix_items_then_items_schema")]
#[test_case(r#"{"enum":[true,false]}"#, "enum_both_booleans" ; "enum_both_booleans")]
#[test_case(r#"{"enum":[true]}"#, "enum_true" ; "enum_true")]
#[test_case(r#"{"enum":[false]}"#, "enum_false" ; "enum_false")]
#[test_case(r#"{"enum":[[1],[2]]}"#, "enum_arrays" ; "enum_arrays")]
#[test_case(r#"{"enum":[{"a":1},{"b":2}]}"#, "enum_objects" ; "enum_objects")]
#[test_case(r#"{"type":"array","uniqueItems":true,"contains":{"type":"integer"}}"#, "array_unique_and_contains" ; "array_unique_and_contains")]
#[test_case(r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"array","items":[{"type":"integer"}],"additionalItems":{"type":"string"}}"#, "items_tuple_additional_items_draft7" ; "items_tuple_additional_items_draft7")]
#[test_case(r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","dependencies":{}}"#, "empty_dependencies" ; "empty_dependencies")]
#[test_case(r#"{"$schema":"https://json-schema.org/draft/2019-09/schema","type":"object","dependentRequired":{}}"#, "empty_dependent_required" ; "empty_dependent_required")]
#[test_case(r#"{"$schema":"https://json-schema.org/draft/2019-09/schema","type":"object","dependentSchemas":{}}"#, "empty_dependent_schemas" ; "empty_dependent_schemas")]
#[test_case(r##"{"$defs":{"base":{"properties":{"a":{}}}},"type":"object","$ref":"#/$defs/base","unevaluatedProperties":false}"##, "uneval_props_toplevel_ref" ; "uneval_props_toplevel_ref")]
#[test_case(r##"{"$defs":{"base":{"prefixItems":[{}]}},"type":"array","$ref":"#/$defs/base","unevaluatedItems":false}"##, "uneval_items_toplevel_ref" ; "uneval_items_toplevel_ref")]
#[test_case(r##"{"$defs":{"anchored":{"$dynamicAnchor":"node","properties":{"a":{}}}},"type":"object","$ref":"#/$defs/anchored","unevaluatedProperties":false}"##, "uneval_props_dynamic_anchor_via_ref" ; "uneval_props_dynamic_anchor_via_ref")]
#[test_case(r#"{"oneOf":[{"type":"string"},{"type":"number"}]}"#, "one_of_no_discriminator" ; "one_of_no_discriminator")]
#[test_case(r#"{"oneOf":[{"type":"object","properties":{"k":{"const":1.5}},"required":["k"]},{"type":"object","properties":{"k":{"const":2.5}},"required":["k"]}]}"#, "one_of_non_integer_const" ; "one_of_non_integer_const")]
#[test_case(r#"{"type":["integer","string"]}"#, "type_integer_string" ; "type_integer_string")]
#[test_case(r#"{"type":["object","array"]}"#, "type_object_array" ; "type_object_array")]
#[test_case(r#"{"type":["integer","string","boolean","null","array","object"]}"#, "type_integer_plus_all" ; "type_integer_plus_all")]
#[test_case(r#"{"type":["number","string","boolean","null","array","object"]}"#, "type_number_plus_all" ; "type_number_plus_all")]
#[test_case(r#"{"not":false}"#, "not_false" ; "not_false")]
#[test_case(r#"{"type":"string","pattern":"^abc"}"#, "pattern_prefix" ; "pattern_prefix")]
#[test_case(r#"{"type":"string","pattern":"^abc$"}"#, "pattern_exact" ; "pattern_exact")]
#[test_case(r#"{"type":"object","properties":{"a":{"type":"integer"}},"additionalProperties":false}"#, "props_and_additional_false" ; "props_and_additional_false")]
#[test_case(r#"{"type":"object","properties":{"a":{"type":"integer"}},"additionalProperties":{"type":"string"}}"#, "props_and_additional_schema" ; "props_and_additional_schema")]
#[test_case(r#"{"type":"string","minLength":2,"maxLength":5}"#, "string_min_and_max_length" ; "string_min_and_max_length")]
#[test_case(r#"{"type":"string","minLength":3}"#, "string_min_length_only" ; "string_min_length_only")]
#[test_case(r#"{"type":"string","maxLength":5}"#, "string_max_length_only" ; "string_max_length_only")]
#[test_case(r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"string","contentEncoding":"base64"}"#, "content_annotation_only_2020" ; "content_annotation_only_2020")]
#[test_case(r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"string","contentMediaType":"application/json"}"#, "content_media_type_only" ; "content_media_type_only")]
#[test_case(r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"string","contentMediaType":"application/json","contentEncoding":"base64"}"#, "content_media_and_encoding_draft7" ; "content_media_and_encoding_draft7")]
#[test_case(r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","anyOf":[{"oneOf":[{"properties":{"a":{}},"required":["a"]},{"properties":{"b":{}},"required":["b"]}]}],"unevaluatedProperties":false}"#, "uneval_guard_nested_one_of" ; "uneval_guard_nested_one_of")]
#[test_case(r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","anyOf":[{"if":{"required":["a"]},"then":{"properties":{"a":{}}},"else":{"properties":{"b":{}}}}],"unevaluatedProperties":false}"#, "uneval_guard_nested_if" ; "uneval_guard_nested_if")]
#[test_case(r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"array","anyOf":[{"oneOf":[{"prefixItems":[{}]},{"prefixItems":[{},{}]}]}],"unevaluatedItems":false}"#, "uneval_items_guard_nested_one_of" ; "uneval_items_guard_nested_one_of")]
#[test_case(r#"{"type":["number","string","boolean","null","array","object"],"minimum":0}"#, "multi_type_number_constraint_fallbacks" ; "multi_type_number_constraint_fallbacks")]
#[test_case(r#"{"oneOf":[{"type":"object","properties":{"k":{"const":"a"}},"required":["k"]},{"type":"object","properties":{"k":{"const":1}},"required":["k"]}]}"#, "one_of_conflicting_discriminator_kinds" ; "one_of_conflicting_discriminator_kinds")]
#[test_case(r#"{"oneOf":[{"type":"object","properties":{"a":{"const":1},"b":{"const":3}},"required":["a","b"]},{"type":"object","properties":{"a":{"const":2},"b":{"const":4}},"required":["a","b"]}]}"#, "one_of_tiebreak_discriminators" ; "one_of_tiebreak_discriminators")]
#[test_case(r#"{"oneOf":[{"type":["object"],"properties":{"k":{"const":1}},"required":["k"]},{"type":["object"],"properties":{"k":{"const":2}},"required":["k"]}]}"#, "one_of_object_only_array_type" ; "one_of_object_only_array_type")]
#[test_case(r#"{"oneOf":[{"type":"object","properties":{"k":{"enum":[1,"x"]}},"required":["k"]},{"type":"object","properties":{"k":{"enum":[2,"y"]}},"required":["k"]}]}"#, "one_of_mixed_kind_enum" ; "one_of_mixed_kind_enum")]
#[test_case(r#"{"type":"object","patternProperties":{"^x":{"type":"integer"}},"additionalProperties":false}"#, "pattern_props_and_additional_false_no_props" ; "pattern_props_and_additional_false_no_props")]
#[test_case(r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"array","items":[true,{"type":"integer"}]}"#, "items_tuple_with_true_member" ; "items_tuple_with_true_member")]
#[test_case(r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"array","items":[{"type":"integer"}],"additionalItems":{"type":"integer"},"contains":{"type":"string"}}"#, "items_tuple_additional_and_contains" ; "items_tuple_additional_and_contains")]
#[test_case(r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{"a":{}},"dependencies":{"a":["b"]}}"#, "object_legacy_dependencies_draft7" ; "object_legacy_dependencies_draft7")]
#[test_case(r#"{"type":"object","properties":{"a":{}},"patternProperties":{"^x":{"type":"integer"}},"additionalProperties":{"type":"integer"},"propertyNames":{"minLength":1},"minProperties":1,"maxProperties":5}"#, "object_all_applicators" ; "object_all_applicators")]
#[test_case(r#"{"type":["integer","string","boolean","null","array","object"],"minimum":0}"#, "multi_type_integer_constraint_fallbacks" ; "multi_type_integer_constraint_fallbacks")]
#[test_case(r#"{"enum":[false,"marker"]}"#, "enum_false_and_string" ; "enum_false_and_string")]
#[test_case(r#"{"enum":[{"a":1},{"b":2}]}"#, "enum_objects_only" ; "enum_objects_only")]
#[test_case(r#"{"type":"array","uniqueItems":false,"items":{"type":"integer"}}"#, "unique_items_false" ; "unique_items_false")]
#[test_case(r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"array","items":[{"type":"integer"}],"additionalItems":true}"#, "additional_items_true" ; "additional_items_true")]
#[test_case(r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"array","prefixItems":[{"type":"integer"}],"items":{"type":"string"}}"#, "prefix_items_then_items_schema_2020" ; "prefix_items_then_items_schema_2020")]
#[test_case(r#"{"type":"object","patternProperties":{"^a":{},"^b":{},"^c$":{},"^d$":{}},"unevaluatedProperties":false}"#, "pattern_props_multi_prefix_and_multi_literal" ; "pattern_props_multi_prefix_and_multi_literal")]
#[test_case(r#"{"type":"object","patternProperties":{"^z$":{}},"unevaluatedProperties":false}"#, "pattern_props_single_literal" ; "pattern_props_single_literal")]
#[test_case(r#"{"type":"object","patternProperties":{"^(x|y)$":{}},"unevaluatedProperties":false}"#, "pattern_props_alternation" ; "pattern_props_alternation")]
#[test_case(r#"{"type":"object","patternProperties":{"^a":{},"^b.c":{}},"unevaluatedProperties":false}"#, "pattern_props_prefix_and_regex_uneval" ; "pattern_props_prefix_and_regex_uneval")]
#[test_case(r#"{"type":"object","patternProperties":{"^\\S*$":{}},"unevaluatedProperties":false}"#, "pattern_props_no_whitespace_uneval" ; "pattern_props_no_whitespace_uneval")]
#[test_case(r#"{"type":"array","prefixItems":[{"type":"integer"}],"items":{}}"#, "prefix_items_then_trivial_items" ; "prefix_items_then_trivial_items")]
#[test_case(r#"{"oneOf":[{"properties":{"k":{"const":"a"},"x":{"type":"integer"}},"required":["k"],"additionalProperties":false},{"properties":{"k":{"const":"b"}},"required":["k"],"additionalProperties":false}]}"#, "one_of_vacuous_object_discriminator" ; "one_of_vacuous_object_discriminator")]
#[test_case(r##"{"$defs":{"a":{"properties":{"k":{"const":"a"}},"required":["k"],"additionalProperties":false},"b":{"properties":{"k":{"const":"b"}},"required":["k"],"additionalProperties":false}},"oneOf":[{"$ref":"#/$defs/a"},{"$ref":"#/$defs/b"}]}"##, "one_of_ref_vacuous_discriminator" ; "one_of_ref_vacuous_discriminator")]
#[test_case(r#"{"oneOf":[{"properties":{"k":{"const":"a"}},"required":["k"]},{"type":"object","properties":{"k":{"const":"b"}},"required":["k"]}]}"#, "one_of_single_vacuous_not_guarded" ; "one_of_single_vacuous_not_guarded")]
#[test_case(r#"{"oneOf":[{"properties":{"k":{"const":"a"}},"required":["k"]},{"properties":{"k":{"const":"b"}},"required":["k"]},{"type":"string"}]}"#, "one_of_vacuous_with_non_object_branch_not_guarded" ; "one_of_vacuous_with_non_object_branch_not_guarded")]
#[test_case(r#"{"oneOf":[{"properties":{"k":{"const":"a"}},"required":["k"]},{"properties":{"k":{"const":"b"}},"required":["k"]},{"properties":{"z":{"type":"integer"}}}]}"#, "one_of_vacuous_uncovered_branch_not_guarded" ; "one_of_vacuous_uncovered_branch_not_guarded")]
#[test_case(r#"{"type":"object","required":["a","b"],"properties":{"a":{"type":"string"}}}"#, "required_fused_into_properties" ; "required_fused_into_properties")]
#[test_case(r#"{"type":"object","required":["a","b"],"properties":{"a":{"type":"string"},"b":{"type":"integer"}},"additionalProperties":false}"#, "required_fused_with_additional_false" ; "required_fused_with_additional_false")]
#[test_case(r#"{"type":"object","required":["a"],"additionalProperties":{"type":"string"}}"#, "required_fused_with_additional_schema" ; "required_fused_with_additional_schema")]
#[test_case(r#"{"type":"object","required":["a","b"]}"#, "required_only_not_fused" ; "required_only_not_fused")]
#[test_case(r#"{"type":"object","required":["k"],"properties":{"k":{"type":"string"}},"oneOf":[{"required":["a"]},{"required":["b"]}]}"#, "typed_check_before_untyped_applicators" ; "typed_check_before_untyped_applicators")]
#[test_case(r##"{"$defs":{"s":{"type":"string","format":"uri"}},"type":"object","propertyNames":{"$ref":"#/$defs/s"}}"##, "property_names_ref_string_only" ; "property_names_ref_string_only")]
#[test_case(r#"{"type":"object","propertyNames":{"type":"string"}}"#, "property_names_type_string_only" ; "property_names_type_string_only")]
#[test_case(r##"{"$defs":{"s":{"type":"string","pattern":"^a"}},"type":"object","propertyNames":{"$ref":"#/$defs/s"}}"##, "property_names_ref_pattern_stays_generic" ; "property_names_ref_pattern_stays_generic")]
#[test_case(r##"{"$defs":{"s":{"minProperties":1}},"type":"object","propertyNames":{"$ref":"#/$defs/s"}}"##, "property_names_ref_non_string_stays_generic" ; "property_names_ref_non_string_stays_generic")]
#[test_case(r##"{"$defs":{"s":{"type":"string"}},"type":"object","propertyNames":{"$ref":"#/$defs/s","minLength":2}}"##, "property_names_ref_with_sibling_stays_generic" ; "property_names_ref_with_sibling_stays_generic")]
#[test_case(r##"{"$defs":{"node":{"$dynamicAnchor":"node","type":["object","boolean"]}},"type":"object","properties":{"child":{"$dynamicRef":"#node"}}}"##, "dynamic_ref_lookup" ; "dynamic_ref_lookup")]
#[test_case(r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"array","items":[{"type":"string"}],"additionalItems":false,"maxItems":1}"#, "additional_items_false_subsumed_by_max_items" ; "additional_items_false_subsumed_by_max_items")]
#[test_case(r#"{"type":"string","allOf":[{"minLength":1},{"maxLength":5}]}"#, "typed_string_before_all_of" ; "typed_string_before_all_of")]
fn codegen_is_valid_body_snapshot(schema_json: &str, snap_name: &str) {
    let schema: Value = serde_json::from_str(schema_json).expect("valid schema json");
    let description = serde_json::to_string(&schema).expect("schema serialization");
    insta::with_settings!({ description => &description }, {
        insta::assert_snapshot!(snap_name, is_valid_body(schema));
    });
}

#[test_case(r##"{"$defs":{"a":{"properties":{"k":{"const":"a"}},"required":["k"],"additionalProperties":false},"b":{"properties":{"k":{"const":"b"}},"required":["k"],"additionalProperties":false}},"oneOf":[{"$ref":"#/$defs/a"},{"$ref":"#/$defs/b"}]}"##, "one_of_ref_vacuous_discriminator_validate" ; "one_of_ref_vacuous_discriminator_validate")]
#[test_case(r#"{"oneOf":[{"type":"object","required":["kind"],"properties":{"kind":{"const":"circle"},"radius":{"type":"number"}}},{"type":"object","required":["kind"],"properties":{"kind":{"const":"square"},"side":{"type":"number"}}}]}"#, "one_of_typed_discriminator_validate" ; "one_of_typed_discriminator_validate")]
#[test_case(r#"{"oneOf":[{"type":"null"},{"type":"object","required":["kind"],"properties":{"kind":{"const":"circle"}}},{"type":"object","required":["kind"],"properties":{"kind":{"const":"square"}}}]}"#, "one_of_typed_with_null_branch_validate" ; "one_of_typed_with_null_branch_validate")]
#[test_case(r#"{"oneOf":[{"type":"string"},{"type":"number"}]}"#, "one_of_no_discriminator_validate" ; "one_of_no_discriminator_validate")]
#[test_case(r#"{"type":"object","required":["a"],"properties":{"a":{"type":"string"}},"additionalProperties":false}"#, "properties_validate_uses_bound_obj" ; "properties_validate_uses_bound_obj")]
#[test_case(r#"{"type":"object","properties":{"a":{"type":"integer"}},"patternProperties":{"^x":{"type":"string"}},"additionalProperties":false}"#, "object_pass_validate_uses_bound_obj" ; "object_pass_validate_uses_bound_obj")]
fn codegen_validate_body_snapshot(schema_json: &str, snap_name: &str) {
    let schema: Value = serde_json::from_str(schema_json).expect("valid schema json");
    let description = serde_json::to_string(&schema).expect("schema serialization");
    insta::with_settings!({ description => &description }, {
        insta::assert_snapshot!(snap_name, validate_body(schema));
    });
}

#[test_case(r#"{"properties":{"a":{"type":"integer"},"b":{"type":"integer"}}}"#, "collect_properties_schema_order" ; "collect_properties_schema_order")]
#[test_case(r#"{"type":"object","required":["a","b"],"properties":{"a":{"type":"string"},"b":{"type":"integer"}},"additionalProperties":false}"#, "collect_required_fused_with_additional_false" ; "collect_required_fused_with_additional_false")]
#[test_case(r#"{"type":"object","properties":{"a":{}},"patternProperties":{"^x":{"type":"integer"}},"additionalProperties":{"type":"integer"},"propertyNames":{"minLength":1}}"#, "collect_object_all_applicators" ; "collect_object_all_applicators")]
#[test_case(r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"array","prefixItems":[{"type":"integer"}],"items":{"type":"string"}}"#, "collect_prefix_items_then_items_schema_2020" ; "collect_prefix_items_then_items_schema_2020")]
#[test_case(r##"{"$defs":{"node":{"$dynamicAnchor":"node","type":["object","boolean"]}},"type":"object","properties":{"child":{"$dynamicRef":"#node"}}}"##, "collect_dynamic_ref_lookup" ; "collect_dynamic_ref_lookup")]
#[cfg_attr(not(feature = "arbitrary-precision"), test_case(r#"{"if":{"type":"integer"},"then":{"minimum":10,"multipleOf":3}}"#, "collect_if_then" ; "collect_if_then"))]
#[cfg_attr(feature = "arbitrary-precision", test_case(r#"{"if":{"type":"integer"},"then":{"minimum":10,"multipleOf":3}}"#, "collect_if_then_arbitrary" ; "collect_if_then"))]
fn codegen_collect_body_snapshot(schema_json: &str, snap_name: &str) {
    let schema: Value = serde_json::from_str(schema_json).expect("valid schema json");
    let description = serde_json::to_string(&schema).expect("schema serialization");
    insta::with_settings!({ description => &description }, {
        insta::assert_snapshot!(snap_name, collect_body(schema));
    });
}

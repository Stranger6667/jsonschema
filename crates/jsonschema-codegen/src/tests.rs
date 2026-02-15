use std::{collections::HashMap, sync::Arc};

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::{Draft, Registry};
use serde_json::Value;

use crate::{codegen::generate_from_config, context::CodegenConfig};

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

fn schema_to_code_with_runtime_alias(
    schema: Value,
    validate_formats: Option<bool>,
    custom_formats: HashMap<String, TokenStream>,
    ignore_unknown_formats: bool,
    runtime_crate_alias: Option<TokenStream>,
) -> String {
    let draft = Draft::default().detect(&schema);
    let resource = draft.create_resource(schema.clone());
    let base_uri_str = "json-schema:///test";
    let registry = Registry::options()
        .draft(draft)
        .build([(base_uri_str, resource)])
        .expect("registry build failed");
    let base_uri = referencing::uri::from_str(base_uri_str)
        .map(Arc::new)
        .expect("valid uri");

    let config = CodegenConfig {
        schema,
        registry,
        base_uri,
        draft,
        runtime_crate_alias,
        validate_formats,
        custom_formats,
        ignore_unknown_formats,
        email_options: None,
        pattern_options: crate::context::PatternEngineConfig::default(),
        backend: crate::codegen::backend::BackendKind::serde_json(),
    };

    let name = format_ident!("Validator");
    let impl_mod_name = format_ident!("__validator_impl");
    let recompile_trigger: TokenStream = quote! {};
    let tokens = generate_from_config(&config, &recompile_trigger, &name, &impl_mod_name);

    // Wrap in a struct declaration so syn can parse as a complete file
    let wrapped: TokenStream = quote! {
        struct #name;
        #tokens
    };
    let file: syn::File = syn::parse2(wrapped).expect("valid token stream");
    prettyplease::unparse(&file)
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use serde_json::json;
    use test_case::test_case;

    // Each case carries the snapshot name explicitly to avoid insta's counter-based
    // naming which is non-deterministic under parallel test execution.
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
    #[test_case(json!({"type": "number", "minimum": 0}), "minimum" ; "minimum")]
    #[test_case(json!({"type": "number", "maximum": 100}), "maximum" ; "maximum")]
    #[test_case(json!({"type": "number", "exclusiveMinimum": 0}), "exclusive_minimum" ; "exclusive_minimum")]
    #[test_case(json!({"type": "number", "exclusiveMaximum": 100}), "exclusive_maximum" ; "exclusive_maximum")]
    #[test_case(json!({"type": "number", "multipleOf": 3}), "multiple_of" ; "multiple_of")]
    #[test_case(json!({"type": "number", "minimum": 5, "maximum": 5}), "min_max_equal" ; "min_max_equal")]
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
    #[test_case(json!({"minProperties": 2, "maxProperties": 2}), "min_max_properties_equal" ; "min_max_properties_equal")]
    #[test_case(json!({"allOf": [{"type": "string"}, {"minLength": 1}]}), "all_of" ; "all_of")]
    #[test_case(json!({"anyOf": [{"type": "string"}, {"type": "number"}]}), "any_of" ; "any_of")]
    #[test_case(json!({"oneOf": [{"type": "string"}, {"type": "number"}]}), "one_of" ; "one_of")]
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
    fn test_codegen_snapshot(schema: Value, snap_name: &str) {
        insta::assert_snapshot!(snap_name, schema_to_code(schema));
    }
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    use quote::quote;
    use serde_json::json;
    use std::{collections::HashMap, path::PathBuf};
    use test_case::test_case;

    #[test_case(json!({"type":"object","patternProperties":{"(": {"type":"string"}}}); "invalid_regex")]
    fn invalid_pattern_properties_emit_compile_error(schema: Value) {
        let code = schema_to_code(schema);
        assert!(
            code.contains("compile_error"),
            "generated code should contain compile_error for invalid patternProperties, got:\n{code}"
        );
    }

    #[test]
    fn backend_stubs_compile() {
        for backend in crate::codegen::backend::BackendKind::compile_only_stub_variants() {
            assert!(
                backend.id().ends_with("_stub"),
                "expected stub backend id, got {}",
                backend.id()
            );
        }
    }

    #[test_case(json!({"$ref":"#/missing"}), "#/missing"; "unresolved_local_ref")]
    #[test_case(json!({"$defs":{"ok":{}},"$ref":"#/$defs/missing"}), "#/$defs/missing"; "unresolved_local_ref_in_defs")]
    fn unresolved_refs_emit_compile_error(schema: Value, reference: &str) {
        let code = schema_to_code(schema);
        assert!(
            code.contains("compile_error"),
            "generated code should contain compile_error for unresolved refs, got:\n{code}"
        );
        assert!(
            code.contains(reference),
            "generated code should mention unresolved reference `{reference}`, got:\n{code}"
        );
    }

    #[test_case(json!({"$schema":"http://json-schema.org/draft-07/schema#","type":"string","format":"made-up"}); "unknown_format")]
    fn unknown_format_respects_ignore_unknown_formats(schema: Value) {
        let ignored = schema_to_code_with_options(schema.clone(), Some(true), HashMap::new(), true);
        assert!(
            !ignored.contains("compile_error"),
            "unknown formats should be ignored when configured, got:\n{ignored}"
        );

        let strict = schema_to_code_with_options(schema, Some(true), HashMap::new(), false);
        assert!(
            strict.contains("compile_error"),
            "unknown formats should fail when ignore_unknown_formats=false, got:\n{strict}"
        );
        assert!(
            strict.contains("Unknown format"),
            "compile error should mention unknown format, got:\n{strict}"
        );
    }

    #[test_case(json!({"$schema":"http://json-schema.org/draft-07/schema#","type":"string","format":123}); "non_string_format")]
    fn non_string_format_value_emits_compile_error(schema: Value) {
        let code = schema_to_code_with_options(schema, Some(true), HashMap::new(), true);
        assert!(
            code.contains("compile_error"),
            "non-string format should fail compilation, got:\n{code}"
        );
        assert!(
            code.contains("123 is not of type"),
            "compile error should mention invalid format type, got:\n{code}"
        );
    }

    #[test]
    fn duplicate_custom_format_entries_fail_parsing() {
        let parsed = syn::parse2::<crate::Config>(quote! {
            schema = r#"{"type":"string"}"#,
            formats = {
                "currency" => crate::formats::currency_a,
                "currency" => crate::formats::currency_b,
            }
        });
        let Err(err) = parsed else {
            panic!("duplicate format entries should be rejected");
        };
        assert!(
            err.to_string().contains("Duplicate format entry"),
            "expected duplicate format error, got: {err}"
        );
    }

    #[test_case(
        quote! { path = "schema.json", schema = r#"{"type":"string"}"# };
        "path_then_schema"
    )]
    #[test_case(
        quote! { schema = r#"{"type":"string"}"#, path = "schema.json" };
        "schema_then_path"
    )]
    #[test_case(
        quote! { path = "one.json", path = "two.json" };
        "duplicate_path"
    )]
    #[test_case(
        quote! { schema = r#"{"type":"string"}"#, schema = r#"{"type":"number"}"# };
        "duplicate_schema"
    )]
    fn conflicting_schema_source_entries_fail_parsing(input: TokenStream) {
        let parsed = syn::parse2::<crate::Config>(input);
        let Err(err) = parsed else {
            panic!("conflicting schema source entries should be rejected");
        };
        assert!(
            err.to_string()
                .contains("Schema source is already specified"),
            "expected schema source conflict error, got: {err}"
        );
    }

    #[test]
    fn relative_custom_format_paths_are_rejected() {
        let parsed = syn::parse2::<crate::Config>(quote! {
            schema = r#"{"type":"string"}"#,
            formats = {
                "currency" => formats::currency,
            }
        });
        let Err(err) = parsed else {
            panic!("relative custom format paths should be rejected");
        };
        assert!(
            err.to_string()
                .contains("Custom format paths must be absolute"),
            "expected custom format path error, got: {err}"
        );
    }

    #[test]
    fn pattern_options_regex_engine_parses() {
        let parsed = syn::parse2::<crate::Config>(quote! {
            schema = r#"{"type":"string","pattern":"^a+$"}"#,
            pattern_options = {
                engine = regex,
                size_limit = 1024,
                dfa_size_limit = 2048,
            }
        });
        parsed.expect("pattern_options with regex engine should parse");
    }

    #[test]
    fn pattern_options_reject_unknown_key() {
        let parsed = syn::parse2::<crate::Config>(quote! {
            schema = r#"{"type":"string"}"#,
            pattern_options = {
                engine = fancy_regex,
                unknown = 1,
            }
        });
        let Err(err) = parsed else {
            panic!("unknown pattern option key should be rejected");
        };
        assert!(
            err.to_string().contains("Unknown pattern_options key"),
            "expected unknown key error, got: {err}"
        );
    }

    #[test]
    fn pattern_options_reject_backtrack_limit_for_regex_engine() {
        let parsed = syn::parse2::<crate::Config>(quote! {
            schema = r#"{"type":"string"}"#,
            pattern_options = {
                engine = regex,
                backtrack_limit = 100,
            }
        });
        let Err(err) = parsed else {
            panic!("backtrack_limit must be rejected for regex engine");
        };
        assert!(
            err.to_string()
                .contains("backtrack_limit is only supported for `fancy_regex`"),
            "expected regex backtrack_limit error, got: {err}"
        );
    }

    #[test]
    fn base_uri_parses() {
        let parsed = syn::parse2::<crate::Config>(quote! {
            schema = r#"{"$ref":"defs.json#/item"}"#,
            base_uri = "json-schema:///root/main.json"
        });
        parsed.expect("base_uri should parse");
    }

    #[test]
    fn email_options_parses() {
        let parsed = syn::parse2::<crate::Config>(quote! {
            schema = r#"{"type":"string","format":"email"}"#,
            email_options = {
                required_tld = true,
                allow_domain_literal = false,
                allow_display_text = false,
            }
        });
        parsed.expect("email_options should parse");
    }

    #[test]
    fn email_options_reject_unknown_key() {
        let parsed = syn::parse2::<crate::Config>(quote! {
            schema = r#"{"type":"string","format":"email"}"#,
            email_options = {
                unknown = true,
            }
        });
        let Err(err) = parsed else {
            panic!("unknown email_options key should be rejected");
        };
        assert!(
            err.to_string().contains("Unknown email_options key"),
            "expected unknown key error, got: {err}"
        );
    }

    #[test]
    fn email_options_reject_conflicting_domain_segment_modes() {
        let parsed = syn::parse2::<crate::Config>(quote! {
            schema = r#"{"type":"string","format":"email"}"#,
            email_options = {
                minimum_sub_domains = 3,
                required_tld = true,
            }
        });
        let Err(err) = parsed else {
            panic!("conflicting email_options domain segment modes should be rejected");
        };
        assert!(
            err.to_string().contains("At most one of"),
            "expected conflict error, got: {err}"
        );
    }

    #[test]
    fn custom_format_emits_direct_function_call() {
        let mut custom_formats = HashMap::new();
        custom_formats.insert(
            "currency".to_string(),
            quote! { crate::formats::is_currency },
        );
        let code = schema_to_code_with_options(
            json!({"$schema":"http://json-schema.org/draft-07/schema#","type":"string","format":"currency"}),
            Some(true),
            custom_formats,
            true,
        );
        assert!(
            code.contains("is_currency"),
            "generated code should call custom format function directly, got:\n{code}"
        );
    }

    #[test]
    fn runtime_crate_alias_is_injected_into_generated_module() {
        let code = schema_to_code_with_runtime_alias(
            json!({"type":"string","minLength":1}),
            None,
            HashMap::new(),
            true,
            Some(quote! { ::js }),
        );
        let normalized: String = code.split_whitespace().collect();
        assert!(
            normalized.contains("use::jsasjsonschema;"),
            "generated module should alias runtime crate path for hardcoded helper calls, got:\n{code}"
        );
    }

    #[test]
    fn duplicate_regex_literals_are_deduplicated() {
        let schema = json!({
            "type": "object",
            "propertyNames": {"pattern": "^[a-z]{2,}$"},
            "patternProperties": {"^[a-z]{2,}$": {"type": "string"}},
            "additionalProperties": false
        });
        let code = schema_to_code(schema);
        let needle = "fn __jsonschema_regex_";
        let occurrences = code.matches(needle).count();
        assert_eq!(
            occurrences, 1,
            "regex helpers should be deduplicated, got {occurrences} helper definitions in:\n{code}"
        );
    }

    #[test_case(json!("42"), r#"\"42\" is not of types \"boolean\", \"object\""#; "invalid_top_level_schema_string")]
    #[test_case(json!(42), r#"42 is not of types \"boolean\", \"object\""#; "invalid_top_level_schema_number")]
    fn invalid_top_level_schema_emits_compile_error(schema: Value, expected_message: &str) {
        let code = schema_to_code(schema);
        assert!(
            code.contains("compile_error"),
            "generated code should contain compile_error for invalid top-level schema, got:\n{code}"
        );
        assert!(
            code.contains(expected_message),
            "generated code should mention invalid top-level schema, got:\n{code}"
        );
    }

    #[test_case(json!({"enum": 42}), r#"42 is not of type \"array\""#; "enum_number")]
    #[test_case(json!({"enum": "x"}), r#"\"x\" is not of type \"array\""#; "enum_string")]
    fn invalid_enum_type_emits_compile_error(schema: Value, expected_message: &str) {
        let code = schema_to_code(schema);
        assert!(
            code.contains("compile_error"),
            "generated code should contain compile_error for invalid enum type, got:\n{code}"
        );
        assert!(
            code.contains(expected_message),
            "generated code should mention invalid enum type, got:\n{code}"
        );
    }

    #[test_case(json!({"$schema":"http://json-schema.org/draft-06/schema#","exclusiveMinimum":true}); "exclusive_minimum_boolean_draft6")]
    #[test_case(json!({"$schema":"http://json-schema.org/draft-06/schema#","exclusiveMaximum":true}); "exclusive_maximum_boolean_draft6")]
    #[test_case(json!({"multipleOf":0}); "multiple_of_zero")]
    #[test_case(json!({"multipleOf":-1}); "multiple_of_negative")]
    fn invalid_numeric_keyword_errors_match_dynamic(schema: Value) {
        let dynamic_error = jsonschema::validator_for(&schema)
            .expect_err("schema should be invalid")
            .to_string();
        let dynamic_error_escaped = dynamic_error.replace('\\', "\\\\").replace('"', "\\\"");

        let code = schema_to_code(schema);
        assert!(
            code.contains("compile_error"),
            "generated code should contain compile_error for invalid numeric keyword, got:\n{code}"
        );
        assert!(
            code.contains(&dynamic_error_escaped),
            "generated code should include runtime-equivalent error `{dynamic_error}`, got:\n{code}"
        );
    }

    #[cfg(feature = "arbitrary-precision")]
    #[test]
    fn invalid_multiple_of_scientific_zero_matches_dynamic() {
        let schema: Value =
            serde_json::from_str(r#"{"multipleOf":0e-10}"#).expect("valid json schema literal");
        let dynamic_error = jsonschema::validator_for(&schema)
            .expect_err("schema should be invalid")
            .to_string();
        let dynamic_error_escaped = dynamic_error.replace('\\', "\\\\").replace('"', "\\\"");

        let code = schema_to_code(schema);
        assert!(
            code.contains("compile_error"),
            "generated code should contain compile_error for invalid scientific zero multipleOf, got:\n{code}"
        );
        assert!(
            code.contains(&dynamic_error_escaped),
            "generated code should include runtime-equivalent error `{dynamic_error}`, got:\n{code}"
        );
    }

    #[test_case(
        r#"{"$schema":"http://json-schema.org/draft-06/schema#","type":"array","minItems":-0.0}"#;
        "draft6_min_items_negative_zero_decimal"
    )]
    #[test_case(
        r#"{"$schema":"http://json-schema.org/draft-06/schema#","type":"array","maxItems":-0e0}"#;
        "draft6_max_items_negative_zero_scientific"
    )]
    #[test_case(
        r#"{"$schema":"http://json-schema.org/draft-06/schema#","type":"string","maxLength":-0.0}"#;
        "draft6_max_length_negative_zero_decimal"
    )]
    #[test_case(
        r#"{"$schema":"http://json-schema.org/draft-06/schema#","type":"object","minProperties":-0e0}"#;
        "draft6_min_properties_negative_zero_scientific"
    )]
    fn signed_zero_nonnegative_keywords_match_dynamic(schema_json: &str) {
        let schema: Value = serde_json::from_str(schema_json).expect("valid schema json");
        assert!(
            jsonschema::validator_for(&schema).is_ok(),
            "runtime validator should accept signed-zero nonnegative keyword schema"
        );

        let code = schema_to_code(schema);
        assert!(
            !code.contains("compile_error"),
            "generated code should not emit compile_error for signed-zero nonnegative keyword, got:\n{code}"
        );
    }

    #[test_case(json!({"type":"array","minItems":"x"}), r#"\"x\" is not of type \"integer\""#; "array_min_items_wrong_type")]
    #[test_case(json!({"type":"array","maxItems":-1}), r"-1 is less than the minimum of 0"; "array_max_items_negative")]
    #[test_case(json!({"type":"string","minLength":"x"}), r#"\"x\" is not of type \"integer\""#; "string_min_length_wrong_type")]
    #[test_case(json!({"type":"string","maxLength":-1}), r"-1 is less than the minimum of 0"; "string_max_length_negative")]
    #[test_case(json!({"type":"object","minProperties":"x"}), r#"\"x\" is not of type \"integer\""#; "object_min_properties_wrong_type")]
    #[test_case(json!({"type":"object","maxProperties":-1}), r"-1 is less than the minimum of 0"; "object_max_properties_negative")]
    #[test_case(json!({"type":"array","contains":{},"minContains":"x"}), r#"\"x\" is not of type \"integer\""#; "array_min_contains_wrong_type")]
    #[test_case(json!({"type":"array","contains":{},"maxContains":-1}), r"-1 is less than the minimum of 0"; "array_max_contains_negative")]
    fn parse_nonnegative_keyword_errors(schema: Value, expected_message: &str) {
        let code = schema_to_code(schema);
        assert!(
            code.contains("compile_error"),
            "generated code should contain compile_error for invalid non-negative keyword, got:\n{code}"
        );
        assert!(
            code.contains(expected_message),
            "generated code should contain expected error message `{expected_message}`, got:\n{code}"
        );
    }

    #[test]
    fn invalid_unique_items_type_emits_compile_error() {
        let code = schema_to_code(json!({"type":"array","uniqueItems":"yes"}));
        assert!(
            code.contains("compile_error"),
            "generated code should contain compile_error for invalid uniqueItems type, got:\n{code}"
        );
        assert!(
            code.contains(r#"\"yes\" is not of type \"boolean\""#),
            "generated code should mention invalid uniqueItems type, got:\n{code}"
        );
    }

    #[test]
    fn generated_validator_exposes_only_is_valid() {
        let code = schema_to_code(json!({"type":"string"}));
        assert!(
            !code.contains("pub fn validate("),
            "generated code should not expose validate(), got:\n{code}"
        );
        assert!(
            !code.contains("pub(super) fn validate("),
            "generated module should not contain validate(), got:\n{code}"
        );
        assert!(
            code.contains("pub fn is_valid("),
            "generated code should expose is_valid(), got:\n{code}"
        );
    }

    #[test]
    fn codegen_module_split_smoke() {
        let code = schema_to_code(json!({"type":"string"}));
        assert!(
            code.contains("pub fn is_valid("),
            "generated code should still contain public is_valid, got:\n{code}"
        );
    }

    #[test]
    fn symbols_layer_serde_paths_smoke() {
        let code = schema_to_code(json!({"type":"string"}));
        assert!(
            code.contains("serde_json::Value"),
            "generated code should include serde_json::Value path, got:\n{code}"
        );
    }

    #[test]
    fn unresolved_remote_refs_fail_macro_expansion() {
        let config: crate::Config = syn::parse2(quote! {
            schema = r#"{"$ref":"http://example.com/remote"}"#
        })
        .expect("Config should parse");
        let item: syn::ItemStruct = syn::parse2(quote! {
            struct RemoteRefValidator;
        })
        .expect("Item should parse");

        let err = crate::validator_impl(&config, &item).expect_err("Macro should fail");
        assert!(
            err.to_string().contains("Registry error"),
            "expected registry error, got: {err}"
        );
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
        assert!(
            rendered.contains(&resource_path),
            "generated code should include resolved resource path `{resource_path}`, got:\n{rendered}"
        );
    }
}

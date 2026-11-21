#![allow(clippy::large_stack_arrays, clippy::needless_pass_by_value)]

mod tests {
    #[cfg(feature = "json-text")]
    use jsonschema::json::JsonText;
    #[cfg(feature = "jsonb")]
    use jsonschema::json::Jsonb;
    use jsonschema::{canonical::CanonicalKind, Draft, PatternOptions, Registry, Retrieve, Uri};
    #[cfg(feature = "jsonb")]
    use jsonschema_value::jsonb::encode::encode;
    #[cfg(not(target_arch = "wasm32"))]
    use std::env;
    #[cfg(not(target_arch = "wasm32"))]
    use std::fs;
    use testsuite::{suite, CodegenValidator, ErrorReport, Test};

    #[suite(
    path = "crates/jsonschema/tests/suite",
    drafts = [
        "draft4",
        "draft6",
        "draft7",
        "draft2019-09",
        "draft2020-12",
    ]
)]
    fn test_suite(test: &Test, codegen_validator: Option<Box<dyn CodegenValidator>>) {
        enum RegexEngine {
            Regex,
            FancyRegex,
        }

        let draft = match test.draft {
            "draft4" => Draft::Draft4,
            "draft6" => Draft::Draft6,
            "draft7" => Draft::Draft7,
            "draft2019-09" => Draft::Draft201909,
            "draft2020-12" => Draft::Draft202012,
            _ => panic!("Unsupported draft"),
        };
        if should_skip_draft(test.draft) {
            return;
        }
        // Literals past `f64` precision survive only with `arbitrary-precision`.
        #[cfg(not(feature = "arbitrary-precision"))]
        if test.file == "optional/bignum.json" {
            return;
        }
        // Without `idna` these are unknown formats, so the suite's expectations no longer hold.
        #[cfg(not(feature = "idna"))]
        if matches!(
            test.schema
                .get("format")
                .and_then(serde_json::Value::as_str),
            Some("idn-hostname" | "idn-email")
        ) {
            return;
        }
        let root_uri = test.schema[draft.id_keyword()]
            .as_str()
            .unwrap_or("json-schema:///");
        let registry = Registry::new()
            .retriever(ExternalSchemas(test.external_schemas.clone()))
            .draft(draft)
            .add(root_uri, &test.schema)
            .expect("Failed to add the test schema to the registry")
            .prepare()
            .expect("Failed to prepare the test schema registry");
        let mut options = jsonschema::options()
            .with_draft(draft)
            .with_registry(&registry);
        if let Some(validate_formats) = test.validate_formats {
            options = options.should_validate_formats(validate_formats);
        }

        #[cfg(feature = "jsonb")]
        {
            let mut options = jsonschema::options_for::<Jsonb>()
                .with_draft(draft)
                .with_registry(&registry);
            if let Some(validate_formats) = test.validate_formats {
                options = options.should_validate_formats(validate_formats);
            }
            let validator = options
                .build(&test.schema)
                .expect("Failed to build a schema");
            let encoded = encode(&test.data);
            let instance = Jsonb::root(&encoded);
            for (method, valid) in [
                ("is_valid", validator.is_valid(instance)),
                ("validate", validator.validate(instance).is_ok()),
            ] {
                assert_eq!(
                    valid,
                    test.valid,
                    "`{method}` over jsonb:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                    test.case,
                    test.description,
                    pretty_json(&test.schema),
                    pretty_json(&test.data),
                );
            }
        }

        #[cfg(feature = "json-text")]
        {
            let mut options = jsonschema::options_for::<JsonText>()
                .with_draft(draft)
                .with_registry(&registry);
            if let Some(validate_formats) = test.validate_formats {
                options = options.should_validate_formats(validate_formats);
            }
            let validator = options
                .build(&test.schema)
                .expect("Failed to build a schema");
            let text = serde_json::to_vec(&test.data).expect("serializable");
            let document = JsonText::parse(&text).expect("valid JSON");
            for (method, valid) in [
                ("is_valid", validator.is_valid(document.root())),
                ("validate", validator.validate(document.root()).is_ok()),
                (
                    "iter_errors",
                    validator.iter_errors(document.root()).next().is_none(),
                ),
                ("evaluate", validator.evaluate(document.root()).flag().valid),
            ] {
                assert_eq!(
                    valid,
                    test.valid,
                    "`{method}` over JSON text:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                    test.case,
                    test.description,
                    pretty_json(&test.schema),
                    pretty_json(&test.data),
                );
            }
        }

        for engine in [RegexEngine::FancyRegex, RegexEngine::Regex] {
            match engine {
                RegexEngine::Regex => {
                    options = options.with_pattern_options(PatternOptions::regex());
                }
                RegexEngine::FancyRegex => {
                    options = options.with_pattern_options(PatternOptions::fancy_regex());
                }
            }
            let validator = options
                .build(&test.schema)
                .expect("Failed to build a schema");

            // Canonicalization must not change what a schema accepts. A `Raw` document round-trips
            // verbatim, so only the modeled subset is worth comparing.
            let mut canonicalize_options = jsonschema::canonical::options()
                .with_draft(draft)
                .with_registry(&registry);
            match engine {
                RegexEngine::Regex => {
                    canonicalize_options =
                        canonicalize_options.with_pattern_options(PatternOptions::regex());
                }
                RegexEngine::FancyRegex => {
                    canonicalize_options =
                        canonicalize_options.with_pattern_options(PatternOptions::fancy_regex());
                }
            }
            if let Some(validate_formats) = test.validate_formats {
                canonicalize_options =
                    canonicalize_options.should_validate_formats(validate_formats);
            }
            // Unsupported constructs come back as `Raw`, never as an error, so any error is a regression.
            let canonical = canonicalize_options
                .canonicalize(&test.schema)
                .unwrap_or_else(|error| {
                    panic!(
                        "Canonicalization failed:\nCase: {}\nTest: {}\nSchema: {}\nError: {error}",
                        test.case,
                        test.description,
                        pretty_json(&test.schema),
                    )
                });
            if canonical.kind() != CanonicalKind::Raw {
                let canonical_value = canonical.to_json_schema();
                let mut canonical_options = jsonschema::options()
                    .with_draft(canonical.draft())
                    .with_retriever(ExternalSchemas(test.external_schemas.clone()));
                if let Some(validate_formats) = test.validate_formats {
                    canonical_options = canonical_options.should_validate_formats(validate_formats);
                }
                match engine {
                    RegexEngine::Regex => {
                        canonical_options =
                            canonical_options.with_pattern_options(PatternOptions::regex());
                    }
                    RegexEngine::FancyRegex => {
                        canonical_options =
                            canonical_options.with_pattern_options(PatternOptions::fancy_regex());
                    }
                }
                let canonical_validator = canonical_options
                    .build(&canonical_value)
                    .unwrap_or_else(|error| {
                        panic!(
                            "Canonical schema failed to compile:\nCase: {}\nTest: {}\nSchema: {}\nCanonical: {}\nError: {error}",
                            test.case,
                            test.description,
                            pretty_json(&test.schema),
                            pretty_json(&canonical_value),
                        )
                    });
                assert_eq!(
                    validator.is_valid(&test.data),
                    canonical_validator.is_valid(&test.data),
                    "Canonicalization changed validation:\nCase: {}\nTest: {}\nSchema: {}\nCanonical: {}\nInstance: {}",
                    test.case,
                    test.description,
                    pretty_json(&test.schema),
                    pretty_json(&canonical_value),
                    pretty_json(&test.data),
                );
            }

            if test.valid {
                if let Some(first) = validator.iter_errors(&test.data).next() {
                    panic!(
                    "Test case should not have validation errors:\nGroup: {}\nTest case: {}\nSchema: {}\nInstance: {}\nError: {first:?}",
                    test.case,
                    test.description,
                    pretty_json(&test.schema),
                    pretty_json(&test.data),
                );
                }
                assert!(
                    validator.is_valid(&test.data),
                    "Test case should be valid:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                    test.case,
                    test.description,
                    pretty_json(&test.schema),
                    pretty_json(&test.data),
                );
                if let Some(codegen_validator) = &codegen_validator {
                    assert!(
                        codegen_validator.is_valid(&test.data),
                        "Test case should be valid:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                        test.case,
                        test.description,
                        pretty_json(&test.schema),
                        pretty_json(&test.data),
                    );
                    assert!(
                        codegen_validator.validate(&test.data).is_ok(),
                        "Test case should be valid:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                        test.case,
                        test.description,
                        pretty_json(&test.schema),
                        pretty_json(&test.data),
                    );
                }
                assert!(
                    validator.validate(&test.data).is_ok(),
                    "Test case should be valid:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                    test.case,
                    test.description,
                    pretty_json(&test.schema),
                    pretty_json(&test.data),
                );
                let evaluation = validator.evaluate(&test.data);
                assert!(
                    evaluation.flag().valid,
                    "Evaluation output should be valid:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                    test.case,
                    test.description,
                    pretty_json(&test.schema),
                    pretty_json(&test.data),
                );
                let _ =
                    serde_json::to_value(evaluation.list()).expect("List output should serialize");
                let _ = serde_json::to_value(evaluation.hierarchical())
                    .expect("Hierarchical output should serialize");
            } else {
                let mut errors = validator.iter_errors(&test.data);
                let Some(first_error) = errors.next() else {
                    panic!(
                        "Test case should have validation errors:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                        test.case,
                        test.description,
                        pretty_json(&test.schema),
                        pretty_json(&test.data),
                    );
                };

                let pointer = first_error.instance_path().as_str();
                assert_eq!(
                    test.data.pointer(pointer),
                    Some(first_error.instance().as_ref()),
                    "Expected error instance did not match actual error instance:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}\nExpected pointer: {:#?}\nActual pointer: {:#?}",
                    test.case,
                    test.description,
                    pretty_json(&test.schema),
                    pretty_json(&test.data),
                    first_error.instance().as_ref(),
                    pointer,
                );

                let first_error_parts = first_error.into_parts();
                let evaluation_path = first_error_parts.evaluation_path;
                assert!(
                    evaluation_path.as_str().is_empty()
                        || evaluation_path.as_str().starts_with('/'),
                    "Evaluation path should be a JSON pointer: {evaluation_path}"
                );

                for error in errors {
                    let pointer = error.instance_path().as_str();
                    assert_eq!(
                    test.data.pointer(pointer), Some(error.instance().as_ref()),
                    "Expected error instance did not match actual error instance:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}\nExpected pointer: {:#?}\nActual pointer: {:#?}",
                    test.case,
                    test.description,
                    pretty_json(&test.schema),
                    pretty_json(&test.data),
                    error.instance().as_ref(),
                    pointer,
                );
                }
                assert!(
                    !validator.is_valid(&test.data),
                    "Test case should be invalid:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                    test.case,
                    test.description,
                    pretty_json(&test.schema),
                    pretty_json(&test.data),
                );
                if let Some(codegen_validator) = &codegen_validator {
                    assert!(
                        !codegen_validator.is_valid(&test.data),
                        "Test case should be invalid:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                        test.case,
                        test.description,
                        pretty_json(&test.schema),
                        pretty_json(&test.data),
                    );
                }
                let Some(error) = validator.validate(&test.data).err() else {
                    panic!(
                    "Test case should be invalid:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                    test.case,
                    test.description,
                    pretty_json(&test.schema),
                    pretty_json(&test.data),
                );
                };
                let pointer = error.instance_path().as_str();
                assert_eq!(
                test.data.pointer(pointer), Some(error.instance().as_ref()),
                "Expected error instance did not match actual error instance:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}\nExpected pointer: {:#?}\nActual pointer: {:#?}",
                test.case,
                test.description,
                pretty_json(&test.schema),
                pretty_json(&test.data),
                error.instance().as_ref(),
                pointer,
            );
                let runtime_message = error.to_string();
                let error_parts = error.into_parts();
                let evaluation_path = &error_parts.evaluation_path;
                assert!(
                    evaluation_path.as_str().is_empty()
                        || evaluation_path.as_str().starts_with('/'),
                    "Evaluation path should be a JSON pointer: {evaluation_path}"
                );

                if let Some(codegen_validator) = &codegen_validator {
                    // Compare codegen validate() error against runtime validate()
                    let codegen_error = codegen_validator
                        .validate(&test.data)
                        .err()
                        .unwrap_or_else(|| panic!(
                            "codegen validate() should return Err:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                            test.case, test.description,
                            pretty_json(&test.schema), pretty_json(&test.data),
                        ));
                    let codegen_validation_error = codegen_error
                        .downcast::<jsonschema::ValidationError<'static>>()
                        .expect("codegen validate() error should downcast to ValidationError");
                    let codegen_message = codegen_validation_error.to_string();
                    let codegen_parts = (*codegen_validation_error).into_parts();
                    assert_eq!(
                        codegen_parts.schema_path, error_parts.schema_path,
                        "codegen validate() schema_path mismatch:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                        test.case, test.description,
                        pretty_json(&test.schema), pretty_json(&test.data),
                    );
                    assert_eq!(
                        codegen_parts.instance_path, error_parts.instance_path,
                        "codegen validate() instance_path mismatch:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                        test.case, test.description,
                        pretty_json(&test.schema), pretty_json(&test.data),
                    );
                    assert_eq!(
                        codegen_parts.evaluation_path, error_parts.evaluation_path,
                        "codegen validate() evaluation_path mismatch:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                        test.case, test.description,
                        pretty_json(&test.schema), pretty_json(&test.data),
                    );
                    assert_eq!(
                        codegen_parts.absolute_keyword_location, error_parts.absolute_keyword_location,
                        "codegen validate() absolute_keyword_location mismatch:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                        test.case, test.description,
                        pretty_json(&test.schema), pretty_json(&test.data),
                    );
                    assert_eq!(
                        codegen_message, runtime_message,
                        "codegen validate() message mismatch:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                        test.case, test.description,
                        pretty_json(&test.schema), pretty_json(&test.data),
                    );

                    // Compare codegen iter_errors() against runtime iter_errors()
                    let runtime_iter: Vec<ErrorReport> = validator
                        .iter_errors(&test.data)
                        .map(|e| ErrorReport {
                            message: e.to_string(),
                            schema_path: e.schema_path().to_string(),
                            evaluation_path: e.evaluation_path().to_string(),
                            absolute_keyword_location: e
                                .absolute_keyword_location()
                                .map(|uri| uri.as_str().to_string()),
                            instance_path: e.instance_path().to_string(),
                        })
                        .collect();
                    let codegen_iter = codegen_validator.iter_errors(&test.data);
                    assert_eq!(
                        codegen_iter,
                        runtime_iter,
                        "codegen iter_errors() mismatch:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                        test.case,
                        test.description,
                        pretty_json(&test.schema),
                        pretty_json(&test.data),
                    );
                }

                let evaluation = validator.evaluate(&test.data);
                assert!(
                    !evaluation.flag().valid,
                    "Evaluation output should be invalid:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                    test.case,
                    test.description,
                    pretty_json(&test.schema),
                    pretty_json(&test.data),
                );
                let _ =
                    serde_json::to_value(evaluation.list()).expect("List output should serialize");
                let _ = serde_json::to_value(evaluation.hierarchical())
                    .expect("Hierarchical output should serialize");
            }
            if let Some(codegen_validator) = codegen_validator.as_ref() {
                let generated = codegen_validator.evaluate(&test.data);
                let evaluation = validator.evaluate(&test.data);
                assert_eq!(
                    generated,
                    (
                        serde_json::to_value(evaluation.list())
                            .expect("List output should serialize"),
                        serde_json::to_value(evaluation.hierarchical())
                            .expect("Hierarchical output should serialize"),
                    ),
                    "codegen evaluate() mismatch:\nCase: {}\nTest: {}\nSchema: {}\nInstance: {}",
                    test.case,
                    test.description,
                    pretty_json(&test.schema),
                    pretty_json(&test.data),
                );
            }
        }
    }

    // A case's `externalSchemas`, served at their retrieval URIs.
    struct ExternalSchemas(Vec<(&'static str, serde_json::Value)>);

    impl Retrieve for ExternalSchemas {
        fn retrieve(
            &self,
            uri: &Uri<String>,
        ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
            self.0
                .iter()
                .find(|(known, _)| *known == uri.as_str())
                .map(|(_, schema)| schema.clone())
                .ok_or_else(|| format!("Unknown remote: {uri}").into())
        }
    }

    fn pretty_json(v: &serde_json::Value) -> String {
        serde_json::to_string_pretty(v).expect("Failed to format JSON")
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn test_instance_path() {
        let expectations: serde_json::Value =
            serde_json::from_str(include_str!("draft7_instance_paths.json")).expect("Valid JSON");
        for (filename, expected) in expectations.as_object().expect("Is object") {
            let test_file = fs::read_to_string(format!("tests/suite/validation/tests/{filename}"))
                .unwrap_or_else(|_| panic!("Valid file: {filename}"));
            let file: serde_json::Value = serde_json::from_str(&test_file).expect("Valid JSON");
            let data = &file["tests"];
            for item in expected.as_array().expect("Is array") {
                let suite_id = usize::try_from(item["suite_id"].as_u64().expect("Is integer"))
                    .expect("suite_id fits in usize");
                let schema = &data[suite_id]["schema"];
                let external_schemas = data[suite_id]["externalSchemas"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(uri, schema)| (&*uri.clone().leak(), schema.clone()))
                    .collect();
                let validator = jsonschema::options()
                    .with_draft(Draft::Draft7)
                    .with_retriever(ExternalSchemas(external_schemas))
                    .build(schema)
                    .unwrap_or_else(|_| {
                        panic!(
                    "Valid schema. File: {filename}; Suite ID: {suite_id}; Schema: {schema}",
                )
                    });
                for test_data in item["tests"].as_array().expect("Valid array") {
                    let test_id = usize::try_from(test_data["id"].as_u64().expect("Is integer"))
                        .expect("test_id fits in usize");
                    let mut expected_instance_path = String::new();

                    for segment in test_data["instance_path"].as_array().expect("Valid array") {
                        expected_instance_path.push('/');
                        expected_instance_path.push_str(segment.as_str().expect("A string"));
                    }
                    let instance = &data[suite_id]["tests"][test_id]["data"];
                    let errors: Vec<_> = validator.iter_errors(instance).collect();
                    assert!(
                        !errors.is_empty(),
                        "\nFile: {}\nSuite: {}\nTest: {}",
                        filename,
                        data[suite_id]["description"],
                        data[suite_id]["tests"][test_id]["description"],
                    );

                    let mut found_paths = Vec::with_capacity(errors.len());
                    let mut matched = false;
                    for error in errors {
                        let actual_path = error.instance_path().as_str().to_string();
                        found_paths.push(actual_path.clone());
                        if actual_path == expected_instance_path {
                            matched = true;
                        }
                    }
                    assert!(
                        matched,
                        "\nFile: {}\nSuite: {}\nTest: {}\nExpected path: {}\nFound paths: {:?}",
                        filename,
                        data[suite_id]["description"],
                        data[suite_id]["tests"][test_id]["description"],
                        expected_instance_path,
                        found_paths
                    );
                }
            }
        }
    }

    fn should_skip_draft(draft: &str) -> bool {
        if let Some(filter) = allowed_draft_filter() {
            for entry in filter.split(',') {
                if entry.trim().eq_ignore_ascii_case(draft) {
                    return false;
                }
            }
            true
        } else {
            false
        }
    }

    fn allowed_draft_filter() -> Option<String> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Ok(value) = env::var("JSONSCHEMA_SUITE_DRAFT_FILTER") {
            return Some(value);
        }
        option_env!("JSONSCHEMA_SUITE_DRAFT_FILTER").map(str::to_string)
    }
}

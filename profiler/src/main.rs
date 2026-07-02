use jsonschema::Registry;
use referencing::{Draft, SPECIFICATIONS};
use serde_json::Value;
use std::fs;

#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

struct Args {
    iterations: usize,
    schema_path: String,
    instance_path: Option<String>,
    draft: Option<Draft>,
    method: String,
    /// When set, `schema_path` is a JSON catalog and this key selects the entry.
    catalog_key: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut pico_args = pico_args::Arguments::from_env();

    let preset = pico_args.value_from_str::<_, String>("--preset").ok();
    let (schema_path, instance_path, draft, preset_catalog_key) = if let Some(preset) = preset {
        match preset.as_str() {
            "openapi" => (
                "../crates/benchmark/data/openapi.json".to_string(),
                Some("../crates/benchmark/data/zuora.json".to_string()),
                Some(Draft::Draft4),
                None,
            ),
            "swagger" => (
                "../crates/benchmark/data/swagger.json".to_string(),
                Some("../crates/benchmark/data/kubernetes.json".to_string()),
                Some(Draft::Draft4),
                None,
            ),
            "geojson" => (
                "../crates/benchmark/data/geojson.json".to_string(),
                Some("../crates/benchmark/data/canada.json".to_string()),
                Some(Draft::Draft4),
                None,
            ),
            "citm" => (
                "../crates/benchmark/data/citm_catalog_schema.json".to_string(),
                Some("../crates/benchmark/data/citm_catalog.json".to_string()),
                Some(Draft::Draft4),
                None,
            ),
            "fast-valid" => (
                "../crates/benchmark/data/fast_schema.json".to_string(),
                Some("../crates/benchmark/data/fast_valid.json".to_string()),
                Some(Draft::Draft7),
                None,
            ),
            "fast-invalid" => (
                "../crates/benchmark/data/fast_schema.json".to_string(),
                Some("../crates/benchmark/data/fast_invalid.json".to_string()),
                Some(Draft::Draft7),
                None,
            ),
            "fhir" => (
                "../crates/benchmark/data/fhir.schema.json".to_string(),
                None,
                None,
                None,
            ),
            "azure-pipelines" => (
                "../crates/jsonschema/tests/fixtures/schemastore/corpus-schemastore-catalog.json"
                    .to_string(),
                None,
                None,
                Some("Azure Pipelines YAML pipelines definition".to_string()),
            ),
            _ => return Err(format!("Unknown preset: {}. Available: openapi, swagger, geojson, citm, fast-valid, fast-invalid, fhir, azure-pipelines", preset).into()),
        }
    } else {
        let schema_path = pico_args
            .value_from_str("--schema")
            .map_err(|_| "--schema is required when not using --preset")?;
        let instance_path = pico_args.value_from_str("--instance").ok();
        (schema_path, instance_path, None, None)
    };

    // `--catalog-key` overrides any preset key.
    let catalog_key = pico_args
        .value_from_str::<_, String>("--catalog-key")
        .ok()
        .or(preset_catalog_key);

    let args = Args {
        iterations: pico_args.value_from_str("--iterations")?,
        schema_path,
        instance_path,
        draft,
        method: pico_args.value_from_str("--method")?,
        catalog_key,
    };

    let remaining = pico_args.finish();
    if !remaining.is_empty() {
        return Err(format!("Unknown arguments: {:?}", remaining).into());
    }

    let schema_str = fs::read_to_string(&args.schema_path)?;
    let raw: Value = serde_json::from_str(&schema_str)?;
    let schema: Value = if let Some(key) = &args.catalog_key {
        raw.get(key)
            .ok_or_else(|| format!("Catalog key not found: {key}"))?
            .clone()
    } else {
        raw
    };
    let draft = args
        .draft
        .unwrap_or_else(|| Draft::default().detect(&schema));

    // To initialise metaschema validators
    let _ = &*SPECIFICATIONS;

    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    match args.method.as_str() {
        "build" => {
            for _ in 0..args.iterations {
                let _ = jsonschema::validator_for(&schema)?;
            }
        }
        "canonicalize" => {
            for _ in 0..args.iterations {
                let _ = jsonschema::canonicalize(&schema);
            }
        }
        "registry" => {
            for _ in 0..args.iterations {
                let _ = Registry::new()
                    .extend([("http://example.com/schema", &schema)])
                    .expect("Invalid resource")
                    .prepare()
                    .expect("Failed to build registry");
            }
        }
        "registry-owned" => {
            for _ in 0..args.iterations {
                let resource = draft.create_resource(schema.clone());
                let _ = Registry::new()
                    .add("http://example.com/schema", resource)
                    .expect("Invalid resource")
                    .prepare()
                    .expect("Failed to build registry");
            }
        }
        "is_valid" | "validate" | "iter_errors" | "evaluate" => {
            let instance_path = args
                .instance_path
                .as_ref()
                .ok_or("--instance or --preset required for this method")?;
            let instance_str = fs::read_to_string(instance_path)?;
            let instance: Value = serde_json::from_str(&instance_str)?;
            let validator = jsonschema::validator_for(&schema)?;

            match args.method.as_str() {
                "is_valid" => {
                    for _ in 0..args.iterations {
                        let _ = validator.is_valid(&instance);
                    }
                }
                "validate" => {
                    for _ in 0..args.iterations {
                        let _ = validator.validate(&instance);
                    }
                }
                "iter_errors" => {
                    for _ in 0..args.iterations {
                        for _error in validator.iter_errors(&instance) {}
                    }
                }
                "evaluate" => {
                    for _ in 0..args.iterations {
                        let evaluation = validator.evaluate(&instance);
                        let _ = evaluation.flag();
                        let _ = serde_json::to_value(evaluation.list())
                            .expect("Failed to serialize list output");
                        let _ = serde_json::to_value(evaluation.hierarchical())
                            .expect("Failed to serialize hierarchical output");
                    }
                }
                _ => unreachable!(),
            }
        }
        _ => {
            return Err(
                "Invalid method. Use 'registry', 'registry-owned', 'build', 'canonicalize', 'is_valid', 'validate', 'iter_errors', or 'evaluate'"
                    .into(),
            );
        }
    }

    Ok(())
}

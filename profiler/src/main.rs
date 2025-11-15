use referencing::{Draft, Registry};
use serde_json::Value;
use std::fs;

#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

struct Args {
    iterations: usize,
    schema_path: String,
    instance_path: Option<String>,
    method: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut pico_args = pico_args::Arguments::from_env();

    // Handle presets
    let preset = pico_args.value_from_str::<_, String>("--preset").ok();
    let (schema_path, instance_path) = if let Some(preset) = preset {
        match preset.as_str() {
            "openapi" => ("../crates/benchmark/data/openapi.json".to_string(), Some("../crates/benchmark/data/zuora.json".to_string())),
            "swagger" => ("../crates/benchmark/data/swagger.json".to_string(), Some("../crates/benchmark/data/kubernetes.json".to_string())),
            "geojson" => ("../crates/benchmark/data/geojson.json".to_string(), Some("../crates/benchmark/data/canada.json".to_string())),
            "citm" => ("../crates/benchmark/data/citm_catalog_schema.json".to_string(), Some("../crates/benchmark/data/citm_catalog.json".to_string())),
            "fast-valid" => ("../crates/benchmark/data/fast_schema.json".to_string(), Some("../crates/benchmark/data/fast_valid.json".to_string())),
            "fast-invalid" => ("../crates/benchmark/data/fast_schema.json".to_string(), Some("../crates/benchmark/data/fast_invalid.json".to_string())),
            _ => return Err(format!("Unknown preset: {}. Available: openapi, swagger, geojson, citm, fast-valid, fast-invalid", preset).into()),
        }
    } else {
        let schema_path = pico_args
            .value_from_str("--schema")
            .map_err(|_| "--schema is required when not using --preset")?;
        let instance_path = pico_args.value_from_str("--instance").ok();
        (schema_path, instance_path)
    };

    let args = Args {
        iterations: pico_args.value_from_str("--iterations")?,
        schema_path,
        instance_path,
        method: pico_args.value_from_str("--method")?,
    };

    // Check for unknown arguments
    let remaining = pico_args.finish();
    if !remaining.is_empty() {
        return Err(format!("Unknown arguments: {:?}", remaining).into());
    }

    let schema_str = fs::read_to_string(&args.schema_path)?;
    let schema: Value = serde_json::from_str(&schema_str)?;

    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    match args.method.as_str() {
        "build" => {
            if args.instance_path.is_some() {
                return Err("--instance is not supported for 'build' method".into());
            }
            for _ in 0..args.iterations {
                let _ = jsonschema::validator_for(&schema)?;
            }
        }
        "registry" => {
            if args.instance_path.is_some() {
                return Err("--instance is not supported for 'registry' method".into());
            }
            for _ in 0..args.iterations {
                let input_resources = vec![(
                    "http://example.com/schema",
                    Draft::Draft202012.create_resource(schema.clone()),
                )];
                let _ = Registry::try_from_resources(input_resources.into_iter());
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
                "Invalid method. Use 'registry', 'build', 'is_valid', 'validate', 'iter_errors', or 'evaluate'".into()
            );
        }
    }

    Ok(())
}

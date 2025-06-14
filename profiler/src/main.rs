use referencing::{Draft, Registry};
use serde_json::Value;
use std::fs;

#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

struct Args {
    iterations: usize,
    schema_path: String,
    instance_path: String,
    method: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = pico_args::Arguments::from_env();
    let args = Args {
        iterations: args.value_from_str("--iterations")?,
        schema_path: args.value_from_str("--schema")?,
        instance_path: args.value_from_str("--instance")?,
        method: args.value_from_str("--method")?,
    };

    let schema_str = fs::read_to_string(&args.schema_path)?;
    let instance_str = fs::read_to_string(&args.instance_path)?;

    let schema: Value = serde_json::from_str(&schema_str)?;
    let instance: Value = serde_json::from_str(&instance_str)?;

    let schema = serde_json::json!({
        "properties": {
            "name": {"maxLength": 3},
            "child": {"$ref": "#"}
        }
    });
    let instance = serde_json::json!({
        "name": "Bob",
        "child": {
            "name": "Ann",
            "child": {
                "name": "Joe",
                "child": {
                    "name": "Sam",
                    "child": {
                        "name": "Max",
                        "child": {
                            "name": "Eve",
                            "child": {
                                "name": "Roy",
                                "child": {
                                    "name": "Zoe",
                                    "child": {
                                        "name": "Leo",
                                        "child": {
                                            "name": "Amy"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    let config = jsonschema::options();
    let validator2 = jsonschema::compiler_v2::build(config, &schema);
    let validator = jsonschema::validator_for(&schema)?;

    let input_resources = vec![(
        "http://example.com/schema",
        Draft::Draft202012.create_resource(schema.clone()),
    )];

    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();
    for _ in 0..args.iterations {
        match args.method.as_str() {
            "build" => {
                let _ = jsonschema::validator_for(&schema)?;
            }
            "is_valid" => {
                let _ = validator2.is_valid(&instance);
            }
            "validate" => {
                let _ = validator.validate(&instance);
            }
            "iter_errors" => for _error in validator.iter_errors(&instance) {},
            "apply" => {
                let _ = validator.apply(&instance).basic();
            }
            "registry" => {
                let _ = Registry::try_from_resources(input_resources.into_iter());
                break;
            }
            _ => panic!(
                "Invalid method. Use 'registry', 'build', 'is_valid', 'validate', 'iter_errors`, or 'apply'"
            ),
        }
    }

    Ok(())
}

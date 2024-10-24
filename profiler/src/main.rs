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

    let validator = jsonschema::validator_for(&schema)?;

    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();
    for _ in 0..args.iterations {
        match args.method.as_str() {
            "build" => {
                let _ = jsonschema::validator_for(&schema)?;
            }
            "is_valid" => {
                let _ = validator.is_valid(&instance);
            }
            "validate" => {
                let _ = validator.validate(&instance);
            }
            "iter_errors" => for _error in validator.iter_errors(&instance) {},
            "apply" => {
                let _ = validator.apply(&instance).basic();
            }
            _ => panic!(
                "Invalid method. Use 'build', 'is_valid', 'validate', 'iter_errors`, or 'apply'"
            ),
        }
    }

    Ok(())
}

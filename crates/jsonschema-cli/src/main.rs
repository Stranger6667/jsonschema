#![allow(clippy::print_stdout)]
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::Parser;

#[derive(Parser)]
#[command(name = "jsonschema")]
struct Cli {
    /// A path to a JSON instance (i.e. filename.json) to validate (may be specified multiple times).
    #[arg(short = 'i', long = "instance")]
    instances: Option<Vec<PathBuf>>,

    /// The JSON Schema to validate with (i.e. schema.json).
    #[arg(value_parser, required_unless_present("version"))]
    schema: Option<PathBuf>,

    /// Show program's version number and exit.
    #[arg(short = 'v', long = "version")]
    version: bool,
}

fn read_json(
    path: &Path,
) -> Result<serde_json::Result<serde_json::Value>, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    Ok(serde_json::from_reader(reader))
}

fn validate_instances(
    instances: &[PathBuf],
    schema_path: &Path,
) -> Result<bool, Box<dyn std::error::Error>> {
    let mut success = true;

    let schema_json = read_json(schema_path)??;
    match jsonschema::validator_for(&schema_json) {
        Ok(validator) => {
            for instance in instances {
                let instance_json = read_json(instance)??;
                let errors = validator.iter_errors(&instance_json).collect::<Vec<_>>();
                let filename = instance.to_string_lossy();
                if errors.is_empty() {
                    println!("{filename} - VALID");
                } else {
                    success = false;

                    println!("{filename} - INVALID. Errors:");
                    for (i, e) in errors.iter().enumerate() {
                        println!("{}. {}", i + 1, e);
                    }
                }
            }
        }
        Err(error) => {
            println!("Schema is invalid. Error: {error}");
            success = false;
        }
    }
    Ok(success)
}

fn main() -> ExitCode {
    let config = Cli::parse();

    if config.version {
        println!(concat!("Version: ", env!("CARGO_PKG_VERSION")));
        return ExitCode::SUCCESS;
    }

    if let Some(schema) = config.schema {
        if let Some(instances) = config.instances {
            return match validate_instances(&instances, &schema) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::FAILURE,
                Err(error) => {
                    println!("Error: {error}");
                    ExitCode::FAILURE
                }
            };
        }
    }
    ExitCode::SUCCESS
}

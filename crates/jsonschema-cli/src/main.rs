#![allow(clippy::print_stdout)]
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::Parser;
use percent_encoding::{percent_encode, AsciiSet, CONTROLS};

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

fn path_to_uri(path: &std::path::Path) -> String {
    const SEGMENT: &AsciiSet = &CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'<')
        .add(b'>')
        .add(b'`')
        .add(b'#')
        .add(b'?')
        .add(b'{')
        .add(b'}')
        .add(b'/')
        .add(b'%');

    let mut result = "file://".to_owned();

    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::ffi::OsStrExt;

        const CUSTOM_SEGMENT: &AsciiSet = &SEGMENT.add(b'\\');
        for component in path.components().skip(1) {
            result.push('/');
            result.extend(percent_encode(
                component.as_os_str().as_bytes(),
                CUSTOM_SEGMENT,
            ));
        }
    }
    #[cfg(target_os = "windows")]
    {
        use std::path::{Component, Prefix};
        let mut components = path.components();

        match components.next() {
            Some(Component::Prefix(ref p)) => match p.kind() {
                Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                    result.push('/');
                    result.push(letter as char);
                    result.push(':');
                }
                _ => panic!("Unexpected path"),
            },
            _ => panic!("Unexpected path"),
        }

        for component in components {
            if component == Component::RootDir {
                continue;
            }

            let component = component.as_os_str().to_str().expect("Unexpected path");

            result.push('/');
            result.extend(percent_encode(component.as_bytes(), SEGMENT));
        }
    }
    result
}

fn validate_instances(
    instances: &[PathBuf],
    schema_path: &Path,
) -> Result<bool, Box<dyn std::error::Error>> {
    let mut success = true;

    let schema_json = read_json(schema_path)??;
    let base_uri = path_to_uri(schema_path);
    let base_uri = referencing::uri::from_str(&base_uri)?;
    match jsonschema::options()
        .with_base_uri(base_uri)
        .build(&schema_json)
    {
        Ok(validator) => {
            for instance in instances {
                let instance_json = read_json(instance)??;
                let mut errors = validator.iter_errors(&instance_json);
                let filename = instance.to_string_lossy();
                if let Some(first) = errors.next() {
                    success = false;
                    println!("{filename} - INVALID. Errors:");
                    println!("1. {first}");
                    for (i, error) in errors.enumerate() {
                        println!("{}. {error}", i + 2);
                    }
                } else {
                    println!("{filename} - VALID");
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

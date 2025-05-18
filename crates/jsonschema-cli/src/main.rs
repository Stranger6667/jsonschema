#![allow(clippy::print_stdout)]
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{ArgAction, Parser, ValueEnum};
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

    /// Which JSON Schema draft to enforce.
    #[arg(
        short = 'd',
        long = "draft",
        value_enum,
        help = "Enforce a specific JSON Schema draft"
    )]
    draft: Option<Draft>,

    /// Enable validation of `format` keywords.
    #[arg(
        long = "assert-format",
        action = ArgAction::SetTrue,
        overrides_with = "no_assert_format",
        help = "Turn ON format validation"
    )]
    assert_format: Option<bool>,

    /// Disable validation of `format` keywords.
    #[arg(
        long = "no-assert-format",
        action = ArgAction::SetTrue,
        overrides_with = "assert_format",
        help = "Turn OFF format validation"
    )]
    no_assert_format: Option<bool>,

    /// Show program's version number and exit.
    #[arg(short = 'v', long = "version")]
    version: bool,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum Draft {
    #[clap(name = "4")]
    Draft4,
    #[clap(name = "6")]
    Draft6,
    #[clap(name = "7")]
    Draft7,
    #[clap(name = "2019")]
    Draft201909,
    #[clap(name = "2020")]
    Draft202012,
}

impl From<Draft> for jsonschema::Draft {
    fn from(d: Draft) -> jsonschema::Draft {
        match d {
            Draft::Draft4 => jsonschema::Draft::Draft4,
            Draft::Draft6 => jsonschema::Draft::Draft6,
            Draft::Draft7 => jsonschema::Draft::Draft7,
            Draft::Draft201909 => jsonschema::Draft::Draft201909,
            Draft::Draft202012 => jsonschema::Draft::Draft202012,
        }
    }
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

    let path = path.canonicalize().expect("Failed to canonicalise path");

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
    draft: Option<Draft>,
    assert_format: Option<bool>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let mut success = true;

    let schema_json = read_json(schema_path)??;
    let base_uri = path_to_uri(schema_path);
    let base_uri = referencing::uri::from_str(&base_uri)?;
    let mut options = jsonschema::options().with_base_uri(base_uri);
    if let Some(draft) = draft {
        options = options.with_draft(draft.into());
    }
    if let Some(assert_format) = assert_format {
        options = options.should_validate_formats(assert_format);
    }
    match options.build(&schema_json) {
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
            // - Some(true)  if --assert-format
            // - Some(false) if --no-assert-format
            // - None        if neither (use builder’s default)
            let assert_format = config.assert_format.or(config.no_assert_format);
            return match validate_instances(&instances, &schema, config.draft, assert_format) {
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

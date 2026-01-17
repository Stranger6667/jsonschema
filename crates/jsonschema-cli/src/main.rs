#![allow(clippy::print_stdout)]
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use clap::{ArgAction, Parser, ValueEnum};
use percent_encoding::{percent_encode, AsciiSet, CONTROLS};
use serde_json::json;

fn parse_non_negative_timeout(s: &str) -> Result<f64, String> {
    let value: f64 = s
        .parse()
        .map_err(|_| format!("'{s}' is not a valid number"))?;
    if value < 0.0 || value.is_nan() || value.is_infinite() {
        return Err("must be a non-negative finite number".to_string());
    }
    Ok(value)
}

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

    /// Select the output format (text, flag, list, hierarchical). All modes emit newline-delimited JSON records.
    #[arg(
        long = "output",
        value_enum,
        default_value_t = Output::Text,
        help = "Select output style: text (default), flag, list, hierarchical"
    )]
    output: Output,

    /// Show program's version number and exit.
    #[arg(short = 'v', long = "version")]
    version: bool,

    /// Only output validation failures, suppress successful validations.
    #[arg(long = "errors-only", help = "Only show validation errors")]
    errors_only: bool,

    /// Timeout for the connect phase (in seconds).
    #[arg(
        long = "connect-timeout",
        value_name = "SECONDS",
        value_parser = parse_non_negative_timeout,
        help = "Timeout for establishing connections (in seconds)"
    )]
    connect_timeout: Option<f64>,

    /// Total request timeout (in seconds).
    #[arg(
        long = "timeout",
        value_name = "SECONDS",
        value_parser = parse_non_negative_timeout,
        help = "Total timeout for HTTP requests (in seconds)"
    )]
    timeout: Option<f64>,

    /// Skip TLS certificate verification (insecure).
    #[arg(
        short = 'k',
        long = "insecure",
        action = ArgAction::SetTrue,
        help = "Skip TLS certificate verification (dangerous!)"
    )]
    insecure: bool,

    /// Path to a custom CA certificate file (PEM format).
    #[arg(
        long = "cacert",
        value_name = "FILE",
        help = "Path to a custom CA certificate file (PEM format)"
    )]
    cacert: Option<PathBuf>,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum Output {
    Text,
    Flag,
    List,
    Hierarchical,
}

impl Output {
    fn as_str(self) -> &'static str {
        match self {
            Output::Text => "text",
            Output::Flag => "flag",
            Output::List => "list",
            Output::Hierarchical => "hierarchical",
        }
    }
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

fn build_http_options(config: &Cli) -> jsonschema::HttpOptions {
    let mut http_options = jsonschema::HttpOptions::new();

    if let Some(connect_timeout) = config.connect_timeout {
        http_options = http_options.connect_timeout(Duration::from_secs_f64(connect_timeout));
    }
    if let Some(timeout) = config.timeout {
        http_options = http_options.timeout(Duration::from_secs_f64(timeout));
    }
    if config.insecure {
        http_options = http_options.danger_accept_invalid_certs(true);
    }
    if let Some(ref cacert) = config.cacert {
        http_options = http_options.add_root_certificate(cacert);
    }

    http_options
}

fn has_http_options(config: &Cli) -> bool {
    config.connect_timeout.is_some()
        || config.timeout.is_some()
        || config.insecure
        || config.cacert.is_some()
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

fn output_schema_validation(
    schema_path: &Path,
    schema_json: &serde_json::Value,
    output: Output,
    errors_only: bool,
    http_options: Option<&jsonschema::HttpOptions>,
) -> Result<bool, Box<dyn std::error::Error>> {
    // First validate against meta-schema
    let meta_validator = jsonschema::meta::validator_for(schema_json)?;
    let evaluation = meta_validator.evaluate(schema_json);
    let flag_output = evaluation.flag();

    // If meta-schema validation passed, also try to build the validator
    // to check that all referenced schemas are valid
    if flag_output.valid {
        let base_uri = path_to_uri(schema_path);
        let base_uri = referencing::uri::from_str(&base_uri)?;
        // Just try to build - if it fails, the error propagates naturally
        let mut options = jsonschema::options().with_base_uri(base_uri);
        if let Some(http_opts) = http_options {
            options = options.with_http_options(http_opts)?;
        }
        options.build(schema_json)?;
    }

    // Skip valid schemas if errors_only is enabled
    if !(errors_only && flag_output.valid) {
        let schema_display = schema_path.to_string_lossy().to_string();
        let output_format = output.as_str();

        let payload = match output {
            Output::Text => unreachable!("text mode should not call this function"),
            Output::Flag => serde_json::to_value(flag_output)?,
            Output::List => serde_json::to_value(evaluation.list())?,
            Output::Hierarchical => serde_json::to_value(evaluation.hierarchical())?,
        };

        let record = json!({
            "output": output_format,
            "schema": &schema_display,
            "payload": payload,
        });
        println!("{}", serde_json::to_string(&record)?);
    }

    Ok(flag_output.valid)
}

fn validate_schema_meta(
    schema_path: &Path,
    output: Output,
    errors_only: bool,
    http_options: Option<&jsonschema::HttpOptions>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let schema_json = read_json(schema_path)??;

    if matches!(output, Output::Text) {
        // Text output mode
        // First validate the schema structure against its meta-schema
        if let Err(error) = jsonschema::meta::validate(&schema_json) {
            println!("Schema is invalid. Error: {error}");
            return Ok(false);
        }

        // Then try to build a validator to check that all referenced schemas are also valid
        let base_uri = path_to_uri(schema_path);
        let base_uri = referencing::uri::from_str(&base_uri)?;
        let mut options = jsonschema::options().with_base_uri(base_uri);
        if let Some(http_opts) = http_options {
            options = options.with_http_options(http_opts)?;
        }
        match options.build(&schema_json) {
            Ok(_) => {
                if !errors_only {
                    println!("Schema is valid");
                }
                Ok(true)
            }
            Err(error) => {
                println!("Schema is invalid. Error: {error}");
                Ok(false)
            }
        }
    } else {
        // Structured output modes using evaluate API
        output_schema_validation(schema_path, &schema_json, output, errors_only, http_options)
    }
}

fn validate_instances(
    instances: &[PathBuf],
    schema_path: &Path,
    draft: Option<Draft>,
    assert_format: Option<bool>,
    output: Output,
    errors_only: bool,
    http_options: Option<&jsonschema::HttpOptions>,
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
    if let Some(http_opts) = http_options {
        options = options.with_http_options(http_opts)?;
    }
    match options.build(&schema_json) {
        Ok(validator) => {
            if matches!(output, Output::Text) {
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
                    } else if !errors_only {
                        println!("{filename} - VALID");
                    }
                }
            } else {
                let schema_display = schema_path.to_string_lossy().to_string();
                let output_format = output.as_str();
                for instance in instances {
                    let instance_json = read_json(instance)??;
                    let evaluation = validator.evaluate(&instance_json);
                    let flag_output = evaluation.flag();

                    // Skip valid instances if errors_only is enabled
                    if errors_only && flag_output.valid {
                        continue;
                    }

                    let payload = match output {
                        Output::Text => unreachable!("handled above"),
                        Output::Flag => serde_json::to_value(flag_output)?,
                        Output::List => serde_json::to_value(evaluation.list())?,
                        Output::Hierarchical => serde_json::to_value(evaluation.hierarchical())?,
                    };

                    let instance_display = instance.to_string_lossy();
                    let record = json!({
                        "output": output_format,
                        "schema": &schema_display,
                        "instance": instance_display,
                        "payload": payload,
                    });
                    println!("{}", serde_json::to_string(&record)?);

                    if !flag_output.valid {
                        success = false;
                    }
                }
            }
        }
        Err(error) => {
            if matches!(output, Output::Text) {
                println!("Schema is invalid. Error: {error}");
            } else {
                // Schema compilation failed - validate the schema itself to get structured output
                output_schema_validation(
                    schema_path,
                    &schema_json,
                    output,
                    errors_only,
                    http_options,
                )?;
            }
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

    if let Some(ref schema) = config.schema {
        // Build HTTP options if any HTTP-related flags are provided
        let http_options = if has_http_options(&config) {
            Some(build_http_options(&config))
        } else {
            None
        };

        if let Some(instances) = config.instances {
            // - Some(true)  if --assert-format
            // - Some(false) if --no-assert-format
            // - None        if neither (use builder's default)
            let assert_format = config.assert_format.or(config.no_assert_format);
            return match validate_instances(
                &instances,
                schema,
                config.draft,
                assert_format,
                config.output,
                config.errors_only,
                http_options.as_ref(),
            ) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::FAILURE,
                Err(error) => {
                    println!("Error: {error}");
                    ExitCode::FAILURE
                }
            };
        }
        // No instances provided - validate the schema itself
        return match validate_schema_meta(
            schema,
            config.output,
            config.errors_only,
            http_options.as_ref(),
        ) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(error) => {
                println!("Error: {error}");
                ExitCode::FAILURE
            }
        };
    }
    ExitCode::SUCCESS
}

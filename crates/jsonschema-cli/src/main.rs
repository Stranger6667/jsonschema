#![allow(clippy::print_stdout, clippy::print_stderr)]
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
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

fn parse_resource_pair(s: &str) -> Result<(String, PathBuf), String> {
    let (uri, path) = s
        .split_once('=')
        .ok_or_else(|| format!("expected URI=FILE, got '{s}'"))?;
    Ok((uri.to_string(), PathBuf::from(path)))
}

#[derive(Parser)]
#[command(name = "jsonschema")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    // Hidden top-level flags for deprecated flat invocation (emits a warning, use `check` instead)
    #[arg(hide = true, value_parser)]
    schema: Option<PathBuf>,
    #[arg(hide = true, short = 'i', long = "instance", num_args = 1..)]
    instances: Option<Vec<PathBuf>>,
    #[arg(hide = true, short = 'd', long = "draft", value_enum)]
    draft: Option<Draft>,
    #[arg(
        hide = true,
        long = "assert-format",
        action = ArgAction::SetTrue,
        overrides_with = "no_assert_format"
    )]
    assert_format: Option<bool>,
    #[arg(
        hide = true,
        long = "no-assert-format",
        action = ArgAction::SetTrue,
        overrides_with = "assert_format"
    )]
    no_assert_format: Option<bool>,
    #[arg(hide = true, long = "output", value_enum, default_value_t = Output::Text)]
    output: Output,
    /// Show program's version number and exit.
    #[arg(short = 'v', long = "version")]
    version: bool,
    #[arg(hide = true, long = "errors-only")]
    errors_only: bool,
    #[arg(
        hide = true,
        long = "connect-timeout",
        value_name = "SECONDS",
        value_parser = parse_non_negative_timeout
    )]
    connect_timeout: Option<f64>,
    #[arg(
        hide = true,
        long = "timeout",
        value_name = "SECONDS",
        value_parser = parse_non_negative_timeout
    )]
    timeout: Option<f64>,
    #[arg(hide = true, short = 'k', long = "insecure", action = ArgAction::SetTrue)]
    insecure: bool,
    #[arg(hide = true, long = "cacert", value_name = "FILE")]
    cacert: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Validate JSON instances against a JSON Schema.
    Validate(ValidateArgs),
    /// Bundle a JSON Schema into a Compound Schema Document.
    Bundle(BundleArgs),
}

#[derive(Args, Clone)]
struct HttpArgs {
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

#[derive(Args)]
struct ValidateArgs {
    /// The JSON Schema to validate with (i.e. schema.json).
    #[arg(value_parser)]
    schema: PathBuf,

    /// A path to a JSON instance (i.e. filename.json) to validate. May be specified multiple times or with multiple values after a single flag (e.g. `-i a.json b.json`).
    #[arg(short = 'i', long = "instance", num_args = 1..)]
    instances: Option<Vec<PathBuf>>,

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

    /// Only output validation failures, suppress successful validations.
    #[arg(long = "errors-only", help = "Only show validation errors")]
    errors_only: bool,

    #[command(flatten)]
    http: HttpArgs,
}

#[derive(Args)]
struct BundleArgs {
    /// Path to the root JSON Schema file to bundle.
    #[arg(value_parser)]
    schema: PathBuf,

    /// Register an external schema resource: URI=FILE (may be repeated).
    #[arg(long = "resource", value_parser = parse_resource_pair)]
    resources: Vec<(String, PathBuf)>,

    /// Write bundled output to FILE instead of stdout.
    #[arg(short = 'o', long = "output")]
    output: Option<PathBuf>,

    #[command(flatten)]
    http: HttpArgs,
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

impl HttpArgs {
    fn into_http_options(self) -> Option<jsonschema::HttpOptions> {
        if self.connect_timeout.is_none()
            && self.timeout.is_none()
            && !self.insecure
            && self.cacert.is_none()
        {
            return None;
        }

        let mut http_options = jsonschema::HttpOptions::new();

        if let Some(connect_timeout) = self.connect_timeout {
            http_options = http_options.connect_timeout(Duration::from_secs_f64(connect_timeout));
        }
        if let Some(timeout) = self.timeout {
            http_options = http_options.timeout(Duration::from_secs_f64(timeout));
        }
        if self.insecure {
            http_options = http_options.danger_accept_invalid_certs(true);
        }
        if let Some(cacert) = self.cacert.as_ref() {
            http_options = http_options.add_root_certificate(cacert);
        }

        Some(http_options)
    }
}

#[derive(Debug)]
enum ReadJsonError {
    Io {
        file: PathBuf,
        err: std::io::Error,
    },
    Json {
        file: PathBuf,
        err: serde_json::Error,
    },
}

impl std::fmt::Display for ReadJsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Io { file, err } => {
                f.write_fmt(format_args!("failed to read {}: {err}", file.display()))
            }
            Self::Json { file, err } => f.write_fmt(format_args!(
                "failed to parse JSON from {}: {err}",
                file.display()
            )),
        }
    }
}

impl std::error::Error for ReadJsonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { file: _, err } => Some(err),
            Self::Json { file: _, err } => Some(err),
        }
    }
}

fn read_json(path: &Path) -> Result<serde_json::Value, ReadJsonError> {
    let file = File::open(path).map_err(|err| ReadJsonError::Io {
        file: path.into(),
        err,
    })?;
    let reader = BufReader::new(file);
    serde_json::from_reader(reader).map_err(|err| ReadJsonError::Json {
        file: path.into(),
        err,
    })
}

#[derive(Debug)]
enum ReadJsonOrYamlError {
    Json {
        file: PathBuf,
        err: serde_json::Error,
    },
    Yaml {
        file: PathBuf,
        err: serde_saphyr::Error,
    },
}

impl std::fmt::Display for ReadJsonOrYamlError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Json { file, err } => f.write_fmt(format_args!(
                "failed to read JSON from {}: {}",
                file.display(),
                err
            )),
            Self::Yaml { file, err } => f.write_fmt(format_args!(
                "failed to read YAML from {}: {}",
                file.display(),
                err
            )),
        }
    }
}

impl std::error::Error for ReadJsonOrYamlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json { file: _, err } => Some(err),
            Self::Yaml { file: _, err } => Some(err),
        }
    }
}

fn read_json_or_yaml(
    path: &Path,
) -> Result<Result<serde_json::Value, ReadJsonOrYamlError>, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    if let Some(ext) = path.extension() {
        if ext == "yaml" || ext == "yml" {
            return Ok(serde_saphyr::from_reader(reader).map_err(|err| {
                ReadJsonOrYamlError::Yaml {
                    file: path.into(),
                    err,
                }
            }));
        }
    }
    Ok(
        serde_json::from_reader(reader).map_err(|err| ReadJsonOrYamlError::Json {
            file: path.into(),
            err,
        }),
    )
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

fn options_for_schema<'a>(
    schema_path: &Path,
    http_options: Option<&jsonschema::HttpOptions>,
) -> Result<jsonschema::ValidationOptions<'a>, Box<dyn std::error::Error>> {
    let base_uri = path_to_uri(schema_path);
    let base_uri = referencing::uri::from_str(&base_uri)?;
    let mut options = jsonschema::options().with_base_uri(base_uri);
    if let Some(http_opts) = http_options {
        options = options.with_http_options(http_opts)?;
    }
    Ok(options)
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
        // Just try to build - if it fails, the error propagates naturally
        let options = options_for_schema(schema_path, http_options)?;
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
    let schema_json = read_json(schema_path)?;

    if matches!(output, Output::Text) {
        // Text output mode
        // First validate the schema structure against its meta-schema
        if let Err(error) = jsonschema::meta::validate(&schema_json) {
            println!("Schema is invalid. Error: {error}");
            return Ok(false);
        }

        // Then try to build a validator to check that all referenced schemas are also valid
        let options = options_for_schema(schema_path, http_options)?;
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

    let schema_json = read_json(schema_path)?;
    let mut options = options_for_schema(schema_path, http_options)?;
    if let Some(draft) = draft {
        options = options.with_draft(draft.into());
    }
    if let Some(assert_format) = assert_format {
        options = options.should_validate_formats(assert_format);
    }
    match options.build(&schema_json) {
        Ok(validator) => {
            if matches!(output, Output::Text) {
                for instance in instances {
                    let instance_json = read_json_or_yaml(instance)??;
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
                    let instance_json = read_json(instance)?;
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

fn validation_result_to_exit(result: Result<bool, Box<dyn std::error::Error>>) -> ExitCode {
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            println!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn fail_with_error(error: impl std::fmt::Display) -> ExitCode {
    eprintln!("error: {error}");
    ExitCode::FAILURE
}

fn run_validate(args: ValidateArgs) -> ExitCode {
    let ValidateArgs {
        schema,
        instances,
        draft,
        assert_format,
        no_assert_format,
        output,
        errors_only,
        http,
    } = args;

    let http_options = http.into_http_options();

    if let Some(instances) = instances {
        return validation_result_to_exit(validate_instances(
            &instances,
            &schema,
            draft,
            assert_format.or(no_assert_format),
            output,
            errors_only,
            http_options.as_ref(),
        ));
    }

    validation_result_to_exit(validate_schema_meta(
        &schema,
        output,
        errors_only,
        http_options.as_ref(),
    ))
}

fn run_bundle(args: BundleArgs) -> ExitCode {
    let BundleArgs {
        schema,
        resources,
        output,
        http,
    } = args;

    let schema_json = match read_json(&schema) {
        Ok(value) => value,
        Err(error) => return fail_with_error(error),
    };
    let http_options = http.into_http_options();
    let mut opts = match options_for_schema(&schema, http_options.as_ref()) {
        Ok(value) => value,
        Err(error) => return fail_with_error(error),
    };

    let mut registry = if let Some(http_opts) = http_options.as_ref() {
        let retriever = match jsonschema::HttpRetriever::new(http_opts) {
            Ok(retriever) => retriever,
            Err(error) => return fail_with_error(error),
        };
        jsonschema::Registry::new().retriever(retriever)
    } else {
        jsonschema::Registry::new()
    };
    for (uri, path) in &resources {
        let resource_json = match read_json(path) {
            Ok(value) => value,
            Err(error) => return fail_with_error(error),
        };
        registry = match registry.add(uri, resource_json) {
            Ok(registry) => registry,
            Err(error) => return fail_with_error(error),
        };
    }
    let registry = match registry.prepare() {
        Ok(registry) => registry,
        Err(error) => return fail_with_error(error),
    };
    opts = opts.with_registry(&registry);

    match opts.bundle(&schema_json) {
        Ok(bundled) => {
            let json = match serde_json::to_string_pretty(&bundled) {
                Ok(s) => s,
                Err(error) => return fail_with_error(error),
            };
            match output {
                Some(path) => {
                    if let Err(error) = std::fs::write(&path, &json) {
                        return fail_with_error(format!("{}: {error}", path.display()));
                    }
                }
                None => {
                    println!("{json}");
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => fail_with_error(error),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if cli.version {
        println!(concat!("Version: ", env!("CARGO_PKG_VERSION")));
        return ExitCode::SUCCESS;
    }

    match cli.command {
        Some(Command::Validate(args)) => run_validate(args),
        Some(Command::Bundle(args)) => run_bundle(args),
        None => {
            // Flat invocation is deprecated — emit a warning, then proceed as `validate`
            if let Some(schema) = cli.schema {
                eprintln!(
                    "warning: flat invocation is deprecated. Use `jsonschema validate {}` instead.",
                    schema.display()
                );
                run_validate(ValidateArgs {
                    schema,
                    instances: cli.instances,
                    draft: cli.draft,
                    assert_format: cli.assert_format,
                    no_assert_format: cli.no_assert_format,
                    output: cli.output,
                    errors_only: cli.errors_only,
                    http: HttpArgs {
                        connect_timeout: cli.connect_timeout,
                        timeout: cli.timeout,
                        insecure: cli.insecure,
                        cacert: cli.cacert,
                    },
                })
            } else {
                eprintln!("A schema argument is required. Use `jsonschema validate --help` or `jsonschema bundle --help`.");
                ExitCode::FAILURE
            }
        }
    }
}

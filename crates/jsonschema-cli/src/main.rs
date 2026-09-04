#![allow(clippy::print_stdout, clippy::print_stderr)]
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use jsonschema::{
    json::{Node, SerdeJson},
    Retrieve,
};
use percent_encoding::{percent_encode, AsciiSet, CONTROLS};
use serde_json::{json, Value};

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
    /// Dereference a JSON Schema, inlining all $ref targets.
    Dereference(DereferenceArgs),
    /// Canonicalize a JSON Schema to its simplified canonical form.
    Canonicalize(CanonicalizeArgs),
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

    /// Refuse to fetch references that are not already registered.
    #[arg(
        long = "offline",
        action = ArgAction::SetTrue,
        help = "Refuse to fetch references outside --resource"
    )]
    offline: bool,
}

/// How references are resolved: HTTP settings, or a refusal to fetch at all.
#[derive(Clone, Copy)]
struct Retrieval<'a> {
    http: Option<&'a jsonschema::HttpOptions>,
    offline: bool,
}

#[derive(Args)]
struct FormatAssertionArgs {
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
}

impl FormatAssertionArgs {
    const fn from_flags(assert_format: Option<bool>, no_assert_format: Option<bool>) -> Self {
        Self {
            assert_format,
            no_assert_format,
        }
    }

    const fn validate_formats(&self) -> Option<bool> {
        if matches!(self.assert_format, Some(true)) {
            Some(true)
        } else if matches!(self.no_assert_format, Some(true)) {
            Some(false)
        } else {
            None
        }
    }
}

#[derive(Args)]
struct ValidateArgs {
    /// The JSON Schema to validate with (i.e. schema.json). When omitted, each instance's own
    /// `$schema` property names the schema to validate it against.
    #[arg(value_parser)]
    schema: Option<PathBuf>,

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

    #[command(flatten)]
    format: FormatAssertionArgs,

    /// Declare support for a vocabulary this tool does not implement (may be repeated).
    #[arg(
        long = "vocabulary",
        value_name = "URI",
        help = "Declare support for a vocabulary required by the meta-schema (may be repeated)"
    )]
    vocabularies: Vec<String>,

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

#[derive(Args)]
struct DereferenceArgs {
    /// Path to the root JSON Schema file to dereference.
    #[arg(value_parser)]
    schema: PathBuf,

    /// Register an external schema resource: URI=FILE (may be repeated).
    #[arg(long = "resource", value_parser = parse_resource_pair)]
    resources: Vec<(String, PathBuf)>,

    /// Write dereferenced output to FILE instead of stdout.
    #[arg(short = 'o', long = "output")]
    output: Option<PathBuf>,

    #[command(flatten)]
    http: HttpArgs,
}

#[derive(Args)]
struct CanonicalizeArgs {
    /// Path to the JSON Schema file to canonicalize.
    #[arg(value_parser)]
    schema: PathBuf,

    /// Canonicalize only the subschema at this JSON Pointer, e.g. `/$defs/Pet`. A leading `#` is
    /// accepted, so a `$ref` can be pasted as-is.
    #[arg(long = "at", value_name = "POINTER", allow_hyphen_values = true)]
    at: Option<String>,

    /// Which JSON Schema draft to enforce (else auto-detected from $schema).
    #[arg(short = 'd', long = "draft", value_enum)]
    draft: Option<Draft>,

    #[command(flatten)]
    format: FormatAssertionArgs,

    /// Write canonical output to FILE instead of stdout.
    #[arg(short = 'o', long = "output")]
    output: Option<PathBuf>,
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

        let mut retrieval = jsonschema::HttpOptions::new();

        if let Some(connect_timeout) = self.connect_timeout {
            retrieval = retrieval.connect_timeout(Duration::from_secs_f64(connect_timeout));
        }
        if let Some(timeout) = self.timeout {
            retrieval = retrieval.timeout(Duration::from_secs_f64(timeout));
        }
        if self.insecure {
            retrieval = retrieval.danger_accept_invalid_certs(true);
        }
        if let Some(cacert) = self.cacert.as_ref() {
            retrieval = retrieval.add_root_certificate(cacert);
        }

        Some(retrieval)
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

fn read_json(path: &Path) -> Result<Value, ReadJsonError> {
    let bytes = fs::read(path).map_err(|err| ReadJsonError::Io {
        file: path.into(),
        err,
    })?;
    serde_json::from_slice(&bytes).map_err(|err| ReadJsonError::Json {
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
) -> Result<Result<Value, ReadJsonOrYamlError>, Box<dyn std::error::Error>> {
    let bytes = fs::read(path)?;
    if let Some(ext) = path.extension() {
        if ext == "yaml" || ext == "yml" {
            return Ok(
                serde_saphyr::from_slice(&bytes).map_err(|err| ReadJsonOrYamlError::Yaml {
                    file: path.into(),
                    err,
                }),
            );
        }
    }
    Ok(
        serde_json::from_slice(&bytes).map_err(|err| ReadJsonOrYamlError::Json {
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

    let path = path.canonicalize().expect("Failed to canonicalize path");

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

fn options_for_base_uri<'a>(
    base_uri: referencing::Uri<String>,
    retrieval: Retrieval<'_>,
) -> Result<jsonschema::ValidationOptions<'a>, Box<dyn std::error::Error>> {
    let mut options = jsonschema::options().with_base_uri(base_uri);
    if retrieval.offline {
        // HTTP settings cannot take effect once retrieval is refused.
        return Ok(options.offline());
    }
    if let Some(http_opts) = retrieval.http {
        options = options.with_http_options(http_opts)?;
    }
    Ok(options)
}

fn file_uri(path: &Path) -> Result<referencing::Uri<String>, Box<dyn std::error::Error>> {
    Ok(referencing::uri::from_str(&path_to_uri(path))?)
}

fn options_for_schema<'a>(
    schema_path: &Path,
    retrieval: Retrieval<'_>,
) -> Result<jsonschema::ValidationOptions<'a>, Box<dyn std::error::Error>> {
    options_for_base_uri(file_uri(schema_path)?, retrieval)
}

// Read `--resource URI=FILE` pairs into a prepared Registry, seeded with an HTTP retriever
// when HTTP options are set. Shared by bundle and dereference.
fn build_resource_registry(
    resources: &[(String, PathBuf)],
    retrieval: Retrieval<'_>,
) -> Result<jsonschema::Registry<'static>, Box<dyn std::error::Error>> {
    let mut builder = match (retrieval.offline, retrieval.http) {
        (false, Some(http_opts)) => {
            jsonschema::Registry::new().retriever(jsonschema::HttpRetriever::new(http_opts)?)
        }
        _ => jsonschema::Registry::new(),
    };
    for (uri, path) in resources {
        let resource_json = read_json(path)?;
        builder = builder.add(uri, resource_json)?;
    }
    Ok(builder.prepare()?)
}

// A `$ref` is usually copied with its leading `#`; an empty pointer is the document root.
fn parse_pointer(at: &str) -> Result<&str, String> {
    let pointer = at.strip_prefix('#').unwrap_or(at);
    if pointer.is_empty() || pointer.starts_with('/') {
        Ok(pointer)
    } else {
        Err(format!(
            "'{at}' is not a JSON Pointer: it must be empty or start with '/'"
        ))
    }
}

// Which segment is wrong is the whole question when a pointer is typed by hand.
const MAX_SELECTION_HOPS: usize = 32;

fn check_pointer(document: &Value, pointer: &str) -> Result<(), String> {
    if let Some(target) = referencing::pointer(document, pointer) {
        return match target {
            Value::Object(_) | Value::Bool(_) => Ok(()),
            other => Err(format!(
                "'{pointer}' is of type {}, not a schema",
                Node::<SerdeJson>::json_type(&other)
            )),
        };
    }
    // The whole pointer did not resolve, so some segment of it does not either. The deepest prefix
    // that does resolve is the parent to name it against, and every prefix is a slice of the
    // pointer itself, so the walk carries an index rather than rebuilding them.
    let mut resolved = 0;
    let mut broken = pointer;
    for segment in pointer.split('/').skip(1) {
        let end = resolved + '/'.len_utf8() + segment.len();
        if referencing::pointer(document, &pointer[..end]).is_none() {
            broken = segment;
            break;
        }
        resolved = end;
    }
    let parent = if resolved == 0 {
        Cow::Borrowed("the document root")
    } else {
        Cow::Owned(format!("'{}'", &pointer[..resolved]))
    };
    let name = referencing::unescape_segment(broken);
    Err(format!(
        "no schema at '{pointer}': {parent} has no '{name}'"
    ))
}

// Which key carries a root `$id`, and whether it counts at all, is the draft's business.
// Generated definitions land under a draft-dependent keyword, and a document's own choice is kept.
const DEFINITION_CONTAINERS: [(&str, &str); 2] =
    [("$defs", "#/$defs/"), ("definitions", "#/definitions/")];

// Registry-reached targets are keyed by percent-encoded URI; name them after its last segment.
fn rename_definitions(schema: &mut Value) {
    for (keyword, prefix) in DEFINITION_CONTAINERS {
        rename_container(schema, keyword, prefix);
    }
}

fn rename_container(schema: &mut Value, keyword: &str, prefix: &str) {
    let Some(definitions) = schema.get_mut(keyword).and_then(Value::as_object_mut) else {
        return;
    };
    let mut names: HashMap<String, String> = HashMap::new();
    let mut taken: HashSet<String> = HashSet::new();
    for key in definitions.keys() {
        let Some(base) = readable_definition_name(key) else {
            continue;
        };
        let mut name = base.clone();
        let mut suffix = 2;
        while definitions.contains_key(&name) || taken.contains(&name) {
            name = format!("{base}_{suffix}");
            suffix += 1;
        }
        taken.insert(name.clone());
        names.insert(key.clone(), name);
    }
    for (key, name) in &names {
        if let Some(body) = definitions.remove(key) {
            definitions.insert(name.clone(), body);
        }
    }
    // The references still name the old keys, and are rewritten once the map is no longer borrowed.
    rename_references(schema, &names, prefix);
}

fn readable_definition_name(key: &str) -> Option<String> {
    if !key.contains('%') {
        return None;
    }
    let decoded = percent_encoding::percent_decode_str(key)
        .decode_utf8()
        .ok()?;
    let last = decoded.rsplit('/').next()?;
    let name: String = last
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .collect();
    (!name.is_empty()).then_some(name)
}

fn rename_references(value: &mut Value, names: &HashMap<String, String>, prefix: &str) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get_mut("$ref") {
                // A pointer escapes the key's own percent signs again, so one decode undoes it.
                if let Some(key) = reference
                    .strip_prefix(prefix)
                    .and_then(|segment| {
                        percent_encoding::percent_decode_str(segment)
                            .decode_utf8()
                            .ok()
                    })
                    .and_then(|key| names.get(key.as_ref()))
                {
                    *reference = format!("{prefix}{key}");
                }
            }
            for (_, child) in map.iter_mut() {
                rename_references(child, names, prefix);
            }
        }
        Value::Array(items) => {
            for item in items {
                rename_references(item, names, prefix);
            }
        }
        _ => {}
    }
}

fn output_schema_validation(
    schema_path: &Path,
    schema_json: &Value,
    schema_options: SchemaOptions<'_>,
    output: Output,
    errors_only: bool,
    retrieval: Retrieval<'_>,
) -> Result<bool, Box<dyn std::error::Error>> {
    // First validate against meta-schema
    let meta_validator = jsonschema::meta::validator_for(schema_json)?;
    let evaluation = meta_validator.evaluate(schema_json);
    let flag_output = evaluation.flag();

    // If meta-schema validation passed, also try to build the validator
    // to check that all referenced schemas are valid
    if flag_output.valid {
        // Just try to build - if it fails, the error propagates naturally
        let options = schema_options.apply(options_for_schema(schema_path, retrieval)?);
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
    schema_options: SchemaOptions<'_>,
    output: Output,
    errors_only: bool,
    retrieval: Retrieval<'_>,
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
        let options = schema_options.apply(options_for_schema(schema_path, retrieval)?);
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
        output_schema_validation(
            schema_path,
            &schema_json,
            schema_options,
            output,
            errors_only,
            retrieval,
        )
    }
}

// Text mode accepts YAML instances, the structured modes do not.
fn read_instance(path: &Path, output: Output) -> Result<Value, Box<dyn std::error::Error>> {
    if matches!(output, Output::Text) {
        Ok(read_json_or_yaml(path)??)
    } else {
        Ok(read_json(path)?)
    }
}

fn report_instance(
    validator: &jsonschema::Validator,
    instance: &Path,
    instance_json: &Value,
    schema_display: &str,
    output: Output,
    errors_only: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    let filename = instance.to_string_lossy();

    if matches!(output, Output::Text) {
        let mut errors = validator.iter_errors(instance_json);
        if let Some(first) = errors.next() {
            println!("{filename} - INVALID. Errors:");
            println!("1. {first}");
            for (i, error) in errors.enumerate() {
                println!("{}. {error}", i + 2);
            }
            return Ok(false);
        }
        if !errors_only {
            println!("{filename} - VALID");
        }
        return Ok(true);
    }

    let evaluation = validator.evaluate(instance_json);
    let flag_output = evaluation.flag();
    let valid = flag_output.valid;

    if errors_only && valid {
        return Ok(valid);
    }

    let payload = match output {
        Output::Text => unreachable!("handled above"),
        Output::Flag => serde_json::to_value(flag_output)?,
        Output::List => serde_json::to_value(evaluation.list())?,
        Output::Hierarchical => serde_json::to_value(evaluation.hierarchical())?,
    };

    let record = json!({
        "output": output.as_str(),
        "schema": schema_display,
        "instance": filename,
        "payload": payload,
    });
    println!("{}", serde_json::to_string(&record)?);

    Ok(valid)
}

/// Emitted as a record in the structured modes so the ndjson stream stays parseable.
fn report_instance_error(
    instance: &Path,
    message: &str,
    output: Output,
) -> Result<(), Box<dyn std::error::Error>> {
    if matches!(output, Output::Text) {
        println!("{} - ERROR: {message}", instance.to_string_lossy());
    } else {
        let record = json!({
            "output": output.as_str(),
            "instance": instance.to_string_lossy(),
            "error": message,
        });
        println!("{}", serde_json::to_string(&record)?);
    }
    Ok(())
}

/// Not JSON Schema's `$schema`: here the instance is data and `$schema` names the schema to
/// validate it against. Relative values resolve against the instance file, not the working directory.
fn resolve_instance_schema_uri(
    instance: &Path,
    instance_json: &Value,
) -> Result<referencing::Uri<String>, String> {
    let raw = instance_json
        .get("$schema")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            format!(
                "no `$schema` property; pass a schema explicitly:\n  jsonschema validate SCHEMA -i {}",
                instance.to_string_lossy()
            )
        })?;
    let base = file_uri(instance).map_err(|error| error.to_string())?;
    referencing::uri::resolve_against(&base.borrow(), raw)
        .map_err(|error| format!("invalid `$schema` value `{raw}`: {error}"))
}

/// The `validate` flags every validator build honors.
#[derive(Clone, Copy)]
struct SchemaOptions<'a> {
    draft: Option<Draft>,
    assert_format: Option<bool>,
    vocabularies: &'a [String],
}

impl SchemaOptions<'_> {
    fn apply(
        self,
        mut options: jsonschema::ValidationOptions<'_>,
    ) -> jsonschema::ValidationOptions<'_> {
        if let Some(draft) = self.draft {
            options = options.with_draft(draft.into());
        }
        if let Some(assert_format) = self.assert_format {
            options = options.should_validate_formats(assert_format);
        }
        for vocabulary in self.vocabularies {
            options = options.with_vocabulary(vocabulary.clone());
        }
        options
    }
}

fn build_validator_for_uri(
    schema_uri: &referencing::Uri<String>,
    instance: &Path,
    schema_options: SchemaOptions<'_>,
    retrieval: Retrieval<'_>,
) -> Result<jsonschema::Validator, Box<dyn std::error::Error>> {
    let has_fragment = schema_uri
        .fragment()
        .is_some_and(|fragment| !fragment.as_str().is_empty());
    // The instance is itself a schema; resolve offline instead of fetching json-schema.org.
    let is_meta_schema = referencing::SPECIFICATIONS.contains_resource(schema_uri.as_str());

    if has_fragment || is_meta_schema {
        // Building the pointed-at subschema alone would drop `$id` scope and sibling `$defs`, so
        // reference it from a synthetic root. Costs a `/$ref` prefix on `evaluationPath`.
        let mut options =
            schema_options.apply(options_for_base_uri(file_uri(instance)?, retrieval)?);
        if is_meta_schema {
            options = options.with_registry(&referencing::SPECIFICATIONS);
        }
        return Ok(options.build(&json!({ "$ref": schema_uri.as_str() }))?);
    }

    // Build the retrieved schema as the root so `evaluationPath` matches an explicit
    // `validate SCHEMA -i ...` run. `HttpRetriever` covers http, https and file alike.
    if retrieval.offline {
        return Err(format!(
            "`--offline` refuses to fetch the schema `{}` this instance describes itself with",
            schema_uri.as_str()
        )
        .into());
    }
    let default_http_options;
    let http_options = if let Some(options) = retrieval.http {
        options
    } else {
        default_http_options = jsonschema::HttpOptions::new();
        &default_http_options
    };
    let retriever = jsonschema::HttpRetriever::new(http_options)?;
    let schema_json = retriever
        .retrieve(schema_uri)
        .map_err(|error| format!("failed to retrieve `{}`: {error}", schema_uri.as_str()))?;
    let options = options_for_base_uri(schema_uri.clone(), retrieval)?;
    Ok(schema_options.apply(options).build(&schema_json)?)
}

fn validate_self_describing_instances(
    instances: &[PathBuf],
    schema_options: SchemaOptions<'_>,
    output: Output,
    errors_only: bool,
    retrieval: Retrieval<'_>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let mut success = true;
    let mut validators: HashMap<String, jsonschema::Validator> = HashMap::new();

    for instance in instances {
        let instance_json = read_instance(instance, output)?;

        let schema_uri = match resolve_instance_schema_uri(instance, &instance_json) {
            Ok(uri) => uri,
            Err(message) => {
                report_instance_error(instance, &message, output)?;
                success = false;
                continue;
            }
        };

        let key = schema_uri.as_str().to_string();
        if !validators.contains_key(&key) {
            match build_validator_for_uri(&schema_uri, instance, schema_options, retrieval) {
                Ok(validator) => {
                    validators.insert(key.clone(), validator);
                }
                Err(error) => {
                    report_instance_error(instance, &error.to_string(), output)?;
                    success = false;
                    continue;
                }
            }
        }

        if !report_instance(
            &validators[&key],
            instance,
            &instance_json,
            &key,
            output,
            errors_only,
        )? {
            success = false;
        }
    }

    Ok(success)
}

fn validate_instances(
    instances: &[PathBuf],
    schema_path: &Path,
    schema_options: SchemaOptions<'_>,
    output: Output,
    errors_only: bool,
    retrieval: Retrieval<'_>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let mut success = true;

    let schema_json = read_json(schema_path)?;
    let options = schema_options.apply(options_for_schema(schema_path, retrieval)?);
    match options.build(&schema_json) {
        Ok(validator) => {
            let schema_display = schema_path.to_string_lossy().to_string();
            for instance in instances {
                let instance_json = read_instance(instance, output)?;
                if !report_instance(
                    &validator,
                    instance,
                    &instance_json,
                    &schema_display,
                    output,
                    errors_only,
                )? {
                    success = false;
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
                    schema_options,
                    output,
                    errors_only,
                    retrieval,
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
        format,
        vocabularies,
        output,
        errors_only,
        http,
    } = args;

    let offline = http.offline;
    let http_options = http.into_http_options();
    let retrieval = Retrieval {
        http: http_options.as_ref(),
        offline,
    };
    let schema_options = SchemaOptions {
        draft,
        assert_format: format.validate_formats(),
        vocabularies: &vocabularies,
    };

    match (schema, instances) {
        (Some(schema), Some(instances)) => validation_result_to_exit(validate_instances(
            &instances,
            &schema,
            schema_options,
            output,
            errors_only,
            retrieval,
        )),
        (Some(schema), None) => validation_result_to_exit(validate_schema_meta(
            &schema,
            schema_options,
            output,
            errors_only,
            retrieval,
        )),
        (None, Some(instances)) => {
            validation_result_to_exit(validate_self_describing_instances(
                &instances,
                schema_options,
                output,
                errors_only,
                retrieval,
            ))
        }
        (None, None) => fail_with_error(
            "either a SCHEMA argument or `--instance` is required. See `jsonschema validate --help`.",
        ),
    }
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
    let offline = http.offline;
    let http_options = http.into_http_options();
    let retrieval = Retrieval {
        http: http_options.as_ref(),
        offline,
    };
    let mut opts = match options_for_schema(&schema, retrieval) {
        Ok(value) => value,
        Err(error) => return fail_with_error(error),
    };

    let registry = match build_resource_registry(&resources, retrieval) {
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

fn run_dereference(args: DereferenceArgs) -> ExitCode {
    let DereferenceArgs {
        schema,
        resources,
        output,
        http,
    } = args;

    let schema_json = match read_json(&schema) {
        Ok(value) => value,
        Err(error) => return fail_with_error(error),
    };
    let offline = http.offline;
    let http_options = http.into_http_options();
    let retrieval = Retrieval {
        http: http_options.as_ref(),
        offline,
    };
    let mut opts = match options_for_schema(&schema, retrieval) {
        Ok(value) => value,
        Err(error) => return fail_with_error(error),
    };

    let registry = match build_resource_registry(&resources, retrieval) {
        Ok(registry) => registry,
        Err(error) => return fail_with_error(error),
    };
    opts = opts.with_registry(&registry);

    match opts.dereference(&schema_json) {
        Ok(dereferenced) => {
            let json = match serde_json::to_string_pretty(&dereferenced) {
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

fn run_canonicalize(args: CanonicalizeArgs) -> ExitCode {
    let CanonicalizeArgs {
        schema,
        at,
        draft,
        format,
        output,
    } = args;

    let schema_json = match read_json_or_yaml(&schema) {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => return fail_with_error(error),
        Err(error) => return fail_with_error(error),
    };

    // An empty pointer is the document root: the no-flag behaviour.
    let pointer = match at.as_deref().map(parse_pointer) {
        None | Some(Ok("")) => None,
        Some(Ok(pointer)) => Some(pointer),
        Some(Err(error)) => return fail_with_error(error),
    };
    if let Some(pointer) = pointer {
        if let Err(error) = check_pointer(&schema_json, pointer) {
            return fail_with_error(error);
        }
    }

    let mut options = jsonschema::canonical::options();
    if let Some(draft) = draft {
        options = options.with_draft(draft.into());
    }
    if let Some(validate_formats) = format.validate_formats() {
        options = options.should_validate_formats(validate_formats);
    }

    let prepared = match options.prepare(&schema_json) {
        Ok(prepared) => prepared,
        Err(error) => return fail_with_error(error),
    };
    let canonical = match pointer {
        Some(pointer) => prepared.canonicalize_at(pointer),
        None => prepared.canonicalize(),
    };
    let canonical = match canonical {
        Ok(canonical) => canonical,
        Err(error) => return fail_with_error(error),
    };

    // A selection that is itself a `$ref` reads better resolved.
    let mut selected = canonical;
    for _ in 0..MAX_SELECTION_HOPS {
        let jsonschema::canonical::CanonicalView::Reference(uri) = selected.view() else {
            break;
        };
        let Some(body) = selected.definition(&uri) else {
            break;
        };
        selected = body;
    }

    let mut emitted = selected.to_json_schema();
    rename_definitions(&mut emitted);
    let rendered = serde_json::to_string_pretty(&emitted);
    let json = match rendered {
        Ok(json) => json,
        Err(error) => return fail_with_error(error),
    };
    match output {
        Some(path) => {
            if let Err(error) = std::fs::write(&path, &json) {
                return fail_with_error(format!("{}: {error}", path.display()));
            }
        }
        None => println!("{json}"),
    }
    ExitCode::SUCCESS
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
        Some(Command::Dereference(args)) => run_dereference(args),
        Some(Command::Canonicalize(args)) => run_canonicalize(args),
        None => {
            // Flat invocation is deprecated — emit a warning, then proceed as `validate`
            if let Some(schema) = cli.schema {
                eprintln!(
                    "warning: flat invocation is deprecated. Use `jsonschema validate {}` instead.",
                    schema.display()
                );
                run_validate(ValidateArgs {
                    schema: Some(schema),
                    instances: cli.instances,
                    draft: cli.draft,
                    format: FormatAssertionArgs::from_flags(
                        cli.assert_format,
                        cli.no_assert_format,
                    ),
                    vocabularies: Vec::new(),
                    output: cli.output,
                    errors_only: cli.errors_only,
                    http: HttpArgs {
                        connect_timeout: cli.connect_timeout,
                        timeout: cli.timeout,
                        insecure: cli.insecure,
                        cacert: cli.cacert,
                        offline: false,
                    },
                })
            } else {
                eprintln!("A schema argument is required. Use `jsonschema validate --help`, `jsonschema bundle --help`, or `jsonschema dereference --help`.");
                ExitCode::FAILURE
            }
        }
    }
}

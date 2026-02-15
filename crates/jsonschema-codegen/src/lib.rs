use std::{
    borrow::Cow, collections::HashSet, env, fs::File, io::BufReader, path::PathBuf, sync::Arc,
};

use proc_macro::TokenStream;
use proc_macro_crate::{crate_name, FoundCrate};
use quote::{format_ident, quote, ToTokens};
use serde_json::Value;
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input, Ident, ItemStruct, LitBool, LitInt, LitStr, Token,
};

mod codegen;
mod context;
#[cfg(test)]
mod tests;
use referencing::{uri, Draft, Registry};

/// Generates a compile-time JSON Schema validator.
///
/// The macro reads and compiles the schema at compile time and generates:
///
/// - `impl <Type> { pub fn is_valid(instance: &serde_json::Value) -> bool }`
///
/// Best fit: static schemas on hot paths where startup schema compilation is undesirable.
/// Trade-offs: higher compile time and binary size, less runtime configurability.
///
/// Current MVP scope is `is_valid` only. `validate`, `iter_errors`, and `evaluate`
/// are planned follow-ups.
///
/// Schema errors are emitted as regular Rust compile errors.
///
/// # Required attribute
///
/// Exactly one schema source must be provided:
///
/// - `path = "relative/or/absolute/path/to/schema.json"`
/// - `schema = r#"{"type":"string"}"#`
///
/// # Optional attributes
///
/// - `draft = referencing::Draft::Draft4|Draft6|Draft7|Draft201909|Draft202012`
/// - `base_uri = "json-schema:///root/main.json"`
/// - `resources = { "uri" => { schema = r#"..."# }, "uri2" => { path = "..." } }`
/// - `validate_formats = true|false`
/// - `ignore_unknown_formats = true|false` (default: `true`)
/// - `formats = { "name" => crate::path::to::format_fn }`
/// - `email_options = { ... }`
/// - `pattern_options = { ... }`
///
/// `email_options` keys:
///
/// - `minimum_sub_domains = <usize>`
/// - `no_minimum_sub_domains = <bool>`
/// - `required_tld = <bool>`
/// - `allow_domain_literal = <bool>`
/// - `allow_display_text = <bool>`
///
/// `pattern_options` keys:
///
/// - `engine = fancy_regex|regex`
/// - `backtrack_limit = <usize>` (only for `fancy_regex`)
/// - `size_limit = <usize>`
/// - `dfa_size_limit = <usize>`
///
/// # Example
///
/// ```ignore
/// #[jsonschema::validator(
///     path = "schema.json",
///     draft = referencing::Draft::Draft202012,
///     validate_formats = true,
///     formats = {
///         "currency" => crate::formats::is_currency,
///     },
///     pattern_options = {
///         engine = regex,
///         size_limit = 1_000_000,
///         dfa_size_limit = 2_000_000,
///     }
/// )]
/// struct Validator;
/// ```
#[proc_macro_attribute]
pub fn validator(attr: TokenStream, item: TokenStream) -> TokenStream {
    let config = parse_macro_input!(attr as Config);
    let item = parse_macro_input!(item as ItemStruct);

    match validator_impl(&config, &item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

struct Config {
    source: SchemaSource,
    draft: Option<syn::Expr>,
    base_uri: Option<LitStr>,
    resources: Vec<ResourceEntry>,
    validate_formats: Option<bool>,
    formats: Vec<FormatEntry>,
    ignore_unknown_formats: Option<bool>,
    email_options: Option<context::EmailOptionsConfig>,
    pattern_options: PatternOptionsConfig,
}

enum SchemaSource {
    Path(LitStr),
    Schema(LitStr),
}

enum ResourceContent {
    Schema(String),
    Path(LitStr),
}

struct ResourceEntry {
    uri: String,
    content: ResourceContent,
}

struct FormatEntry {
    name: String,
    path: syn::Path,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PatternEngine {
    FancyRegex,
    Regex,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PatternOptionsConfig {
    engine: PatternEngine,
    backtrack_limit: Option<usize>,
    size_limit: Option<usize>,
    dfa_size_limit: Option<usize>,
}

impl Default for PatternOptionsConfig {
    fn default() -> Self {
        Self {
            engine: PatternEngine::FancyRegex,
            backtrack_limit: None,
            size_limit: None,
            dfa_size_limit: None,
        }
    }
}

fn parse_pattern_limit(value: LitInt, key: &str) -> syn::Result<usize> {
    value.base10_parse::<usize>().map_err(|err| {
        syn::Error::new_spanned(
            value,
            format!("`{key}` must be a non-negative integer that fits in usize: {err}"),
        )
    })
}

fn parse_pattern_options(input: ParseStream) -> syn::Result<PatternOptionsConfig> {
    let content;
    syn::braced!(content in input);

    let mut options = PatternOptionsConfig::default();
    let mut seen = HashSet::new();
    let mut backtrack_limit_span = None;

    while !content.is_empty() {
        let key: Ident = content.parse()?;
        let key_name = key.to_string();
        if !seen.insert(key_name.clone()) {
            return Err(syn::Error::new_spanned(
                key,
                format!("Duplicate pattern_options key: `{key_name}`"),
            ));
        }

        content.parse::<Token![=]>()?;
        match key_name.as_str() {
            "engine" => {
                let engine: Ident = content.parse()?;
                options.engine = match engine.to_string().as_str() {
                    "fancy_regex" => PatternEngine::FancyRegex,
                    "regex" => PatternEngine::Regex,
                    _ => {
                        return Err(syn::Error::new_spanned(
                            engine,
                            "Unknown regex engine. Expected `fancy_regex` or `regex`",
                        ));
                    }
                };
            }
            "backtrack_limit" => {
                let value: LitInt = content.parse()?;
                options.backtrack_limit = Some(parse_pattern_limit(value, "backtrack_limit")?);
                backtrack_limit_span = Some(key.span());
            }
            "size_limit" => {
                let value: LitInt = content.parse()?;
                options.size_limit = Some(parse_pattern_limit(value, "size_limit")?);
            }
            "dfa_size_limit" => {
                let value: LitInt = content.parse()?;
                options.dfa_size_limit = Some(parse_pattern_limit(value, "dfa_size_limit")?);
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    key,
                    "Unknown pattern_options key. Expected `engine`, `backtrack_limit`, `size_limit`, or `dfa_size_limit`",
                ));
            }
        }

        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        }
    }

    if matches!(options.engine, PatternEngine::Regex) && options.backtrack_limit.is_some() {
        return Err(syn::Error::new(
            backtrack_limit_span.unwrap_or(proc_macro2::Span::call_site()),
            "backtrack_limit is only supported for `fancy_regex` engine",
        ));
    }

    Ok(options)
}

fn parse_email_options(input: ParseStream) -> syn::Result<context::EmailOptionsConfig> {
    let content;
    syn::braced!(content in input);

    let mut options = context::EmailOptionsConfig::default();
    let mut seen = HashSet::new();

    while !content.is_empty() {
        let key: Ident = content.parse()?;
        let key_name = key.to_string();
        if !seen.insert(key_name.clone()) {
            return Err(syn::Error::new_spanned(
                key,
                format!("Duplicate email_options key: `{key_name}`"),
            ));
        }

        content.parse::<Token![=]>()?;
        match key_name.as_str() {
            "minimum_sub_domains" => {
                let value: LitInt = content.parse()?;
                options.minimum_sub_domains =
                    Some(parse_pattern_limit(value, "minimum_sub_domains")?);
            }
            "no_minimum_sub_domains" => {
                let value: LitBool = content.parse()?;
                options.no_minimum_sub_domains = value.value;
            }
            "required_tld" => {
                let value: LitBool = content.parse()?;
                options.required_tld = value.value;
            }
            "allow_domain_literal" => {
                let value: LitBool = content.parse()?;
                options.allow_domain_literal = Some(value.value);
            }
            "allow_display_text" => {
                let value: LitBool = content.parse()?;
                options.allow_display_text = Some(value.value);
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    key,
                    "Unknown email_options key. Expected `minimum_sub_domains`, `no_minimum_sub_domains`, `required_tld`, `allow_domain_literal`, or `allow_display_text`",
                ));
            }
        }

        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        }
    }

    let domain_mode_count = usize::from(options.minimum_sub_domains.is_some())
        + usize::from(options.no_minimum_sub_domains)
        + usize::from(options.required_tld);
    if domain_mode_count > 1 {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "At most one of `minimum_sub_domains`, `no_minimum_sub_domains`, or `required_tld` may be specified",
        ));
    }

    Ok(options)
}

impl Parse for Config {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut schema_source = None;
        let mut draft = None;
        let mut base_uri = None;
        let mut resources: Vec<ResourceEntry> = Vec::new();
        let mut validate_formats = None;
        let mut formats: Vec<FormatEntry> = Vec::new();
        let mut ignore_unknown_formats = None;
        let mut email_options = None;
        let mut pattern_options = PatternOptionsConfig::default();

        // Parse key-value pairs
        while !input.is_empty() {
            let ident: Ident = input.parse()?;

            match ident.to_string().as_str() {
                "path" => {
                    if schema_source.is_some() {
                        return Err(syn::Error::new_spanned(
                            ident,
                            "Schema source is already specified. Use exactly one of `path` or `schema`",
                        ));
                    }
                    input.parse::<Token![=]>()?;
                    let value: LitStr = input.parse()?;
                    schema_source = Some(SchemaSource::Path(value));
                }
                "schema" => {
                    if schema_source.is_some() {
                        return Err(syn::Error::new_spanned(
                            ident,
                            "Schema source is already specified. Use exactly one of `path` or `schema`",
                        ));
                    }
                    input.parse::<Token![=]>()?;
                    let value: LitStr = input.parse()?;
                    schema_source = Some(SchemaSource::Schema(value));
                }
                "draft" => {
                    input.parse::<Token![=]>()?;
                    // Parse as expression: referencing::Draft::Draft4, etc.
                    draft = Some(input.parse()?);
                }
                "base_uri" => {
                    input.parse::<Token![=]>()?;
                    let value: LitStr = input.parse()?;
                    base_uri = Some(value);
                }
                "resources" => {
                    input.parse::<Token![=]>()?;
                    let content;
                    syn::braced!(content in input);
                    let mut local_resources = Vec::new();
                    while !content.is_empty() {
                        let uri: LitStr = content.parse()?;
                        content.parse::<Token![=>]>()?;
                        let entry_content_tokens;
                        syn::braced!(entry_content_tokens in content);
                        let entry_key: Ident = entry_content_tokens.parse()?;
                        entry_content_tokens.parse::<Token![=]>()?;
                        let resource_content = match entry_key.to_string().as_str() {
                            "schema" => {
                                let v: LitStr = entry_content_tokens.parse()?;
                                ResourceContent::Schema(v.value())
                            }
                            "path" => {
                                let v: LitStr = entry_content_tokens.parse()?;
                                ResourceContent::Path(v)
                            }
                            other => {
                                return Err(syn::Error::new_spanned(
                                    entry_key,
                                    format!("Expected `schema` or `path`, got `{other}`"),
                                ));
                            }
                        };
                        local_resources.push(ResourceEntry {
                            uri: uri.value(),
                            content: resource_content,
                        });
                        // consume trailing commas inside entry braces (if any)
                        if entry_content_tokens.peek(Token![,]) {
                            entry_content_tokens.parse::<Token![,]>()?;
                        }
                        // consume trailing comma after each entry
                        if content.peek(Token![,]) {
                            content.parse::<Token![,]>()?;
                        }
                    }
                    resources = local_resources;
                }
                "validate_formats" => {
                    input.parse::<Token![=]>()?;
                    let value: LitBool = input.parse()?;
                    validate_formats = Some(value.value);
                }
                "formats" => {
                    input.parse::<Token![=]>()?;
                    let content;
                    syn::braced!(content in input);
                    let mut local_formats = Vec::new();
                    let mut seen = HashSet::new();
                    while !content.is_empty() {
                        let name: LitStr = content.parse()?;
                        content.parse::<Token![=>]>()?;
                        let path: syn::Path = content.parse()?;
                        if path.leading_colon.is_none() {
                            let is_crate_path = path
                                .segments
                                .first()
                                .is_some_and(|segment| segment.ident == "crate");
                            if !is_crate_path {
                                return Err(syn::Error::new_spanned(
                                    path,
                                    "Custom format paths must be absolute (`::...`) or start with `crate::`",
                                ));
                            }
                        }
                        let name_value = name.value();
                        if !seen.insert(name_value.clone()) {
                            return Err(syn::Error::new_spanned(
                                name,
                                format!("Duplicate format entry: `{name_value}`"),
                            ));
                        }
                        local_formats.push(FormatEntry {
                            name: name_value,
                            path,
                        });
                        if content.peek(Token![,]) {
                            content.parse::<Token![,]>()?;
                        }
                    }
                    formats = local_formats;
                }
                "ignore_unknown_formats" => {
                    input.parse::<Token![=]>()?;
                    let value: LitBool = input.parse()?;
                    ignore_unknown_formats = Some(value.value);
                }
                "email_options" => {
                    input.parse::<Token![=]>()?;
                    email_options = Some(parse_email_options(input)?);
                }
                "pattern_options" => {
                    input.parse::<Token![=]>()?;
                    pattern_options = parse_pattern_options(input)?;
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        ident,
                        "Expected `path`, `schema`, `draft`, `base_uri`, `resources`, `validate_formats`, `formats`, `ignore_unknown_formats`, `email_options`, or `pattern_options` attribute",
                    ));
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        let schema_source = schema_source.ok_or_else(|| {
            syn::Error::new(
                input.span(),
                "Missing required `path` or `schema` attribute",
            )
        })?;

        Ok(Config {
            source: schema_source,
            draft,
            base_uri,
            resources,
            validate_formats,
            formats,
            ignore_unknown_formats,
            email_options,
            pattern_options,
        })
    }
}

fn validator_impl(config: &Config, input: &ItemStruct) -> syn::Result<proc_macro2::TokenStream> {
    let (schema, mut recompile_triggers): (
        serde_json::Result<serde_json::Value>,
        Vec<proc_macro2::TokenStream>,
    ) = match &config.source {
        SchemaSource::Path(value) => {
            let path = resolve_schema_path(value)?;
            let file = File::open(&path).map_err(|err| {
                syn::Error::new_spanned(
                    value,
                    format!("Failed to read `{}`: {err}", path.display()),
                )
            })?;
            let reader = BufReader::new(file);
            let path = path.to_string_lossy();
            (
                serde_json::from_reader(reader),
                vec![quote! {
                    const _: &str = include_str!(#path);
                }],
            )
        }
        SchemaSource::Schema(value) => {
            let content = value.value();
            (serde_json::from_str(&content), Vec::new())
        }
    };

    let schema =
        schema.map_err(|err| syn::Error::new_spanned(input, format!("Invalid JSON: {err}")))?;

    let name = &input.ident;

    let draft = detect_draft(&schema, config.draft.as_ref())?;
    let runtime_crate_alias = resolve_runtime_crate_alias()?;

    let resource_ref = draft.create_resource_ref(&schema);
    let base_uri = if let Some(base_uri) = &config.base_uri {
        Cow::Owned(base_uri.value())
    } else if let Some(id) = resource_ref.id() {
        Cow::Borrowed(id)
    } else {
        Cow::Owned(format!("json-schema:///{name}"))
    };

    // Build resource pairs: start with the root schema
    let mut resource_pairs: Vec<(String, referencing::Resource)> =
        vec![(base_uri.to_string(), draft.create_resource(schema.clone()))];

    // Load additional resources from the `resources` attribute
    for entry in &config.resources {
        let schema_value: serde_json::Value = match &entry.content {
            ResourceContent::Schema(s) => serde_json::from_str(s).map_err(|e| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("Invalid JSON for resource '{}': {e}", entry.uri),
                )
            })?,
            ResourceContent::Path(lit) => {
                let path = resolve_schema_path(lit)?;
                let file = File::open(&path).map_err(|e| {
                    syn::Error::new_spanned(lit, format!("Cannot open '{}': {e}", path.display()))
                })?;
                let path_trigger = path.to_string_lossy().to_string();
                recompile_triggers.push(quote! {
                    const _: &str = include_str!(#path_trigger);
                });
                serde_json::from_reader(BufReader::new(file)).map_err(|e| {
                    syn::Error::new_spanned(
                        lit,
                        format!("Invalid JSON in '{}': {e}", path.display()),
                    )
                })?
            }
        };
        resource_pairs.push((entry.uri.clone(), draft.create_resource(schema_value)));
    }

    // Build registry from all resource pairs.
    let registry = Registry::options()
        .draft(draft)
        .build(
            resource_pairs
                .iter()
                .map(|(uri, r)| (uri.as_str(), r.clone())),
        )
        .map_err(|err| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("Registry error: {err}"),
            )
        })?;

    let base_uri_uri = uri::from_str(&base_uri).map_err(|err| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("Invalid base URI: {err}"),
        )
    })?;

    let config = context::CodegenConfig {
        schema,
        registry,
        base_uri: Arc::new(base_uri_uri),
        draft,
        runtime_crate_alias: Some(runtime_crate_alias),
        validate_formats: config.validate_formats,
        custom_formats: config
            .formats
            .iter()
            .map(|entry| {
                let path = &entry.path;
                (entry.name.clone(), quote! { #path })
            })
            .collect(),
        ignore_unknown_formats: config.ignore_unknown_formats.unwrap_or(true),
        email_options: config.email_options,
        pattern_options: match config.pattern_options.engine {
            PatternEngine::FancyRegex => context::PatternEngineConfig::FancyRegex {
                backtrack_limit: config.pattern_options.backtrack_limit,
                size_limit: config.pattern_options.size_limit,
                dfa_size_limit: config.pattern_options.dfa_size_limit,
            },
            PatternEngine::Regex => context::PatternEngineConfig::Regex {
                size_limit: config.pattern_options.size_limit,
                dfa_size_limit: config.pattern_options.dfa_size_limit,
            },
        },
        backend: crate::codegen::backend::BackendKind::serde_json(),
    };
    let impl_mod_name = format_ident!("__{}_impl", name.to_string().to_lowercase());
    let recompile_trigger = quote! {
        #(#recompile_triggers)*
    };
    let tokens =
        crate::codegen::generate_from_config(&config, &recompile_trigger, name, &impl_mod_name);

    Ok(quote! {
        #input

        #tokens
    })
}

fn detect_draft(schema: &Value, draft: Option<&syn::Expr>) -> syn::Result<Draft> {
    // Check the explicit draft first
    if let Some(draft_expr) = draft {
        let syn::Expr::Path(path) = draft_expr else {
            return Err(syn::Error::new_spanned(
                draft_expr,
                "Invalid `draft` expression. Expected `referencing::Draft::<Variant>`",
            ));
        };
        let Some(item) = path.path.segments.last() else {
            return Err(syn::Error::new_spanned(
                path,
                format!("Invalid draft: `{}`", path.to_token_stream()),
            ));
        };

        // Can't match on ident, but comparisons are faster than allocating a string + match
        let detected = if item.ident == "Draft4" {
            Draft::Draft4
        } else if item.ident == "Draft6" {
            Draft::Draft6
        } else if item.ident == "Draft7" {
            Draft::Draft7
        } else if item.ident == "Draft201909" {
            Draft::Draft201909
        } else if item.ident == "Draft202012" {
            Draft::Draft202012
        } else {
            return Err(syn::Error::new_spanned(
                path,
                format!(
                    "Unsupported draft `{}`. Expected one of: Draft4, Draft6, Draft7, Draft201909, Draft202012",
                    path.to_token_stream()
                ),
            ));
        };

        Ok(detected)
    } else {
        Ok(Draft::default().detect(schema))
    }
}

fn resolve_runtime_crate_alias() -> syn::Result<proc_macro2::TokenStream> {
    match crate_name("jsonschema") {
        Ok(FoundCrate::Itself) => Ok(quote! { crate }),
        Ok(FoundCrate::Name(name)) => {
            let ident = format_ident!("{name}");
            Ok(quote! { ::#ident })
        }
        Err(error) => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("Failed to resolve `jsonschema` runtime crate: {error}"),
        )),
    }
}

fn resolve_schema_path(lit: &LitStr) -> syn::Result<PathBuf> {
    let raw = lit.value();
    let path = PathBuf::from(&raw);
    if path.is_absolute() {
        return Ok(path);
    }
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")
        .map_err(|_| syn::Error::new_spanned(lit, "CARGO_MANIFEST_DIR is not set"))?;
    let full = PathBuf::from(manifest_dir).join(&path);
    if full.exists() {
        return Ok(full);
    }
    Err(syn::Error::new_spanned(
        lit,
        format!("Schema file not found: `{}`", full.display()),
    ))
}

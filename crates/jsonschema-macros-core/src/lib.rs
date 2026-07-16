use std::{
    borrow::Cow, collections::HashSet, env, fs::File, io::BufReader, path::PathBuf, sync::Arc,
};

use proc_macro2::TokenStream;
use proc_macro_crate::{crate_name, FoundCrate};
use quote::{format_ident, quote, ToTokens};
use serde_json::Value;
use syn::{
    parse::{Parse, ParseStream},
    Ident, ItemStruct, LitBool, LitInt, LitStr, Token,
};

mod codegen;
mod context;
#[cfg(test)]
mod tests;
use referencing::{uri, Draft, Registry};

/// Expand `#[jsonschema::validator(...)]`. `attr` is the attribute arguments, `item` the annotated
/// struct; returns the generated impl (or a `compile_error!`).
#[must_use]
pub fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    let config = match syn::parse2::<Config>(attr) {
        Ok(config) => config,
        Err(err) => return err.to_compile_error(),
    };
    let item = match syn::parse2::<ItemStruct>(item) {
        Ok(item) => item,
        Err(err) => return err.to_compile_error(),
    };
    match validator_impl(&config, &item) {
        Ok(tokens) => tokens,
        Err(err) => err.to_compile_error(),
    }
}

#[cfg(feature = "bench")]
pub mod bench {
    use std::{collections::HashMap, sync::Arc};

    use quote::{format_ident, quote};
    use referencing::{Draft, Registry, RegistryBuilder};
    use serde_json::Value;

    use crate::context::{CodegenConfig, MethodGates, PatternEngineConfig};

    pub struct Input(CodegenConfig);

    #[must_use]
    #[allow(clippy::missing_panics_doc)]
    pub fn prepare(schema: Value) -> Input {
        let draft = Draft::default().detect(&schema);
        let resource = draft.create_resource(schema.clone());
        let base = "json-schema:///bench";
        let registry = Registry::new()
            .draft(draft)
            .extend([(base, resource)])
            .and_then(RegistryBuilder::prepare)
            .expect("registry build failed");
        let base_uri = referencing::uri::from_str(base)
            .map(Arc::new)
            .expect("valid uri");
        let (uses_unevaluated_properties, uses_unevaluated_items) =
            crate::codegen::scan_uses_unevaluated_over(std::iter::once(&schema));
        Input(CodegenConfig {
            schema,
            registry,
            base_uri,
            draft,
            runtime_crate_alias: None,
            validate_formats: None,
            custom_formats: HashMap::new(),
            custom_keywords: HashMap::new(),
            content_media_types: HashMap::new(),
            content_encodings: HashMap::new(),
            ignore_unknown_formats: true,
            email_options: None,
            pattern_options: PatternEngineConfig::default(),
            uses_unevaluated_properties,
            uses_unevaluated_items,
            method_gates: MethodGates::default(),
        })
    }

    #[must_use]
    pub fn generate(input: &Input) -> proc_macro2::TokenStream {
        crate::codegen::generate_from_config(
            &input.0,
            &quote! {},
            &format_ident!("Validator"),
            &format_ident!("__validator_impl"),
        )
    }

    /// Test-only accessor: the `CodegenConfig` field is private to keep `Input` opaque
    /// outside benchmarks, but borrow-checking tests need it directly.
    #[cfg(test)]
    pub(crate) fn input_config(input: &Input) -> &CodegenConfig {
        &input.0
    }
}

struct Config {
    source: SchemaSource,
    draft: Option<syn::Expr>,
    base_uri: Option<LitStr>,
    resources: Vec<ResourceEntry>,
    validate_formats: Option<bool>,
    formats: Vec<FormatEntry>,
    keywords: Vec<FormatEntry>,
    content_media_types: Vec<FormatEntry>,
    content_encodings: Vec<ContentEncodingEntry>,
    ignore_unknown_formats: Option<bool>,
    email_options: Option<context::EmailOptionsConfig>,
    pattern_options: PatternOptionsConfig,
    methods: context::MethodGates,
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

// Built-in keyword names; a custom keyword under one of these would run in addition to the
// built-in check instead of replacing it, so overrides are rejected.
const BUILTIN_KEYWORDS: &[&str] = &[
    "$anchor",
    "$dynamicAnchor",
    "$dynamicRef",
    "$id",
    "$recursiveAnchor",
    "$recursiveRef",
    "$ref",
    "$schema",
    "$vocabulary",
    "additionalItems",
    "additionalProperties",
    "allOf",
    "anyOf",
    "const",
    "contains",
    "contentEncoding",
    "contentMediaType",
    "dependencies",
    "dependentRequired",
    "dependentSchemas",
    "else",
    "enum",
    "exclusiveMaximum",
    "exclusiveMinimum",
    "format",
    "if",
    "items",
    "maxContains",
    "maxItems",
    "maxLength",
    "maxProperties",
    "maximum",
    "minContains",
    "minItems",
    "minLength",
    "minProperties",
    "minimum",
    "multipleOf",
    "not",
    "oneOf",
    "pattern",
    "patternProperties",
    "prefixItems",
    "properties",
    "propertyNames",
    "required",
    "then",
    "type",
    "unevaluatedItems",
    "unevaluatedProperties",
    "uniqueItems",
];

struct ContentEncodingEntry {
    name: String,
    check: syn::Path,
    convert: syn::Path,
}

fn ensure_callable_path(path: &syn::Path, entry_kind: &str) -> syn::Result<()> {
    if path.leading_colon.is_none() {
        let is_crate_path = path
            .segments
            .first()
            .is_some_and(|segment| segment.ident == "crate");
        if !is_crate_path {
            return Err(syn::Error::new_spanned(
                path,
                format!(
                    "Custom {entry_kind} paths must be absolute (`::...`) or start with `crate::` (they are emitted into generated code, where relative paths do not resolve)"
                ),
            ));
        }
    }
    Ok(())
}

fn parse_content_encodings(input: ParseStream) -> syn::Result<Vec<ContentEncodingEntry>> {
    let content;
    syn::braced!(content in input);
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    while !content.is_empty() {
        let name: LitStr = content.parse()?;
        let name_value = name.value();
        if !seen.insert(name_value.clone()) {
            return Err(syn::Error::new_spanned(
                name,
                format!("Duplicate content encoding entry: `{name_value}`"),
            ));
        }
        content.parse::<Token![=>]>()?;
        let entry_content;
        syn::braced!(entry_content in content);
        let mut check = None;
        let mut convert = None;
        while !entry_content.is_empty() {
            let key: Ident = entry_content.parse()?;
            entry_content.parse::<Token![=]>()?;
            let path: syn::Path = entry_content.parse()?;
            ensure_callable_path(&path, "content encoding")?;
            match key.to_string().as_str() {
                "check" if check.is_none() => check = Some(path),
                "convert" if convert.is_none() => convert = Some(path),
                "check" | "convert" => {
                    return Err(syn::Error::new_spanned(
                        key,
                        "Duplicate content encoding key",
                    ));
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        key,
                        "Expected `check` or `convert`",
                    ));
                }
            }
            if entry_content.peek(Token![,]) {
                entry_content.parse::<Token![,]>()?;
            }
        }
        let (Some(check), Some(convert)) = (check, convert) else {
            return Err(syn::Error::new_spanned(
                name,
                format!("Content encoding `{name_value}` requires both `check` and `convert`"),
            ));
        };
        entries.push(ContentEncodingEntry {
            name: name_value,
            check,
            convert,
        });
        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        }
    }
    Ok(entries)
}

fn parse_named_fn_paths(input: ParseStream, entry_kind: &str) -> syn::Result<Vec<FormatEntry>> {
    let content;
    syn::braced!(content in input);
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    while !content.is_empty() {
        let name: LitStr = content.parse()?;
        content.parse::<Token![=>]>()?;
        let path: syn::Path = content.parse()?;
        ensure_callable_path(&path, entry_kind)?;
        let name_value = name.value();
        if !seen.insert(name_value.clone()) {
            return Err(syn::Error::new_spanned(
                name,
                format!("Duplicate {entry_kind} entry: `{name_value}`"),
            ));
        }
        entries.push(FormatEntry {
            name: name_value,
            path,
        });
        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        }
    }
    Ok(entries)
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

fn parse_methods(input: ParseStream<'_>) -> syn::Result<context::MethodGates> {
    let content;
    syn::parenthesized!(content in input);
    let mut gates = context::MethodGates::default();
    let mut seen = HashSet::new();
    while !content.is_empty() {
        let key: Ident = content.parse()?;
        if !seen.insert(key.to_string()) {
            return Err(syn::Error::new(
                key.span(),
                format!("duplicate method `{key}`"),
            ));
        }
        content.parse::<Token![=]>()?;
        let value: LitBool = content.parse()?;
        match key.to_string().as_str() {
            "is_valid" => gates.is_valid = value.value(),
            "validate" => gates.validate = value.value(),
            "iter_errors" => gates.iter_errors = value.value(),
            "evaluate" => gates.evaluate = value.value(),
            other => {
                return Err(syn::Error::new(
                    key.span(),
                    format!("unknown method `{other}`"),
                ));
            }
        }
        // Require a comma between entries, not just before more input: a missing separator is an error.
        if !content.is_empty() {
            content.parse::<Token![,]>()?;
        }
    }
    Ok(gates)
}

impl Parse for Config {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut schema_source = None;
        let mut draft = None;
        let mut base_uri = None;
        let mut resources: Vec<ResourceEntry> = Vec::new();
        let mut validate_formats = None;
        let mut formats: Vec<FormatEntry> = Vec::new();
        let mut keywords: Vec<FormatEntry> = Vec::new();
        let mut content_media_types: Vec<FormatEntry> = Vec::new();
        let mut content_encodings: Vec<ContentEncodingEntry> = Vec::new();
        let mut ignore_unknown_formats = None;
        let mut email_options = None;
        let mut pattern_options = PatternOptionsConfig::default();
        let mut methods = context::MethodGates::default();

        let mut seen_keys = HashSet::new();
        while !input.is_empty() {
            let ident: Ident = input.parse()?;

            let key_name = ident.to_string();
            if !seen_keys.insert(key_name.clone()) {
                return Err(syn::Error::new_spanned(
                    &ident,
                    format!("Duplicate attribute: `{key_name}`"),
                ));
            }

            match key_name.as_str() {
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
                    // Accept a bare variant or any qualified path to it; only the final segment matters.
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
                    let mut seen_uris = HashSet::new();
                    while !content.is_empty() {
                        let uri: LitStr = content.parse()?;
                        if !seen_uris.insert(uri.value()) {
                            return Err(syn::Error::new_spanned(
                                &uri,
                                format!("Duplicate resource URI: `{}`", uri.value()),
                            ));
                        }
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
                        if entry_content_tokens.peek(Token![,]) {
                            entry_content_tokens.parse::<Token![,]>()?;
                        }
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
                    formats = parse_named_fn_paths(input, "format")?;
                }
                "keywords" => {
                    input.parse::<Token![=]>()?;
                    let entries = parse_named_fn_paths(input, "keyword")?;
                    if let Some(entry) = entries
                        .iter()
                        .find(|entry| BUILTIN_KEYWORDS.contains(&entry.name.as_str()))
                    {
                        return Err(syn::Error::new_spanned(
                            ident,
                            format!(
                                "Overriding the built-in `{}` keyword is not supported",
                                entry.name
                            ),
                        ));
                    }
                    keywords = entries;
                }
                "content_media_types" => {
                    input.parse::<Token![=]>()?;
                    content_media_types = parse_named_fn_paths(input, "content media type")?;
                }
                "content_encodings" => {
                    input.parse::<Token![=]>()?;
                    content_encodings = parse_content_encodings(input)?;
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
                "methods" => {
                    methods = parse_methods(input)?;
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        ident,
                        "Expected `path`, `schema`, `draft`, `base_uri`, `resources`, `validate_formats`, `formats`, `keywords`, `content_media_types`, `content_encodings`, `ignore_unknown_formats`, `email_options`, `pattern_options`, or `methods` attribute",
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
            keywords,
            content_media_types,
            content_encodings,
            ignore_unknown_formats,
            email_options,
            pattern_options,
            methods,
        })
    }
}

fn validator_impl(attr: &Config, input: &ItemStruct) -> syn::Result<proc_macro2::TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "Generic structs are not supported: the schema is fixed at compile time, \
so the generated methods cannot depend on type parameters",
        ));
    }
    let (schema, mut recompile_triggers): (
        serde_json::Result<serde_json::Value>,
        Vec<proc_macro2::TokenStream>,
    ) = match &attr.source {
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

    let draft = detect_draft(&schema, attr.draft.as_ref())?;
    let runtime_crate_alias = resolve_runtime_crate_alias();

    let resource_ref = draft.create_resource_ref(&schema);
    let base_uri = if let Some(base_uri) = &attr.base_uri {
        Cow::Owned(base_uri.value())
    } else if let Some(id) = resource_ref.id() {
        Cow::Borrowed(id)
    } else {
        Cow::Owned(format!("json-schema:///{name}"))
    };

    let mut resource_pairs: Vec<(String, referencing::Resource)> =
        vec![(base_uri.to_string(), draft.create_resource(schema.clone()))];

    for entry in &attr.resources {
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

    // Cover the root schema AND every registered resource: the keyword in any one of them means the
    // generated validator needs `unevaluated*` helpers.
    let (uses_unevaluated_properties, uses_unevaluated_items) =
        crate::codegen::scan_uses_unevaluated_over(
            resource_pairs.iter().map(|(_, r)| r.contents()),
        );

    // Validate the base URI first so a malformed one gives a clear error, not an opaque registry one.
    let base_uri_uri = uri::from_str(&base_uri).map_err(|err| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("Invalid base URI: {err}"),
        )
    })?;

    let registry = Registry::new()
        .draft(draft)
        .extend(
            resource_pairs
                .iter()
                .map(|(uri, r)| (uri.as_str(), r.clone())),
        )
        .and_then(referencing::RegistryBuilder::prepare)
        .map_err(|err| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("Registry error: {err}"),
            )
        })?;

    let config = context::CodegenConfig {
        schema,
        registry,
        base_uri: Arc::new(base_uri_uri),
        draft,
        runtime_crate_alias: Some(runtime_crate_alias),
        validate_formats: attr.validate_formats,
        custom_formats: attr
            .formats
            .iter()
            .map(|entry| {
                let path = &entry.path;
                (entry.name.clone(), quote! { #path })
            })
            .collect(),
        custom_keywords: attr
            .keywords
            .iter()
            .map(|entry| {
                let path = &entry.path;
                (entry.name.clone(), quote! { #path })
            })
            .collect(),
        content_media_types: attr
            .content_media_types
            .iter()
            .map(|entry| {
                let path = &entry.path;
                (entry.name.clone(), quote! { #path })
            })
            .collect(),
        content_encodings: attr
            .content_encodings
            .iter()
            .map(|entry| {
                let check = &entry.check;
                let convert = &entry.convert;
                (entry.name.clone(), (quote! { #check }, quote! { #convert }))
            })
            .collect(),
        ignore_unknown_formats: attr.ignore_unknown_formats.unwrap_or(true),
        email_options: attr.email_options,
        pattern_options: match attr.pattern_options.engine {
            PatternEngine::FancyRegex => context::PatternEngineConfig::FancyRegex {
                backtrack_limit: attr.pattern_options.backtrack_limit,
                size_limit: attr.pattern_options.size_limit,
                dfa_size_limit: attr.pattern_options.dfa_size_limit,
            },
            PatternEngine::Regex => context::PatternEngineConfig::Regex {
                size_limit: attr.pattern_options.size_limit,
                dfa_size_limit: attr.pattern_options.dfa_size_limit,
            },
        },
        uses_unevaluated_properties,
        uses_unevaluated_items,
        method_gates: attr.methods,
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
    if let Some(draft_expr) = draft {
        let syn::Expr::Path(path) = draft_expr else {
            return Err(syn::Error::new_spanned(
                draft_expr,
                "Invalid `draft` expression. Expected a draft variant such as `Draft202012`",
            ));
        };
        let item = path
            .path
            .segments
            .last()
            .expect("a parsed path expression always has at least one segment");

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

fn resolve_runtime_crate_alias() -> proc_macro2::TokenStream {
    // `jsonschema` is always resolvable here: the `#[jsonschema::validator]` attribute path
    // only resolves when the annotated crate depends on `jsonschema`.
    match crate_name("jsonschema").expect("`jsonschema` runtime crate is resolvable") {
        FoundCrate::Itself => quote! { crate },
        FoundCrate::Name(found) => {
            let ident = format_ident!("{found}");
            quote! { ::#ident }
        }
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

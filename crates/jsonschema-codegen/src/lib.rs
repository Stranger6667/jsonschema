use std::{borrow::Cow, env, fs::File, io::BufReader, path::PathBuf, sync::Arc};

use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use serde_json::Value;
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input, Ident, ItemStruct, LitStr, Token,
};

mod codegen;
use codegen::Codegen;
use referencing::{uri, Draft, Registry};

/// Generates a JSON Schema validator for the given schema.
///
/// # Usage
///
/// ```ignore
/// // From file path
/// #[jsonschema::validator(path = "schema.json")]
/// struct FooValidator;
///
/// // From inline JSON string
/// #[jsonschema::validator(schema = r#"{"type": "string"}"#)]
/// struct BarValidator;
/// ```
///
/// This generates an `impl` block with `is_valid` and `validate` methods.
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
}

enum SchemaSource {
    Path(LitStr),
    Schema(LitStr),
}

impl Parse for Config {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut schema_source = None;
        let mut draft = None;

        // Parse key-value pairs
        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match ident.to_string().as_str() {
                "path" => {
                    let value: LitStr = input.parse()?;
                    schema_source = Some(SchemaSource::Path(value));
                }
                "schema" => {
                    let value: LitStr = input.parse()?;
                    schema_source = Some(SchemaSource::Schema(value));
                }
                "draft" => {
                    // Parse as expression: referencing::Draft::Draft4, etc.
                    draft = Some(input.parse()?);
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        ident,
                        "Expected `path`, `schema`, or `draft` attribute",
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
        })
    }
}

fn validator_impl(config: &Config, input: &ItemStruct) -> syn::Result<proc_macro2::TokenStream> {
    let (schema, recompile_trigger): (serde_json::Result<serde_json::Value>, _) =
        match &config.source {
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
                    quote! {
                        const _: &str = include_str!(#path);
                    },
                )
            }
            SchemaSource::Schema(value) => {
                let content = value.value();
                (serde_json::from_str(&content), quote! {})
            }
        };

    let schema =
        schema.map_err(|err| syn::Error::new_spanned(input, format!("Invalid JSON: {err}")))?;

    let name = &input.ident;

    let draft = detect_draft(&schema, config.draft.as_ref())?;

    // TODO: it will be simpler just to add `id` to `Resource`
    let resource_ref = draft.create_resource_ref(&schema);
    let base_uri = if let Some(id) = resource_ref.id() {
        Cow::Borrowed(id)
    } else {
        Cow::Owned(format!("json-schema:///{name}"))
    };

    // TODO: it will be nice if registry will accept resources by ref
    let resource = draft.create_resource(schema.clone());
    let registry = Registry::options()
        .draft(draft)
        .build([(&base_uri, resource)])
        .map_err(|err| {
            // TODO: it is better to return a compile error. But keep it for now to avoid compile errors in the test suite
            eprintln!("Registry build failed: {err:?}");
            err
        })
        .ok();

    // TODO: don't hide errors on uri parsing
    let base_uri = uri::from_str(&base_uri).ok().map(Arc::new);
    let codegen = Codegen::new(&schema, draft, registry, base_uri);

    let impl_methods = codegen.generate(&recompile_trigger);

    Ok(quote! {
        #input

        impl #name {
            #impl_methods
        }
    })
}

fn detect_draft(schema: &Value, draft: Option<&syn::Expr>) -> syn::Result<Draft> {
    // Check the explicit draft first
    if let Some(syn::Expr::Path(path)) = draft {
        let Some(item) = path.path.segments.last() else {
            return Err(syn::Error::new_spanned(
                path,
                format!("Invalid draft: `{}`", path.to_token_stream()),
            ));
        };

        // Can't match on ident, but comparisons are faster than allocating a string + match
        Ok(if item.ident == "Draft4" {
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
            Draft::default().detect(schema)
        })
    } else {
        Ok(Draft::default().detect(schema))
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

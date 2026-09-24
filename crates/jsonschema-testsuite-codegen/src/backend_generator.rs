use std::{fs, path::Path};

use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use serde_json::Value;
use testsuite_internal::Case;

use crate::{files, loader};

/// One validator per suite case for the given backend, and the table entry that finds it by id.
pub(crate) fn generate(
    backend: &Ident,
    suite_path: &str,
    drafts: &[String],
    remote_resources: &[(String, String)],
) -> Result<TokenStream, Box<dyn std::error::Error>> {
    let mut definitions = Vec::new();
    let mut entries = Vec::new();
    for draft in drafts {
        let root = Path::new(suite_path).join("tests").join(draft);
        let draft_variant = match draft.as_str() {
            "draft4" => format_ident!("Draft4"),
            "draft6" => format_ident!("Draft6"),
            "draft7" => format_ident!("Draft7"),
            "draft2019-09" => format_ident!("Draft201909"),
            _ => format_ident!("Draft202012"),
        };
        for path in files::json_files(&root)? {
            let relative = path
                .strip_prefix(&root)?
                .to_str()
                .ok_or("Invalid filename")?
                .replace('\\', "/");
            let contents = fs::read_to_string(&path)?;
            let cases: Vec<Case> =
                serde_json::from_str(&loader::sanitize_lone_surrogates(&contents))?;
            let validate_formats = if relative.starts_with("optional/") {
                quote! { , validate_formats = true }
            } else {
                quote! {}
            };
            for (index, case) in cases.iter().enumerate() {
                let schema = serde_json::to_string(&case.schema)?;
                let resources = if schema.contains("localhost:1234") {
                    let resources = remote_resources
                        .iter()
                        .map(|(uri, contents)| quote! { #uri => { schema = #contents } });
                    quote! { , resources = { #(#resources),* } }
                } else {
                    quote! {}
                };
                let ident = format_ident!("Validator{}", definitions.len());
                let id = format!("{draft}|{relative}|{index}");
                definitions.push(quote! {
                    #[jsonschema::validator(
                        schema = #schema,
                        draft = #draft_variant,
                        backend = #backend
                        #resources
                        #validate_formats
                    )]
                    struct #ident;
                });
                entries.push(entry(&ident, &id));
            }
        }
    }
    Ok(quote! {
        #(#definitions)*

        pub static SUITE_ENTRIES: &[SuiteEntry] = &[#(#entries),*];
    })
}

/// One validator per schema in `{"keywords": {name: path}, "schemas": [...]}` for the given
/// backend, found by the schema's JSON with sorted keys. Every schema gets every listed custom
/// keyword.
pub(crate) fn generate_schemas(
    backend: &Ident,
    path: &str,
) -> Result<TokenStream, Box<dyn std::error::Error>> {
    let file: Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    let keywords = file["keywords"]
        .as_object()
        .ok_or("`keywords` must be an object")?
        .iter()
        .map(|(name, factory)| {
            let factory: syn::Path =
                syn::parse_str(factory.as_str().ok_or("factory must be a path")?)?;
            Ok(quote! { #name => #factory })
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
    let mut definitions = Vec::new();
    let mut entries = Vec::new();
    for (index, schema) in file["schemas"]
        .as_array()
        .ok_or("`schemas` must be an array")?
        .iter()
        .enumerate()
    {
        let schema = serde_json::to_string(&sorted(schema))?;
        let ident = format_ident!("SchemaValidator{index}");
        definitions.push(quote! {
            #[jsonschema::validator(schema = #schema, backend = #backend, keywords = { #(#keywords),* })]
            struct #ident;
        });
        entries.push(entry(&ident, &schema));
    }
    Ok(quote! {
        // Rebuilds when the schema list changes.
        const _: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../", #path));

        #(#definitions)*

        pub static SCHEMA_ENTRIES: &[SuiteEntry] = &[#(#entries),*];
    })
}

fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut members: Vec<_> = object.iter().collect();
            members.sort_by_key(|(key, _)| *key);
            Value::Object(
                members
                    .into_iter()
                    .map(|(key, member)| (key.clone(), sorted(member)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

fn entry(ident: &Ident, id: &str) -> TokenStream {
    quote! {
        SuiteEntry {
            id: #id,
            is_valid: |instance| #ident::is_valid(instance),
            validate: |instance| {
                Ok(#ident::validate(instance)?.err().map(|error| error.to_string()))
            },
            iter_errors: |instance| {
                Ok(#ident::iter_errors(instance)?.map(|error| error.to_string()).collect())
            },
        }
    }
}

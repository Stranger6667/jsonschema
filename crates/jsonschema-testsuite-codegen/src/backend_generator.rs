use std::fs;

use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use serde_json::Value;

use crate::loader;

/// One validator per suite case for the given backend, and the table entry that finds it by id.
pub(crate) fn generate(
    backend: &Ident,
    suite_path: &str,
    drafts: &[String],
) -> Result<TokenStream, Box<dyn std::error::Error>> {
    let mut definitions = Vec::new();
    let mut entries = Vec::new();
    for draft in drafts {
        let draft_variant = match draft.as_str() {
            "draft4" => format_ident!("Draft4"),
            "draft6" => format_ident!("Draft6"),
            "draft7" => format_ident!("Draft7"),
            "draft2019-09" => format_ident!("Draft201909"),
            _ => format_ident!("Draft202012"),
        };
        for file in loader::load_files(suite_path, draft)? {
            let relative = &file.relative;
            for case in &file.cases {
                let index = case.index;
                let schema = serde_json::to_string(&case.schema)?;
                let resources = if case.external_schemas.is_empty() {
                    quote! {}
                } else {
                    let resources = case
                        .external_schemas
                        .iter()
                        .map(|(uri, schema)| {
                            let contents = serde_json::to_string(schema)?;
                            Ok(quote! { #uri => { schema = #contents } })
                        })
                        .collect::<Result<Vec<_>, serde_json::Error>>()?;
                    quote! { , resources = { #(#resources),* } }
                };
                let (validate_formats, runtime_formats) = match loader::validate_formats(case) {
                    Some(value) => (
                        quote! { , validate_formats = #value },
                        quote! { .should_validate_formats(#value) },
                    ),
                    None => (quote! {}, quote! {}),
                };
                let ident = format_ident!("Validator{}", definitions.len());
                let id = format!("{draft}|{relative}|{index}");
                // The runtime gets the documents the generated validator was given, not the network.
                let externals = case
                    .external_schemas
                    .iter()
                    .map(|(uri, schema)| {
                        let contents = serde_json::to_string(schema)?;
                        Ok(quote! {
                            (#uri, serde_json::from_str(#contents).expect("suite resource is JSON"))
                        })
                    })
                    .collect::<Result<Vec<_>, serde_json::Error>>()?;
                let evaluate = quote! {
                    |instance| {
                        static RUNTIME: std::sync::LazyLock<
                            jsonschema::Validator<jsonschema::json::#backend>,
                        > = std::sync::LazyLock::new(|| {
                            let externals: Vec<(&str, serde_json::Value)> = vec![#(#externals),*];
                            let mut registry =
                                jsonschema::Registry::new().draft(jsonschema::Draft::#draft_variant);
                            for (uri, contents) in &externals {
                                // Like a retrieved document, each is in the draft its own `$schema` names.
                                let resource = jsonschema::Draft::#draft_variant
                                    .detect(contents)
                                    .create_resource(contents.clone());
                                registry = registry
                                    .add(*uri, resource)
                                    .expect("suite resource is accepted");
                            }
                            let registry = registry.prepare().expect("suite resources resolve");
                            jsonschema::options_for::<jsonschema::json::#backend>()
                                .with_draft(jsonschema::Draft::#draft_variant)
                                .with_registry(&registry)
                                #runtime_formats
                                .build(&serde_json::from_str(#schema).expect("suite schema is JSON"))
                                .expect("suite schema compiles")
                        });
                        let generated = #ident::evaluate(instance)?;
                        let runtime = runtime_evaluate(&RUNTIME, instance)?;
                        Ok(Some(
                            serde_json::json!([outputs(&generated), outputs(&runtime)]).to_string(),
                        ))
                    }
                };
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
                entries.push(entry(&ident, &id, &evaluate));
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
        entries.push(entry(&ident, &schema, &quote! { |_| Ok(None) }));
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

fn entry(ident: &Ident, id: &str, evaluate: &TokenStream) -> TokenStream {
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
            evaluate: #evaluate,
        }
    }
}

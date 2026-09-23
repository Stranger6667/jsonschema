use std::{fs, path::Path};

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use testsuite_internal::Case;

use crate::{files, loader};

/// One `backend = Pyo3` validator per suite case, and the table entry that finds it by id.
pub(crate) fn generate(
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
                        backend = Pyo3
                        #resources
                        #validate_formats
                    )]
                    struct #ident;
                });
                entries.push(quote! {
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
                });
            }
        }
    }
    Ok(quote! {
        #(#definitions)*

        pub static SUITE_ENTRIES: &[SuiteEntry] = &[#(#entries),*];
    })
}

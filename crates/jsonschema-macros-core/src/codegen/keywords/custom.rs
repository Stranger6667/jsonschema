use quote::{format_ident, quote};
use serde_json::{Map, Value};

use super::super::{CompileContext, CompiledExpr};

/// Compile a user-registered keyword into a lazily constructed `Keyword`. The factory runs at
/// first use; a failing factory panics, since the schema is fixed at compile time and the failure
/// is deterministic.
pub(crate) fn compile(
    ctx: &mut CompileContext<'_>,
    name: &str,
    parent: &Map<String, Value>,
    value: &Value,
) -> CompiledExpr {
    let factory_path = ctx.config.custom_keywords[name].clone();
    let index = ctx.custom_keyword_counter;
    ctx.custom_keyword_counter += 1;
    let static_ident = format_ident!("__CUSTOM_KEYWORD_{index}");
    let parent_json = serde_json::to_string(&Value::Object(parent.clone()))
        .expect("Failed to serialize parent schema");
    let value_json = serde_json::to_string(value).expect("Failed to serialize keyword value");
    let schema_path = ctx.schema_path_for_keyword(name);

    let lazy = quote! {
        static #static_ident: std::sync::LazyLock<Box<dyn jsonschema::Keyword>> =
            std::sync::LazyLock::new(|| {
                let parent: serde_json::Value = serde_json::from_str(#parent_json)
                    .expect("Failed to parse parent schema");
                let parent = parent.as_object().expect("parent schema is an object");
                let value: serde_json::Value = serde_json::from_str(#value_json)
                    .expect("Failed to parse keyword value");
                match #factory_path(
                    parent,
                    &value,
                    jsonschema::__private::custom::location(#schema_path),
                ) {
                    Ok(keyword) => keyword,
                    Err(error) => {
                        panic!("Custom keyword `{}` factory failed: {error}", #name)
                    }
                }
            });
    };

    CompiledExpr::with_validate_blocks(
        quote! {
            {
                #lazy
                #static_ident.is_valid(instance)
            }
        },
        quote! {
            #lazy
            if let Some(__err) = jsonschema::__private::custom::validate(
                &**#static_ident,
                instance,
                __path.into(),
                #schema_path,
                #name,
            ) {
                return Some(__err);
            }
        },
    )
}

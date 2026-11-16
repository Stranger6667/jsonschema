use proc_macro2::TokenStream;
use quote::quote;
use referencing::Draft;
use serde_json::{Map, Value};

use super::{
    emit::ValueEmitter,
    keywords::{format::format_emits_assertion, type_::simple_type_pattern},
};
use crate::context::CompileContext;

/// Keywords whose units `evaluate` reports, in the runtime's priority order.
const LEAF_KEYWORDS: [&str; 21] = [
    "type",
    "const",
    "enum",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "minProperties",
    "maxProperties",
    "required",
    "pattern",
    "format",
    "contentEncoding",
    "contentMediaType",
    "contentSchema",
    "uniqueItems",
];

/// A unit: the schema's own, by default, or a keyword's.
#[derive(Default)]
struct Unit {
    location: String,
    /// Other locations its errors are reported at.
    aliases: Vec<&'static str>,
    /// Its annotation of a string instance, as JSON.
    string_annotation: Option<String>,
}

impl Unit {
    fn new(keyword: &str) -> Self {
        Unit {
            location: format!("/{keyword}"),
            aliases: Vec::new(),
            string_annotation: None,
        }
    }

    fn with_alias(mut self, alias: &'static str) -> Self {
        self.aliases.push(alias);
        self
    }

    fn annotating(mut self, value: &Value) -> Self {
        self.string_annotation = Some(value.to_string());
        self
    }
}

/// The body of the generated `evaluate`. Keyword units take their errors from one run of the
/// schema's errors function, by schema location, so no keyword's error code is emitted twice.
pub(super) fn compile<E: ValueEmitter>(
    ctx: &CompileContext<'_, E>,
    schema: &Value,
) -> syn::Result<TokenStream> {
    if let Some(message) = unsupported(schema, ctx.draft) {
        return Err(syn::Error::new(proc_macro2::Span::call_site(), message));
    }
    // The schema's own unit comes first.
    let mut units = vec![Unit::default()];
    let mut annotations = quote! { None };
    if let Value::Object(keywords) = schema {
        if let Some(keyword) = keywords
            .keys()
            .find(|keyword| ctx.config.custom_keywords.contains_key(*keyword))
        {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`evaluate` does not support the custom `{keyword}` keyword yet"),
            ));
        }
        units.extend(
            LEAF_KEYWORDS
                .iter()
                .filter_map(|keyword| unit(ctx, keyword, keywords.get(*keyword)?, keywords)),
        );
        // The draft's unknown keywords annotate the schema, as the runtime reports them.
        let unknown: serde_json::Map<_, _> = keywords
            .iter()
            .filter(|(keyword, _)| !ctx.draft.is_known_keyword(keyword))
            .map(|(keyword, value)| (keyword.clone(), value.clone()))
            .collect();
        if !unknown.is_empty() {
            let json = Value::Object(unknown).to_string();
            annotations = quote! { Some(#json) };
        }
    }
    let pointers = units.iter().map(|unit| &unit.location);
    let aliases = units.iter().enumerate().flat_map(|(index, unit)| {
        unit.aliases
            .iter()
            .map(move |alias| quote! { (#alias, #index) })
    });
    let string_annotations = units.iter().enumerate().filter_map(|(index, unit)| {
        let json = unit.string_annotation.as_ref()?;
        Some(quote! { (#index, #json) })
    });
    // Only string annotations read it.
    let string = if units.iter().any(|unit| unit.string_annotation.is_some()) {
        E::type_matches(simple_type_pattern::<E>("string").into_iter().collect())
    } else {
        quote! { false }
    };
    let base = ctx.config.base_uri.as_str();
    // Without a base of its own the schema gets a synthetic one, which the output leaves out.
    let base = if base.starts_with("json-schema:///") {
        quote! { None }
    } else {
        let base = base.trim_end_matches('#');
        quote! { Some(#base) }
    };
    Ok(quote! {
        static __LOCATIONS: __eval::Locations = __eval::Locations::new(
            &[#(#pointers),*],
            &[#(#aliases),*],
            #base,
            #annotations,
            &[#(#string_annotations),*],
        );
        let mut __errors = __err::Errors::new(false);
        let _ = errors(instance, &__paths::LazyLocation::new(), &mut __errors);
        __LOCATIONS.evaluate(&__errors.into_vec(), #string)
    })
}

/// A keyword's unit, or `None` when the runtime gives the keyword none.
fn unit<E: ValueEmitter>(
    ctx: &CompileContext<'_, E>,
    keyword: &str,
    value: &Value,
    keywords: &Map<String, Value>,
) -> Option<Unit> {
    // The draft's unknown keywords annotate the schema instead.
    if !ctx.draft.is_known_keyword(keyword) {
        return None;
    }
    let unit = Unit::new(keyword);
    match (ctx.draft, keyword) {
        // Draft 4's boolean flags make a bound exclusive; the bound's unit reports their errors.
        (Draft::Draft4, "exclusiveMinimum" | "exclusiveMaximum") => None,
        (Draft::Draft4, "minimum") => Some(unit.with_alias("/exclusiveMinimum")),
        (Draft::Draft4, "maximum") => Some(unit.with_alias("/exclusiveMaximum")),
        (_, "uniqueItems") if value == &Value::Bool(false) => None,
        // The runtime skips a format it validates formats with but does not know.
        (_, "format") if ctx.validates_formats() && !format_emits_assertion(ctx, value) => None,
        (_, "format") => Some(unit.annotating(value)),
        // Draft 6 and 7 assert content, reporting both keywords in the `contentMediaType` unit.
        (Draft::Draft6 | Draft::Draft7, "contentEncoding")
            if keywords.contains_key("contentMediaType") =>
        {
            None
        }
        (Draft::Draft6 | Draft::Draft7, "contentMediaType")
            if keywords.contains_key("contentEncoding") =>
        {
            Some(unit.with_alias("/contentEncoding"))
        }
        (Draft::Draft6 | Draft::Draft7, _) => Some(unit),
        (_, "contentSchema") if !keywords.contains_key("contentMediaType") => None,
        (_, "contentEncoding" | "contentMediaType" | "contentSchema") => {
            Some(unit.annotating(value))
        }
        _ => Some(unit),
    }
}

/// Why the generated `evaluate` cannot handle `schema` yet, if it cannot.
pub(crate) fn unsupported(schema: &Value, draft: Draft) -> Option<String> {
    match schema {
        Value::Bool(_) => None,
        Value::Object(keywords) => keywords
            .iter()
            .find(|(keyword, value)| !supports(keyword, value, draft))
            .map(|(keyword, _)| format!("`evaluate` does not support the `{keyword}` keyword yet")),
        _ => Some("`evaluate` needs a schema that is a boolean or an object".to_string()),
    }
}

fn supports(keyword: &str, value: &Value, draft: Draft) -> bool {
    match keyword {
        // A custom meta-schema could change which keywords apply.
        "$schema" => value
            .as_str()
            .is_some_and(|uri| Draft::from_schema_uri(uri) == draft),
        // The runtime skips a divisor beyond `f64`.
        "multipleOf" => value.as_f64().is_some_and(f64::is_finite),
        // An unknown keyword is an annotation.
        _ => LEAF_KEYWORDS.contains(&keyword) || !draft.is_known_keyword(keyword),
    }
}

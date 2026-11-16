use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::{write_escaped_str, Draft};
use serde_json::{json, Map, Value};

use super::{
    emit::ValueEmitter,
    keywords::{format::format_emits_assertion, pattern_properties::name_match_expr},
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

/// A keyword's unit that checks the instance alone.
struct Unit {
    keyword: String,
    /// Another keyword its errors are reported at.
    alias: Option<&'static str>,
    /// Its annotation of a string instance.
    string_annotation: Option<Value>,
}

impl Unit {
    fn new(keyword: &str) -> Self {
        Unit {
            keyword: keyword.to_string(),
            alias: None,
            string_annotation: None,
        }
    }

    fn with_alias(mut self, alias: &'static str) -> Self {
        self.alias = Some(alias);
        self
    }

    fn annotating(mut self, value: &Value) -> Self {
        self.string_annotation = Some(value.clone());
        self
    }
}

enum TreeUnit {
    Leaf(Unit),
    /// A range of `Tree::properties`.
    Properties {
        declared: (usize, usize),
        in_member_order: bool,
    },
    /// A range of `Tree::patterns`.
    Patterns((usize, usize)),
    Additional {
        schema: usize,
        closed: bool,
        declared: Option<(usize, usize)>,
        patterns: Option<(usize, usize)>,
    },
    Names {
        schema: usize,
        closed: bool,
    },
}

struct TreeSchema {
    pointer: String,
    /// The draft's unknown keywords.
    annotations: Option<Value>,
    /// A range of `Tree::units`.
    first: usize,
    count: usize,
}

/// Every schema `evaluate` reports, the root first, with their units in the runtime's priority
/// order.
#[derive(Default)]
struct Tree {
    schemas: Vec<TreeSchema>,
    units: Vec<TreeUnit>,
    /// Each declared property, with its schema's index.
    properties: Vec<(String, usize)>,
    /// Each `patternProperties` subschema's index, with the name match its matcher runs.
    patterns: Vec<(usize, TokenStream)>,
}

impl Tree {
    fn add<E: ValueEmitter>(
        &mut self,
        ctx: &mut CompileContext<'_, E>,
        schema: &Value,
        pointer: String,
    ) -> syn::Result<usize> {
        let index = self.schemas.len();
        self.schemas.push(TreeSchema {
            pointer: String::new(),
            annotations: None,
            first: 0,
            count: 0,
        });
        let mut units = Vec::new();
        let mut annotations = None;
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
                    .filter_map(|keyword| unit(ctx, keyword, keywords.get(*keyword)?, keywords))
                    .map(TreeUnit::Leaf),
            );
            let additional = keywords
                .get("additionalProperties")
                .filter(|value| **value != Value::Bool(true));
            let mut declared = None;
            if let Some(Value::Object(properties)) = keywords.get("properties") {
                let mut entries = Vec::with_capacity(properties.len());
                for (name, subschema) in properties {
                    let mut child = format!("{pointer}/properties/");
                    write_escaped_str(&mut child, name);
                    entries.push((name.clone(), self.add(ctx, subschema, child)?));
                }
                let range = (self.properties.len(), entries.len());
                self.properties.extend(entries);
                declared = Some(range);
                // Beside `additionalProperties`, the runtime checks both in one pass.
                units.push(TreeUnit::Properties {
                    declared: range,
                    in_member_order: additional.is_some(),
                });
            }
            let mut patterns = None;
            if let Some(Value::Object(subschemas)) = keywords.get("patternProperties") {
                let mut entries = Vec::with_capacity(subschemas.len());
                for (pattern, subschema) in subschemas {
                    let mut child = format!("{pointer}/patternProperties/");
                    write_escaped_str(&mut child, pattern);
                    let schema = self.add(ctx, subschema, child)?;
                    let matches = ctx
                        .with_schema_path_swap(pointer.clone(), |ctx| {
                            name_match_expr(ctx, pattern, &quote! { name })
                        })
                        .map_err(|_| {
                            syn::Error::new(
                                proc_macro2::Span::call_site(),
                                format!("`evaluate` does not support the `{pattern}` pattern"),
                            )
                        })?;
                    entries.push((schema, matches.is_valid));
                }
                let range = (self.patterns.len(), entries.len());
                self.patterns.extend(entries);
                patterns = Some(range);
                units.push(TreeUnit::Patterns(range));
            }
            if let Some(subschema) = additional {
                let schema = self.add(ctx, subschema, format!("{pointer}/additionalProperties"))?;
                units.push(TreeUnit::Additional {
                    schema,
                    closed: *subschema == Value::Bool(false),
                    declared,
                    patterns,
                });
            }
            if let Some(subschema) = keywords
                .get("propertyNames")
                .filter(|value| **value != Value::Bool(true))
                .filter(|_| ctx.draft.is_known_keyword("propertyNames"))
            {
                let schema = self.add(ctx, subschema, format!("{pointer}/propertyNames"))?;
                units.push(TreeUnit::Names {
                    schema,
                    closed: *subschema == Value::Bool(false),
                });
            }
            // The draft's unknown keywords annotate the schema, as the runtime reports them.
            let unknown: Map<_, _> = keywords
                .iter()
                .filter(|(keyword, _)| !ctx.draft.is_known_keyword(keyword))
                .map(|(keyword, value)| (keyword.clone(), value.clone()))
                .collect();
            if !unknown.is_empty() {
                annotations = Some(Value::Object(unknown));
            }
        }
        self.schemas[index] = TreeSchema {
            pointer,
            annotations,
            first: self.units.len(),
            count: units.len(),
        };
        self.units.extend(units);
        Ok(index)
    }
}

/// The body of the generated `evaluate`. Keyword units take their errors from one run of the
/// schema's errors function, by schema location and instance location, so no keyword's error code
/// is emitted twice.
pub(super) fn compile<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    schema: &Value,
) -> syn::Result<TokenStream> {
    if let Some(message) = unsupported(schema, ctx.draft) {
        return Err(syn::Error::new(proc_macro2::Span::call_site(), message));
    }
    let mut tree = Tree::default();
    tree.add(ctx, schema, String::new())?;
    let schemas: Vec<Value> = tree
        .schemas
        .into_iter()
        .map(|schema| {
            let mut entry = json!({
                "pointer": schema.pointer,
                "first": schema.first,
                "count": schema.count,
            });
            if let Some(annotations) = schema.annotations {
                entry["annotations"] = annotations;
            }
            entry
        })
        .collect();
    let units: Vec<Value> = tree
        .units
        .into_iter()
        .map(|unit| match unit {
            TreeUnit::Leaf(unit) => {
                let mut entry = json!({"keyword": unit.keyword});
                if let Some(alias) = unit.alias {
                    entry["alias"] = json!(alias);
                }
                if let Some(annotation) = unit.string_annotation {
                    entry["strings"] = annotation;
                }
                entry
            }
            TreeUnit::Properties {
                declared,
                in_member_order,
            } => {
                json!({"keyword": "properties", "properties": declared, "members": in_member_order})
            }
            TreeUnit::Patterns(patterns) => {
                json!({"keyword": "patternProperties", "patterns": patterns})
            }
            TreeUnit::Additional {
                schema,
                closed,
                declared,
                patterns,
            } => json!({
                "keyword": "additionalProperties",
                "additional": schema,
                "closed": closed,
                "properties": declared,
                "patterns": patterns,
            }),
            TreeUnit::Names { schema, closed } => {
                json!({"keyword": "propertyNames", "names": schema, "closed": closed})
            }
        })
        .collect();
    let mut tables = json!({"schemas": schemas, "units": units});
    if !tree.properties.is_empty() {
        tables["properties"] = json!(tree.properties);
    }
    if !tree.patterns.is_empty() {
        tables["patterns"] = json!(tree
            .patterns
            .iter()
            .map(|(schema, _)| schema)
            .collect::<Vec<_>>());
    }
    let matchers: Vec<_> = (0..tree.patterns.len())
        .map(|index| format_ident!("__matches_{index}"))
        .collect();
    let matches = tree.patterns.iter().map(|(_, matches)| matches);
    let base = ctx.config.base_uri.as_str();
    // Without a base of its own the schema gets a synthetic one, which the output leaves out.
    if !base.starts_with("json-schema:///") {
        tables["base"] = json!(base.trim_end_matches('#'));
    }
    let tables = tables.to_string();
    let json = E::json_representation();
    Ok(quote! {
        #(fn #matchers(name: &str) -> bool { #matches })*
        static __TREE: __eval::Tree = __eval::Tree {
            tables: #tables,
            matchers: &[#(#matchers),*],
            built: ::std::sync::OnceLock::new(),
        };
        let mut __errors = __err::Errors::new(false);
        let _ = errors(instance, &__paths::LazyLocation::new(), &mut __errors);
        <#json as __eval::Walk>::evaluate(&__TREE, &instance, &__errors.into_vec())
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
        (Draft::Draft4, "minimum") => Some(unit.with_alias("exclusiveMinimum")),
        (Draft::Draft4, "maximum") => Some(unit.with_alias("exclusiveMaximum")),
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
            Some(unit.with_alias("contentEncoding"))
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
        Value::Object(keywords) => keywords.iter().find_map(|(keyword, value)| {
            if !draft.is_known_keyword(keyword) {
                // An unknown keyword is an annotation.
                return None;
            }
            match (keyword.as_str(), value) {
                ("properties" | "patternProperties", Value::Object(subschemas)) => {
                    return subschemas
                        .values()
                        .find_map(|subschema| unsupported(subschema, draft));
                }
                ("additionalProperties" | "propertyNames", Value::Bool(_) | Value::Object(_)) => {
                    return unsupported(value, draft);
                }
                _ => {}
            }
            (!supports(keyword, value, draft))
                .then(|| format!("`evaluate` does not support the `{keyword}` keyword yet"))
        }),
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
        _ => LEAF_KEYWORDS.contains(&keyword),
    }
}

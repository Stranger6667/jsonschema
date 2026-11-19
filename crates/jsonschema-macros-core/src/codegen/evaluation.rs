use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::{write_escaped_str, Draft, Registry, Resolved, Resolver, Uri, VocabularySet};
use serde_json::{json, Map, Value};
use std::{
    borrow::Cow,
    collections::{BTreeSet, HashMap, HashSet},
    sync::Arc,
};

use super::{
    compile_schema,
    draft::DraftExt,
    emit::ValueEmitter,
    helpers::{
        collect_dynamic_anchor_bindings, dynamic_ref_anchor_name, get_or_create_is_valid_fn,
        get_or_create_item_eval_fn, get_or_create_key_eval_fn, location_pointer,
    },
    keywords::{
        format::format_emits_assertion,
        one_of::{branch_discriminator, BranchDiscriminator, DiscriminatorLiteral},
        pattern_properties::name_match_expr,
        unevaluated_items::unevaluated_items,
        unevaluated_properties::unevaluated_members,
    },
    parse_nonnegative_integer_keyword,
    refs::resolve_ref,
    stack_emit::{
        pop_dynamic_errors_n, pop_recursive_errors, pop_recursive_is_valid,
        pop_recursive_item_eval, pop_recursive_key_eval, push_dynamic_errors,
        push_recursive_errors, push_recursive_is_valid, push_recursive_item_eval,
        push_recursive_key_eval, IS_VALID_FAMILY, ITEM_EVAL_FAMILY, KEY_EVAL_FAMILY,
    },
};
use crate::context::CompileContext;

/// Keywords whose units `evaluate` reports, in the runtime's priority order.
const LEAF_KEYWORDS: [&str; 22] = [
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
    "dependentRequired",
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
    /// Its keyword alone, to check where the errors run is skipped.
    check: Option<LeafCheck>,
    /// Its generated check, which reports its errors without the errors run.
    direct: Option<usize>,
}

/// How a leaf unit reporting its errors itself words them.
enum DirectErrors {
    /// The errors code of its keyword alone.
    Reported(TokenStream),
    /// The message of a single `type`, with a slot for the instance.
    NotOfType(String),
}

/// A leaf unit's check among the generated ones of its shape.
enum DirectSlot {
    NotOfType(usize),
    Reported(usize),
}

/// The `minContains` and `maxContains` beside a `contains`.
struct ContainsBounds {
    min: Option<u64>,
    max: Option<u64>,
}

/// A leaf keyword alone, with the dialect it is read in.
struct LeafCheck {
    schema: Value,
    draft: Draft,
    vocabularies: VocabularySet,
}

impl Unit {
    fn new(keyword: &str) -> Self {
        Unit {
            keyword: keyword.to_string(),
            alias: None,
            string_annotation: None,
            check: None,
            direct: None,
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
    /// `contains`, with its subschema's index in `Tree::valid`, and its `minContains` and
    /// `maxContains`.
    Contains {
        schema: usize,
        valid: usize,
        bounds: ContainsBounds,
    },
    /// `allOf`, `anyOf` or `oneOf`, with a range of `Tree::tuples` and, unless every branch
    /// applies, where their checks start in `Tree::valid`.
    Branches {
        keyword: &'static str,
        schemas: (usize, usize),
        valid: Option<usize>,
        /// Its entry in `Tree::discriminators`.
        discriminator: Option<usize>,
    },
    /// `if`, with its subschema's index in `Tree::valid`.
    Condition {
        schema: usize,
        valid: usize,
    },
    /// `then` or `else`.
    Branch {
        keyword: &'static str,
        schema: usize,
    },
    /// `$ref`, `$dynamicRef` or `$recursiveRef`, with its static target's index.
    Ref {
        keyword: &'static str,
        target: usize,
        /// Its static target's code, an index of `Tree::functions`.
        function: usize,
        /// Its target is in another document than the schema holding it.
        leaves: bool,
        /// The `$dynamicAnchor` a `$dynamicRef` resolves through the dynamic scope.
        anchor: Option<String>,
        /// A `$recursiveRef` resolves through the dynamic scope.
        recursive: bool,
    },
    /// Array-form `items` or `prefixItems`, with a range of `Tree::tuples`.
    Tuple {
        keyword: &'static str,
        schemas: (usize, usize),
    },
    /// Schema-form `items` or `additionalItems`: a subschema for each item from `skip` on.
    Rest {
        keyword: &'static str,
        schema: usize,
        skip: usize,
        closed: bool,
    },
    /// `dependencies` or `dependentSchemas`, with a range of `Tree::dependencies`.
    Dependencies {
        keyword: &'static str,
        schemas: (usize, usize),
    },
    /// `unevaluatedProperties` or `unevaluatedItems`: its subschema for each member or item the
    /// schema leaves unevaluated, which `Tree::unevaluated` lists.
    Unevaluated {
        keyword: &'static str,
        schema: usize,
        closed: bool,
        evaluated: usize,
    },
}

struct TreeSchema {
    pointer: String,
    /// Its index in `Tree::resources`.
    resource: usize,
    /// The resource it sits in, where that is not the root's.
    base: Option<String>,
    /// It sits in another resource than the root's, under the default base: its keywords have no
    /// absolute location.
    default_base: bool,
    /// Its pointer within that resource, where that is not `pointer`.
    at: Option<String>,
    /// The draft's unknown keywords.
    annotations: Option<Value>,
    /// A range of `Tree::units`.
    first: usize,
    count: usize,
    /// It is `false`.
    rejects: bool,
}

/// Where a schema sits: the resource holding it, its pointer, which counts from the document a
/// reference names and may enter a resource it embeds, and the length of the pointer's prefix
/// before that resource starts.
#[derive(PartialEq, Eq, Hash)]
struct TreePlace {
    base: String,
    pointer: String,
    resource_start: usize,
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
    /// Each property whose presence applies a subschema, with that subschema's index.
    dependencies: Vec<(String, usize)>,
    /// Each subschema listed in an array, by position.
    tuples: Vec<usize>,
    /// The `is_valid` code of each subschema a unit checks, run against `instance`.
    valid: Vec<TokenStream>,
    /// For each discriminated `anyOf` or `oneOf`, code giving the branches that may accept
    /// `instance`.
    discriminators: Vec<TokenStream>,
    /// The resource each `valid` entry runs in.
    valid_resources: Vec<usize>,
    /// For each `unevaluated*` unit, code pushing to `__unevaluated` the positions of the members
    /// or items of `instance` its schema leaves unevaluated, and the errors code of its subschema:
    /// the errors run only checks that subschema's validity.
    unevaluated: Vec<(TokenStream, TokenStream)>,
    /// The resource each `unevaluated` entry runs in.
    unevaluated_resources: Vec<usize>,
    /// Each schema's index by where it sits, so a `$ref` target is added once.
    by_pointer: HashMap<TreePlace, usize>,
    /// Where the resource being added starts, as a prefix of the pointers below it.
    resource_root: String,
    /// The next schema added is a reference target, whose resolution already applied its `$id`.
    based: bool,
    /// Each resource a schema sits in, the root's first.
    resources: Vec<TreeResource>,
    resource_ids: HashMap<String, usize>,
    /// The location of each reference target's code a `$ref` enters.
    functions: Vec<String>,
    function_ids: HashMap<String, usize>,
    /// The code the schemas being added run in: the innermost reference target's, `None` for the
    /// root's.
    function: Option<usize>,
    /// The addresses of the schemas being added from there on, the outermost first.
    inline: Vec<usize>,
}

/// A resource the dynamic scope may hold.
struct TreeResource {
    base: Arc<Uri<String>>,
    /// Its root, where a `$recursiveRef` may resolve to.
    root: Option<usize>,
    /// Its root has `$recursiveAnchor: true`.
    recursive: bool,
    /// Its `$dynamicAnchor` schemas, where a `$dynamicRef` may resolve to.
    anchors: Vec<(String, usize)>,
    /// Anchor names already looked up in it.
    looked_up: HashSet<String>,
}

impl Tree {
    /// The `is_valid` code of the subschema at `location`. One applying to the instance of its
    /// unit is compiled as in the code of the reference targets holding that unit, so its
    /// references re-entering them on the instance pass their cycle guards as the runtime does.
    fn add_validity<E: ValueEmitter>(
        &mut self,
        ctx: &mut CompileContext<'_, E>,
        subschema: &Value,
        location: String,
        same_instance: bool,
    ) {
        let targets = if same_instance {
            self.compiling(ctx)
        } else {
            Vec::new()
        };
        let check = ctx.with_schema_path_swap(location, |ctx| {
            ctx.within_targets(&targets, |ctx| compile_schema(ctx, subschema))
        });
        self.valid.push(check.is_valid_token_stream());
        let resource = self.resource(ctx);
        self.valid_resources.push(resource);
    }

    fn add_tuple<E: ValueEmitter>(
        &mut self,
        ctx: &mut CompileContext<'_, E>,
        schemas: &[Value],
        pointer: &str,
        keyword: &str,
    ) -> syn::Result<(usize, usize)> {
        let mut entries = Vec::with_capacity(schemas.len());
        for (index, subschema) in schemas.iter().enumerate() {
            entries.push(self.add(ctx, subschema, format!("{pointer}/{keyword}/{index}"))?);
        }
        let range = (self.tuples.len(), entries.len());
        self.tuples.extend(entries);
        Ok(range)
    }

    fn add<E: ValueEmitter>(
        &mut self,
        ctx: &mut CompileContext<'_, E>,
        schema: &Value,
        pointer: String,
    ) -> syn::Result<usize> {
        let based = std::mem::take(&mut self.based);
        let place = TreePlace {
            base: ctx.current_base_uri.to_string(),
            pointer,
            resource_start: self.resource_root.len(),
        };
        if let Some(&index) = self.by_pointer.get(&place) {
            return Ok(index);
        }
        let pointer = place.pointer.clone();
        let index = self.schemas.len();
        self.schemas.push(TreeSchema {
            pointer: String::new(),
            resource: 0,
            base: None,
            default_base: false,
            at: None,
            annotations: None,
            first: 0,
            count: 0,
            rejects: false,
        });
        self.by_pointer.insert(place, index);
        self.inline.push(std::ptr::from_ref(schema).addr());
        let added = match resource_base(ctx, schema).filter(|_| index != 0 && !based) {
            Some(base) => {
                let outer = std::mem::replace(&mut self.resource_root, pointer.clone());
                let added = ctx.with_base_uri_scope(base, |ctx| {
                    self.add_keywords(ctx, schema, index, pointer)
                });
                self.resource_root = outer;
                added
            }
            None => self.add_keywords(ctx, schema, index, pointer),
        };
        self.inline.pop();
        added
    }

    /// The locations of the reference targets whose code holds the schema being added, while
    /// it compiles: the targets whose code reached the innermost one's, and the targets the
    /// schemas from there on are.
    fn compiling<E: ValueEmitter>(&self, ctx: &CompileContext<'_, E>) -> Vec<String> {
        let mut locations = Vec::new();
        let mut current = self.function.map(|function| &self.functions[function]);
        while let Some(location) = current {
            locations.push(location.clone());
            current = ctx.compiled_within.get(location).and_then(Option::as_ref);
        }
        for location in self
            .inline
            .iter()
            .filter_map(|schema| ctx.target_locations.get(schema))
            .flatten()
        {
            if !locations.contains(location) {
                locations.push(location.clone());
            }
        }
        locations
    }

    fn add_keywords<E: ValueEmitter>(
        &mut self,
        ctx: &mut CompileContext<'_, E>,
        schema: &Value,
        index: usize,
        pointer: String,
    ) -> syn::Result<usize> {
        let mut units = Vec::new();
        let mut annotations = None;
        self.schemas[index].rejects = *schema == Value::Bool(false);
        if let Value::Object(keywords) = schema {
            let keywords = &active_keywords(ctx, keywords);
            let reference = keywords.get("$ref").and_then(Value::as_str);
            if let Some(reference) = reference.filter(|_| !ctx.draft.supports_adjacent_validation())
            {
                // The keywords beside `$ref` apply from Draft 2019-09 on.
                let units = self.reference(ctx, index, "$ref", reference)?;
                self.finish(ctx, index, pointer, None, units.into_iter().collect());
                return Ok(index);
            }
            let leaves: Vec<Unit> = LEAF_KEYWORDS
                .iter()
                .filter_map(|keyword| unit(ctx, keyword, keywords.get(*keyword)?, keywords))
                .collect();
            for mut leaf in leaves {
                if leaf.alias.is_none() {
                    let value = keywords
                        .get(&leaf.keyword)
                        .expect("a leaf unit's keyword is in its schema");
                    leaf.check = Some(LeafCheck {
                        schema: Value::Object(Map::from_iter([(
                            leaf.keyword.clone(),
                            value.clone(),
                        )])),
                        draft: ctx.draft,
                        vocabularies: ctx.vocabularies.clone(),
                    });
                }
                units.push(TreeUnit::Leaf(leaf));
            }
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
            let prefix = keywords
                .get("prefixItems")
                .and_then(Value::as_array)
                .filter(|_| ctx.draft.is_known_keyword("prefixItems"));
            match keywords.get("items") {
                Some(Value::Array(schemas)) => {
                    let schemas = self.add_tuple(ctx, schemas, &pointer, "items")?;
                    units.push(TreeUnit::Tuple {
                        keyword: "items",
                        schemas,
                    });
                }
                Some(subschema @ (Value::Object(_) | Value::Bool(false))) => {
                    let schema = self.add(ctx, subschema, format!("{pointer}/items"))?;
                    units.push(TreeUnit::Rest {
                        keyword: "items",
                        schema,
                        skip: prefix.map_or(0, Vec::len),
                        closed: false,
                    });
                }
                _ => {}
            }
            if let Some(prefix) = prefix {
                let schemas = self.add_tuple(ctx, prefix, &pointer, "prefixItems")?;
                units.push(TreeUnit::Tuple {
                    keyword: "prefixItems",
                    schemas,
                });
            }
            // `additionalItems` applies past an array-form `items` only.
            if let (
                Some(Value::Array(tuple)),
                Some(subschema @ (Value::Object(_) | Value::Bool(false))),
            ) = (keywords.get("items"), keywords.get("additionalItems"))
            {
                let schema = self.add(ctx, subschema, format!("{pointer}/additionalItems"))?;
                units.push(TreeUnit::Rest {
                    keyword: "additionalItems",
                    schema,
                    skip: tuple.len(),
                    // The errors run reports one error for every extra item of `false`.
                    closed: *subschema == Value::Bool(false),
                });
            }
            if let Some(subschema @ (Value::Bool(_) | Value::Object(_))) = keywords
                .get("contains")
                .filter(|_| ctx.draft.is_known_keyword("contains"))
            {
                let location = format!("{pointer}/contains");
                let schema = self.add(ctx, subschema, location.clone())?;
                // A bound that is not a count fails the build in the validation code.
                let bound = |keyword: &str| {
                    keywords
                        .get(keyword)
                        .filter(|_| ctx.draft.supports_contains_bounds_keyword())
                        .and_then(|value| parse_nonnegative_integer_keyword(ctx.draft, value).ok())
                };
                units.push(TreeUnit::Contains {
                    schema,
                    valid: self.valid.len(),
                    bounds: ContainsBounds {
                        min: bound("minContains"),
                        max: bound("maxContains"),
                    },
                });
                self.add_validity(ctx, subschema, location, false);
            }
            for keyword in ["dependencies", "dependentSchemas"] {
                let Some(Value::Object(dependencies)) = keywords
                    .get(keyword)
                    .filter(|_| ctx.draft.is_known_keyword(keyword))
                else {
                    continue;
                };
                let mut entries = Vec::new();
                for (name, dependency) in dependencies {
                    // Required names are errors of the keyword's own unit.
                    if dependency.is_array() {
                        continue;
                    }
                    let mut child = format!("{pointer}/{keyword}/");
                    write_escaped_str(&mut child, name);
                    entries.push((name.clone(), self.add(ctx, dependency, child)?));
                }
                let schemas = (self.dependencies.len(), entries.len());
                self.dependencies.extend(entries);
                units.push(TreeUnit::Dependencies { keyword, schemas });
            }
            for keyword in ["allOf", "anyOf", "oneOf"] {
                let Some(Value::Array(schemas)) = keywords.get(keyword) else {
                    continue;
                };
                // Every `allOf` branch applies; the others apply the valid ones if any.
                let valid = (keyword != "allOf").then_some(self.valid.len());
                let mut discriminator = None;
                if valid.is_some() {
                    for (index, subschema) in schemas.iter().enumerate() {
                        self.add_validity(
                            ctx,
                            subschema,
                            format!("{pointer}/{keyword}/{index}"),
                            true,
                        );
                    }
                    discriminator = branch_discriminator(ctx, schemas).map(|discriminator| {
                        self.discriminators.push(candidates(&discriminator));
                        self.discriminators.len() - 1
                    });
                }
                let schemas = self.add_tuple(ctx, schemas, &pointer, keyword)?;
                units.push(TreeUnit::Branches {
                    keyword,
                    schemas,
                    valid,
                    discriminator,
                });
            }
            // `not` reports its error alone.
            if let Some(negated) = keywords.get("not") {
                let mut unit = Unit::new("not");
                // Checked apart from the walk, its subschema has no scope to resolve references in.
                if !references(negated) {
                    unit.check = Some(LeafCheck {
                        schema: json!({"not": negated}),
                        draft: ctx.draft,
                        vocabularies: ctx.vocabularies.clone(),
                    });
                }
                units.push(TreeUnit::Leaf(unit));
            }
            // `if` without `then` or `else` checks nothing.
            if let Some(condition) = keywords
                .get("if")
                .filter(|_| ctx.draft.is_known_keyword("if"))
                .filter(|_| keywords.contains_key("then") || keywords.contains_key("else"))
            {
                let location = format!("{pointer}/if");
                let schema = self.add(ctx, condition, location.clone())?;
                units.push(TreeUnit::Condition {
                    schema,
                    valid: self.valid.len(),
                });
                self.add_validity(ctx, condition, location, true);
                for keyword in ["then", "else"] {
                    if let Some(branch) = keywords.get(keyword) {
                        let schema = self.add(ctx, branch, format!("{pointer}/{keyword}"))?;
                        units.push(TreeUnit::Branch { keyword, schema });
                    }
                }
            }
            for keyword in ["unevaluatedProperties", "unevaluatedItems"] {
                let Some(subschema) = keywords
                    .get(keyword)
                    .filter(|value| **value != Value::Bool(true))
                    .filter(|_| ctx.draft.is_known_keyword(keyword))
                else {
                    continue;
                };
                let location = format!("{pointer}/{keyword}");
                let schema = self.add(ctx, subschema, location.clone())?;
                let closed = *subschema == Value::Bool(false);
                let errors = if closed {
                    TokenStream::new()
                } else {
                    let compiled =
                        ctx.with_schema_path_swap(location, |ctx| compile_schema(ctx, subschema));
                    compiled.guard_errors(&compiled.errors.as_token_stream())
                };
                let evaluated = self.unevaluated.len();
                let positions = ctx.with_schema_path_swap(pointer.clone(), |ctx| {
                    if keyword == "unevaluatedProperties" {
                        unevaluated_members(ctx, keywords)
                    } else {
                        unevaluated_items(ctx, keywords)
                    }
                });
                self.unevaluated.push((positions, errors));
                let resource = self.resource(ctx);
                self.unevaluated_resources.push(resource);
                units.push(TreeUnit::Unevaluated {
                    keyword,
                    schema,
                    closed,
                    evaluated,
                });
            }
            if let Some(reference) = reference {
                units.extend(self.reference(ctx, index, "$ref", reference)?);
            }
            for keyword in ["$recursiveRef", "$dynamicRef"] {
                if let Some(reference) = keywords
                    .get(keyword)
                    .and_then(Value::as_str)
                    .filter(|_| ctx.draft.is_known_keyword(keyword))
                {
                    units.extend(self.reference(ctx, index, keyword, reference)?);
                }
            }
            // Custom keywords apply last, in the schema's order.
            units.extend(
                keywords
                    .keys()
                    .filter(|keyword| ctx.config.custom_keywords.contains_key(*keyword))
                    .map(|keyword| TreeUnit::Leaf(Unit::new(keyword))),
            );
            // The draft's unknown keywords annotate the schema, as the runtime reports them.
            let unknown: Map<_, _> = keywords
                .iter()
                .filter(|(keyword, _)| {
                    !ctx.draft.is_known_keyword(keyword)
                        && !ctx.config.custom_keywords.contains_key(*keyword)
                })
                .map(|(keyword, value)| (keyword.clone(), value.clone()))
                .collect();
            if !unknown.is_empty() {
                annotations = Some(Value::Object(unknown));
            }
        }
        self.finish(ctx, index, pointer, annotations, units);
        Ok(index)
    }

    fn finish<E: ValueEmitter>(
        &mut self,
        ctx: &CompileContext<'_, E>,
        index: usize,
        pointer: String,
        annotations: Option<Value>,
        units: Vec<TreeUnit>,
    ) {
        let resource = self.resource(ctx);
        let base = &ctx.current_base_uri;
        let other = base.as_str() != ctx.config.base_uri.as_str();
        // The output locates keywords of a resource under the default base by pointer alone.
        let default_base = other && base.scheme().as_str() == "json-schema";
        let base =
            (other && !default_base).then(|| base.as_str().trim_end_matches('#').to_string());
        let at = pointer
            .strip_prefix(self.resource_root.as_str())
            .filter(|at| at.len() != pointer.len())
            .map(str::to_string);
        self.schemas[index] = TreeSchema {
            pointer,
            resource,
            base,
            default_base,
            at,
            annotations,
            first: self.units.len(),
            count: units.len(),
            rejects: self.schemas[index].rejects,
        };
        self.units.extend(units);
    }

    /// The index in `resources` of the resource `ctx` is in.
    fn resource<E: ValueEmitter>(&mut self, ctx: &CompileContext<'_, E>) -> usize {
        let base = ctx.current_base_uri.to_string();
        if let Some(&id) = self.resource_ids.get(&base) {
            return id;
        }
        let id = self.resources.len();
        self.resources.push(TreeResource {
            base: ctx.current_base_uri.clone(),
            root: None,
            recursive: false,
            anchors: Vec::new(),
            looked_up: HashSet::new(),
        });
        self.resource_ids.insert(base, id);
        id
    }

    /// The unit of a `$ref`, `$dynamicRef` or `$recursiveRef` in the schema at `index`. A
    /// reference to that schema itself has none, as the runtime compiles none.
    fn reference<E: ValueEmitter>(
        &mut self,
        ctx: &mut CompileContext<'_, E>,
        index: usize,
        keyword: &'static str,
        reference: &str,
    ) -> syn::Result<Option<TreeUnit>> {
        let resolved =
            resolve_ref(ctx, reference).expect("`unsupported` resolves every `$ref` first");
        let anchor = (keyword == "$dynamicRef")
            .then(|| dynamic_ref_anchor_name(reference, resolved.schema))
            .flatten();
        let recursive = keyword == "$recursiveRef"
            && resolved.schema.get("$recursiveAnchor") == Some(&Value::Bool(true));
        let function = self.function_id(resolved.location);
        // The target's schemas run in its own code.
        let outer_function = self.function.replace(function);
        let outer_inline = std::mem::take(&mut self.inline);
        let target = self.add_reference(ctx, reference);
        self.function = outer_function;
        self.inline = outer_inline;
        let target = target?;
        if target == index {
            return Ok(None);
        }
        Ok(Some(TreeUnit::Ref {
            keyword,
            leaves: leaves_document(ctx, reference),
            target,
            function,
            anchor,
            recursive,
        }))
    }

    /// The index in `functions` of the reference target code at `location`.
    fn function_id(&mut self, location: String) -> usize {
        if let Some(&id) = self.function_ids.get(&location) {
            return id;
        }
        let id = self.functions.len();
        self.functions.push(location.clone());
        self.function_ids.insert(location, id);
        id
    }

    /// Every schema a reference resolving through the dynamic scope may land on, in every
    /// resource the scope may hold.
    fn add_scope_targets<E: ValueEmitter>(
        &mut self,
        ctx: &mut CompileContext<'_, E>,
    ) -> syn::Result<()> {
        loop {
            let mut names: Vec<String> = Vec::new();
            let mut recursive = false;
            for unit in &self.units {
                if let TreeUnit::Ref {
                    anchor,
                    recursive: through_scope,
                    ..
                } = unit
                {
                    recursive |= *through_scope;
                    if let Some(anchor) = anchor {
                        if !names.contains(anchor) {
                            names.push(anchor.clone());
                        }
                    }
                }
            }
            let added = (self.schemas.len(), self.resources.len());
            for index in 0..self.resources.len() {
                let base = self.resources[index].base.clone();
                let resolver = ctx.config.registry.resolver((*base).clone());
                for name in &names {
                    if !self.resources[index].looked_up.insert(name.clone()) {
                        continue;
                    }
                    let reference = format!("#{name}");
                    let Ok(resolved) = resolver.lookup(&reference) else {
                        continue;
                    };
                    let (contents, _, draft) = resolved.into_inner();
                    if dynamic_ref_anchor_name(&reference, contents).is_none() {
                        continue;
                    }
                    let location = format!("{}{reference}", base.as_str());
                    let pointer = location_pointer(ctx, &location, &base);
                    let target = self.add_in(ctx, &base, draft, contents, pointer)?;
                    self.resources[index].anchors.push((name.clone(), target));
                }
                if recursive && self.resources[index].root.is_none() {
                    let (contents, _, draft) = resolver
                        .lookup("#")
                        .expect("a resource's root resolves")
                        .into_inner();
                    self.resources[index].recursive = draft.supports_recursive_ref_keyword()
                        && contents.get("$recursiveAnchor") == Some(&Value::Bool(true));
                    let root = self.add_in(ctx, &base, draft, contents, String::new())?;
                    self.resources[index].root = Some(root);
                }
            }
            if (self.schemas.len(), self.resources.len()) == added {
                return Ok(());
            }
        }
    }

    /// `schema` at `pointer` within the resource at `base`, in that resource's `draft`.
    fn add_in<E: ValueEmitter>(
        &mut self,
        ctx: &mut CompileContext<'_, E>,
        base: &Arc<Uri<String>>,
        draft: Draft,
        schema: &Value,
        pointer: String,
    ) -> syn::Result<usize> {
        let outer = std::mem::take(&mut self.resource_root);
        let added = in_resource(ctx, base.clone(), draft, schema, |ctx| {
            self.add(ctx, schema, pointer)
        });
        self.resource_root = outer;
        added
    }

    /// The schema a `$ref` points to, located within its resource.
    fn add_reference<E: ValueEmitter>(
        &mut self,
        ctx: &mut CompileContext<'_, E>,
        reference: &str,
    ) -> syn::Result<usize> {
        let resolved =
            resolve_ref(ctx, reference).expect("`unsupported` resolves every `$ref` first");
        let pointer = location_pointer(ctx, &resolved.location, &resolved.base_uri);
        // A pointer can reach into a resource embedded in the document it starts from.
        let resource = ctx
            .config
            .registry
            .resolver((*resolved.base_uri).clone())
            .lookup("")
            .map_err(|error| syn::Error::new(proc_macro2::Span::call_site(), error.to_string()))?;
        let within = pointer_within(resource.contents(), &pointer, resolved.schema)
            .unwrap_or_else(|| pointer.clone());
        let root = pointer
            .strip_suffix(within.as_str())
            .unwrap_or_default()
            .to_string();
        let outer = std::mem::replace(&mut self.resource_root, root);
        let added = in_resource(
            ctx,
            resolved.base_uri,
            resolved.draft,
            resolved.schema,
            |ctx| {
                self.based = true;
                self.add(ctx, resolved.schema, pointer)
            },
        );
        self.resource_root = outer;
        added
    }
}

/// Where `target` sits below `root`: at `pointer` if that leads to it, otherwise wherever it is.
fn pointer_within(root: &Value, pointer: &str, target: &Value) -> Option<String> {
    let mut node = Some(root);
    for segment in pointer.split('/').skip(1) {
        let segment = segment.replace("~1", "/").replace("~0", "~");
        node = node.and_then(|node| match node {
            Value::Object(map) => map.get(&segment),
            Value::Array(items) => segment.parse::<usize>().ok().and_then(|i| items.get(i)),
            _ => None,
        });
    }
    if node.is_some_and(|node| std::ptr::eq(node, target)) {
        return Some(pointer.to_string());
    }
    let mut found = String::new();
    find_pointer(root, target, &mut found).then_some(found)
}

fn find_pointer(node: &Value, target: &Value, pointer: &mut String) -> bool {
    if std::ptr::eq(node, target) {
        return true;
    }
    let len = pointer.len();
    let found = match node {
        Value::Object(map) => map.iter().any(|(key, child)| {
            pointer.truncate(len);
            pointer.push('/');
            write_escaped_str(pointer, key);
            find_pointer(child, target, pointer)
        }),
        Value::Array(items) => items.iter().enumerate().any(|(index, child)| {
            pointer.truncate(len);
            pointer.push('/');
            pointer.push_str(&index.to_string());
            find_pointer(child, target, pointer)
        }),
        _ => false,
    };
    if !found {
        pointer.truncate(len);
    }
    found
}

/// `f` with `ctx` at a reference target `contents` in the resource at `base`: in that resource's
/// `draft`, with the vocabularies the target or its resource names.
fn in_resource<'cfg, E: ValueEmitter, T>(
    ctx: &mut CompileContext<'cfg, E>,
    base: Arc<Uri<String>>,
    draft: Draft,
    contents: &Value,
    f: impl FnOnce(&mut CompileContext<'cfg, E>) -> T,
) -> T {
    let vocabularies = ctx
        .config
        .registry
        .resolver((*base).clone())
        .find_vocabularies(draft, contents);
    let outer_draft = std::mem::replace(&mut ctx.draft, draft);
    let outer_vocabularies = std::mem::replace(&mut ctx.vocabularies, vocabularies);
    let result = ctx.with_base_uri_scope(base, f);
    ctx.draft = outer_draft;
    ctx.vocabularies = outer_vocabularies;
    result
}

/// `keywords` without those the active vocabularies leave without effect: the runtime gives them
/// neither a unit nor an annotation.
fn active_keywords<'k, E: ValueEmitter>(
    ctx: &CompileContext<'_, E>,
    keywords: &'k Map<String, Value>,
) -> Cow<'k, Map<String, Value>> {
    let inactive = |keyword: &String| {
        ctx.draft.is_known_keyword(keyword) && !ctx.keyword_in_vocabulary(keyword)
    };
    if keywords.keys().any(inactive) {
        Cow::Owned(
            keywords
                .iter()
                .filter(|(keyword, _)| !inactive(keyword))
                .map(|(keyword, value)| (keyword.clone(), value.clone()))
                .collect(),
        )
    } else {
        Cow::Borrowed(keywords)
    }
}

/// Whether `reference` names another document than the one `ctx` is in, so following it adds that
/// one to the dynamic scope.
fn leaves_document<E: ValueEmitter>(ctx: &CompileContext<'_, E>, reference: &str) -> bool {
    if reference.starts_with('#') {
        return false;
    }
    let document = reference
        .split_once('#')
        .map_or(reference, |(document, _)| document);
    ctx.config
        .registry
        .resolver((*ctx.current_base_uri).clone())
        .lookup(document)
        .map_or(true, |resolved| {
            resolved.resolver().base_uri().as_str() != ctx.current_base_uri.as_str()
        })
}

/// The base URI `schema`'s own `$id` sets, if it names another resource.
fn resource_base<E: ValueEmitter>(
    ctx: &CompileContext<'_, E>,
    schema: &Value,
) -> Option<Arc<Uri<String>>> {
    // Up to Draft 7 the keywords beside `$ref`, `$id` included, do not apply.
    if !ctx.draft.supports_adjacent_validation() && schema.get("$ref").is_some() {
        return None;
    }
    let id = schema.get(id_keyword(ctx.draft))?.as_str()?;
    let base = ctx
        .config
        .registry
        .resolver((*ctx.current_base_uri).clone())
        .lookup(id)
        .ok()?
        .resolver()
        .base_uri();
    (base.as_str() != ctx.current_base_uri.as_str()).then_some(base)
}

fn id_keyword(draft: Draft) -> &'static str {
    if draft == Draft::Draft4 {
        "id"
    } else {
        "$id"
    }
}

/// The body of the generated `evaluate`. Where every unit can report its own errors, each leaf
/// keyword gets one check shared by its equal copies; otherwise units take their errors from one
/// run of the schema's errors function, by schema location and instance location.
pub(super) fn compile<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    schema: &Value,
) -> syn::Result<TokenStream> {
    if let Some(message) = unsupported(
        &ctx.config.registry,
        &ctx.config.base_uri,
        schema,
        ctx.draft,
    ) {
        return Err(syn::Error::new(proc_macro2::Span::call_site(), message));
    }
    let config = ctx.config;
    // The registry's copy, which the code of reference targets compiles from: a schema has one
    // address in both.
    let root = config
        .registry
        .resolver((*config.base_uri).clone())
        .lookup("")
        .map_err(|error| syn::Error::new(proc_macro2::Span::call_site(), error.to_string()))?
        .contents();
    let mut tree = Tree::default();
    tree.add(ctx, root, String::new())?;
    tree.add_scope_targets(ctx)?;
    // Every error comes from a unit reporting it itself.
    let without_errors_run = tree.units.iter().all(|unit| match unit {
        TreeUnit::Leaf(unit) => unit.check.is_some(),
        TreeUnit::Properties { .. }
        | TreeUnit::Patterns(_)
        | TreeUnit::Additional { .. }
        | TreeUnit::Names { .. }
        | TreeUnit::Branches { .. }
        | TreeUnit::Condition { .. }
        | TreeUnit::Branch { .. }
        | TreeUnit::Ref { .. }
        | TreeUnit::Rest { .. }
        | TreeUnit::Tuple { .. }
        | TreeUnit::Contains { .. } => true,
        TreeUnit::Dependencies { .. } | TreeUnit::Unevaluated { .. } => false,
    });
    let mut direct = Vec::new();
    if without_errors_run {
        let mut by_code = HashMap::new();
        for unit in &mut tree.units {
            let TreeUnit::Leaf(unit) = unit else {
                continue;
            };
            let LeafCheck {
                schema,
                draft,
                vocabularies,
            } = unit.check.take().expect("every leaf unit is checked alone");
            let outer_draft = std::mem::replace(&mut ctx.draft, draft);
            let outer_vocabularies = std::mem::replace(&mut ctx.vocabularies, vocabularies);
            // Its errors do not depend on where it is, so equal leaves share a check.
            let check =
                ctx.with_schema_path_swap(String::new(), |ctx| compile_schema(ctx, &schema));
            ctx.draft = outer_draft;
            ctx.vocabularies = outer_vocabularies;
            let is_valid = check.is_valid_token_stream();
            let errors = match schema.get("type") {
                // The runtime words a single type's error without building it.
                Some(Value::String(type_)) if !check.is_compile_error() => {
                    DirectErrors::NotOfType(format!(r#"{{}} is not of type "{type_}""#))
                }
                _ => DirectErrors::Reported(check.errors.as_token_stream()),
            };
            let key = match &errors {
                DirectErrors::NotOfType(message) => format!("{is_valid}{message}"),
                DirectErrors::Reported(errors) => format!("{is_valid}{errors}"),
            };
            let count = direct.len();
            let index = *by_code.entry(key).or_insert(count);
            if index == count {
                direct.push((is_valid, errors));
            }
            unit.direct = Some(index);
        }
    }
    // Each check's place among the generated checks of its shape.
    let mut shapes = (0, 0);
    let direct_slots: Vec<DirectSlot> = direct
        .iter()
        .map(|(_, errors)| match errors {
            DirectErrors::NotOfType(_) => {
                shapes.0 += 1;
                DirectSlot::NotOfType(shapes.0 - 1)
            }
            DirectErrors::Reported(_) => {
                shapes.1 += 1;
                DirectSlot::Reported(shapes.1 - 1)
            }
        })
        .collect();
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
            if schema.resource != 0 {
                entry["resource"] = json!(schema.resource);
            }
            if let Some(base) = schema.base {
                entry["base"] = json!(base);
            }
            if schema.default_base {
                entry["default_base"] = json!(true);
            }
            if let Some(at) = schema.at {
                entry["at"] = json!(at);
            }
            if schema.rejects {
                entry["rejects"] = json!(true);
            }
            entry
        })
        .collect();
    // References through the dynamic scope enter their targets under guards of their own.
    let entered_statically: HashSet<usize> = tree
        .units
        .iter()
        .filter_map(|unit| match unit {
            TreeUnit::Ref {
                function,
                anchor: None,
                recursive: false,
                ..
            } => Some(*function),
            _ => None,
        })
        .collect();
    let units: Vec<Value> = tree
        .units
        .into_iter()
        .map(|unit| match unit {
            TreeUnit::Leaf(unit) => {
                let mut entry = json!({"keyword": unit.keyword});
                match unit.direct.map(|index| &direct_slots[index]) {
                    Some(DirectSlot::NotOfType(slot)) => entry["not_of_type"] = json!(slot),
                    Some(DirectSlot::Reported(slot)) => entry["direct"] = json!(slot),
                    None => {}
                }
                if let Some(alias) = unit.alias {
                    entry["aliases"] = json!([alias]);
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
            TreeUnit::Dependencies { keyword, schemas } => {
                json!({"keyword": keyword, "dependencies": schemas})
            }
            TreeUnit::Tuple { keyword, schemas } => json!({"keyword": keyword, "tuple": schemas}),
            TreeUnit::Branches {
                keyword,
                schemas,
                valid,
                discriminator,
            } => {
                let mut entry = json!({"keyword": keyword, "branches": schemas, "valid": valid});
                if let Some(discriminator) = discriminator {
                    entry["discriminator"] = json!(discriminator);
                }
                entry
            }
            TreeUnit::Condition { schema, valid } => {
                json!({"keyword": "if", "condition": [schema, valid]})
            }
            TreeUnit::Branch { keyword, schema } => json!({"keyword": keyword, "branch": schema}),
            TreeUnit::Ref {
                keyword,
                target,
                function,
                leaves,
                anchor,
                recursive,
            } => {
                let mut entry = json!({"keyword": keyword, "target": target, "function": function});
                let mut scope = Map::new();
                if leaves {
                    scope.insert("leaves".to_string(), json!(true));
                }
                if let Some(anchor) = anchor {
                    scope.insert("anchor".to_string(), json!(anchor));
                }
                if recursive {
                    scope.insert("recursive".to_string(), json!(true));
                }
                if !scope.is_empty() {
                    entry["scope"] = Value::Object(scope);
                }
                entry
            }
            // The errors run reports the count errors at the bound keywords, unless the unit
            // counts the matching items itself.
            TreeUnit::Contains {
                schema,
                valid,
                bounds: ContainsBounds { min, max },
            } => json!({
                "keyword": "contains",
                "contains": [schema, valid],
                "aliases": ["minContains", "maxContains"],
                "bounds": {"min": min, "max": max},
            }),
            TreeUnit::Rest {
                keyword,
                schema,
                skip,
                closed,
            } => json!({"keyword": keyword, "rest": schema, "skip": skip, "closed": closed}),
            TreeUnit::Unevaluated {
                keyword,
                schema,
                closed,
                evaluated,
            } => json!({
                "keyword": keyword,
                "unevaluated": [schema, evaluated],
                "closed": closed,
            }),
        })
        .collect();
    let mut tables = json!({"schemas": schemas, "units": units});
    if tree
        .resources
        .iter()
        .any(|resource| resource.root.is_some() || !resource.anchors.is_empty())
    {
        tables["resources"] = tree
            .resources
            .iter()
            .map(|resource| {
                json!({
                    "root": resource.root,
                    "recursive": resource.recursive,
                    "anchors": resource.anchors,
                })
            })
            .collect();
    }
    // A `$ref` re-entering a target while the target's code compiles enters it under the guard
    // that code shares with its own references to the target.
    let mut guarded = Vec::new();
    let functions: Vec<Value> = tree
        .functions
        .iter()
        .enumerate()
        .map(|(id, location)| {
            let mut entry = json!({});
            let mut within = ctx.compiled_within.get(location).and_then(Option::as_ref);
            while let Some(outer) = within {
                if let Some(parent) = tree.function_ids.get(outer) {
                    entry["parent"] = json!(parent);
                    break;
                }
                within = ctx.compiled_within.get(outer).and_then(Option::as_ref);
            }
            if entered_statically.contains(&id) && ctx.reentered_targets.contains(location) {
                entry["guard"] = json!(guarded.len());
                guarded.push(format_ident!(
                    "{}",
                    ctx.is_valid_fns
                        .get_name(location)
                        .expect("a re-entered target has code")
                ));
            }
            entry
        })
        .collect();
    if !tree.functions.is_empty() {
        tables["functions"] = json!(functions);
    }
    if !guarded.is_empty() {
        ctx.uses_ref_cycle = true;
    }
    let guards: Vec<_> = (0..guarded.len())
        .map(|index| format_ident!("__guard_{index}"))
        .collect();
    let node = E::node_param(None);
    let node_address = E::node_address(format_ident!("instance"));
    let scope = scope_stacks(ctx, &tree.resources);
    if scope.is_some() {
        if !tree.valid_resources.is_empty() {
            tables["valid_resources"] = json!(tree.valid_resources);
        }
        if !tree.unevaluated_resources.is_empty() {
            tables["unevaluated_resources"] = json!(tree.unevaluated_resources);
        }
    }
    if !tree.properties.is_empty() {
        tables["properties"] = json!(tree.properties);
    }
    if !tree.tuples.is_empty() {
        tables["tuples"] = json!(tree.tuples);
    }
    if !tree.dependencies.is_empty() {
        tables["dependencies"] = json!(tree.dependencies);
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
    let validities: Vec<_> = (0..tree.valid.len())
        .map(|index| format_ident!("__valid_{index}"))
        .collect();
    let checks = &tree.valid;
    let discriminators: Vec<_> = (0..tree.discriminators.len())
        .map(|index| format_ident!("__candidates_{index}"))
        .collect();
    let candidate_bodies = &tree.discriminators;
    let unevaluated: Vec<_> = (0..tree.unevaluated.len())
        .map(|index| format_ident!("__unevaluated_{index}"))
        .collect();
    let unevaluated_errors: Vec<_> = (0..tree.unevaluated.len())
        .map(|index| format_ident!("__unevaluated_errors_{index}"))
        .collect();
    let unevaluated_errors_bodies: Vec<_> = (0..tree.unevaluated.len())
        .map(|index| format_ident!("__unevaluated_errors_{index}_body"))
        .collect();
    let positions = tree.unevaluated.iter().map(|(positions, _)| positions);
    let subschema_errors = tree.unevaluated.iter().map(|(_, errors)| errors);
    let borrowed_node = E::node_param(Some(quote! { '__i }));
    let prelude = E::function_prelude();
    let base = ctx.config.base_uri.as_str();
    // Without a base of its own the schema gets a synthetic one, which the output leaves out.
    if !base.starts_with("json-schema:///") {
        tables["base"] = json!(base.trim_end_matches('#'));
    }
    let tables = tables.to_string();
    let json = E::json_representation();
    let mut type_checks = Vec::new();
    let mut type_names = Vec::new();
    let mut reported_checks = Vec::new();
    let mut reported_names = Vec::new();
    for (index, (is_valid, errors)) in direct.iter().enumerate() {
        match errors {
            DirectErrors::NotOfType(message) => {
                let name = format_ident!("__not_of_type_{index}");
                type_checks.push(quote! {
                    fn #name(
                        instance: &<#json as jsonschema::json::Json>::Node<'_>,
                    ) -> Vec<__eval::ErrorDescription> {
                        if {
                            let instance = ::core::clone::Clone::clone(instance);
                            #prelude
                            #is_valid
                        } {
                            return Vec::new();
                        }
                        vec![__eval::ErrorDescription::described(
                            "type",
                            format!(#message, __eval::instance_value::<#json>(instance)),
                        )]
                    }
                });
                type_names.push(name);
            }
            DirectErrors::Reported(errors) => {
                let name = format_ident!("__direct_{index}");
                reported_checks.push(quote! {
                    fn #name<'__i>(
                        instance: #borrowed_node,
                        __path: &__paths::LazyLocation,
                        __errors: &mut __eval::Descriptions,
                    ) -> bool {
                        #prelude
                        if !(#is_valid) {
                            #errors
                        }
                        false
                    }
                });
                reported_names.push(name);
            }
        }
    }
    let run = if without_errors_run {
        quote! { <#json as __eval::Walk>::evaluate_reporting(&__TREE, &instance, is_valid(instance)) }
    } else {
        quote! {
            let mut __errors = __err::Errors::new(false);
            let _ = errors(instance, &__paths::LazyLocation::new(), &mut __errors);
            <#json as __eval::Walk>::evaluate(&__TREE, &instance, &__errors.into_vec())
        }
    };
    let scope_field = if scope.is_some() {
        quote! { Some(__eval::ScopeStacks { enter: __enter_resource, leave: __leave_resource }) }
    } else {
        quote! { None }
    };
    Ok(quote! {
        #scope
        #(fn #matchers(name: &str) -> bool { #matches })*
        #(
            fn #validities(instance: &<#json as jsonschema::json::Json>::Node<'_>) -> bool {
                let instance = ::core::clone::Clone::clone(instance);
                #prelude
                #checks
            }
        )*
        #(
            fn #guards(instance: &<#json as jsonschema::json::Json>::Node<'_>, enter: bool) -> bool {
                let instance = ::core::clone::Clone::clone(instance);
                #prelude
                __eval::guard(
                    &__JSONSCHEMA_VALIDATE_MARK,
                    ((#guarded as fn(#node) -> bool) as usize, #node_address),
                    enter,
                )
            }
        )*
        #(#type_checks)*
        #(#reported_checks)*
        #(
            fn #discriminators(
                instance: &<#json as jsonschema::json::Json>::Node<'_>,
            ) -> Option<&'static [usize]> {
                let instance = ::core::clone::Clone::clone(instance);
                #prelude
                #candidate_bodies
            }
        )*
        #(
            fn #unevaluated(instance: &<#json as jsonschema::json::Json>::Node<'_>) -> Vec<usize> {
                let instance = ::core::clone::Clone::clone(instance);
                #prelude
                let mut __unevaluated = Vec::new();
                #positions
                __unevaluated
            }
            fn #unevaluated_errors<'__i>(
                instance: &<#json as jsonschema::json::Json>::Node<'__i>,
                __path: &__paths::LazyLocation,
            ) -> Vec<__VE<'static>> {
                let mut __errors = __err::Errors::new(false);
                let _ = #unevaluated_errors_bodies(::core::clone::Clone::clone(instance), __path, &mut __errors);
                __errors.into_vec().into_iter().map(__VE::to_owned).collect()
            }
            fn #unevaluated_errors_bodies<'__i>(
                instance: #borrowed_node,
                __path: &__paths::LazyLocation,
                __errors: &mut __err::Errors<'__i>,
            ) -> bool {
                #prelude
                #subschema_errors
                false
            }
        )*
        static __TREE: __eval::Tree<#json> = __eval::Tree {
            tables: #tables,
            matchers: &[#(#matchers),*],
            valid: &[#(#validities),*],
            guards: &[#(#guards),*],
            discriminators: &[#(#discriminators),*],
            not_of_type: &[#(#type_names),*],
            direct: &[#(#reported_names),*],
            unevaluated: &[#(#unevaluated),*],
            unevaluated_errors: &[#(#unevaluated_errors),*],
            scope: #scope_field,
            built: ::std::sync::OnceLock::new(),
        };
        #run
    })
}

/// Functions putting a resource's entries on the generated code's dynamic-scope stacks, and taking
/// them off, so the code the walk calls resolves dynamic references in the walk's scope. The
/// `$recursiveRef` entry goes on only when `anchored`.
fn scope_stacks<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    resources: &[TreeResource],
) -> Option<TokenStream> {
    let mut enter_arms = Vec::new();
    let mut leave_arms = Vec::new();
    for (index, resource) in resources.iter().enumerate() {
        let mut enter = Vec::new();
        let mut leave = Vec::new();
        if ctx.uses_dynamic_ref {
            let bindings = collect_dynamic_anchor_bindings(ctx, resource.base.clone());
            if !bindings.is_empty() {
                let count = bindings.len();
                for family in [&IS_VALID_FAMILY, &KEY_EVAL_FAMILY, &ITEM_EVAL_FAMILY] {
                    enter.extend(family.dynamic_pushes(&bindings));
                    leave.push((family.pop_dynamic_n)(count));
                }
                enter.extend(bindings.iter().map(|binding| {
                    push_dynamic_errors(
                        &binding.anchor,
                        &format_ident!("{}_errors", binding.is_valid_name),
                    )
                }));
                leave.push(pop_dynamic_errors_n(count));
            }
        }
        if ctx.uses_recursive_ref && resource.recursive {
            let resolved = ctx
                .with_base_uri_scope(resource.base.clone(), |ctx| resolve_ref(ctx, "#"))
                .expect("a resource's root resolves");
            let (location, schema, base) = (resolved.location, resolved.schema, resolved.base_uri);
            let is_valid = format_ident!(
                "{}",
                get_or_create_is_valid_fn(ctx, &location, schema, base.clone())
            );
            let key_eval = format_ident!(
                "{}",
                get_or_create_key_eval_fn(ctx, &location, schema, base.clone())
            );
            let item_eval = format_ident!(
                "{}",
                get_or_create_item_eval_fn(ctx, &location, schema, base)
            );
            let errors = format_ident!("{is_valid}_errors");
            let pushes = [
                push_recursive_is_valid(&is_valid, true),
                push_recursive_key_eval(&key_eval, true),
                push_recursive_item_eval(&item_eval, true),
                push_recursive_errors(&errors, true),
            ];
            let pops = [
                pop_recursive_errors(),
                pop_recursive_item_eval(),
                pop_recursive_key_eval(),
                pop_recursive_is_valid(),
            ];
            enter.push(quote! { if anchored { #(#pushes)* } });
            leave.push(quote! { if anchored { #(#pops)* } });
        }
        if !enter.is_empty() {
            enter_arms.push(quote! { #index => { #(#enter)* } });
            leave_arms.push(quote! { #index => { #(#leave)* } });
        }
    }
    if enter_arms.is_empty() {
        return None;
    }
    Some(quote! {
        fn __enter_resource(resource: usize, anchored: bool) {
            match resource {
                #(#enter_arms)*
                _ => {}
            }
        }
        fn __leave_resource(resource: usize, anchored: bool) {
            match resource {
                #(#leave_arms)*
                _ => {}
            }
        }
    })
}

/// The branches that may accept `instance`, by its discriminator value: those accepting the value
/// and those accepting any.
fn candidates(discriminator: &BranchDiscriminator) -> TokenStream {
    let any: Vec<usize> = discriminator
        .values
        .iter()
        .enumerate()
        .filter(|(_, values)| values.is_none())
        .map(|(branch, _)| branch)
        .collect();
    let literals: BTreeSet<&DiscriminatorLiteral> =
        discriminator.values.iter().flatten().flatten().collect();
    let arms = literals.into_iter().map(|literal| {
        let branches = discriminator
            .values
            .iter()
            .enumerate()
            .filter(|(_, values)| {
                values
                    .as_ref()
                    .is_none_or(|values| values.contains(literal))
            })
            .map(|(branch, _)| branch);
        let pattern = literal.to_inner_token();
        quote! { Some(#pattern) => Some(&[#(#branches),*][..]), }
    });
    let read = &discriminator.read;
    quote! {
        match #read {
            #(#arms)*
            Some(_) => Some(&[#(#any),*][..]),
            None => None,
        }
    }
}

/// Whether `schema` has a reference anywhere in it.
fn references(schema: &Value) -> bool {
    match schema {
        Value::Object(keywords) => keywords.iter().any(|(keyword, value)| {
            matches!(keyword.as_str(), "$ref" | "$dynamicRef" | "$recursiveRef")
                || references(value)
        }),
        Value::Array(items) => items.iter().any(references),
        _ => false,
    }
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

/// Why the generated `evaluate` cannot handle `schema`, the resource at `base` in `registry`, yet,
/// if it cannot.
pub(crate) fn unsupported(
    registry: &Registry,
    base: &Uri<String>,
    schema: &Value,
    draft: Draft,
) -> Option<String> {
    let resolver = registry.resolver(base.clone());
    let mut support = Support {
        draft,
        resolver: resolver.clone(),
        checked: HashSet::from([std::ptr::from_ref(schema).addr()]),
        root: schema
            .as_object()
            .map(|keywords| std::ptr::from_ref(keywords).addr()),
        resources: vec![resolver],
        anchors: Vec::new(),
        recursive: false,
    };
    support
        .check(schema, true)
        .or_else(|| support.dynamic_targets())
}

/// Walks what `evaluate` reports, following each reference once.
struct Support<'r> {
    draft: Draft,
    /// Resolves the references of the resource being walked.
    resolver: Resolver<'r>,
    /// The schemas already walked, by address.
    checked: HashSet<usize>,
    /// The root's keywords, by address: its `$schema` sets the draft and vocabularies `evaluate`
    /// compiles with.
    root: Option<usize>,
    /// Each resource walked, as a resolver based at it.
    resources: Vec<Resolver<'r>>,
    /// The `$dynamicAnchor` names a `$dynamicRef` resolves through the dynamic scope.
    anchors: Vec<String>,
    /// A `$recursiveRef` resolves through the dynamic scope.
    recursive: bool,
}

impl<'r> Support<'r> {
    /// `based`: the resolver already sits at the schema's own base, as at a reference target.
    fn check(&mut self, schema: &Value, based: bool) -> Option<String> {
        let keywords = match schema {
            Value::Bool(_) => return None,
            Value::Object(keywords) => keywords,
            _ => {
                return Some("`evaluate` needs a schema that is a boolean or an object".to_string())
            }
        };
        let adjacent = self.draft.supports_adjacent_validation();
        if !based
            && (adjacent || !keywords.contains_key("$ref"))
            && keywords.get(id_keyword(self.draft)).is_some()
        {
            // Its references resolve against its own base.
            let inner = match self
                .resolver
                .in_subresource(self.draft.create_resource_ref(schema))
            {
                Ok(inner) => inner,
                Err(error) => return Some(error.to_string()),
            };
            self.entered(&inner);
            let outer = std::mem::replace(&mut self.resolver, inner);
            let message = self.check_keywords(keywords);
            self.resolver = outer;
            return message;
        }
        self.check_keywords(keywords)
    }

    fn check_keywords(&mut self, keywords: &Map<String, Value>) -> Option<String> {
        if let Some(reference) = keywords.get("$ref") {
            if let Some(message) = self.reference(reference) {
                return Some(message);
            }
            if !self.draft.supports_adjacent_validation() {
                // Earlier drafts ignore the keywords beside `$ref`.
                return None;
            }
        }
        if let Some(reference) = keywords
            .get("$dynamicRef")
            .filter(|_| self.draft.is_known_keyword("$dynamicRef"))
        {
            if let Some(name) = reference.as_str().and_then(|reference| {
                let resolved = self.resolver.lookup(reference).ok()?;
                dynamic_ref_anchor_name(reference, resolved.contents())
            }) {
                if !self.anchors.contains(&name) {
                    self.anchors.push(name);
                }
            }
            if let Some(message) = self.reference(reference) {
                return Some(message);
            }
        }
        if let Some(reference) = keywords
            .get("$recursiveRef")
            .filter(|_| self.draft.is_known_keyword("$recursiveRef"))
        {
            if reference.as_str().is_some_and(|reference| {
                self.resolver.lookup(reference).is_ok_and(|resolved| {
                    resolved.contents().get("$recursiveAnchor") == Some(&Value::Bool(true))
                })
            }) {
                self.recursive = true;
            }
            if let Some(message) = self.reference(reference) {
                return Some(message);
            }
        }
        keywords.iter().find_map(|(keyword, value)| {
            if !self.draft.is_known_keyword(keyword) {
                // An unknown keyword is an annotation.
                return None;
            }
            match (keyword.as_str(), value) {
                ("properties" | "patternProperties", Value::Object(subschemas)) => {
                    return subschemas
                        .values()
                        .find_map(|subschema| self.check(subschema, false));
                }
                ("dependencies" | "dependentSchemas", Value::Object(dependencies)) => {
                    return dependencies
                        .values()
                        .filter(|dependency| !dependency.is_array())
                        .find_map(|subschema| self.check(subschema, false));
                }
                // Only its validity is checked.
                ("not", _) => return None,
                // `additionalItems` applies past an array-form `items` only; `if` beside `then` or
                // `else` only, and they beside `if` only.
                ("additionalItems", _) if !keywords.get("items").is_some_and(Value::is_array) => {
                    return None;
                }
                ("if", _) if !keywords.contains_key("then") && !keywords.contains_key("else") => {
                    return None;
                }
                ("then" | "else", _) if !keywords.contains_key("if") => return None,
                ("items" | "prefixItems" | "allOf" | "anyOf" | "oneOf", Value::Array(schemas)) => {
                    return schemas
                        .iter()
                        .find_map(|subschema| self.check(subschema, false));
                }
                (
                    "additionalProperties"
                    | "propertyNames"
                    | "items"
                    | "additionalItems"
                    | "contains"
                    | "if"
                    | "then"
                    | "else"
                    | "unevaluatedProperties"
                    | "unevaluatedItems",
                    Value::Bool(_) | Value::Object(_),
                ) => {
                    return self.check(value, false);
                }
                _ => {}
            }
            if keyword == "$schema" && self.root == Some(std::ptr::from_ref(keywords).addr()) {
                return None;
            }
            (!supports(keyword, value, self.draft))
                .then(|| format!("`evaluate` does not support the `{keyword}` keyword yet"))
        })
    }

    /// Checks the target of a reference the first time it is reached.
    fn reference(&mut self, reference: &Value) -> Option<String> {
        let reference = reference.as_str()?;
        let resolved = match self.resolver.lookup(reference) {
            Ok(resolved) => resolved,
            Err(error) => return Some(error.to_string()),
        };
        self.target(resolved)
    }

    /// The target's references resolve against its own resource, and its keywords apply as its
    /// resource's draft defines them.
    fn target(&mut self, resolved: Resolved<'r>) -> Option<String> {
        let (target, inner, draft) = resolved.into_inner();
        if !self.checked.insert(std::ptr::from_ref(target).addr()) {
            return None;
        }
        self.entered(&inner);
        let outer = std::mem::replace(&mut self.resolver, inner);
        let outer_draft = std::mem::replace(&mut self.draft, draft);
        let message = self.check(target, true);
        self.draft = outer_draft;
        self.resolver = outer;
        message
    }

    fn entered(&mut self, resolver: &Resolver<'r>) {
        let base = resolver.base_uri();
        if !self.resources.iter().any(|known| known.base_uri() == base) {
            self.resources.push(resolver.clone());
        }
    }

    /// Checks what a reference resolving through the dynamic scope may land on: the anchors it
    /// names, or the roots, in every resource the scope may hold.
    fn dynamic_targets(&mut self) -> Option<String> {
        let mut index = 0;
        while index < self.resources.len() {
            let resolver = self.resources[index].clone();
            for name in self.anchors.clone() {
                let reference = format!("#{name}");
                if let Ok(resolved) = resolver.lookup(&reference) {
                    if dynamic_ref_anchor_name(&reference, resolved.contents()).is_some() {
                        if let Some(message) = self.target(resolved) {
                            return Some(message);
                        }
                    }
                }
            }
            if self.recursive {
                if let Ok(resolved) = resolver.lookup("#") {
                    if let Some(message) = self.target(resolved) {
                        return Some(message);
                    }
                }
            }
            index += 1;
        }
        None
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
        // `contains` reports the bounds' errors, and `Support` walks each `$ref` target.
        "minContains" | "maxContains" | "$ref" | "$defs" | "definitions" | "$anchor"
        | "$comment" | "$id" | "id" | "$dynamicRef" | "$dynamicAnchor" | "$recursiveRef"
        | "$recursiveAnchor" => true,
        _ => LEAF_KEYWORDS.contains(&keyword),
    }
}

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::{write_escaped_str, Draft, Uri, VocabularySet};
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
        content::{checks_encoding, checks_media_type},
        format::{format_emits_assertion, reads_numbers},
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
use crate::context::{CompileContext, Dialect};

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
    /// Its annotation of an instance of the type it reads.
    annotation: Option<Value>,
    /// The annotation goes to numbers rather than strings.
    annotates_numbers: bool,
    /// A schema of its keyword alone, checked directly when `evaluate` skips the errors run.
    check: Option<LeafCheck>,
    /// Index of its generated check, which reports its errors without the errors run.
    direct: Option<usize>,
}

/// The error text of a leaf unit that checks the instance itself.
enum DirectErrors {
    /// The errors code of its keyword alone.
    Reported(TokenStream),
    /// The message of a single `type`, with a slot for the instance.
    NotOfType(String),
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
            annotation: None,
            annotates_numbers: false,
            check: None,
            direct: None,
        }
    }

    fn with_alias(mut self, alias: &'static str) -> Self {
        self.alias = Some(alias);
        self
    }

    fn annotating(mut self, value: &Value, annotates_numbers: bool) -> Self {
        self.annotation = Some(value.clone());
        self.annotates_numbers = annotates_numbers;
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
        /// Its target is in another resource than the schema holding it: in another document, or
        /// embedded in the same one.
        leaves: bool,
        /// The `$dynamicAnchor` a `$dynamicRef` resolves through the dynamic scope.
        anchor: Option<String>,
        /// A `$recursiveRef` resolves through the dynamic scope.
        recursive: bool,
        /// The address of the schema holding a `$ref` or `$dynamicRef`, which applies nothing where
        /// it lands on that schema, as the runtime compiles nothing for it there.
        holder: Option<usize>,
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
    /// The address of its JSON value.
    address: usize,
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
    /// The resource holding it, where it starts a resource of its own below a schema in another
    /// one.
    enclosing: Option<usize>,
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
    /// Addresses of the schemas being added since entering `function`'s code, outermost first.
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
        let address = std::ptr::from_ref(schema).addr();
        self.schemas.push(TreeSchema {
            address,
            pointer: String::new(),
            resource: 0,
            base: None,
            default_base: false,
            at: None,
            annotations: None,
            first: 0,
            count: 0,
            rejects: false,
            enclosing: None,
        });
        self.by_pointer.insert(place, index);
        self.inline.push(address);
        // The root keeps the configured draft.
        let added = match schema {
            Value::Object(keywords) if index != 0 => ctx.with_dialect_of(keywords, |ctx| {
                self.add_in_dialect(ctx, schema, index, pointer, based)
            }),
            _ => self.add_in_dialect(ctx, schema, index, pointer, based),
        };
        self.inline.pop();
        added
    }

    fn add_in_dialect<E: ValueEmitter>(
        &mut self,
        ctx: &mut CompileContext<'_, E>,
        schema: &Value,
        index: usize,
        pointer: String,
        based: bool,
    ) -> syn::Result<usize> {
        match resource_base(ctx, schema).filter(|_| index != 0 && !based) {
            Some(base) => {
                self.schemas[index].enclosing = Some(self.resource(ctx));
                let outer = std::mem::replace(&mut self.resource_root, pointer.clone());
                let added = ctx.with_base_uri_scope(base, |ctx| {
                    self.add_keywords(ctx, schema, index, pointer)
                });
                self.resource_root = outer;
                added
            }
            None => self.add_keywords(ctx, schema, index, pointer),
        }
    }

    /// Locations of the reference targets whose code holds the schema being added: `function`'s
    /// target and the targets its code compiled within, then the targets the `inline` schemas are.
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
                let units = self.reference(ctx, schema, "$ref", reference)?;
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
                    // An invalid pattern's matcher fails the build as the validation code does.
                    let matches = ctx
                        .with_schema_path_swap(pointer.clone(), |ctx| {
                            name_match_expr(ctx, pattern, &quote! { name })
                        })
                        .map_or_else(
                            |invalid| invalid.is_valid_token_stream(),
                            |matches| matches.is_valid,
                        );
                    entries.push((schema, matches));
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
                    // With `additionalItems: false`, the errors run reports each extra item.
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
                // Checked apart from the evaluation, its subschema has no scope to resolve
                // references in.
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
                units.extend(self.reference(ctx, schema, "$ref", reference)?);
            }
            for keyword in ["$recursiveRef", "$dynamicRef"] {
                if let Some(reference) = keywords
                    .get(keyword)
                    .and_then(Value::as_str)
                    .filter(|_| ctx.draft.is_known_keyword(keyword))
                {
                    units.extend(self.reference(ctx, schema, keyword, reference)?);
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
            address: self.schemas[index].address,
            pointer,
            resource,
            base,
            default_base,
            at,
            annotations,
            first: self.units.len(),
            count: units.len(),
            rejects: self.schemas[index].rejects,
            enclosing: self.schemas[index].enclosing,
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

    /// The unit of a `$ref`, `$dynamicRef` or `$recursiveRef` in `holder`; none for one that does
    /// not resolve, or for a `$ref`, or a `$dynamicRef` resolving without a dynamic anchor, naming
    /// `holder` itself, as the runtime compiles none.
    fn reference<E: ValueEmitter>(
        &mut self,
        ctx: &mut CompileContext<'_, E>,
        holder: &Value,
        keyword: &'static str,
        reference: &str,
    ) -> syn::Result<Option<TreeUnit>> {
        let Ok(resolved) = resolve_ref(ctx, reference) else {
            return Ok(None);
        };
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
        // A `$recursiveRef` enters the schema it lands on, the one holding it too.
        let holder = (keyword != "$recursiveRef").then(|| std::ptr::from_ref(holder).addr());
        if anchor.is_none() && holder == Some(self.schemas[target].address) {
            return Ok(None);
        }
        Ok(Some(TreeUnit::Ref {
            keyword,
            leaves: leaves_document(ctx, reference),
            target,
            function,
            anchor,
            recursive,
            holder,
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

    /// Marks each reference whose static target sits in another resource than the schema holding
    /// it as leaving that resource, as one into another document does.
    fn mark_references_leaving_their_resource(&mut self) {
        let Tree { schemas, units, .. } = self;
        for schema in schemas.iter() {
            for unit in &mut units[schema.first..schema.first + schema.count] {
                if let TreeUnit::Ref { target, leaves, .. } = unit {
                    *leaves |= schemas[*target].resource != schema.resource;
                }
            }
        }
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
        let vocabularies = ctx
            .config
            .registry
            .resolver((**base).clone())
            .find_vocabularies(draft, schema);
        let dialect = Dialect {
            draft,
            vocabularies,
        };
        let added = in_resource(ctx, base.clone(), dialect, |ctx| {
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
        let resolved = resolve_ref(ctx, reference).expect("`reference` resolved it");
        let pointer = location_pointer(ctx, &resolved.location, &resolved.base_uri);
        let root = pointer[..resolved.resource_start].to_string();
        let outer = std::mem::replace(&mut self.resource_root, root);
        let added = in_resource(ctx, resolved.base_uri, resolved.dialect, |ctx| {
            self.based = true;
            self.add(ctx, resolved.schema, pointer)
        });
        self.resource_root = outer;
        added
    }
}

/// `f` with `ctx` at a reference target in the resource at `base`, written in `dialect`.
fn in_resource<'cfg, E: ValueEmitter, T>(
    ctx: &mut CompileContext<'cfg, E>,
    base: Arc<Uri<String>>,
    dialect: Dialect,
    f: impl FnOnce(&mut CompileContext<'cfg, E>) -> T,
) -> T {
    let outer_draft = std::mem::replace(&mut ctx.draft, dialect.draft);
    let outer_vocabularies = std::mem::replace(&mut ctx.vocabularies, dialect.vocabularies);
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
) -> syn::Result<TokenStream> {
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
    tree.mark_references_leaving_their_resource();
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
    let direct = without_errors_run
        .then(|| direct_checks(ctx, &mut tree.units))
        .flatten();
    let without_errors_run = direct.is_some();
    let direct = direct.unwrap_or_default();
    let schemas: Vec<Value> = tree
        .schemas
        .iter()
        .map(|schema| {
            let mut entry = json!({
                "pointer": schema.pointer,
                "first": schema.first,
                "count": schema.count,
            });
            if let Some(annotations) = &schema.annotations {
                entry["annotations"] = annotations.clone();
            }
            if schema.resource != 0 {
                entry["resource"] = json!(schema.resource);
            }
            if let Some(base) = &schema.base {
                entry["base"] = json!(base);
            }
            if schema.default_base {
                entry["default_base"] = json!(true);
            }
            if let Some(at) = &schema.at {
                entry["at"] = json!(at);
            }
            entry
        })
        .collect();
    // Every unit's location is its keyword below its schema's; a leaf unit may also report the
    // errors of other keywords and annotate strings, and a reference names its static target.
    let units: Vec<Value> = tree
        .units
        .iter()
        .map(|unit| {
            if let TreeUnit::Ref {
                keyword,
                target,
                function,
                leaves,
                ..
            } = unit
            {
                let mut entry = json!({"keyword": keyword, "target": target, "function": function});
                if *leaves {
                    entry["leaves"] = json!(true);
                }
                return entry;
            }
            let (keyword, aliases, strings) = match unit {
                TreeUnit::Leaf(unit) => (
                    unit.keyword.as_str(),
                    unit.alias.map(|alias| vec![alias]).unwrap_or_default(),
                    unit.annotation.as_ref(),
                ),
                // The errors run reports the count errors at the bound keywords.
                TreeUnit::Contains { .. } => ("contains", vec!["minContains", "maxContains"], None),
                TreeUnit::Properties { .. } => ("properties", Vec::new(), None),
                TreeUnit::Patterns(_) => ("patternProperties", Vec::new(), None),
                TreeUnit::Additional { .. } => ("additionalProperties", Vec::new(), None),
                TreeUnit::Names { .. } => ("propertyNames", Vec::new(), None),
                TreeUnit::Condition { .. } => ("if", Vec::new(), None),
                TreeUnit::Branches { keyword, .. }
                | TreeUnit::Branch { keyword, .. }
                | TreeUnit::Ref { keyword, .. }
                | TreeUnit::Tuple { keyword, .. }
                | TreeUnit::Rest { keyword, .. }
                | TreeUnit::Dependencies { keyword, .. }
                | TreeUnit::Unevaluated { keyword, .. } => (*keyword, Vec::new(), None),
            };
            if aliases.is_empty() && strings.is_none() {
                return json!(keyword);
            }
            let mut entry = json!({"keyword": keyword});
            if !aliases.is_empty() {
                entry["aliases"] = json!(aliases);
            }
            if let Some(annotation) = strings {
                entry["strings"] = annotation.clone();
            }
            entry
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
    // A reference through the dynamic scope landing elsewhere enters there under a guard of its
    // own; landing on its static target, it enters it as a `$ref` does.
    let entered_statically: HashSet<usize> = tree
        .units
        .iter()
        .filter_map(|unit| match unit {
            TreeUnit::Ref { function, .. } => Some(*function),
            _ => None,
        })
        .collect();
    // A `$recursiveRef` naming the schema holding it re-enters that schema's code on its instance.
    let reentered_by_holder: HashSet<usize> = tree
        .schemas
        .iter()
        .flat_map(|schema| {
            tree.units[schema.first..schema.first + schema.count]
                .iter()
                .filter_map(|unit| match unit {
                    TreeUnit::Ref {
                        target,
                        function,
                        holder: None,
                        ..
                    } if tree.schemas[*target].address == schema.address => Some(*function),
                    _ => None,
                })
        })
        .collect();
    // A `$ref` re-entering a target while the target's code compiles enters it under the guard
    // that code shares with its own references to the target.
    let mut guarded = Vec::new();
    let mut guards = HashMap::new();
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
            if reentered_by_holder.contains(&id)
                || (entered_statically.contains(&id) && ctx.reentered_targets.contains(location))
            {
                guards.insert(id, format_ident!("__guard_{}", guarded.len()));
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
    if let Some(function) = ctx
        .root_target
        .as_ref()
        .and_then(|location| tree.function_ids.get(location))
    {
        tables["root_function"] = json!(function);
    }
    let scope = scope_stacks(ctx, &tree.resources);
    let dynamic = tree.units.iter().any(|unit| {
        matches!(unit, TreeUnit::Ref { anchor, recursive, .. } if anchor.is_some() || *recursive)
    });
    let keeps_scope = scope.is_some() || dynamic;
    if keeps_scope {
        tables["keeps_scope"] = json!(true);
    }
    if !guarded.is_empty() {
        ctx.uses_ref_cycle = true;
    }
    let guard_names: Vec<_> = (0..guarded.len())
        .map(|index| format_ident!("__guard_{index}"))
        .collect();
    let node = E::node_param(None);
    let node_address = E::node_address(format_ident!("instance"));
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
    let direct_checks = direct
        .iter()
        .enumerate()
        .map(|(index, (is_valid, errors))| match errors {
            DirectErrors::NotOfType(message) => {
                let name = format_ident!("__not_of_type_{index}");
                quote! {
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
                }
            }
            DirectErrors::Reported(errors) => {
                let name = format_ident!("__direct_{index}");
                quote! {
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
                }
            }
        });
    let scope_field = if scope.is_some() {
        quote! { Some(__eval::ScopeStacks { enter: __enter_resource, leave: __leave_resource }) }
    } else {
        quote! { None }
    };
    let item_runs: Vec<bool> = tree
        .schemas
        .iter()
        .map(|schema| {
            schema.annotations.is_none()
                && tree.units[schema.first..schema.first + schema.count]
                    .iter()
                    .all(|unit| matches!(unit, TreeUnit::Leaf(unit) if unit.annotation.is_none()))
        })
        .collect();
    let mut callable = callable(&tree);
    let outlined = outlined(&tree, &item_runs, &mut callable);
    let code = Code {
        tree: &tree,
        not_of_type: direct
            .iter()
            .map(|(_, errors)| matches!(errors, DirectErrors::NotOfType(_)))
            .collect(),
        reporting: !without_errors_run,
        scoped: scope.is_some(),
        keeps_scope,
        guards,
        item_runs,
        callable,
        outlined,
    };
    let schema_fns = (0..tree.schemas.len())
        .filter(|&index| code.callable[index])
        .map(|index| code.schema_fn::<E>(index));
    let mut runs = BTreeSet::new();
    for unit in &tree.units {
        if let TreeUnit::Rest {
            schema,
            closed: false,
            ..
        } = unit
        {
            if code.item_runs[*schema] {
                runs.insert(*schema);
            }
        }
    }
    let leaf_item_fns = runs.into_iter().map(|index| code.leaf_item_fn::<E>(index));
    let entry = if without_errors_run {
        quote! { let mut __e = __SITES.checking(is_valid(instance)); }
    } else {
        quote! {
            let mut __errors = __err::Errors::new(false);
            let _ = errors(instance, &__paths::LazyLocation::new(), &mut __errors);
            let __errors = __errors.into_vec();
            let mut __e = __SITES.reporting(&__errors);
        }
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
            fn #guard_names(
                instance: &<#json as jsonschema::json::Json>::Node<'_>,
                enter: bool,
            ) -> bool {
                let instance = ::core::clone::Clone::clone(instance);
                #prelude
                __eval::guard(
                    &__JSONSCHEMA_VALIDATE_MARK,
                    ((#guarded as fn(#node) -> bool) as usize, #node_address),
                    enter,
                )
            }
        )*
        #(#direct_checks)*
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
                let _ = #unevaluated_errors_bodies(
                    ::core::clone::Clone::clone(instance),
                    __path,
                    &mut __errors,
                );
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
        type __J = #json;
        #(#schema_fns)*
        #(#leaf_item_fns)*
        static __SITES: __eval::Sites = __eval::Sites {
            tables: #tables,
            scope: #scope_field,
            built: ::std::sync::OnceLock::new(),
        };
        #entry
        let __root = __evaluate_0(
            &mut __e,
            instance,
            &__paths::LazyLocation::new(),
            __eval::InstancePosition::ROOT,
        );
        __e.finish(__root)
    })
}

/// One check per distinct leaf keyword, shared by its equal copies; `None` if a keyword does not
/// compile on its own, and the errors run reports for every unit.
fn direct_checks<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    units: &mut [TreeUnit],
) -> Option<Vec<(TokenStream, DirectErrors)>> {
    let mut compiled = Vec::new();
    for unit in units {
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
        let check = ctx.with_schema_path_swap(String::new(), |ctx| compile_schema(ctx, &schema));
        ctx.draft = outer_draft;
        ctx.vocabularies = outer_vocabularies;
        if check.is_compile_error() {
            return None;
        }
        compiled.push((unit, schema, check));
    }
    let mut direct = Vec::new();
    let mut by_code = HashMap::new();
    for (unit, schema, check) in compiled {
        let is_valid = check.is_valid_token_stream();
        let errors = match schema.get("type") {
            // The runtime words a single type's error without building it.
            Some(Value::String(type_)) => {
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
    Some(direct)
}

/// What the code of every schema shares.
struct Code<'t> {
    tree: &'t Tree,
    /// Whether each leaf check is a single `type`'s, which words its error itself.
    not_of_type: Vec<bool>,
    /// Units take their errors from the errors run instead of checking the instance themselves.
    reporting: bool,
    /// Generated validity code runs with the evaluation's dynamic scope on the generated stacks.
    scoped: bool,
    /// References keep the dynamic scope: `$dynamicRef`, `$recursiveRef` or validity code reads it.
    keeps_scope: bool,
    /// The guard of each reference target's code, by index of `Tree::functions`.
    guards: HashMap<usize, proc_macro2::Ident>,
    /// Whether each schema has leaf units only and nothing in it annotates, so its valid items
    /// join runs.
    item_runs: Vec<bool>,
    /// Whether each schema has a function of its own: one many places reach, or one whose code is
    /// large. Every other schema's code sits in the one place reaching it.
    callable: Vec<bool>,
    /// Whether each schema's function is one for its large code, which stays out of line.
    outlined: Vec<bool>,
}

impl Code<'_> {
    fn evaluate_fn(index: usize) -> proc_macro2::Ident {
        format_ident!("__evaluate_{index}")
    }

    fn leaf_item_name(index: usize) -> proc_macro2::Ident {
        format_ident!("__leaf_item_{index}")
    }

    /// The node of the schema at `schema` for `instance` at `__path` and `__position`, which the
    /// schema holding it descends into.
    fn child<E: ValueEmitter>(&self, schema: usize) -> TokenStream {
        let node = self.node::<E>(schema);
        self.descending(schema, node)
    }

    /// `node`, the node of the schema at `schema`, built after descending into it: a schema
    /// starting a resource of its own puts the resource holding it on the dynamic scope.
    fn descending(&self, schema: usize, node: TokenStream) -> TokenStream {
        match self.enclosing(schema) {
            Some(resource) => quote! {{
                let __entered = __e.enter_resource(#resource);
                let __node = #node;
                __e.leave_resource(__entered);
                __node
            }},
            None => node,
        }
    }

    /// The resource holding the schema at `schema`, where descending into it enters a resource of
    /// its own and the dynamic scope is kept.
    fn enclosing(&self, schema: usize) -> Option<usize> {
        self.tree.schemas[schema]
            .enclosing
            .filter(|_| self.keeps_scope)
    }

    /// The node of the schema at `schema` for `instance` at `__path` and `__position`.
    fn node<E: ValueEmitter>(&self, schema: usize) -> TokenStream {
        if self.callable[schema] {
            let name = Self::evaluate_fn(schema);
            return quote! { #name(__e, instance, __path, __position) };
        }
        if let Some((target, function)) = self.static_reference(schema) {
            // The runtime's code for a schema whose only unit is a static reference.
            let guard = self.guard(function);
            let evaluate = Self::evaluate_fn(target);
            return quote! {
                __e.reference_schema::<__J>(
                    #schema,
                    #guard,
                    #evaluate,
                    instance,
                    __path,
                    __position,
                )
            };
        }
        let body = self.body::<E>(schema);
        quote! {{ #body }}
    }

    /// The static target and its code, of the schema at `schema` whose only unit is a reference
    /// to it.
    fn static_reference(&self, schema: usize) -> Option<(usize, usize)> {
        let entry = &self.tree.schemas[schema];
        if entry.count != 1 || entry.rejects {
            return None;
        }
        match &self.tree.units[entry.first] {
            TreeUnit::Ref {
                target,
                function,
                anchor: None,
                recursive: false,
                ..
            } => Some((*target, *function)),
            _ => None,
        }
    }

    /// The guard argument of a reference into the code at `function`.
    fn guard(&self, function: usize) -> TokenStream {
        self.guards
            .get(&function)
            .map_or_else(|| quote! { None }, |guard| quote! { Some(#guard) })
    }

    /// The check of the subschema at `Tree::valid` entry `valid` for `instance`.
    fn validity(&self, valid: usize) -> TokenStream {
        let check = format_ident!("__valid_{valid}");
        if self.scoped {
            let resource = self.tree.valid_resources[valid];
            quote! {{
                __e.enter_scope(#resource);
                let __valid = #check(&instance);
                __e.leave_scope(#resource);
                __valid
            }}
        } else {
            quote! { #check(&instance) }
        }
    }

    /// The errors of the leaf `unit` at `index` for `instance`.
    fn leaf_errors(&self, unit: &Unit, index: usize) -> TokenStream {
        if self.reporting {
            return Self::unit_errors(index);
        }
        let check = unit.direct.expect("every leaf unit checks itself");
        if self.not_of_type[check] {
            let check = format_ident!("__not_of_type_{check}");
            quote! { if __e.checks() { #check(&instance) } else { Vec::new() } }
        } else {
            let check = format_ident!("__direct_{check}");
            quote! { if __e.checks() { __eval::leaf_errors(#check, instance) } else { Vec::new() } }
        }
    }

    /// The errors the errors run reports at the unit at `index`.
    fn unit_errors(index: usize) -> TokenStream {
        quote! { __e.unit_errors::<__J>(#index, &instance, __path) }
    }

    /// The errors of the schema at `index` itself for `instance`.
    fn schema_errors(&self, index: usize) -> TokenStream {
        if self.reporting {
            quote! { __e.schema_errors::<__J>(#index, &instance, __path) }
        } else if self.tree.schemas[index].rejects {
            quote! { __e.rejection::<__J>(#index, &instance, __path) }
        } else {
            quote! { Vec::new() }
        }
    }

    /// The code evaluating the schema at `index` for `instance` at `__path` and `__position`.
    fn body<E: ValueEmitter>(&self, index: usize) -> TokenStream {
        let schema = &self.tree.schemas[index];
        let prelude = E::function_prelude();
        let units = (schema.first..schema.first + schema.count).map(|unit| self.unit::<E>(unit));
        let errors = self.schema_errors(index);
        quote! {
            #prelude
            let mut __units = __eval::ChildList::default();
            #(#units)*
            __e.schema(#index, #errors, __units, __position)
        }
    }

    /// The function evaluating the schema at `index`.
    fn schema_fn<E: ValueEmitter>(&self, index: usize) -> TokenStream {
        let name = Self::evaluate_fn(index);
        let node = E::node_param(Some(quote! { '__i }));
        let body = self.body::<E>(index);
        let inline = self.outlined[index].then(|| quote! { #[inline(never)] });
        quote! {
            #inline
            fn #name<'__i>(
                __e: &mut __eval::Evaluator<'_, '_>,
                instance: #node,
                __path: &__paths::LazyLocation,
                __position: __eval::InstancePosition,
            ) -> __eval::EvaluationNode {
                #body
            }
        }
    }

    /// The function evaluating the schema at `index`, which joins item runs, at an item of the
    /// array at `__parent`: `None` when nothing reports an error.
    fn leaf_item_fn<E: ValueEmitter>(&self, index: usize) -> TokenStream {
        let schema = &self.tree.schemas[index];
        let name = Self::leaf_item_name(index);
        let node = E::node_param(Some(quote! { '__i }));
        let prelude = E::function_prelude();
        let units: Vec<usize> = (schema.first..schema.first + schema.count).collect();
        let errors: Vec<_> = units
            .iter()
            .map(|&unit| format_ident!("__errors_{unit}"))
            .collect();
        let checks = units.iter().map(|&unit| {
            let TreeUnit::Leaf(leaf) = &self.tree.units[unit] else {
                unreachable!("an item run's schema has leaf units only")
            };
            self.leaf_errors(leaf, unit)
        });
        let schema_errors = self.schema_errors(index);
        quote! {
            fn #name<'__i>(
                __e: &mut __eval::Evaluator<'_, '_>,
                instance: #node,
                __path: &__paths::LazyLocation,
                __parent: __eval::InstancePosition,
            ) -> Option<__eval::EvaluationNode> {
                #prelude
                #(let #errors = #checks;)*
                let __errors = #schema_errors;
                if #(#errors.is_empty() &&)* __errors.is_empty() {
                    return None;
                }
                let __position = __e.position_below(__parent, __path);
                let mut __units = __eval::ChildList::default();
                #(__units = __e.leaf(__units, #units, #errors, false, __position);)*
                Some(__e.schema(#index, __errors, __units, __position))
            }
        }
    }

    /// Code adding the node of the unit at `index` to `__units`.
    fn unit<E: ValueEmitter>(&self, index: usize) -> TokenStream {
        let empty = quote! {
            __units = __e.unit(
                __units,
                #index,
                Vec::new(),
                None,
                __eval::ChildList::default(),
                __position,
            )
        };
        // Below a member or an item, `instance`, `__path` and `__position` are its own.
        let below = quote! {
            let __member = __path.push(__name);
            let __path = &__member;
            let __position = __e.position_below(__position, __path);
        };
        let below_item = quote! {
            let __member = __path.push(__idx);
            let __path = &__member;
            let __position = __e.position_below(__position, __path);
        };
        match &self.tree.units[index] {
            TreeUnit::Leaf(unit) => {
                let annotated = match (&unit.annotation, unit.annotates_numbers) {
                    (Some(_), false) => E::instance_is_string(),
                    (Some(_), true) => E::instance_is_number(),
                    (None, _) => quote! { false },
                };
                if self.reporting {
                    return quote! {
                        __units = __e.reported::<__J>(
                            __units,
                            #index,
                            &instance,
                            __path,
                            #annotated,
                            __position,
                        );
                    };
                }
                let check = unit.direct.expect("every leaf unit checks itself");
                if self.not_of_type[check] {
                    let check = format_ident!("__not_of_type_{check}");
                    quote! {
                        __units =
                            __e.not_of_type::<__J>(__units, #index, #check, &instance, __position);
                    }
                } else {
                    let check = format_ident!("__direct_{check}");
                    quote! {
                        __units = __e.checked::<__J>(
                            __units,
                            #index,
                            #check,
                            &instance,
                            #annotated,
                            __position,
                        );
                    }
                }
            }
            TreeUnit::Properties {
                declared: (first, count),
                in_member_order,
            } => {
                let declared = &self.tree.properties[*first..first + count];
                if declared.is_empty() {
                    let body = quote! {
                        __units = __e.unit(
                            __units,
                            #index,
                            Vec::new(),
                            Some(__eval::names(Vec::new())),
                            __eval::ChildList::default(),
                            __position,
                        )
                    };
                    return E::sole_object_match(quote! { #body; }, quote! {{ #empty; }});
                }
                let names: Vec<&str> = declared.iter().map(|(name, _)| name.as_str()).collect();
                let count = names.len();
                let offsets = 0..count;
                let members = E::object_iter_entries(quote! { obj });
                let key = E::key_as_str(quote! { __key });
                // One declared property needs no order.
                let order = (!in_member_order && count > 1).then(|| {
                    let len = E::object_len(quote! { obj });
                    quote! {
                        if __eval::in_declared_order::<__J>(#len, #count) {
                            __eval::sort_declared(&mut __matched);
                        }
                    }
                });
                // The last declared property takes the match's remaining arm.
                let mut children: Vec<TokenStream> = declared
                    .iter()
                    .map(|(_, subschema)| self.child::<E>(*subschema))
                    .collect();
                let last = children.pop().expect("declared properties");
                let child_offsets = 0..children.len();
                let body = quote! {
                    const __NAMES: [&str; #count] = [#(#names),*];
                    let mut __matched = Vec::new();
                    for (__key, __value) in #members {
                        let __offset: usize = match #key {
                            #(#names => #offsets,)*
                            _ => continue,
                        };
                        __matched.push((__offset, __value));
                    }
                    #order
                    let mut __names = Vec::with_capacity(__matched.len());
                    let mut __children = __eval::ChildList::default();
                    for (__offset, instance) in __matched {
                        let __name = __NAMES[__offset];
                        __names.push(__name);
                        #below
                        let __child = match __offset {
                            #(#child_offsets => #children,)*
                            _ => #last,
                        };
                        __children = __e.push(__children, __child);
                    }
                    __units = __e.unit(
                        __units,
                        #index,
                        Vec::new(),
                        Some(__eval::names(__names)),
                        __children,
                        __position,
                    );
                };
                E::sole_object_match(body, quote! {{ #empty; }})
            }
            TreeUnit::Patterns((first, count)) => {
                let members = E::object_iter_entries(quote! { obj });
                let key = E::key_as_str(quote! { __key });
                let loops = (*first..first + count).map(|pattern| {
                    let matcher = format_ident!("__matches_{pattern}");
                    let child = self.child::<E>(self.tree.patterns[pattern].0);
                    quote! {
                        for (__key, instance) in #members {
                            let __name: &str = #key;
                            if #matcher(__name) {
                                __names.push(__sj::Value::String(__name.to_owned()));
                                #below
                                let __child = #child;
                                __children = __e.push(__children, __child);
                            }
                        }
                    }
                });
                let body = quote! {
                    let mut __names = Vec::new();
                    let mut __children = __eval::ChildList::default();
                    #(#loops)*
                    __units = __e.unit(
                        __units,
                        #index,
                        Vec::new(),
                        Some(__eval::values(__names)),
                        __children,
                        __position,
                    );
                };
                E::sole_object_match(body, quote! {{ #empty; }})
            }
            TreeUnit::Additional {
                schema: subschema,
                closed,
                declared,
                patterns,
            } => {
                let names: Vec<&str> = declared
                    .map(|(first, count)| &self.tree.properties[first..first + count])
                    .unwrap_or_default()
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect();
                let matchers: Vec<_> = patterns
                    .map(|(first, count)| first..first + count)
                    .unwrap_or_default()
                    .map(|pattern| format_ident!("__matches_{pattern}"))
                    .collect();
                let skip = (!names.is_empty() || !matchers.is_empty()).then(|| {
                    let declared =
                        (!names.is_empty()).then(|| quote! { matches!(__name, #(#names)|*) });
                    let matched = matchers.iter().map(|matcher| quote! { #matcher(__name) });
                    let conditions = declared.into_iter().chain(matched);
                    quote! {
                        if #(#conditions)||* {
                            continue;
                        }
                    }
                });
                let members = E::object_iter_entries(quote! { obj });
                let key = E::key_as_str(quote! { __key });
                let (names_init, child, annotations) = if *closed {
                    (
                        None,
                        quote! { __e.rejected::<__J>(#subschema, &instance, __path, __position) },
                        quote! { None },
                    )
                } else {
                    let child = self.child::<E>(*subschema);
                    (
                        Some(quote! { let mut __names = Vec::new(); }),
                        quote! {{
                            __names.push(__sj::Value::String(__name.to_owned()));
                            #child
                        }},
                        quote! { Some(__eval::values(__names)) },
                    )
                };
                let body = quote! {
                    #names_init
                    let mut __children = __eval::ChildList::default();
                    for (__key, instance) in #members {
                        let __name: &str = #key;
                        #skip
                        #below
                        let __child = #child;
                        __children = __e.push(__children, __child);
                    }
                    __units = __e.unit(
                        __units,
                        #index,
                        Vec::new(),
                        #annotations,
                        __children,
                        __position,
                    );
                };
                E::sole_object_match(body, quote! {{ #empty; }})
            }
            TreeUnit::Names {
                schema: subschema,
                closed,
            } => {
                let members = E::object_iter_entries(quote! { obj });
                let declare = E::declare_key_node(quote! { __key });
                let name = E::key_node_expr(quote! { __key });
                let entry = &self.tree.schemas[*subschema];
                let child = if *closed {
                    quote! { __e.rejected::<__J>(#subschema, &instance, __path, __position) }
                } else if self.tree.units[entry.first..entry.first + entry.count]
                    .iter()
                    .all(|unit| matches!(unit, TreeUnit::Leaf(_)))
                {
                    // A schema of leaf units holds no reference, so no cycle guard sees the name.
                    self.child::<E>(*subschema)
                } else {
                    // The name is evaluated at the object's location, as a different instance.
                    let child = self.child::<E>(*subschema);
                    quote! {{
                        let __active = __e.take_active();
                        let __child = #child;
                        __e.restore_active(__active);
                        __child
                    }}
                };
                let body = quote! {
                    let mut __children = __eval::ChildList::default();
                    for (__key, _) in #members {
                        #declare
                        let instance = #name;
                        let __child = #child;
                        __children = __e.push(__children, __child);
                    }
                    __units = __e.unit(__units, #index, Vec::new(), None, __children, __position);
                };
                E::sole_object_match(body, quote! {{ #empty; }})
            }
            TreeUnit::Dependencies {
                schemas: (first, count),
                ..
            } => {
                let present = self.tree.dependencies[*first..first + count].iter().map(
                    |(name, subschema)| {
                        let contains = E::object_contains_key(quote! { obj }, name);
                        let child = self.child::<E>(*subschema);
                        quote! {
                            if #contains {
                                let __child = #child;
                                __children = __e.push(__children, __child);
                            }
                        }
                    },
                );
                let errors = Self::unit_errors(index);
                let body = quote! {
                    let __errors = #errors;
                    let mut __children = __eval::ChildList::default();
                    #(#present)*
                    __units = __e.unit(__units, #index, __errors, None, __children, __position);
                };
                E::sole_object_match(body, quote! {{ #empty; }})
            }
            TreeUnit::Tuple {
                schemas: (first, count),
                ..
            } => {
                let schemas = &self.tree.tuples[*first..first + count];
                let items = E::array_iter(quote! { arr });
                let len = E::array_len(quote! { arr });
                // The last position takes the match's remaining arm.
                let mut children: Vec<TokenStream> = schemas
                    .iter()
                    .map(|subschema| self.child::<E>(*subschema))
                    .collect();
                let count = schemas.len();
                let items_loop = children.pop().map(|last| {
                    let offsets = 0..children.len();
                    quote! {
                        for (__idx, instance) in #items.enumerate() {
                            if __idx == #count {
                                break;
                            }
                            #below_item
                            let __child = match __idx {
                                #(#offsets => #children,)*
                                _ => #last,
                            };
                            __children = __e.push(__children, __child);
                        }
                    }
                });
                let body = quote! {
                    if #len == 0 {
                        #empty;
                    } else {
                        let mut __children = __eval::ChildList::default();
                        #items_loop
                        let __annotation = __eval::tuple(__children, #len);
                        __units = __e.unit(
                            __units,
                            #index,
                            Vec::new(),
                            Some(__annotation),
                            __children,
                            __position,
                        );
                    }
                };
                E::sole_array_match(body, quote! {{ #empty; }})
            }
            TreeUnit::Rest {
                schema: subschema,
                skip,
                closed,
                ..
            } => {
                if !*closed && self.item_runs[*subschema] {
                    let leaf_item = Self::leaf_item_name(*subschema);
                    return quote! {
                        __units = __e.leaf_items::<__J>(
                            __units,
                            #index,
                            #subschema,
                            #skip,
                            #leaf_item,
                            &instance,
                            __path,
                            __position,
                        );
                    };
                }
                if let Some((target, function)) = self.static_reference(*subschema).filter(|_| {
                    !*closed && !self.callable[*subschema] && self.enclosing(*subschema).is_none()
                }) {
                    let guard = self.guard(function);
                    let evaluate = Self::evaluate_fn(target);
                    return quote! {
                        __units = __e.reference_items::<__J>(
                            __units,
                            #index,
                            #subschema,
                            #skip,
                            #guard,
                            #evaluate,
                            &instance,
                            __path,
                            __position,
                        );
                    };
                }
                let items = E::array_iter(quote! { arr });
                let len = E::array_len(quote! { arr });
                let enumerate = if *skip == 0 {
                    quote! { #items.enumerate() }
                } else {
                    quote! { #items.enumerate().skip(#skip) }
                };
                let child = if *closed {
                    quote! { __e.rejected::<__J>(#subschema, &instance, __path, __position) }
                } else {
                    self.child::<E>(*subschema)
                };
                let body = quote! {
                    let mut __children = __eval::ChildList::default();
                    for (__idx, instance) in #enumerate {
                        #below_item
                        let __child = #child;
                        __children = __e.push(__children, __child);
                    }
                    __units = __e.unit(
                        __units,
                        #index,
                        Vec::new(),
                        Some(__eval::boolean(#len > #skip)), __children, __position,
                    );
                };
                E::sole_array_match(body, quote! {{ #empty; }})
            }
            TreeUnit::Contains {
                schema: subschema,
                valid,
                bounds: ContainsBounds { min, max },
            } => {
                let items = E::array_iter(quote! { arr });
                let check = self.validity(*valid);
                let child = self.child::<E>(*subschema);
                let errors = if self.reporting {
                    Self::unit_errors(index)
                } else {
                    let min = min.map_or_else(|| quote! { None }, |min| quote! { Some(#min) });
                    let max = max.map_or_else(|| quote! { None }, |max| quote! { Some(#max) });
                    quote! {
                        if __e.checks() {
                            __eval::contains_errors::<__J>(__indices.len(), #min, #max, &instance)
                        } else {
                            Vec::new()
                        }
                    }
                };
                let body = quote! {
                    let mut __indices = Vec::new();
                    let mut __children = __eval::ChildList::default();
                    for (__idx, instance) in #items.enumerate() {
                        if !#check {
                            continue;
                        }
                        __indices.push(__idx);
                        #below_item
                        let __child = #child;
                        __children = __e.push(__children, __child);
                    }
                    let __errors = #errors;
                    let __annotations = if __indices.is_empty() {
                        None
                    } else {
                        Some(__eval::indices(__indices))
                    };
                    __units = __e.unit(
                        __units,
                        #index,
                        __errors,
                        __annotations,
                        __children,
                        __position,
                    );
                };
                E::sole_array_match(body, quote! {{ #empty; }})
            }
            TreeUnit::Branches {
                keyword,
                schemas: (first, count),
                valid,
                discriminator,
            } => {
                let schemas = &self.tree.tuples[*first..first + count];
                let children: Vec<TokenStream> = schemas
                    .iter()
                    .map(|subschema| self.child::<E>(*subschema))
                    .collect();
                let Some(valid) = valid else {
                    return quote! {{
                        let mut __children = __eval::ChildList::default();
                        #(
                            let __child = #children;
                            __children = __e.push(__children, __child);
                        )*
                        __units = __e.unit(
                            __units,
                            #index,
                            Vec::new(),
                            None,
                            __children,
                            __position,
                        );
                    }};
                };
                let candidates = discriminator.map(|discriminator| {
                    let candidates = format_ident!("__candidates_{discriminator}");
                    quote! { let __candidates = #candidates(&instance); }
                });
                let accepts = (0..schemas.len()).map(|offset| {
                    let check = self.validity(valid + offset);
                    if discriminator.is_some() {
                        quote! { __eval::candidate(__candidates, #offset) && #check }
                    } else {
                        check
                    }
                });
                let offsets = 0..schemas.len();
                let errors = if self.reporting {
                    Self::unit_errors(index)
                } else if *keyword == "oneOf" {
                    quote! {
                        if __e.checks() && __eval::several(&__accepts) {
                            __eval::one_of_multiple_valid::<__J>(&instance)
                        } else {
                            Vec::new()
                        }
                    }
                } else {
                    quote! { Vec::new() }
                };
                quote! {{
                    #candidates
                    let __accepts = [#(#accepts),*];
                    let __any = __accepts.contains(&true);
                    let mut __children = __eval::ChildList::default();
                    #(
                        if !__any || __accepts[#offsets] {
                            let __child = #children;
                            __children = __e.push(__children, __child);
                        }
                    )*
                    __units = __e.unit(__units, #index, #errors, None, __children, __position);
                }}
            }
            TreeUnit::Condition {
                schema: subschema,
                valid,
            } => {
                let check = self.validity(*valid);
                let child = self.child::<E>(*subschema);
                quote! {
                    let __passed = #check;
                    {
                        let mut __children = __eval::ChildList::default();
                        if __passed {
                            let __child = #child;
                            __children = __e.push(__children, __child);
                        }
                        __units = __e.unit(
                            __units,
                            #index,
                            Vec::new(),
                            None,
                            __children,
                            __position,
                        );
                    }
                }
            }
            TreeUnit::Branch {
                keyword,
                schema: subschema,
            } => {
                let child = self.child::<E>(*subschema);
                let passed = if *keyword == "then" {
                    quote! { __passed }
                } else {
                    quote! { !__passed }
                };
                quote! {
                    if #passed {
                        let mut __children = __eval::ChildList::default();
                        let __child = #child;
                        __children = __e.push(__children, __child);
                        __units = __e.unit(
                            __units,
                            #index,
                            Vec::new(),
                            None,
                            __children,
                            __position,
                        );
                    }
                }
            }
            TreeUnit::Unevaluated {
                schema: subschema,
                closed,
                evaluated,
                ..
            } => {
                let resource = self.tree.unevaluated_resources[*evaluated];
                let positions = format_ident!("__unevaluated_{evaluated}");
                let subschema_errors = format_ident!("__unevaluated_errors_{evaluated}");
                let (enter, leave) = if self.scoped {
                    (
                        Some(quote! { __e.enter_scope(#resource); }),
                        Some(quote! { __e.leave_scope(#resource); }),
                    )
                } else {
                    (None, None)
                };
                let evaluate = Self::evaluate_fn(*subschema);
                let child = if *closed {
                    quote! { __e.rejected::<__J>(#subschema, &instance, __path, __position) }
                } else {
                    self.descending(*subschema, quote! {{
                        #enter
                        let __subschema_errors = #subschema_errors(&instance, __path);
                        #leave
                        if __subschema_errors.is_empty() {
                            #evaluate(__e, instance, __path, __position)
                        } else {
                            // Its errors, reported at the evaluation paths its keywords get here.
                            let __nested_errors = __e.nested_errors(&__subschema_errors);
                            let mut __nested = __e.nest(&__nested_errors);
                            let __child = #evaluate(&mut __nested, instance, __path, __position);
                            __e.unnest(__nested);
                            __child
                        }
                    }})
                };
                let members = E::object_iter_entries(quote! { obj });
                let key = E::key_as_str(quote! { __key });
                let items = E::array_iter(quote! { arr });
                let objects = E::if_object(
                    quote! { instance },
                    quote! {
                        for (__offset, (__key, instance)) in #members.enumerate() {
                            if __positions.next_if_eq(&__offset).is_some() {
                                let __name: &str = #key;
                                #below
                                let __child = #child;
                                __children = __e.push(__children, __child);
                            }
                        }
                    },
                );
                let arrays = E::if_array(
                    quote! { instance },
                    quote! {
                        for (__idx, instance) in #items.enumerate() {
                            if __positions.next_if_eq(&__idx).is_some() {
                                #below_item
                                let __child = #child;
                                __children = __e.push(__children, __child);
                            }
                        }
                    },
                );
                let errors = Self::unit_errors(index);
                quote! {{
                    let __errors = #errors;
                    #enter
                    let __positions = #positions(&instance);
                    #leave
                    let mut __positions = __positions.into_iter().peekable();
                    let mut __children = __eval::ChildList::default();
                    #objects
                    #arrays
                    __units = __e.unit(__units, #index, __errors, None, __children, __position);
                }}
            }
            TreeUnit::Ref {
                target,
                function,
                anchor,
                recursive,
                holder,
                ..
            } => {
                let guard = self.guard(*function);
                let evaluate = Self::evaluate_fn(*target);
                if anchor.is_none() && !recursive {
                    return quote! {
                        __units = __e.reference::<__J>(
                            __units,
                            #index,
                            #guard,
                            #evaluate,
                            instance,
                            __path,
                            __position,
                        );
                    };
                }
                let lands_on_holder =
                    |schema: usize| *holder == Some(self.tree.schemas[schema].address);
                let (on_holder, landings): (Vec<usize>, Vec<usize>) = self
                    .landings(*target, anchor.as_deref(), *recursive)
                    .into_iter()
                    .partition(|&landed| lands_on_holder(landed));
                let anchor = anchor
                    .as_ref()
                    .map_or_else(|| quote! { None }, |anchor| quote! { Some(#anchor) });
                let landed_nodes = landings.iter().map(|&landed| {
                    let evaluate = Self::evaluate_fn(landed);
                    quote! {
                        __e.landed::<__J>(
                            #index,
                            #landed,
                            __landing.pushed,
                            #evaluate,
                            instance,
                            __path,
                            __position,
                        )
                    }
                });
                let static_node = quote! {
                    __e.reference_node::<__J>(
                        #index,
                        #guard,
                        __landing.pushed,
                        #evaluate,
                        instance,
                        __path,
                        __position,
                    )
                };
                if on_holder.is_empty() && !lands_on_holder(*target) {
                    return quote! {{
                        let __landing = __e.resolve(#index, #anchor, #recursive);
                        let __node = match __landing.schema {
                            #(#landings => #landed_nodes,)*
                            _ => #static_node,
                        };
                        __units = __e.push(__units, __node);
                    }};
                }
                // Landing on the schema holding it, it applies nothing.
                let on_holder = (!on_holder.is_empty() && !lands_on_holder(*target))
                    .then(|| quote! { #(#on_holder)|* => {} });
                let fallback = (!lands_on_holder(*target)).then(|| {
                    quote! {
                        let __node = #static_node;
                        __units = __e.push(__units, __node);
                    }
                });
                quote! {{
                    let __landing = __e.resolve(#index, #anchor, #recursive);
                    match __landing.schema {
                        #(
                            #landings => {
                                let __node = #landed_nodes;
                                __units = __e.push(__units, __node);
                            }
                        )*
                        #on_holder
                        _ => { #fallback }
                    }
                }}
            }
        }
    }

    /// The schemas other than `target` a reference resolving through the dynamic scope may land
    /// on: the `anchor` schemas, or the roots under `$recursiveRef`, of every resource.
    fn landings(&self, target: usize, anchor: Option<&str>, recursive: bool) -> BTreeSet<usize> {
        landings(self.tree, anchor, recursive)
            .into_iter()
            .filter(|&landed| landed != target)
            .collect()
    }
}

/// The schemas a reference resolving through the dynamic scope may land on: the `anchor` schemas,
/// or the roots under `$recursiveRef`, of every resource.
fn landings(tree: &Tree, anchor: Option<&str>, recursive: bool) -> BTreeSet<usize> {
    let resources = tree.resources.iter();
    if recursive {
        resources
            .filter(|resource| resource.recursive)
            .filter_map(|resource| resource.root)
            .collect()
    } else if let Some(anchor) = anchor {
        resources
            .flat_map(|resource| &resource.anchors)
            .filter(|(name, _)| name == anchor)
            .map(|&(_, schema)| schema)
            .collect()
    } else {
        BTreeSet::new()
    }
}

/// Whether each schema of `tree` gets a function of its own: the root, every schema a reference
/// may land on, and every `unevaluated*` subschema, which two evaluations call.
fn callable(tree: &Tree) -> Vec<bool> {
    let mut callable = vec![false; tree.schemas.len()];
    callable[0] = true;
    for unit in &tree.units {
        match unit {
            TreeUnit::Ref {
                target,
                anchor,
                recursive,
                ..
            } => {
                callable[*target] = true;
                for landed in landings(tree, anchor.as_deref(), *recursive) {
                    callable[landed] = true;
                }
            }
            TreeUnit::Unevaluated {
                schema,
                closed: false,
                ..
            } => callable[*schema] = true,
            _ => {}
        }
    }
    callable
}

/// Units of inlined code from which a schema gets a function of its own: LLVM optimizes one
/// function holding a whole document's code slower than the same code in parts.
const OUTLINED_UNITS: usize = 16;

/// Whether each schema gets a function for its large code, marking it in `callable`. A schema's
/// code counts its units and those of the schemas inlined into it.
fn outlined(tree: &Tree, item_runs: &[bool], callable: &mut [bool]) -> Vec<bool> {
    fn size(
        tree: &Tree,
        schema: usize,
        item_runs: &[bool],
        callable: &mut [bool],
        outlined: &mut [bool],
    ) -> usize {
        let entry = &tree.schemas[schema];
        let mut total = entry.count;
        for unit in &tree.units[entry.first..entry.first + entry.count] {
            for child in inlined_children(tree, unit) {
                if callable[child] {
                    continue;
                }
                let inlined = size(tree, child, item_runs, callable, outlined);
                // An item run's schema below `items` has its leaf item function instead.
                if inlined >= OUTLINED_UNITS && !item_runs[child] {
                    callable[child] = true;
                    outlined[child] = true;
                } else {
                    total += inlined;
                }
            }
        }
        total
    }
    let mut outlined = vec![false; callable.len()];
    let roots: Vec<usize> = (0..callable.len())
        .filter(|&schema| callable[schema])
        .collect();
    for root in roots {
        size(tree, root, item_runs, callable, &mut outlined);
    }
    outlined
}

/// The subschemas whose code the code of `unit` holds, unless they have functions of their own.
fn inlined_children(tree: &Tree, unit: &TreeUnit) -> Vec<usize> {
    match unit {
        TreeUnit::Leaf(_) | TreeUnit::Ref { .. } => Vec::new(),
        TreeUnit::Properties {
            declared: (first, count),
            ..
        } => tree.properties[*first..first + count]
            .iter()
            .map(|(_, schema)| *schema)
            .collect(),
        TreeUnit::Patterns((first, count)) => tree.patterns[*first..first + count]
            .iter()
            .map(|(schema, _)| *schema)
            .collect(),
        TreeUnit::Dependencies {
            schemas: (first, count),
            ..
        } => tree.dependencies[*first..first + count]
            .iter()
            .map(|(_, schema)| *schema)
            .collect(),
        TreeUnit::Branches {
            schemas: (first, count),
            ..
        }
        | TreeUnit::Tuple {
            schemas: (first, count),
            ..
        } => tree.tuples[*first..first + count].to_vec(),
        TreeUnit::Additional { schema, .. }
        | TreeUnit::Names { schema, .. }
        | TreeUnit::Contains { schema, .. }
        | TreeUnit::Condition { schema, .. }
        | TreeUnit::Branch { schema, .. }
        | TreeUnit::Rest { schema, .. }
        | TreeUnit::Unevaluated { schema, .. } => vec![*schema],
    }
}

/// Functions putting a resource's entries on the generated code's dynamic-scope stacks, and taking
/// them off, so the generated validity code resolves dynamic references in the evaluation's scope.
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
            let (location, schema, base, dialect) = (
                resolved.location,
                resolved.schema,
                resolved.base_uri,
                resolved.dialect,
            );
            let is_valid = format_ident!(
                "{}",
                get_or_create_is_valid_fn(
                    ctx,
                    &location,
                    resolved.resource_start,
                    schema,
                    base.clone(),
                    dialect.clone(),
                )
            );
            let key_eval = format_ident!(
                "{}",
                get_or_create_key_eval_fn(ctx, &location, schema, base.clone(), dialect.clone())
            );
            let item_eval = format_ident!(
                "{}",
                get_or_create_item_eval_fn(ctx, &location, schema, base, dialect)
            );
            let errors = format_ident!("{is_valid}_errors");
            enter.extend([
                push_recursive_is_valid(&is_valid, true),
                push_recursive_key_eval(&key_eval, true),
                push_recursive_item_eval(&item_eval, true),
                push_recursive_errors(&errors, true),
            ]);
            leave.extend([
                pop_recursive_errors(),
                pop_recursive_item_eval(),
                pop_recursive_key_eval(),
                pop_recursive_is_valid(),
            ]);
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
        fn __enter_resource(resource: usize) {
            match resource {
                #(#enter_arms)*
                _ => {}
            }
        }
        fn __leave_resource(resource: usize) {
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
        // With format validation on, the runtime gives an ignored unknown format no unit.
        (_, "format") if ctx.validates_formats() && !format_emits_assertion(ctx, value) => None,
        (_, "format") => Some(unit.annotating(value, reads_numbers(ctx, value))),
        // Draft 7 asserts content: the runtime skips an unsupported media type or encoding, and
        // reports both keywords in the unit of a supported media type.
        (Draft::Draft7, "contentMediaType")
            if !checks_media_type(ctx, value)
                || keywords
                    .get("contentEncoding")
                    .is_some_and(|encoding| !checks_encoding(ctx, encoding)) =>
        {
            None
        }
        (Draft::Draft7, "contentEncoding")
            if !checks_encoding(ctx, value)
                || keywords
                    .get("contentMediaType")
                    .is_some_and(|media_type| checks_media_type(ctx, media_type)) =>
        {
            None
        }
        (Draft::Draft7, "contentMediaType") if keywords.contains_key("contentEncoding") => {
            Some(unit.with_alias("contentEncoding"))
        }
        (Draft::Draft7, _) => Some(unit),
        (_, "contentSchema") if !keywords.contains_key("contentMediaType") => None,
        (_, "contentEncoding" | "contentMediaType" | "contentSchema") => {
            Some(unit.annotating(value, false))
        }
        _ => Some(unit),
    }
}

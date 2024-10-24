use std::{rc::Rc, sync::Arc};

use crate::{
    compiler,
    error::ErrorIterator,
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location},
    primitive_type::PrimitiveType,
    validator::{PartialApplication, Validate},
    ValidationError, ValidationOptions,
};
use once_cell::sync::OnceCell;
use referencing::{Draft, List, Registry, Resource, Uri, VocabularySet};
use serde_json::{Map, Value};

pub(crate) enum RefValidator {
    Default { inner: SchemaNode },
    Lazy(LazyRefValidator),
}

impl RefValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        reference: &str,
        is_recursive: bool,
        keyword: &str,
    ) -> Option<CompilationResult<'a>> {
        let location = ctx.location().join(keyword);
        Some(
            if let Some((base_uri, scopes, resource)) = {
                match ctx.lookup_maybe_recursive(reference, is_recursive) {
                    Ok(resolved) => resolved,
                    Err(error) => return Some(Err(error)),
                }
            } {
                // NOTE: A better approach would be to compare the absolute locations
                if let Value::Object(contents) = resource.contents() {
                    if let Some(Some(resolved)) = contents.get(keyword).map(Value::as_str) {
                        if resolved == reference {
                            return None;
                        }
                    }
                }
                Ok(Box::new(RefValidator::Lazy(LazyRefValidator {
                    resource,
                    config: Arc::clone(ctx.config()),
                    registry: Arc::clone(&ctx.registry),
                    base_uri,
                    scopes,
                    location,
                    vocabularies: ctx.vocabularies().clone(),
                    draft: ctx.draft(),
                    inner: OnceCell::default(),
                })))
            } else {
                let (contents, resolver, draft) = match ctx.lookup(reference) {
                    Ok(resolved) => resolved.into_inner(),
                    Err(error) => return Some(Err(error.into())),
                };
                let vocabularies = ctx.registry.find_vocabularies(draft, contents);
                let resource_ref = draft.create_resource_ref(contents);
                let ctx = ctx.with_resolver_and_draft(
                    resolver,
                    resource_ref.draft(),
                    vocabularies,
                    location,
                );
                let inner = match compiler::compile_with(&ctx, resource_ref)
                    .map_err(|err| err.into_owned())
                {
                    Ok(inner) => inner,
                    Err(error) => return Some(Err(error)),
                };
                Ok(Box::new(RefValidator::Default { inner }))
            },
        )
    }
}

/// Lazily evaluated validator used for recursive references.
///
/// The validator tree nodes can't be arbitrary looked up in the current
/// implementation to build a cycle, therefore recursive references are validated
/// by building and caching the next subtree lazily. Though, other memory
/// representation for the validation tree may allow building cycles easier and
/// lazy evaluation won't be needed.
pub(crate) struct LazyRefValidator {
    resource: Resource,
    config: Arc<ValidationOptions>,
    registry: Arc<Registry>,
    scopes: List<Uri<String>>,
    base_uri: Arc<Uri<String>>,
    vocabularies: VocabularySet,
    location: Location,
    draft: Draft,
    inner: OnceCell<SchemaNode>,
}

impl LazyRefValidator {
    #[inline]
    pub(crate) fn compile<'a>(ctx: &compiler::Context) -> CompilationResult<'a> {
        let scopes = ctx.scopes();
        let resolved = ctx.lookup_recursive_reference()?;
        let resource = ctx.draft().create_resource(resolved.contents().clone());
        let resolver = resolved.resolver();
        let mut base_uri = resolver.base_uri();
        if let Some(id) = resource.id() {
            base_uri = resolver.resolve_against(&base_uri.borrow(), id)?;
        };
        Ok(Box::new(LazyRefValidator {
            resource,
            config: Arc::clone(ctx.config()),
            registry: Arc::clone(&ctx.registry),
            base_uri,
            scopes,
            vocabularies: ctx.vocabularies().clone(),
            location: ctx.location().join("$recursiveRef"),
            draft: ctx.draft(),
            inner: OnceCell::default(),
        }))
    }
    fn lazy_compile(&self) -> &SchemaNode {
        self.inner.get_or_init(|| {
            let resolver = self
                .registry
                .resolver_from_raw_parts(self.base_uri.clone(), self.scopes.clone());

            let ctx = compiler::Context::new(
                Arc::clone(&self.config),
                Arc::clone(&self.registry),
                Rc::new(resolver),
                self.vocabularies.clone(),
                self.draft,
                self.location.clone(),
            );
            // INVARIANT: This schema was already used during compilation before detecting a
            // reference cycle that lead to building this validator.
            compiler::compile(&ctx, self.resource.as_ref()).expect("Invalid schema")
        })
    }
}

impl Validate for LazyRefValidator {
    fn is_valid(&self, instance: &Value) -> bool {
        self.lazy_compile().is_valid(instance)
    }
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        self.lazy_compile().validate(instance, location)
    }
    fn iter_errors<'i>(&self, instance: &'i Value, location: &LazyLocation) -> ErrorIterator<'i> {
        self.lazy_compile().iter_errors(instance, location)
    }
    fn apply<'a>(&'a self, instance: &Value, location: &LazyLocation) -> PartialApplication<'a> {
        self.lazy_compile().apply(instance, location)
    }
}

impl Validate for RefValidator {
    fn is_valid(&self, instance: &Value) -> bool {
        match self {
            RefValidator::Default { inner } => inner.is_valid(instance),
            RefValidator::Lazy(lazy) => lazy.is_valid(instance),
        }
    }
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        match self {
            RefValidator::Default { inner } => inner.validate(instance, location),
            RefValidator::Lazy(lazy) => lazy.validate(instance, location),
        }
    }
    fn iter_errors<'i>(&self, instance: &'i Value, location: &LazyLocation) -> ErrorIterator<'i> {
        match self {
            RefValidator::Default { inner } => inner.iter_errors(instance, location),
            RefValidator::Lazy(lazy) => lazy.iter_errors(instance, location),
        }
    }
    fn apply<'a>(&'a self, instance: &Value, location: &LazyLocation) -> PartialApplication<'a> {
        match self {
            RefValidator::Default { inner } => inner.apply(instance, location),
            RefValidator::Lazy(lazy) => lazy.apply(instance, location),
        }
    }
}

fn invalid_reference<'a>(ctx: &compiler::Context, schema: &'a Value) -> ValidationError<'a> {
    ValidationError::single_type_error(
        Location::new(),
        ctx.location().clone(),
        schema,
        PrimitiveType::String,
    )
}

#[inline]
pub(crate) fn compile_impl<'a>(
    ctx: &compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
    keyword: &str,
) -> Option<CompilationResult<'a>> {
    let is_recursive = parent
        .get("$recursiveAnchor")
        .and_then(Value::as_bool)
        .unwrap_or_default();
    if let Some(reference) = schema.as_str() {
        RefValidator::compile(ctx, reference, is_recursive, keyword)
    } else {
        Some(Err(invalid_reference(ctx, schema)))
    }
}

#[inline]
pub(crate) fn compile_dynamic_ref<'a>(
    ctx: &compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    compile_impl(ctx, parent, schema, "$dynamicRef")
}

#[inline]
pub(crate) fn compile_ref<'a>(
    ctx: &compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    compile_impl(ctx, parent, schema, "$ref")
}

#[inline]
pub(crate) fn compile_recursive_ref<'a>(
    ctx: &compiler::Context,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    Some(
        schema
            .as_str()
            .ok_or_else(|| invalid_reference(ctx, schema))
            .and_then(|_| LazyRefValidator::compile(ctx)),
    )
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use referencing::{Draft, Retrieve, Uri};
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(
        &json!({
            "properties": {
                "foo": {"$ref": "#/definitions/foo"}
            },
            "definitions": {
                "foo": {"type": "string"}
            }
        }),
        &json!({"foo": 42}),
        "/properties/foo/$ref/type"
    )]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected)
    }

    #[test]
    fn multiple_errors_locations() {
        let instance = json!({
            "things": [
                { "code": "CC" },
                { "code": "CC" },
            ]
        });
        let schema = json!({
                "type": "object",
                "properties": {
                    "things": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "code": {
                                    "type": "string",
                                    "$ref": "#/$defs/codes"
                                }
                            },
                            "required": ["code"]
                        }
                    }
                },
                "required": ["things"],
                "$defs": { "codes": { "enum": ["AA", "BB"] } }
        });
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let mut iter = validator.iter_errors(&instance);
        let expected = "/properties/things/items/properties/code/$ref/enum";
        assert_eq!(
            iter.next()
                .expect("Should be present")
                .schema_path
                .to_string(),
            expected
        );
        assert_eq!(
            iter.next()
                .expect("Should be present")
                .schema_path
                .to_string(),
            expected
        );
    }

    #[test]
    fn test_relative_base_uri() {
        let schema = json!({
            "$id": "/root",
            "$ref": "#/foo",
            "foo": {
                "$id": "#/foo",
                "$ref": "#/bar"
            },
            "bar": {
                "$id": "#/bar",
                "type": "integer"
            },
        });
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        assert!(validator.is_valid(&json!(2)));
        assert!(!validator.is_valid(&json!("a")));
    }

    #[test_case(
        json!({
            "$id": "https://example.com/schema.json",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "foo": {
                    "type": "array",
                    "items": { "$ref": "#/$defs/item" }
                }
            },
            "$defs": {
                "item": {
                    "type": "object",
                    "required": ["name", "value"],
                    "properties": {
                        "name": { "type": "string" },
                        "value": { "type": "boolean" }
                    }
                }
            }
        }),
        json!({
            "foo": [{"name": "item1", "value": true}]
        }),
        vec![
            ("", "/properties"),
            ("/foo", "/properties/foo/items"),
            ("/foo/0", "/properties/foo/items/$ref/properties"),
        ]
    ; "standard $ref")]
    #[test_case(
        json!({
            "$id": "https://example.com/schema.json",
            "$schema": "https://json-schema.org/draft/2019-09/schema",
            "$recursiveAnchor": true,
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "child": { "$recursiveRef": "#" }
            }
        }),
        json!({
            "name": "parent",
            "child": {
                "name": "child",
                "child": { "name": "grandchild" }
            }
        }),
        vec![
            ("", "/properties"),
            ("/child", "/properties/child/$recursiveRef/properties"),
            ("/child/child", "/properties/child/$recursiveRef/properties/child/$recursiveRef/properties"),
        ]
    ; "$recursiveRef")]
    #[test_case(
        json!({
            "$id": "https://example.com/schema.json",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$dynamicAnchor": "node",
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "child": { "$dynamicRef": "#node" }
            }
        }),
        json!({
            "name": "parent",
            "child": {
                "name": "child",
                "child": { "name": "grandchild" }
            }
        }),
        vec![
            ("", "/properties"),
            ("/child", "/properties/child/$dynamicRef/properties"),
            ("/child/child", "/properties/child/$dynamicRef/properties/child/$dynamicRef/properties"),
        ]
    ; "$dynamicRef")]
    fn test_reference_types_location(
        schema: serde_json::Value,
        instance: serde_json::Value,
        expected_locations: Vec<(&str, &str)>,
    ) {
        let validator = crate::validator_for(&schema).unwrap();

        let crate::BasicOutput::Valid(output) = validator.apply(&instance).basic() else {
            panic!("Should pass validation");
        };

        for (idx, (instance_location, keyword_location)) in expected_locations.iter().enumerate() {
            assert_eq!(
                output[idx].instance_location().to_string(),
                *instance_location,
                "Instance location mismatch at index {idx}"
            );
            assert_eq!(
                output[idx].keyword_location().to_string(),
                *keyword_location,
                "Keyword location mismatch at index {idx}",
            );
        }
    }

    #[test]
    fn test_resolving_finds_references_in_referenced_resources() {
        let schema = json!({"$ref": "/indirection#/baz"});

        struct MyRetrieve;

        impl Retrieve for MyRetrieve {
            fn retrieve(
                &self,
                uri: &Uri<&str>,
            ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
                match uri.path().as_str() {
                    "/indirection" => Ok(json!({
                        "$id": "/indirection",
                        "baz": {
                            "$ref": "/types#/foo"
                        }
                    })),
                    "/types" => Ok(json!({
                        "$id": "/types",
                        "foo": {
                            "$ref": "#/bar"
                        },
                        "bar": {
                            "type": "integer"
                        }
                    })),
                    _ => panic!("Not found"),
                }
            }
        }

        let validator = match crate::options()
            .with_draft(Draft::Draft201909)
            .with_retriever(MyRetrieve)
            .build(&schema)
        {
            Ok(validator) => validator,
            Err(error) => panic!("{error}"),
        };

        assert!(validator.is_valid(&json!(2)));
        assert!(!validator.is_valid(&json!("")));
    }

    #[test]
    fn test_infinite_loop() {
        let validator = crate::validator_for(&json!({"$ref": "#"})).expect("Invalid schema");
        assert!(validator.is_valid(&json!(42)));
    }
}

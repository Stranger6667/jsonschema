use ahash::AHashSet;
use referencing::{Draft, Resolver};
use serde_json::{Map, Value};

use crate::{compiler, options::ValidationOptions};

fn dereference_from_registry(
    schema: &Value,
    draft: Draft,
    registry: &referencing::Registry<'_>,
    base_uri: &referencing::Uri<String>,
) -> Result<Value, referencing::Error> {
    let resolver = registry.resolver(base_uri.clone());
    let mut visited = AHashSet::new();
    walk(schema, draft, &resolver, &mut visited)
}

pub(crate) fn dereference_with_options(
    config: &ValidationOptions<'_>,
    schema: &Value,
) -> Result<Value, referencing::Error> {
    let draft = config.draft_for(schema)?;
    let resource = draft.create_resource_ref(schema);
    if let Some(registry) = config.registry {
        let base_uri = compiler::resolve_base_uri(config.base_uri.as_ref(), resource.id())?;
        let registry = registry
            .add(base_uri.as_str(), resource)?
            .retriever(config.retriever.clone())
            .draft(draft)
            .prepare()?;
        let base_uri = compiler::normalize_base_uri(&registry, &base_uri);
        return dereference_from_registry(schema, draft, &registry, &base_uri);
    }
    let (registry, base_uri) = compiler::build_registry(config, draft, resource, resource.id())?;
    dereference_from_registry(schema, draft, &registry, &base_uri)
}

#[cfg(feature = "resolve-async")]
pub(crate) async fn dereference_with_options_async(
    config: &crate::options::ValidationOptions<'_, std::sync::Arc<dyn referencing::AsyncRetrieve>>,
    schema: &Value,
) -> Result<Value, referencing::Error> {
    let draft = config.draft_for(schema).await?;
    let resource = draft.create_resource_ref(schema);
    if let Some(registry) = config.registry {
        let base_uri = compiler::resolve_base_uri(config.base_uri.as_ref(), resource.id())?;
        let registry = registry
            .add(base_uri.as_str(), resource)?
            .async_retriever(config.retriever.clone())
            .draft(draft)
            .async_prepare()
            .await?;
        let base_uri = compiler::normalize_base_uri(&registry, &base_uri);
        return dereference_from_registry(schema, draft, &registry, &base_uri);
    }
    let (registry, base_uri) =
        compiler::build_registry_async(config, draft, resource, resource.id()).await?;
    dereference_from_registry(schema, draft, &registry, &base_uri)
}

fn walk(
    value: &Value,
    draft: Draft,
    resolver: &Resolver<'_>,
    visited: &mut AHashSet<String>,
) -> Result<Value, referencing::Error> {
    match value {
        Value::Object(obj) => {
            // Update resolver context for embedded resources (handles nested $id)
            let resolver = resolver.in_subresource(draft.create_resource_ref(value))?;

            if let Some(Value::String(ref_str)) = obj.get("$ref") {
                let resolved = resolver.lookup(ref_str)?;
                let (contents, inner_resolver, inner_draft) = resolved.into_inner();

                // Build a cycle-detection key that uniquely identifies the resolved
                // location. We extract the fragment from ref_str itself so that refs
                // like "./other.json#/$defs/Foo" and "./other.json#/$defs/Bar" — both
                // resolving to the same document URI — are treated as distinct nodes.
                let visited_key = match ref_str.split_once('#') {
                    Some((_, fragment)) => {
                        format!("{}#{}", inner_resolver.base_uri().as_str(), fragment)
                    }
                    None => inner_resolver.base_uri().as_str().to_string(),
                };

                if visited.contains(&visited_key) {
                    // Circular back-edge: leave $ref in place
                    return Ok(value.clone());
                }

                visited.insert(visited_key.clone());
                let inlined = walk(contents, inner_draft, &inner_resolver, visited)?;
                visited.remove(&visited_key);

                // Merge non-$ref siblings into inlined result
                if let Value::Object(mut merged) = inlined {
                    for (k, v) in obj {
                        if k != "$ref" && !merged.contains_key(k.as_str()) {
                            // walk sibling values so nested $refs within siblings are also resolved
                            let walked = walk(v, draft, &resolver, visited)?;
                            merged.insert(k.clone(), walked);
                        }
                    }
                    Ok(Value::Object(merged))
                } else {
                    Ok(inlined)
                }
            } else {
                // No $ref: recurse into all values
                let mut result = Map::with_capacity(obj.len());
                for (k, v) in obj {
                    result.insert(k.clone(), walk(v, draft, &resolver, visited)?);
                }
                Ok(Value::Object(result))
            }
        }
        Value::Array(arr) => {
            let items: Result<Vec<_>, _> = arr
                .iter()
                .map(|v| walk(v, draft, resolver, visited))
                .collect();
            Ok(Value::Array(items?))
        }
        _ => Ok(value.clone()),
    }
}

use crate::{compiler, options::ValidationOptions};
use ahash::AHashSet;
use referencing::{Draft, Resolver};
use serde_json::{Map, Value};

fn bundle_from_registry(
    schema: &Value,
    draft: Draft,
    registry: &referencing::Registry,
    base_uri: &referencing::Uri<String>,
) -> Result<Value, referencing::Error> {
    let resolver = registry.resolver(base_uri.clone());
    let mut defs: Map<String, Value> = Map::new();
    let mut visited: AHashSet<String> = AHashSet::new();
    // Seed visited with the root URI so back-references to root are not re-embedded
    visited.insert(base_uri.to_string());
    walk(schema, draft, draft, &resolver, &mut defs, &mut visited)?;
    Ok(merge_defs(schema.clone(), defs, draft))
}

pub(crate) fn bundle_with_options(
    config: &ValidationOptions,
    schema: &Value,
) -> Result<Value, referencing::Error> {
    let draft = config.draft_for(schema)?;
    let resource = draft.create_resource(schema.clone());
    let resource_ref = draft.create_resource_ref(schema);
    let (registry, base_uri) =
        compiler::build_registry(config, draft, resource, resource_ref.id())?;
    bundle_from_registry(schema, draft, &registry, &base_uri)
}

#[cfg(feature = "resolve-async")]
pub(crate) async fn bundle_with_options_async(
    config: &crate::options::ValidationOptions<std::sync::Arc<dyn referencing::AsyncRetrieve>>,
    schema: &Value,
) -> Result<Value, referencing::Error> {
    let draft = config.draft_for(schema).await?;
    let resource = draft.create_resource(schema.clone());
    let resource_ref = draft.create_resource_ref(schema);
    let (registry, base_uri) =
        compiler::build_registry_async(config, draft, resource, resource_ref.id()).await?;
    bundle_from_registry(schema, draft, &registry, &base_uri)
}

fn definitions_keyword(draft: Draft) -> &'static str {
    if matches!(draft, Draft::Draft4 | Draft::Draft6 | Draft::Draft7) {
        "definitions"
    } else {
        "$defs"
    }
}

fn id_keyword(draft: Draft) -> &'static str {
    if matches!(draft, Draft::Draft4) {
        "id"
    } else {
        "$id"
    }
}

fn merge_entries(target: &mut Map<String, Value>, defs: Map<String, Value>) {
    for (k, v) in defs {
        target.entry(k).or_insert(v);
    }
}

fn merge_defs(mut root: Value, defs: Map<String, Value>, root_draft: Draft) -> Value {
    if !defs.is_empty() {
        if let Some(obj) = root.as_object_mut() {
            let container_keyword = definitions_keyword(root_draft);
            if let Some(Value::Object(existing)) = obj.get_mut(container_keyword) {
                merge_entries(existing, defs);
                return root;
            }

            // In 2019-09/2020-12, both `definitions` and `$defs` are traversed as
            // schema containers. If the opposite one already exists, merge into it.
            // For draft-06/07 we intentionally do not merge into `$defs` because it is
            // not a subresource location there.
            if matches!(
                root_draft,
                Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
            ) {
                let alternate = if container_keyword == "$defs" {
                    "definitions"
                } else {
                    "$defs"
                };
                if let Some(Value::Object(existing)) = obj.get_mut(alternate) {
                    merge_entries(existing, defs);
                    return root;
                }
            }

            obj.insert(container_keyword.to_string(), Value::Object(defs));
        }
    }
    root
}

fn ensure_embedded_ids(
    obj: &mut Map<String, Value>,
    uri: &str,
    root_draft: Draft,
    embedded_draft: Draft,
) {
    let root_keyword = id_keyword(root_draft);
    let embedded_keyword = id_keyword(embedded_draft);
    let uri_value = Value::String(uri.to_owned());

    obj.entry(embedded_keyword.to_string())
        .or_insert_with(|| uri_value.clone());
    if root_keyword != embedded_keyword {
        obj.entry(root_keyword.to_string()).or_insert(uri_value);
    }
}

fn walk(
    schema: &Value,
    root_draft: Draft,
    draft: Draft,
    resolver: &Resolver<'_>,
    defs: &mut Map<String, Value>,
    visited: &mut AHashSet<String>,
) -> Result<(), referencing::Error> {
    if let Value::Object(obj) = schema {
        let resolver = resolver.in_subresource(draft.create_resource_ref(schema))?;

        if let Some(Value::String(ref_str)) = obj.get("$ref") {
            // Fragment-only refs (e.g. "#/$defs/Foo") are local — skip
            if !ref_str.starts_with('#') {
                let resolved = resolver.lookup(ref_str)?;
                let (contents, inner_resolver, inner_draft) = resolved.into_inner();
                let base_uri = inner_resolver.base_uri();

                if !visited.contains(base_uri.as_str()) {
                    let uri = base_uri.as_str().to_owned();
                    visited.insert(uri.clone());

                    let mut embedded = contents.clone();
                    // Ensure the embedded resource is discoverable both by its own draft and
                    // by the root draft that will index subresources in the final bundle.
                    if let Some(obj) = embedded.as_object_mut() {
                        ensure_embedded_ids(obj, &uri, root_draft, inner_draft);
                    }

                    // Recurse into the embedded schema BEFORE inserting so
                    // transitive deps are collected in the same defs map
                    walk(
                        &embedded,
                        root_draft,
                        inner_draft,
                        &inner_resolver,
                        defs,
                        visited,
                    )?;

                    defs.insert(uri, embedded);
                }
            }
        }

        // Recurse only into draft-defined subresource locations. This avoids
        // treating annotation payloads as schemas.
        for subresource in draft.subresources_of(schema) {
            walk(subresource, root_draft, draft, &resolver, defs, visited)?;
        }
    }
    Ok(())
}

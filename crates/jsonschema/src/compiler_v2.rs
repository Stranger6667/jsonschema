use std::{borrow::Cow, iter::once, sync::Arc};

use referencing::{uri, Registry};
use serde_json::Value;

use crate::{compiler::DEFAULT_BASE_URI, ValidationOptions};

pub(crate) fn build(mut config: ValidationOptions, schema: &Value) {
    let draft = config.draft_for(schema).unwrap();
    let resource_ref = draft.create_resource_ref(schema);
    let resource = draft.create_resource(schema.clone());
    let base_uri = if let Some(base_uri) = config.base_uri.as_ref() {
        uri::from_str(base_uri).unwrap()
    } else {
        uri::from_str(resource_ref.id().unwrap_or(DEFAULT_BASE_URI)).unwrap()
    };

    // Build a registry & resolver needed for validator compilation
    let resources = &mut config.resources;
    let pairs = once((Cow::Borrowed(base_uri.as_str()), resource)).chain(
        resources
            .drain()
            .map(|(uri, resource)| (Cow::Owned(uri), resource)),
    );

    let registry = if let Some(registry) = config.registry.take() {
        registry
            .try_with_resources_and_retriever(pairs, &*config.retriever, draft)
            .unwrap()
    } else {
        Registry::options()
            .draft(draft)
            .retriever(Arc::clone(&config.retriever))
            .build(pairs)
            .unwrap()
    };
    let a = jsonschema_ir::build(base_uri.clone(), draft, schema, &registry);
    let root = a.root();
    for node in a.children(root) {
        dbg!(a.get(node));
    }
    //for node in jsonschema_ir::traverse(&a) {
    //    dbg!(node);
    //}
}

#[cfg(test)]
mod tests {
    use super::build;
    use serde_json::json;

    #[test]
    fn test_debug() {
        let schema = json!({"type": "object"});
        let config = crate::options();
        build(config, &schema);
    }
}

use std::{borrow::Cow, sync::Arc};

use referencing::Uri;
use serde_json::Value;

use super::CompileContext;

pub(crate) struct ResolvedRef {
    pub(crate) schema: Value,
    pub(crate) location: String,
    /// Base URI of the resolved schema (for resolving nested references)
    pub(crate) base_uri: Arc<Uri<String>>,
}

/// Resolve a short top-level `$ref` chain for branch-shape analysis.
pub(super) fn resolve_top_level_ref_for_one_of_analysis<'b>(
    ctx: &mut CompileContext<'_>,
    schema: &'b Value,
) -> Cow<'b, Value> {
    let mut current = Cow::Borrowed(schema);
    for _ in 0..8 {
        let Value::Object(obj) = current.as_ref() else {
            break;
        };
        let Some(reference) = obj.get("$ref").and_then(Value::as_str) else {
            break;
        };
        let Ok(resolved) = resolve_ref(ctx, reference) else {
            break;
        };
        current = Cow::Owned(resolved.schema);
    }
    current
}

/// Resolve a reference using the Registry.
pub(crate) fn resolve_ref(
    ctx: &mut CompileContext<'_>,
    reference: &str,
) -> Result<ResolvedRef, String> {
    let base_uri = ctx.current_base_uri.clone();

    let resolver = ctx.config.registry.resolver((*base_uri).clone());
    let resolved = resolver
        .lookup(reference)
        .map_err(|e| format!("Failed to resolve {reference}: {e}"))?;

    let resolved_base_uri = resolved.resolver().base_uri().clone();

    let location_key = if reference.starts_with('#') {
        format!("{base_uri}{reference}")
    } else if let Some((_, fragment)) = reference.rsplit_once('#') {
        if fragment.is_empty() {
            resolved_base_uri.to_string()
        } else {
            format!("{resolved_base_uri}#{fragment}")
        }
    } else {
        resolved_base_uri.to_string()
    };
    let (contents, _, _) = resolved.into_inner();

    Ok(ResolvedRef {
        schema: contents.clone(),
        location: location_key,
        base_uri: resolved_base_uri,
    })
}

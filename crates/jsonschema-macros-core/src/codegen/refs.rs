use std::sync::Arc;

use referencing::Uri;
use serde_json::Value;

use super::{draft::DraftExt, CompileContext};

pub(crate) struct ResolvedRef<'cfg> {
    pub(crate) schema: &'cfg Value,
    pub(crate) location: String,
    /// Base URI of the resolved schema (for resolving nested references)
    pub(crate) base_uri: Arc<Uri<String>>,
}

/// Resolve a short top-level `$ref` chain for schema-shape analysis. Returns the resolved schema
/// and whether a `$ref` was followed, so callers can distinguish an inline schema from a hop
/// target. `'cfg: 'b` lets the registry-borrowed hop targets outlive the `schema` borrow.
pub(super) fn resolve_lone_top_level_ref<'cfg, 'b>(
    ctx: &mut CompileContext<'cfg>,
    schema: &'b Value,
) -> (&'b Value, bool)
where
    'cfg: 'b,
{
    let mut current = schema;
    let mut hopped = false;
    // Bounded hop count: discriminator analysis is a best-effort optimization,
    // so give up on longer (or cyclic) $ref chains instead of tracking visited
    // locations. 8 hops covers realistic hand-written indirection.
    for _ in 0..8 {
        let Value::Object(obj) = current else {
            break;
        };
        let Some(reference) = obj.get("$ref").and_then(Value::as_str) else {
            break;
        };
        // Hopping discards `$ref` siblings; on drafts where siblings validate
        // alongside `$ref`, only a lone `$ref` can be followed.
        if ctx.draft.supports_adjacent_validation() && obj.len() > 1 {
            break;
        }
        let Ok(resolved) = resolve_ref(ctx, reference) else {
            break;
        };
        current = resolved.schema;
        hopped = true;
    }
    (current, hopped)
}

/// Resolve a reference using the Registry.
pub(crate) fn resolve_ref<'cfg>(
    ctx: &mut CompileContext<'cfg>,
    reference: &str,
) -> Result<ResolvedRef<'cfg>, String> {
    // Borrow the config out of `ctx` first: `config` is `&'cfg CodegenConfig`, disjoint from
    // `&mut ctx`, so the registry lookup below borrows for `'cfg` instead of the shorter `&mut
    // ctx` borrow, letting `ResolvedRef` hold `&'cfg Value` instead of cloning it.
    let config = ctx.config;
    let base_uri = ctx.current_base_uri.clone();

    let resolver = config.registry.resolver((*base_uri).clone());
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
        schema: contents,
        location: location_key,
        base_uri: resolved_base_uri,
    })
}

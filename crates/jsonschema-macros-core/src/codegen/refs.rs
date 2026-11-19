use crate::codegen::emit::ValueEmitter;
use std::{borrow::Cow, sync::Arc};

use referencing::{Draft, Resolved, Uri};
use serde_json::Value;

use super::{draft::DraftExt, helpers::anchor_pointer, CompileContext};

pub(crate) struct ResolvedRef<'cfg> {
    pub(crate) schema: &'cfg Value,
    pub(crate) location: String,
    /// Base URI of the resolved schema (for resolving nested references)
    pub(crate) base_uri: Arc<Uri<String>>,
    /// The draft of the resource it sits in.
    pub(crate) draft: Draft,
}

/// Resolve a short top-level `$ref` chain for schema-shape analysis.
pub(super) fn resolve_lone_top_level_ref<'cfg: 'b, 'b, E: ValueEmitter>(
    ctx: &mut CompileContext<'cfg, E>,
    schema: &'b Value,
) -> &'b Value {
    let config = ctx.config;
    let mut current = schema;
    // Each hop resolves in the resource the previous one landed in.
    let mut base = ctx.current_base_uri.clone();
    let mut draft = ctx.draft;
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
        if draft.supports_adjacent_validation() && obj.len() > 1 {
            break;
        }
        let Ok(resolved) = config.registry.resolver((*base).clone()).lookup(reference) else {
            break;
        };
        base = resolved.resolver().base_uri().clone();
        (current, _, draft) = resolved.into_inner();
    }
    current
}

/// Resolve a reference using the Registry.
pub(crate) fn resolve_ref<'cfg, E: ValueEmitter>(
    ctx: &mut CompileContext<'cfg, E>,
    reference: &str,
) -> Result<ResolvedRef<'cfg>, String> {
    let base_uri = ctx.current_base_uri.clone();
    // Copy the `&'cfg` out so the lookup result is not tied to the `ctx` borrow.
    let config = ctx.config;

    let resolver = config.registry.resolver((*base_uri).clone());
    let target = resolver
        .lookup_target(reference)
        .map_err(|e| format!("Failed to resolve {reference}: {e}"))?;
    let resolved = resolver.enter(&target);
    let location_key = target_location(ctx, reference, target.document(), &resolved);
    let resolved_base_uri = resolved.resolver().base_uri().clone();
    let (contents, _, draft) = resolved.into_inner();

    Ok(ResolvedRef {
        schema: contents,
        location: location_key,
        base_uri: resolved_base_uri,
        draft,
    })
}

/// The location of the schema `reference` resolves to: the document it names and the JSON Pointer
/// from that document's root, which every way of naming one schema in that document shares. The
/// pointer counts from that document even when it enters a resource embedded in it, as the
/// locations of errors below the target do.
fn target_location<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    reference: &str,
    document: &Uri<String>,
    resolved: &Resolved<'_>,
) -> String {
    let fragment = reference
        .split_once('#')
        .map_or("", |(_, fragment)| fragment);
    let pointer = if !fragment.starts_with('/') && !fragment.is_empty() {
        let root = resolved
            .resolver()
            .lookup("")
            .expect("an anchor lives in the document it is looked up in");
        Cow::Owned(anchor_pointer(ctx, root.contents(), resolved.contents()))
    } else if fragment.contains('%') {
        percent_encoding::percent_decode_str(fragment).decode_utf8_lossy()
    } else {
        Cow::Borrowed(fragment)
    };
    let document = document.as_str();
    if pointer.is_empty() {
        return document.to_owned();
    }
    let mut location = String::with_capacity(document.len() + 1 + pointer.len());
    location.push_str(document);
    location.push('#');
    // `location_pointer` takes the fragment after the last `#` and percent-decodes it back into
    // the pointer.
    if pointer.contains('%') || pointer.contains('#') {
        location.push_str(&pointer.replace('%', "%25").replace('#', "%23"));
    } else {
        location.push_str(&pointer);
    }
    location
}

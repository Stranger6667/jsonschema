use crate::codegen::emit::ValueEmitter;
use std::{borrow::Cow, sync::Arc};

use referencing::{unescape_segment, Registry, Resolved, Uri};
use serde_json::Value;

use super::{draft::DraftExt, helpers::anchor_pointer, CompileContext};
use crate::context::Dialect;

pub(crate) struct ResolvedRef<'cfg> {
    pub(crate) schema: &'cfg Value,
    pub(crate) location: String,
    /// Length of the prefix of the location's JSON Pointer before the resource the schema sits in
    /// starts, where the pointer enters a resource embedded in the document it counts from.
    pub(crate) resource_start: usize,
    /// Base URI of the resolved schema (for resolving nested references)
    pub(crate) base_uri: Arc<Uri<String>>,
    /// The dialect it is written in: its resource's, or the one a `$schema` on the way to it names.
    pub(crate) dialect: Dialect,
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
    // Bounded hop count: shape analysis is best-effort, so give up on longer (or
    // cyclic) $ref chains instead of tracking visited locations. 8 hops covers
    // realistic hand-written indirection.
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
    let fragment = reference
        .split_once('#')
        .map_or("", |(_, fragment)| fragment);
    // An anchor names a schema of the resource it sits in, by its pointer from that resource's
    // root.
    let (pointer, resource_start) = if !fragment.starts_with('/') && !fragment.is_empty() {
        let root = resolved
            .resolver()
            .lookup("")
            .expect("an anchor lives in the document it is looked up in");
        let pointer = anchor_pointer(ctx, root.contents(), resolved.contents());
        (Cow::Owned(pointer), 0)
    } else {
        let pointer = if fragment.contains('%') {
            percent_encoding::percent_decode_str(fragment).decode_utf8_lossy()
        } else {
            Cow::Borrowed(fragment)
        };
        let resource_start =
            resource_start(&config.registry, target.document(), &pointer, &resolved);
        (pointer, resource_start)
    };
    let location_key = target_location(target.document(), &pointer);
    let resolved_base_uri = resolved.resolver().base_uri().clone();
    let (contents, resolver, draft) = resolved.into_inner();
    let vocabularies = resolver.find_vocabularies(draft, contents);

    Ok(ResolvedRef {
        schema: contents,
        location: location_key,
        resource_start,
        base_uri: resolved_base_uri,
        dialect: Dialect {
            draft,
            vocabularies,
        },
    })
}

/// Length of the prefix of `pointer`, from the root of `document`, before the resource `resolved`
/// sits in starts: the pointer may pass through resources embedded in `document`, and keywords
/// are located from the root of the innermost one.
fn resource_start(
    registry: &Registry<'_>,
    document: &Uri<String>,
    pointer: &str,
    resolved: &Resolved<'_>,
) -> usize {
    // A target in the document's own resource: that resource starts at the root.
    if resolved.resolver().base_uri().as_str() == document.as_str() {
        return 0;
    }
    let resource = resolved
        .resolver()
        .lookup("")
        .expect("a resource's root resolves")
        .contents();
    let mut node = registry
        .resolver(document.clone())
        .lookup("")
        .expect("the document a reference names resolves")
        .contents();
    let mut offset = 0;
    let mut start = 0;
    for segment in pointer.split('/').skip(1) {
        offset += 1 + segment.len();
        node = match node {
            Value::Array(_) => &node[segment.parse::<usize>().expect("the pointer resolved")],
            _ => &node[unescape_segment(segment).as_ref()],
        };
        if std::ptr::eq(node, resource) {
            start = offset;
        }
    }
    start
}

/// The location of the schema `reference` resolves to: the document it names and the JSON Pointer
/// from that document's root, which every way of naming one schema in that document shares. The
/// pointer counts from that document even when it enters a resource embedded in it, as the schema
/// paths of errors below the target do.
fn target_location(document: &Uri<String>, pointer: &str) -> String {
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
        location.push_str(pointer);
    }
    location
}

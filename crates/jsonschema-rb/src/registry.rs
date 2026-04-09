use std::sync::Arc;

use magnus::{
    function,
    gc::{register_address, unregister_address},
    method,
    prelude::*,
    scan_args::{get_kwargs, scan_args},
    value::Opaque,
    DataTypeFunctions, Error, RArray, RModule, Ruby, TryConvert, Value,
};

use crate::{
    options::parse_draft_symbol,
    retriever::make_retriever,
    ser::{to_value, value_to_ruby},
};

struct RetrieverBuildRootGuard {
    // Keep roots in a heap allocation so addresses passed to Ruby GC are stable
    // even if the guard value itself is moved.
    roots: Vec<Value>,
}

impl RetrieverBuildRootGuard {
    fn new(root: Option<Value>) -> Self {
        let mut roots = Vec::new();
        if let Some(value) = root {
            roots.push(value);
        }
        for value in &roots {
            register_address(value);
        }
        Self { roots }
    }
}

impl Drop for RetrieverBuildRootGuard {
    fn drop(&mut self) {
        for value in &self.roots {
            unregister_address(value);
        }
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Registry", free_immediately, size, mark)]
pub struct Registry {
    pub inner: Arc<jsonschema::Registry<'static>>,
    retriever_root: Option<Opaque<Value>>,
}

impl DataTypeFunctions for Registry {
    fn mark(&self, marker: &magnus::gc::Marker) {
        if let Some(root) = self.retriever_root {
            marker.mark(root);
        }
    }
}

impl TryConvert for Registry {
    fn try_convert(val: Value) -> Result<Self, Error> {
        let typed: &Registry = TryConvert::try_convert(val)?;
        Ok(Registry {
            inner: Arc::clone(&typed.inner),
            retriever_root: typed.retriever_root,
        })
    }
}

struct ResolverData {
    registry: Arc<jsonschema::Registry<'static>>,
    retriever_root: Option<Opaque<Value>>,
    base_uri: jsonschema::Uri<String>,
    dynamic_scope: Vec<jsonschema::Uri<String>>,
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Resolver", free_immediately, size, mark)]
pub struct Resolver(ResolverData);

impl DataTypeFunctions for Resolver {
    fn mark(&self, marker: &magnus::gc::Marker) {
        if let Some(root) = self.0.retriever_root {
            marker.mark(root);
        }
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Resolved", free_immediately, size, mark)]
pub struct Resolved {
    contents: Opaque<Value>,
    resolver: ResolverData,
    draft: u8,
}

impl DataTypeFunctions for Resolved {
    fn mark(&self, marker: &magnus::gc::Marker) {
        marker.mark(self.contents);
        if let Some(root) = self.resolver.retriever_root {
            marker.mark(root);
        }
    }
}

fn parse_uri(ruby: &Ruby, uri: &str) -> Result<jsonschema::Uri<String>, Error> {
    jsonschema::uri::from_str(uri)
        .map_err(|e| Error::new(ruby.exception_arg_error(), format!("{e}")))
}

fn draft_to_u8(draft: jsonschema::Draft) -> u8 {
    match draft {
        jsonschema::Draft::Draft4 => 4,
        jsonschema::Draft::Draft6 => 6,
        jsonschema::Draft::Draft7 => 7,
        jsonschema::Draft::Draft201909 => 19,
        _ => 20,
    }
}

impl Registry {
    fn new_impl(ruby: &Ruby, args: &[Value]) -> Result<Self, Error> {
        let parsed_args = scan_args::<(RArray,), (), (), (), _, ()>(args)?;
        let (resources,) = parsed_args.required;
        #[allow(clippy::type_complexity)]
        let kw: magnus::scan_args::KwArgs<(), (Option<Option<Value>>, Option<Value>), ()> =
            get_kwargs(parsed_args.keywords, &[], &["draft", "retriever"])?;
        let draft_val = kw.optional.0.flatten();
        let retriever_val = kw.optional.1;

        let mut builder = jsonschema::Registry::new();
        let mut retriever_root = None;
        let mut retriever_build_root = None;

        if let Some(val) = draft_val {
            let draft = parse_draft_symbol(ruby, val)?;
            builder = builder.draft(draft);
        }

        if let Some(val) = retriever_val {
            if let Some(ret) = make_retriever(ruby, val)? {
                builder = builder.retriever(ret);
                retriever_root = Some(Opaque::from(val));
                retriever_build_root = Some(val);
            }
        }

        // Keep the retriever proc GC-rooted for the entire build, because `build`
        // may call into retriever callbacks while traversing referenced resources.
        let _retriever_build_guard = RetrieverBuildRootGuard::new(retriever_build_root);
        for item in resources {
            let pair: RArray = TryConvert::try_convert(item)?;
            if pair.len() != 2 {
                return Err(Error::new(
                    ruby.exception_arg_error(),
                    "Each resource must be a [uri, schema] pair",
                ));
            }
            let uri: String = pair.entry(0)?;
            let schema_val: Value = pair.entry(1)?;
            let schema = to_value(ruby, schema_val)?;
            builder = builder
                .add(uri, schema)
                .map_err(|e| Error::new(ruby.exception_arg_error(), format!("{e}")))?;
        }
        let registry = builder
            .prepare()
            .map_err(|e| Error::new(ruby.exception_arg_error(), format!("{e}")))?;

        Ok(Registry {
            inner: Arc::new(registry),
            retriever_root,
        })
    }

    fn inspect(&self) -> String {
        "#<JSONSchema::Registry>".to_string()
    }

    pub(crate) fn retriever_value(&self, ruby: &Ruby) -> Option<Value> {
        self.retriever_root.map(|root| ruby.get_inner(root))
    }

    #[allow(clippy::needless_pass_by_value)]
    fn resolver(ruby: &Ruby, rb_self: &Registry, base_uri: String) -> Result<Resolver, Error> {
        Ok(Resolver(ResolverData {
            registry: Arc::clone(&rb_self.inner),
            retriever_root: rb_self.retriever_root,
            base_uri: parse_uri(ruby, &base_uri)?,
            dynamic_scope: Vec::new(),
        }))
    }
}

impl Resolver {
    fn base_uri(&self) -> String {
        self.0.base_uri.as_str().to_string()
    }

    fn dynamic_scope(ruby: &Ruby, rb_self: &Resolver) -> Result<Value, Error> {
        let arr = ruby.ary_new_capa(rb_self.0.dynamic_scope.len());
        for scope in &rb_self.0.dynamic_scope {
            arr.push(ruby.into_value(scope.as_str()))?;
        }
        Ok(arr.as_value())
    }

    #[allow(clippy::needless_pass_by_value)]
    fn lookup(ruby: &Ruby, rb_self: &Resolver, reference: String) -> Result<Resolved, Error> {
        let resolver = if rb_self.0.dynamic_scope.is_empty() {
            rb_self
                .0
                .registry
                .as_ref()
                .resolver(rb_self.0.base_uri.clone())
        } else {
            let oldest_uri = rb_self
                .0
                .dynamic_scope
                .last()
                .expect("dynamic_scope is not empty")
                .clone();
            let mut resolver = rb_self.0.registry.as_ref().resolver(oldest_uri);
            for next_uri in rb_self
                .0
                .dynamic_scope
                .iter()
                .rev()
                .skip(1)
                .chain(std::iter::once(&rb_self.0.base_uri))
            {
                let next_resolved = resolver
                    .lookup(next_uri.as_str())
                    .map_err(|e| crate::referencing_error(ruby, e.to_string()))?;
                resolver = next_resolved.into_inner().1;
            }
            resolver
        };

        let resolved = resolver
            .lookup(&reference)
            .map_err(|e| crate::referencing_error(ruby, e.to_string()))?;
        let (contents, resolver, draft) = resolved.into_inner();

        let contents_val = value_to_ruby(ruby, contents)?;

        Ok(Resolved {
            contents: Opaque::from(contents_val),
            resolver: ResolverData {
                registry: Arc::clone(&rb_self.0.registry),
                retriever_root: rb_self.0.retriever_root,
                base_uri: resolver.base_uri().as_ref().clone(),
                dynamic_scope: resolver.dynamic_scope().iter().cloned().collect(),
            },
            draft: draft_to_u8(draft),
        })
    }
}

impl Resolved {
    fn contents(ruby: &Ruby, rb_self: &Resolved) -> Value {
        ruby.get_inner(rb_self.contents)
    }

    fn resolver(_ruby: &Ruby, rb_self: &Resolved) -> Resolver {
        Resolver(ResolverData {
            registry: Arc::clone(&rb_self.resolver.registry),
            retriever_root: rb_self.resolver.retriever_root,
            base_uri: rb_self.resolver.base_uri.clone(),
            dynamic_scope: rb_self.resolver.dynamic_scope.clone(),
        })
    }

    fn draft(&self) -> u8 {
        self.draft
    }
}

pub fn define_class(ruby: &Ruby, module: &RModule) -> Result<(), Error> {
    let class = module.define_class("Registry", ruby.class_object())?;
    class.define_singleton_method("new", function!(Registry::new_impl, -1))?;
    class.define_method("inspect", method!(Registry::inspect, 0))?;
    class.define_method("resolver", method!(Registry::resolver, 1))?;

    let resolver_class = module.define_class("Resolver", ruby.class_object())?;
    resolver_class.define_method("base_uri", method!(Resolver::base_uri, 0))?;
    resolver_class.define_method("dynamic_scope", method!(Resolver::dynamic_scope, 0))?;
    resolver_class.define_method("lookup", method!(Resolver::lookup, 1))?;

    let resolved_class = module.define_class("Resolved", ruby.class_object())?;
    resolved_class.define_method("contents", method!(Resolved::contents, 0))?;
    resolved_class.define_method("resolver", method!(Resolved::resolver, 0))?;
    resolved_class.define_method("draft", method!(Resolved::draft, 0))?;

    Ok(())
}

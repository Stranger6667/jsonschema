use magnus::{
    function,
    gc::{register_address, unregister_address},
    method,
    prelude::*,
    scan_args::{get_kwargs, scan_args},
    value::Opaque,
    DataTypeFunctions, Error, RArray, RModule, Ruby, TryConvert, Value,
};

use crate::{options::parse_draft_symbol, retriever::make_retriever, ser::to_value};

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
    pub inner: jsonschema::Registry,
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
            inner: typed.inner.clone(),
            retriever_root: typed.retriever_root,
        })
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

        let mut builder = jsonschema::Registry::options();
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

        let pairs: Vec<(String, jsonschema::Resource)> = resources
            .into_iter()
            .map(|item| {
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
                let resource = jsonschema::Resource::from_contents(schema);
                Ok((uri, resource))
            })
            .collect::<Result<Vec<_>, Error>>()?;

        // Keep the retriever proc GC-rooted for the entire build, because `build`
        // may call into retriever callbacks while traversing referenced resources.
        let _retriever_build_guard = RetrieverBuildRootGuard::new(retriever_build_root);
        let registry = builder
            .build(pairs)
            .map_err(|e| Error::new(ruby.exception_arg_error(), e.to_string()))?;

        Ok(Registry {
            inner: registry,
            retriever_root,
        })
    }

    fn inspect(&self) -> String {
        "#<JSONSchema::Registry>".to_string()
    }

    pub(crate) fn retriever_value(&self, ruby: &Ruby) -> Option<Value> {
        self.retriever_root.map(|root| ruby.get_inner(root))
    }
}

pub fn define_class(ruby: &Ruby, module: &RModule) -> Result<(), Error> {
    let class = module.define_class("Registry", ruby.class_object())?;
    class.define_singleton_method("new", function!(Registry::new_impl, -1))?;
    class.define_method("inspect", method!(Registry::inspect, 0))?;

    Ok(())
}

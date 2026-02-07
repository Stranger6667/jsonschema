use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use magnus::{
    block::Proc,
    function,
    gc::{register_address, unregister_address},
    method,
    prelude::*,
    scan_args::{get_kwargs, scan_args, KwArgs},
    value::Opaque,
    Error, RHash, RModule, Ruby, TryConvert, Value,
};

use crate::{
    registry::Registry,
    retriever::{make_retriever, RubyRetriever},
    ser::{map_to_ruby, value_to_ruby},
    static_id::{define_rb_intern, StaticId},
    LAST_CALLBACK_ERROR,
};

// Base kwarg names
define_rb_intern!(static KW_DRAFT: "draft");
define_rb_intern!(static KW_VALIDATE_FORMATS: "validate_formats");
define_rb_intern!(static KW_IGNORE_UNKNOWN_FORMATS: "ignore_unknown_formats");
define_rb_intern!(static KW_MASK: "mask");
define_rb_intern!(static KW_BASE_URI: "base_uri");
define_rb_intern!(static KW_RETRIEVER: "retriever");
define_rb_intern!(static KW_FORMATS: "formats");
define_rb_intern!(static KW_KEYWORDS: "keywords");
define_rb_intern!(pub(crate) static KW_REGISTRY: "registry");
// Extra kwarg names (extracted before get_kwargs)
define_rb_intern!(static KW_PATTERN_OPTIONS: "pattern_options");
define_rb_intern!(static KW_EMAIL_OPTIONS: "email_options");
define_rb_intern!(static KW_HTTP_OPTIONS: "http_options");
// EmailOptions kwargs
define_rb_intern!(static KW_REQUIRE_TLD: "require_tld");
define_rb_intern!(static KW_ALLOW_DOMAIN_LITERAL: "allow_domain_literal");
define_rb_intern!(static KW_ALLOW_DISPLAY_TEXT: "allow_display_text");
define_rb_intern!(static KW_MINIMUM_SUB_DOMAINS: "minimum_sub_domains");
// RegexOptions / FancyRegexOptions kwargs
define_rb_intern!(static KW_SIZE_LIMIT: "size_limit");
define_rb_intern!(static KW_DFA_SIZE_LIMIT: "dfa_size_limit");
define_rb_intern!(static KW_BACKTRACK_LIMIT: "backtrack_limit");
// HttpOptions kwargs
define_rb_intern!(static KW_TIMEOUT: "timeout");
define_rb_intern!(static KW_CONNECT_TIMEOUT: "connect_timeout");
define_rb_intern!(static KW_TLS_VERIFY: "tls_verify");
define_rb_intern!(static KW_CA_CERT: "ca_cert");
// Method symbols for respond_to? / method_defined? checks
define_rb_intern!(static SYM_CALL: "call");
define_rb_intern!(static SYM_NEW: "new");
define_rb_intern!(static SYM_VALIDATE: "validate");

pub struct ParsedOptions {
    pub mask: Option<String>,
    pub options: jsonschema::ValidationOptions,
    pub retriever: Option<RubyRetriever>,
    // Runtime callbacks invoked during `validator.*` calls (formats / custom keywords).
    // Retriever callbacks are used at build time and do not affect GVL behavior at runtime.
    pub has_ruby_callbacks: bool,
    pub callback_roots: CallbackRoots,
    pub compilation_roots: CompilationRootsRef,
}

/// Ruby callbacks (format checkers, custom keyword instances, retrievers) are stored
/// inside the Rust `jsonschema::Validator` as trait objects. Ruby's GC cannot see
/// these references — it only scans its own heap — so without explicit protection
/// the GC would collect the callbacks while the validator still holds them, causing
/// use-after-free crashes.
///
/// Two complementary collections keep every callback alive:
///
/// * **`CallbackRoots`** — held by the `Validator` wrapper and marked during Ruby's
///   GC mark phase (`DataTypeFunctions::mark`). This is the standard Magnus/Ruby
///   mechanism for preventing collection of objects referenced by native extensions.
///   For one-off validation functions (module-level `valid?`, `validate!`, etc.),
///   where no persistent `Validator` exists, a [`CallbackRootGuard`](crate::CallbackRootGuard)
///   temporarily registers the same roots via `register_address` for the duration
///   of the call.
///
/// * **`CompilationRoots`** — registered via `register_address` immediately when
///   added, so callbacks are protected during schema compilation (before the
///   `Validator` wrapper and its mark function exist). Unregistered on drop.
///
/// Both collections hold the **same** Ruby objects; they differ only in *when* and
/// *how* they protect them from the GC.
pub type CallbackRoots = Arc<Mutex<Vec<Opaque<Value>>>>;
pub type CompilationRootsRef = Arc<CompilationRoots>;

#[derive(Default)]
pub struct CompilationRoots {
    // Values are pinned so their addresses stay stable after GC registration.
    roots: Mutex<Vec<Pin<Box<Opaque<Value>>>>>,
}

impl CompilationRoots {
    fn add(&self, value: Opaque<Value>) -> Result<(), ()> {
        let mut roots = self.roots.lock().map_err(|_| ())?;
        let pinned = Box::pin(value);
        register_address(pinned.as_ref().get_ref());
        roots.push(pinned);
        Ok(())
    }
}

impl Drop for CompilationRoots {
    fn drop(&mut self) {
        let roots = match self.roots.get_mut() {
            Ok(roots) => roots,
            Err(poisoned) => poisoned.into_inner(),
        };
        for root in roots.drain(..) {
            unregister_address(root.as_ref().get_ref());
        }
    }
}

fn base_option_ids() -> [StaticId; 9] {
    [
        *KW_DRAFT,
        *KW_VALIDATE_FORMATS,
        *KW_IGNORE_UNKNOWN_FORMATS,
        *KW_MASK,
        *KW_BASE_URI,
        *KW_RETRIEVER,
        *KW_FORMATS,
        *KW_KEYWORDS,
        *KW_REGISTRY,
    ]
}
fn base_option_ids_no_mask() -> [StaticId; 8] {
    [
        *KW_DRAFT,
        *KW_VALIDATE_FORMATS,
        *KW_IGNORE_UNKNOWN_FORMATS,
        *KW_BASE_URI,
        *KW_RETRIEVER,
        *KW_FORMATS,
        *KW_KEYWORDS,
        *KW_REGISTRY,
    ]
}

type BaseKwargs = (
    Option<Value>,
    Option<bool>,
    Option<bool>,
    Option<String>,
    Option<String>,
    Option<Value>,
    Option<RHash>,
    Option<RHash>,
    Option<Value>,
);
type BaseKwargsNoMask = (
    Option<Value>,
    Option<bool>,
    Option<bool>,
    Option<String>,
    Option<Value>,
    Option<RHash>,
    Option<RHash>,
    Option<Value>,
);
type BaseKwargsNoDraft = (
    Option<bool>,
    Option<bool>,
    Option<String>,
    Option<String>,
    Option<Value>,
    Option<RHash>,
    Option<RHash>,
    Option<Value>,
);

fn base_option_ids_no_draft() -> [StaticId; 8] {
    [
        *KW_VALIDATE_FORMATS,
        *KW_IGNORE_UNKNOWN_FORMATS,
        *KW_MASK,
        *KW_BASE_URI,
        *KW_RETRIEVER,
        *KW_FORMATS,
        *KW_KEYWORDS,
        *KW_REGISTRY,
    ]
}

pub fn parse_draft_symbol(ruby: &Ruby, val: Value) -> Result<jsonschema::Draft, Error> {
    let sym: magnus::Symbol = TryConvert::try_convert(val).map_err(|_| {
        Error::new(
            ruby.exception_type_error(),
            "draft must be a Symbol (e.g. :draft7)",
        )
    })?;
    let name = sym.name().map_err(|_| {
        Error::new(
            ruby.exception_arg_error(),
            "Failed to read draft symbol name",
        )
    })?;
    match name.as_ref() {
        "draft4" => Ok(jsonschema::Draft::Draft4),
        "draft6" => Ok(jsonschema::Draft::Draft6),
        "draft7" => Ok(jsonschema::Draft::Draft7),
        "draft201909" => Ok(jsonschema::Draft::Draft201909),
        "draft202012" => Ok(jsonschema::Draft::Draft202012),
        _ => Err(Error::new(
            ruby.exception_arg_error(),
            format!(
                "Unknown draft: :{name}. Valid drafts: :draft4, :draft6, :draft7, :draft201909, :draft202012"
            ),
        )),
    }
}

pub struct ExtractedKwargs {
    pub base: BaseKwargs,
    pub pattern_options: Option<Value>,
    pub email_options: Option<Value>,
    pub http_options: Option<Value>,
}

pub fn extract_kwargs(_ruby: &Ruby, kw: RHash) -> Result<ExtractedKwargs, Error> {
    let pattern_options = extract_and_delete(&kw, *KW_PATTERN_OPTIONS)?;
    let email_options = extract_and_delete(&kw, *KW_EMAIL_OPTIONS)?;
    let http_options = extract_and_delete(&kw, *KW_HTTP_OPTIONS)?;

    let ids = base_option_ids();
    let base_kw: KwArgs<(), BaseKwargs, ()> = get_kwargs(kw, &[], &ids)?;

    Ok(ExtractedKwargs {
        base: base_kw.optional,
        pattern_options,
        email_options,
        http_options,
    })
}

pub fn extract_evaluate_kwargs(_ruby: &Ruby, kw: RHash) -> Result<ExtractedKwargs, Error> {
    let pattern_options = extract_and_delete(&kw, *KW_PATTERN_OPTIONS)?;
    let email_options = extract_and_delete(&kw, *KW_EMAIL_OPTIONS)?;
    let http_options = extract_and_delete(&kw, *KW_HTTP_OPTIONS)?;

    let ids = base_option_ids_no_mask();
    let base_kw: KwArgs<(), BaseKwargsNoMask, ()> = get_kwargs(kw, &[], &ids)?;
    let (
        draft,
        validate_formats,
        ignore_unknown_formats,
        base_uri,
        retriever,
        formats,
        keywords,
        registry,
    ) = base_kw.optional;

    Ok(ExtractedKwargs {
        base: (
            draft,
            validate_formats,
            ignore_unknown_formats,
            None,
            base_uri,
            retriever,
            formats,
            keywords,
            registry,
        ),
        pattern_options,
        email_options,
        http_options,
    })
}

pub fn extract_kwargs_no_draft(_ruby: &Ruby, kw: RHash) -> Result<ExtractedKwargs, Error> {
    let pattern_options = extract_and_delete(&kw, *KW_PATTERN_OPTIONS)?;
    let email_options = extract_and_delete(&kw, *KW_EMAIL_OPTIONS)?;
    let http_options = extract_and_delete(&kw, *KW_HTTP_OPTIONS)?;

    let ids = base_option_ids_no_draft();
    let base_kw: KwArgs<(), BaseKwargsNoDraft, ()> = get_kwargs(kw, &[], &ids)?;
    let (
        validate_formats,
        ignore_unknown_formats,
        mask,
        base_uri,
        retriever,
        formats,
        keywords,
        registry,
    ) = base_kw.optional;

    Ok(ExtractedKwargs {
        base: (
            None,
            validate_formats,
            ignore_unknown_formats,
            mask,
            base_uri,
            retriever,
            formats,
            keywords,
            registry,
        ),
        pattern_options,
        email_options,
        http_options,
    })
}

/// Extract a key from a Ruby Hash and remove it, returning None if not present or nil.
fn extract_and_delete(hash: &RHash, key: StaticId) -> Result<Option<Value>, Error> {
    let val: Option<Value> = hash.delete(key.to_symbol())?;
    match val {
        Some(v) if v.is_nil() => Ok(None),
        other => Ok(other),
    }
}

fn timeout_duration(ruby: &Ruby, field: &str, value: f64) -> Result<Duration, Error> {
    if !value.is_finite() || value < 0.0 {
        return Err(Error::new(
            ruby.exception_arg_error(),
            format!("http_options.{field} must be a finite non-negative number"),
        ));
    }
    Duration::try_from_secs_f64(value).map_err(|_| {
        Error::new(
            ruby.exception_arg_error(),
            format!("http_options.{field} is too large"),
        )
    })
}

/// Wrapper for a Ruby format checker proc that can be called from Rust.
struct RubyFormatChecker {
    proc: Opaque<Proc>,
}

impl RubyFormatChecker {
    fn check(&self, value: &str) -> bool {
        let ruby = Ruby::get().expect("Ruby VM should be initialized");
        let proc = ruby.get_inner(self.proc);
        let result: Result<bool, _> = proc.call((value,));
        match result {
            Ok(v) => v,
            Err(e) => {
                LAST_CALLBACK_ERROR.with(|last| {
                    *last.borrow_mut() = Some(e);
                });
                panic!("Format checker failed")
            }
        }
    }
}

/// Wrapper for a Ruby custom keyword validator factory.
struct RubyKeywordFactory {
    class: Opaque<Value>,
}

/// Wrapper for a Ruby custom keyword validator instance.
struct RubyKeyword {
    instance: Opaque<Value>,
}

impl jsonschema::Keyword for RubyKeyword {
    fn validate<'i>(
        &self,
        instance: &'i serde_json::Value,
    ) -> Result<(), jsonschema::ValidationError<'i>> {
        let ruby = Ruby::get().expect("Ruby VM should be initialized");
        let rb_instance = value_to_ruby(&ruby, instance).map_err(|e| {
            jsonschema::ValidationError::custom(format!("Failed to convert instance to Ruby: {e}"))
        })?;

        let keyword = ruby.get_inner(self.instance);
        let result: Result<Value, _> = keyword.funcall("validate", (rb_instance,));
        match result {
            Ok(_) => Ok(()),
            Err(e) => Err(jsonschema::ValidationError::custom(e.to_string())),
        }
    }

    fn is_valid(&self, instance: &serde_json::Value) -> bool {
        let ruby = Ruby::get().expect("Ruby VM should be initialized");
        let Ok(rb_instance) = value_to_ruby(&ruby, instance) else {
            return false;
        };
        let inst = ruby.get_inner(self.instance);
        let result: Result<Value, _> = inst.funcall("validate", (rb_instance,));
        result.is_ok()
    }
}

#[allow(clippy::too_many_arguments)]
pub fn make_options_from_kwargs(
    ruby: &Ruby,
    draft: Option<jsonschema::Draft>,
    validate_formats: Option<bool>,
    ignore_unknown_formats: Option<bool>,
    mask: Option<String>,
    base_uri: Option<String>,
    retriever_val: Option<Value>,
    formats: Option<RHash>,
    keywords: Option<RHash>,
    registry_val: Option<Value>,
    pattern_options_val: Option<Value>,
    email_options_val: Option<Value>,
    http_options_val: Option<Value>,
) -> Result<ParsedOptions, Error> {
    let mut opts = jsonschema::options();
    let mut retriever = None;
    let retriever_was_provided = retriever_val.is_some();
    let mut has_ruby_callbacks = false;
    let callback_roots = Arc::new(Mutex::new(Vec::new()));
    let compilation_roots = Arc::new(CompilationRoots::default());

    if let Some(draft) = draft {
        opts = opts.with_draft(draft);
    }

    if let Some(validate) = validate_formats {
        opts = opts.should_validate_formats(validate);
    }

    if let Some(ignore) = ignore_unknown_formats {
        opts = opts.should_ignore_unknown_formats(ignore);
    }

    if let Some(uri) = base_uri {
        opts = opts.with_base_uri(uri);
    }

    if let Some(val) = retriever_val {
        if let Some(ret) = make_retriever(ruby, val)? {
            compilation_roots.add(Opaque::from(val)).map_err(|()| {
                Error::new(
                    ruby.exception_runtime_error(),
                    "Compilation callback root storage is poisoned",
                )
            })?;
            {
                let mut roots = callback_roots.lock().map_err(|_| {
                    Error::new(
                        ruby.exception_runtime_error(),
                        "Callback root storage is poisoned",
                    )
                })?;
                roots.push(Opaque::from(val));
            }
            retriever = Some(ret);
        }
    }

    if let Some(val) = registry_val {
        if !val.is_nil() {
            let reg: &Registry = TryConvert::try_convert(val).map_err(|_| {
                Error::new(
                    ruby.exception_type_error(),
                    "registry must be a JSONSchema::Registry instance",
                )
            })?;
            opts = opts.with_registry(reg.inner.clone());

            if !retriever_was_provided && retriever.is_none() {
                if let Some(registry_retriever_value) = reg.retriever_value(ruby) {
                    if let Some(ret) = make_retriever(ruby, registry_retriever_value)? {
                        compilation_roots
                            .add(Opaque::from(registry_retriever_value))
                            .map_err(|()| {
                                Error::new(
                                    ruby.exception_runtime_error(),
                                    "Compilation callback root storage is poisoned",
                                )
                            })?;
                        {
                            let mut roots = callback_roots.lock().map_err(|_| {
                                Error::new(
                                    ruby.exception_runtime_error(),
                                    "Callback root storage is poisoned",
                                )
                            })?;
                            roots.push(Opaque::from(registry_retriever_value));
                        }
                        retriever = Some(ret);
                    }
                }
            }
        }
    }

    if let Some(formats_hash) = formats {
        for item in formats_hash.enumeratorize("each", ()) {
            has_ruby_callbacks = true;
            let pair: magnus::RArray = magnus::TryConvert::try_convert(item?)?;
            let name: String = pair.entry(0)?;
            let callback: Value = pair.entry(1)?;

            let responds_to_call: bool =
                callback.funcall("respond_to?", (SYM_CALL.to_symbol(),))?;
            if !responds_to_call {
                return Err(Error::new(
                    ruby.exception_type_error(),
                    format!("Format checker for '{name}' must be a callable (Proc or Lambda)"),
                ));
            }

            let proc = Proc::from_value(callback).ok_or_else(|| {
                Error::new(
                    ruby.exception_type_error(),
                    format!("Failed to convert format checker '{name}' to Proc"),
                )
            })?;

            compilation_roots
                .add(Opaque::from(callback))
                .map_err(|()| {
                    Error::new(
                        ruby.exception_runtime_error(),
                        "Compilation callback root storage is poisoned",
                    )
                })?;
            {
                // During configuration, fail fast on poisoned state so we don't create
                // validators with partially captured callback roots.
                let mut roots = callback_roots.lock().map_err(|_| {
                    Error::new(
                        ruby.exception_runtime_error(),
                        "Callback root storage is poisoned",
                    )
                })?;
                roots.push(Opaque::from(callback));
            }

            let checker = RubyFormatChecker {
                proc: Opaque::from(proc),
            };

            opts = opts.with_format(name, move |value: &str| checker.check(value));
        }
    }

    if let Some(keywords_hash) = keywords {
        for item in keywords_hash.enumeratorize("each", ()) {
            has_ruby_callbacks = true;
            let pair: magnus::RArray = magnus::TryConvert::try_convert(item?)?;
            let name: String = pair.entry(0)?;
            let callback: Value = pair.entry(1)?;

            let responds_to_new: bool = callback.funcall("respond_to?", (SYM_NEW.to_symbol(),))?;
            if !responds_to_new {
                return Err(Error::new(
                    ruby.exception_type_error(),
                    format!(
                        "Keyword validator for '{name}' must be a class with 'new' and 'validate' methods"
                    ),
                ));
            }

            let has_validate: bool =
                callback.funcall("method_defined?", (SYM_VALIDATE.to_symbol(),))?;
            if !has_validate {
                return Err(Error::new(
                    ruby.exception_type_error(),
                    format!(
                        "Keyword validator for '{name}' must define a 'validate' instance method"
                    ),
                ));
            }

            let callback_wrapper = Arc::new(RubyKeywordFactory {
                class: Opaque::from(callback),
            });
            compilation_roots
                .add(Opaque::from(callback))
                .map_err(|()| {
                    Error::new(
                        ruby.exception_runtime_error(),
                        "Compilation callback root storage is poisoned",
                    )
                })?;
            {
                let mut roots = callback_roots.lock().map_err(|_| {
                    Error::new(
                        ruby.exception_runtime_error(),
                        "Callback root storage is poisoned",
                    )
                })?;
                roots.push(Opaque::from(callback));
            }
            let callback_roots_for_keyword = Arc::clone(&callback_roots);
            let compilation_roots_for_keyword = Arc::clone(&compilation_roots);
            let name_for_error = name.clone();

            opts = opts.with_keyword(
                name,
                move |parent: &serde_json::Map<String, serde_json::Value>,
                      value: &serde_json::Value,
                      path: jsonschema::paths::Location| {
                    let inner_ruby = Ruby::get().expect("Ruby VM should be initialized");
                    let name_err = name_for_error.clone();
                    let factory = callback_wrapper.clone();

                    // Convert parent schema map to Ruby hash directly
                    let rb_schema = map_to_ruby(&inner_ruby, parent).map_err(|e| {
                        jsonschema::ValidationError::custom(format!(
                            "Failed to convert schema to Ruby: {e}"
                        ))
                    })?;

                    // Convert keyword value to Ruby
                    let rb_value = value_to_ruby(&inner_ruby, value).map_err(|e| {
                        jsonschema::ValidationError::custom(format!(
                            "Failed to convert keyword value to Ruby: {e}"
                        ))
                    })?;

                    // Convert path to Ruby array
                    let rb_path =
                        inner_ruby.ary_from_iter(path.iter().map(|segment| match segment {
                            jsonschema::paths::LocationSegment::Property(p) => {
                                inner_ruby.into_value(p.as_ref())
                            }
                            jsonschema::paths::LocationSegment::Index(i) => {
                                inner_ruby.into_value(i)
                            }
                        }));

                    // Instantiate the keyword validator class with (parent_schema, value, path)
                    let class = inner_ruby.get_inner(factory.class);
                    let instance: Result<Value, _> =
                        class.funcall("new", (rb_schema, rb_value, rb_path));

                    match instance {
                        Ok(inst) => {
                            let opaque_inst = Opaque::from(inst);
                            compilation_roots_for_keyword
                                .add(opaque_inst)
                                .map_err(|()| {
                                    jsonschema::ValidationError::custom(
                                        "Compilation callback root storage is poisoned",
                                    )
                                })?;
                            let mut roots = callback_roots_for_keyword.lock().map_err(|_| {
                                jsonschema::ValidationError::custom(
                                    "Callback root storage is poisoned",
                                )
                            })?;
                            roots.push(opaque_inst);
                            Ok(Box::new(RubyKeyword {
                                instance: opaque_inst,
                            })
                                as Box<dyn jsonschema::Keyword>)
                        }
                        Err(e) => Err(jsonschema::ValidationError::custom(format!(
                            "Failed to instantiate keyword class '{name_err}': {e}"
                        ))),
                    }
                },
            );
        }
    }

    if let Some(val) = pattern_options_val {
        if let Ok(fancy_opts) = <&FancyRegexOptions>::try_convert(val) {
            let mut po = jsonschema::PatternOptions::fancy_regex();
            if let Some(limit) = fancy_opts.backtrack_limit {
                po = po.backtrack_limit(limit);
            }
            if let Some(limit) = fancy_opts.size_limit {
                po = po.size_limit(limit);
            }
            if let Some(limit) = fancy_opts.dfa_size_limit {
                po = po.dfa_size_limit(limit);
            }
            opts = opts.with_pattern_options(po);
        } else if let Ok(regex_opts) = <&RegexOptions>::try_convert(val) {
            let mut po = jsonschema::PatternOptions::regex();
            if let Some(limit) = regex_opts.size_limit {
                po = po.size_limit(limit);
            }
            if let Some(limit) = regex_opts.dfa_size_limit {
                po = po.dfa_size_limit(limit);
            }
            opts = opts.with_pattern_options(po);
        } else {
            return Err(Error::new(
                ruby.exception_type_error(),
                "pattern_options must be a RegexOptions or FancyRegexOptions instance",
            ));
        }
    }

    if let Some(val) = email_options_val {
        let eopts: &EmailOptions = magnus::TryConvert::try_convert(val).map_err(|_| {
            Error::new(
                ruby.exception_type_error(),
                "email_options must be an EmailOptions instance",
            )
        })?;
        let mut email_opts = jsonschema::EmailOptions::default();
        if eopts.require_tld {
            email_opts = email_opts.with_required_tld();
        }
        if eopts.allow_domain_literal {
            email_opts = email_opts.with_domain_literal();
        } else {
            email_opts = email_opts.without_domain_literal();
        }
        if eopts.allow_display_text {
            email_opts = email_opts.with_display_text();
        } else {
            email_opts = email_opts.without_display_text();
        }
        if let Some(min) = eopts.minimum_sub_domains {
            email_opts = email_opts.with_minimum_sub_domains(min);
        }
        opts = opts.with_email_options(email_opts);
    }

    if let Some(val) = http_options_val {
        let hopts: &HttpOptions = magnus::TryConvert::try_convert(val).map_err(|_| {
            Error::new(
                ruby.exception_type_error(),
                "http_options must be an HttpOptions instance",
            )
        })?;
        let mut http_opts = jsonschema::HttpOptions::new();
        if let Some(timeout) = hopts.timeout {
            http_opts = http_opts.timeout(timeout_duration(ruby, "timeout", timeout)?);
        }
        if let Some(connect_timeout) = hopts.connect_timeout {
            http_opts = http_opts.connect_timeout(timeout_duration(
                ruby,
                "connect_timeout",
                connect_timeout,
            )?);
        }
        if !hopts.tls_verify {
            http_opts = http_opts.danger_accept_invalid_certs(true);
        }
        if let Some(ref ca_cert) = hopts.ca_cert {
            http_opts = http_opts.add_root_certificate(ca_cert);
        }
        opts = opts
            .with_http_options(&http_opts)
            .map_err(|e| Error::new(ruby.exception_arg_error(), e.to_string()))?;
    }

    Ok(ParsedOptions {
        mask,
        options: opts,
        retriever,
        has_ruby_callbacks,
        callback_roots,
        compilation_roots,
    })
}

#[magnus::wrap(class = "JSONSchema::EmailOptions", free_immediately, size)]
pub struct EmailOptions {
    pub require_tld: bool,
    pub allow_domain_literal: bool,
    pub allow_display_text: bool,
    pub minimum_sub_domains: Option<usize>,
}

impl EmailOptions {
    #[allow(clippy::type_complexity)]
    fn new_impl(args: &[Value]) -> Result<Self, Error> {
        let parsed = scan_args::<(), (), (), (), _, ()>(args)?;
        let ids = [
            *KW_REQUIRE_TLD,
            *KW_ALLOW_DOMAIN_LITERAL,
            *KW_ALLOW_DISPLAY_TEXT,
            *KW_MINIMUM_SUB_DOMAINS,
        ];
        let kw: KwArgs<(), (Option<bool>, Option<bool>, Option<bool>, Option<usize>), ()> =
            get_kwargs(parsed.keywords, &[], &ids)?;
        let (require_tld, allow_domain_literal, allow_display_text, minimum_sub_domains) =
            kw.optional;
        Ok(EmailOptions {
            require_tld: require_tld.unwrap_or(false),
            allow_domain_literal: allow_domain_literal.unwrap_or(true),
            allow_display_text: allow_display_text.unwrap_or(true),
            minimum_sub_domains,
        })
    }

    fn require_tld(&self) -> bool {
        self.require_tld
    }

    fn allow_domain_literal(&self) -> bool {
        self.allow_domain_literal
    }

    fn allow_display_text(&self) -> bool {
        self.allow_display_text
    }

    fn minimum_sub_domains(&self) -> Option<usize> {
        self.minimum_sub_domains
    }

    fn inspect(&self) -> String {
        use std::fmt::Write;
        let mut s = String::from("#<JSONSchema::EmailOptions require_tld=");
        s.push_str(if self.require_tld { "true" } else { "false" });
        s.push_str(", allow_domain_literal=");
        s.push_str(if self.allow_domain_literal {
            "true"
        } else {
            "false"
        });
        s.push_str(", allow_display_text=");
        s.push_str(if self.allow_display_text {
            "true"
        } else {
            "false"
        });
        s.push_str(", minimum_sub_domains=");
        match self.minimum_sub_domains {
            Some(n) => write!(s, "{n}").expect("Failed to write minimum_sub_domains"),
            None => s.push_str("nil"),
        }
        s.push('>');
        s
    }
}

#[magnus::wrap(class = "JSONSchema::RegexOptions", free_immediately, size)]
pub struct RegexOptions {
    pub size_limit: Option<usize>,
    pub dfa_size_limit: Option<usize>,
}

impl RegexOptions {
    fn new_impl(args: &[Value]) -> Result<Self, Error> {
        let parsed = scan_args::<(), (), (), (), _, ()>(args)?;
        let ids = [*KW_SIZE_LIMIT, *KW_DFA_SIZE_LIMIT];
        let kw: KwArgs<(), (Option<usize>, Option<usize>), ()> =
            get_kwargs(parsed.keywords, &[], &ids)?;
        let (size_limit, dfa_size_limit) = kw.optional;
        Ok(RegexOptions {
            size_limit,
            dfa_size_limit,
        })
    }

    fn size_limit(&self) -> Option<usize> {
        self.size_limit
    }

    fn dfa_size_limit(&self) -> Option<usize> {
        self.dfa_size_limit
    }

    fn inspect(&self) -> String {
        use std::fmt::Write;
        let mut s = String::from("#<JSONSchema::RegexOptions size_limit=");
        match self.size_limit {
            Some(n) => write!(s, "{n}").expect("Failed to write size_limit"),
            None => s.push_str("nil"),
        }
        s.push_str(", dfa_size_limit=");
        match self.dfa_size_limit {
            Some(n) => write!(s, "{n}").expect("Failed to write dfa_size_limit"),
            None => s.push_str("nil"),
        }
        s.push('>');
        s
    }
}

#[magnus::wrap(class = "JSONSchema::FancyRegexOptions", free_immediately, size)]
pub struct FancyRegexOptions {
    pub backtrack_limit: Option<usize>,
    pub size_limit: Option<usize>,
    pub dfa_size_limit: Option<usize>,
}

impl FancyRegexOptions {
    #[allow(clippy::type_complexity)]
    fn new_impl(args: &[Value]) -> Result<Self, Error> {
        let parsed = scan_args::<(), (), (), (), _, ()>(args)?;
        let ids = [*KW_BACKTRACK_LIMIT, *KW_SIZE_LIMIT, *KW_DFA_SIZE_LIMIT];
        let kw: KwArgs<(), (Option<usize>, Option<usize>, Option<usize>), ()> =
            get_kwargs(parsed.keywords, &[], &ids)?;
        let (backtrack_limit, size_limit, dfa_size_limit) = kw.optional;
        Ok(FancyRegexOptions {
            backtrack_limit,
            size_limit,
            dfa_size_limit,
        })
    }

    fn backtrack_limit(&self) -> Option<usize> {
        self.backtrack_limit
    }

    fn size_limit(&self) -> Option<usize> {
        self.size_limit
    }

    fn dfa_size_limit(&self) -> Option<usize> {
        self.dfa_size_limit
    }

    fn inspect(&self) -> String {
        use std::fmt::Write;
        let mut s = String::from("#<JSONSchema::FancyRegexOptions backtrack_limit=");
        match self.backtrack_limit {
            Some(n) => write!(s, "{n}").expect("Failed to write backtrack_limit"),
            None => s.push_str("nil"),
        }
        s.push_str(", size_limit=");
        match self.size_limit {
            Some(n) => write!(s, "{n}").expect("Failed to write size_limit"),
            None => s.push_str("nil"),
        }
        s.push_str(", dfa_size_limit=");
        match self.dfa_size_limit {
            Some(n) => write!(s, "{n}").expect("Failed to write dfa_size_limit"),
            None => s.push_str("nil"),
        }
        s.push('>');
        s
    }
}

#[magnus::wrap(class = "JSONSchema::HttpOptions", free_immediately, size)]
pub struct HttpOptions {
    pub timeout: Option<f64>,
    pub connect_timeout: Option<f64>,
    pub tls_verify: bool,
    pub ca_cert: Option<String>,
}

impl HttpOptions {
    #[allow(clippy::type_complexity)]
    fn new_impl(args: &[Value]) -> Result<Self, Error> {
        let parsed = scan_args::<(), (), (), (), _, ()>(args)?;
        let ids = [
            *KW_TIMEOUT,
            *KW_CONNECT_TIMEOUT,
            *KW_TLS_VERIFY,
            *KW_CA_CERT,
        ];
        let kw: KwArgs<(), (Option<f64>, Option<f64>, Option<bool>, Option<String>), ()> =
            get_kwargs(parsed.keywords, &[], &ids)?;
        let (timeout, connect_timeout, tls_verify, ca_cert) = kw.optional;
        Ok(HttpOptions {
            timeout,
            connect_timeout,
            tls_verify: tls_verify.unwrap_or(true),
            ca_cert,
        })
    }

    fn timeout(&self) -> Option<f64> {
        self.timeout
    }

    fn connect_timeout(&self) -> Option<f64> {
        self.connect_timeout
    }

    fn tls_verify(&self) -> bool {
        self.tls_verify
    }

    fn ca_cert(&self) -> Option<String> {
        self.ca_cert.clone()
    }

    fn inspect(&self) -> String {
        use std::fmt::Write;
        let mut s = String::from("#<JSONSchema::HttpOptions timeout=");
        match self.timeout {
            Some(t) => write!(s, "{t}").expect("Failed to write timeout"),
            None => s.push_str("nil"),
        }
        s.push_str(", connect_timeout=");
        match self.connect_timeout {
            Some(t) => write!(s, "{t}").expect("Failed to write connect_timeout"),
            None => s.push_str("nil"),
        }
        s.push_str(", tls_verify=");
        s.push_str(if self.tls_verify { "true" } else { "false" });
        s.push_str(", ca_cert=");
        match &self.ca_cert {
            Some(c) => write!(s, "\"{c}\"").expect("Failed to write ca_cert"),
            None => s.push_str("nil"),
        }
        s.push('>');
        s
    }
}

pub fn define_classes(ruby: &Ruby, module: &RModule) -> Result<(), Error> {
    let email_class = module.define_class("EmailOptions", ruby.class_object())?;
    email_class.define_singleton_method("new", function!(EmailOptions::new_impl, -1))?;
    email_class.define_method("require_tld", method!(EmailOptions::require_tld, 0))?;
    email_class.define_method(
        "allow_domain_literal",
        method!(EmailOptions::allow_domain_literal, 0),
    )?;
    email_class.define_method(
        "allow_display_text",
        method!(EmailOptions::allow_display_text, 0),
    )?;
    email_class.define_method(
        "minimum_sub_domains",
        method!(EmailOptions::minimum_sub_domains, 0),
    )?;
    email_class.define_method("inspect", method!(EmailOptions::inspect, 0))?;

    let regex_class = module.define_class("RegexOptions", ruby.class_object())?;
    regex_class.define_singleton_method("new", function!(RegexOptions::new_impl, -1))?;
    regex_class.define_method("size_limit", method!(RegexOptions::size_limit, 0))?;
    regex_class.define_method("dfa_size_limit", method!(RegexOptions::dfa_size_limit, 0))?;
    regex_class.define_method("inspect", method!(RegexOptions::inspect, 0))?;

    let fancy_regex_class = module.define_class("FancyRegexOptions", ruby.class_object())?;
    fancy_regex_class.define_singleton_method("new", function!(FancyRegexOptions::new_impl, -1))?;
    fancy_regex_class.define_method(
        "backtrack_limit",
        method!(FancyRegexOptions::backtrack_limit, 0),
    )?;
    fancy_regex_class.define_method("size_limit", method!(FancyRegexOptions::size_limit, 0))?;
    fancy_regex_class.define_method(
        "dfa_size_limit",
        method!(FancyRegexOptions::dfa_size_limit, 0),
    )?;
    fancy_regex_class.define_method("inspect", method!(FancyRegexOptions::inspect, 0))?;

    let http_class = module.define_class("HttpOptions", ruby.class_object())?;
    http_class.define_singleton_method("new", function!(HttpOptions::new_impl, -1))?;
    http_class.define_method("timeout", method!(HttpOptions::timeout, 0))?;
    http_class.define_method("connect_timeout", method!(HttpOptions::connect_timeout, 0))?;
    http_class.define_method("tls_verify", method!(HttpOptions::tls_verify, 0))?;
    http_class.define_method("ca_cert", method!(HttpOptions::ca_cert, 0))?;
    http_class.define_method("inspect", method!(HttpOptions::inspect, 0))?;

    Ok(())
}

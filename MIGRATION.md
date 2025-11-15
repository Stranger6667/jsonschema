# Migration Guide

## Upgrading from 0.34.x to 0.35.0

### Custom meta-schemas require explicit registration

Schemas with custom/unknown `$schema` URIs now require their meta-schema to be registered before building validators. Custom meta-schemas automatically inherit the draft-specific behavior of their underlying draft by walking the meta-schema chain. Validators always honor an explicitly configured draft (e.g., via `ValidationOptions::with_draft`), so overriding the draft is still the highest priority and bypasses auto-detection and the registry check intentionally.

```rust
// Old (0.34.x) - would fail with unclear error
let schema = json!({
    "$schema": "http://example.com/custom",
    "type": "object"
});
let validator = jsonschema::validator_for(&schema)?;

// New (0.35.x) - explicit registration required
use jsonschema::{Registry, Resource, Draft};

let meta_schema = json!({
    "$id": "http://example.com/custom",
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "$vocabulary": {
        "https://json-schema.org/draft/2020-12/vocab/core": true,
        "https://json-schema.org/draft/2020-12/vocab/validation": true,
    }
});

let registry = Registry::try_from_resources(
    [("http://example.com/custom", Resource::from_contents(meta_schema))]
)?;

let validator = jsonschema::options()
    .with_registry(registry)
    .build(&schema)?;
```

**Draft Resolution:** Custom meta-schemas inherit draft-specific behavior from their underlying draft. For example, a custom meta-schema based on Draft 7 will preserve Draft 7 semantics (ignoring `$ref` siblings, validating formats by default). The validator walks the meta-schema chain to determine the appropriate draft. To override this behavior, use `.with_draft()` to explicitly set a draft version.

**Note:** `meta::is_valid` / `meta::validate` now behave strictly: they only succeed for schemas whose `$schema` resolves to one of the bundled meta-schemas. Unknown `$schema` values trigger the same error/panic you get during validator compilation. To meta-validate against custom specs, build a registry that contains those meta-schemas and call `jsonschema::meta::options().with_registry(registry)` before invoking `is_valid` / `validate` through the options builder.

### Removed `meta::try_is_valid` and `meta::try_validate`

The `try_*` variants have been removed. Use the non-`try_` versions which treat unknown `$schema` values as Draft 2020-12.

```rust
// Old (0.34.x)
let result = jsonschema::meta::try_is_valid(&schema)?;

// New (0.35.x)
let result = jsonschema::meta::is_valid(&schema); // Returns bool
```

### `Resource::from_contents` no longer returns `Result`

The method now always succeeds and returns `Resource` directly, since draft detection no longer fails for unknown `$schema` values.

```rust
// Old (0.34.x)
let resource = Resource::from_contents(schema)?;

// New (0.35.x)
let resource = Resource::from_contents(schema); // No ? needed
```

## Upgrading from 0.33.x to 0.34.0

### Removed `Validator::config()`

The `Validator::config()` method has been removed to reduce memory footprint. The validator no longer stores the configuration internally.

```rust
// Old (0.33.x)
let validator = jsonschema::validator_for(&schema)?;
let config = validator.config(); // Returns Arc<ValidationOptions>

// New (0.34.x)
// No replacement - the config is not stored after compilation
// If you need config values, keep a reference to your ValidationOptions
let options = jsonschema::options().with_draft(Draft::Draft7);
let validator = options.build(&schema)?;
// Keep `options` around if you need to access configuration later
```

### Meta-validator statics replaced with functions

Public `DRAFT*_META_VALIDATOR` statics have been removed. Use the new `draftX::meta::validator()` helper functions instead. Dropping the `Send + Sync` bounds for retrievers means the old `LazyLock` statics can't store validators on `wasm32` anymore, so the new helper borrows cached validators on native platforms and builds owned copies on WebAssembly.

```rust
// Old (0.33.x)
use jsonschema::DRAFT7_META_VALIDATOR;
DRAFT7_META_VALIDATOR.is_valid(&schema);

// Also removed:
use jsonschema::DRAFT4_META_VALIDATOR;
use jsonschema::DRAFT6_META_VALIDATOR;
use jsonschema::DRAFT201909_META_VALIDATOR;
use jsonschema::DRAFT202012_META_VALIDATOR;

// New (0.34.x)
let validator = jsonschema::draft7::meta::validator();
validator.is_valid(&schema);

// Or use the module-specific helper:
jsonschema::draft7::meta::is_valid(&schema);
```

### Lifetime parameters removed from output types

`BasicOutput` and `Annotations` no longer have lifetime parameters. This simplifies the API and uses `Arc` for internal ownership.

```rust
// Old (0.33.x)
fn process_output<'a>(output: BasicOutput<'a>) -> Result<(), Error> {
    match output {
        BasicOutput::Valid(units) => {
            for unit in units {
                let annotations: &Annotations<'a> = unit.annotations();
                // ...
            }
        }
        BasicOutput::Invalid(errors) => { /* ... */ }
    }
    Ok(())
}

// New (0.34.x)
fn process_output(output: BasicOutput) -> Result<(), Error> {
    match output {
        BasicOutput::Valid(units) => {
            for unit in units {
                let annotations: &Annotations = unit.annotations();
                // ...
            }
        }
        BasicOutput::Invalid(errors) => { /* ... */ }
    }
    Ok(())
}
```

### WASM: Relaxed `Send + Sync` bounds

`Retrieve` / `AsyncRetrieve` on `wasm32` no longer require `Send + Sync`.

```rust
use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use std::error::Error;

// Old (0.33.x)
use std::sync::{Arc, Mutex};
struct BrowserRetriever(Arc<Mutex<JsFetcher>>);

impl Retrieve for BrowserRetriever {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn Error + Send + Sync>> {
        self.0.lock().unwrap().fetch(uri)
    }
}

// New (0.34.x)
use std::rc::Rc;
struct BrowserRetriever(Rc<JsFetcher>);

impl Retrieve for BrowserRetriever {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn Error + Send + Sync>> {
        self.0.fetch(uri)
    }
}
```

Async retrievers follow the same pattern—switch `async_trait::async_trait` to `async_trait::async_trait(?Send)` on wasm so the implementation can hold non-thread-safe types.

```rust
// Old (0.33.x)
#[async_trait::async_trait]
impl AsyncRetrieve for BrowserRetriever {
    async fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn Error + Send + Sync>> {
        self.0.lock().unwrap().fetch(uri).await
    }
}

// New (0.34.x, wasm32)
#[async_trait::async_trait(?Send)]
impl AsyncRetrieve for BrowserRetriever {
    async fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn Error + Send + Sync>> {
        self.0.fetch(uri).await
    }
}
```

## Upgrading from 0.32.x to 0.33.0

In 0.33 `LocationSegment::Property` now holds a `Cow<'_, str>` and `LocationSegment` is no longer `Copy`. 

If your code matches the enum and treats the property as `&str`, update it like this.

This change was made to support proper round-trips for JSON Pointer segments (escaped vs. unescaped forms).

```rust
// Old (0.32.x)
match segment {
    LocationSegment::Property(p) => do_something(p), // p: &str
    LocationSegment::Index(i)    => ...
}

do_something_else(segment);

// New (0.33.0)
match segment {
    LocationSegment::Property(p) => do_something(&*p), // p: Cow<'_, str>
    LocationSegment::Index(i)    => ...
}

// `LocationSegment` is no longer Copy, use `.clone()` if you need ownership
do_something_else(segment.clone());
```

## Upgrading from 0.29.x to 0.30.0

`PrimitiveType` was replaced by `JsonType`, and `PrimitiveTypesBitMap` with `JsonTypeSet`.

```rust
// Old (0.29.x)
use jsonschema::primitive_types::PrimitiveType;
use jsonschema::primitive_types::PrimitiveTypesBitMap;

// New (0.30.0)
use jsonschema::JsonType;
use jsonschema::JsonTypeSet;
```

## Upgrading from 0.28.x to 0.29.0

The builder methods on `ValidationOptions` now take ownership of `self`. Change your code to use method chaining instead of reusing the options instance:

```rust
// Old (0.28.x)
let mut options = jsonschema::options();
options.with_draft(Draft::Draft202012);
options.with_format("custom", |s| s.len() > 3);
let validator = options.build(&schema)?;

// New (0.29.0)
let validator = jsonschema::options()
    .with_draft(Draft::Draft202012)
    .with_format("custom", |s| s.len() > 3)
    .build(&schema)?;
```

If you implement the `Retrieve` trait, update the `uri` parameter type in the `retrieve` method:

```rust
// Old (0.28.x)
impl Retrieve for MyRetriever {
    fn retrieve(&self, uri: &Uri<&str>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        // ...
    }
}

// New (0.29.0)
impl Retrieve for MyRetriever {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        // ...
    }
}
```

This is a type-level change only; the behavior and available methods remain the same.

The Registry API has been simplified to use a consistent builder pattern. Replace direct creation methods with `Registry::options()`:

```rust
// Before (0.28.x)
let registry = Registry::options()
    .draft(Draft::Draft7)
    .try_new(
        "http://example.com/schema",
        resource
    )?;

let registry = Registry::options()
    .draft(Draft::Draft7)
    .try_from_resources([
        ("http://example.com/schema", resource)
    ].into_iter())?;
    
let registry = Registry.try_with_resource_and_retriever(
    "http://example.com/schema",
    resource,
    retriever
)?;

// After (0.29.0)
let registry = Registry::options()
    .draft(Draft::Draft7)
    .build([
        ("http://example.com/schema", resource)
    ])?;

let registry = Registry::options()
    .draft(Draft::Draft7)
    .build([
        ("http://example.com/schema", resource)
    ])?;

let registry = Registry::options()
    .retriever(retriever)
    .build(resources)?;
```

## Upgrading from 0.25.x to 0.26.0

The `Validator::validate` method now returns `Result<(), ValidationError<'i>>` instead of an error iterator. If you need to iterate over all validation errors, use the new `Validator::iter_errors` method.

Example:

```rust
// Old (0.25.x)
let validator = jsonschema::validator_for(&schema)?;

if let Err(errors) = validator.validate(&instance) {
    for error in errors {
        println!("Error: {error}");
    }
}

// New (0.26.0)
let validator = jsonschema::validator_for(&schema)?;

// To get the first error only
match validator.validate(&instance) {
    Ok(()) => println!("Valid!"),
    Err(error) => println!("Error: {error}"),
}

// To iterate over all errors
for error in validator.iter_errors(&instance) {
    println!("Error: {error}");
}
```

## Upgrading from 0.22.x to 0.23.0

Replace:

 - `JsonPointer` to `Location`
 - `PathChunkRef` to `LocationSegment`
 - `JsonPointerNode` to `LazyLocation`

## Upgrading from 0.21.x to 0.22.0

Replace `UriRef<&str>` with `Uri<&str>` in your custom retriever implementation.

Example:

```rust
// Old (0.21.x)
use jsonschema::{UriRef, Retrieve};

struct MyCustomRetriever;

impl Retrieve for MyCustomRetriever {
    fn retrieve(&self, uri: &UriRef<&str>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        // ...
    }
}

// New (0.21.0)
use jsonschema::{Uri, Retrieve};

struct MyCustomRetriever;
impl Retrieve for MyCustomRetriever {
    fn retrieve(&self, uri: &Uri<&str>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        // ...
    }
}
```

## Upgrading from 0.20.x to 0.21.0

1. Replace `SchemaResolver` with `Retrieve`:
   - Implement `Retrieve` trait instead of `SchemaResolver`
   - Use `Box<dyn std::error::Error>` for error handling
   - Update `ValidationOptions` to use `with_retriever` instead of `with_resolver`

Example:

```rust
// Old (0.20.x)
struct MyCustomResolver;

impl SchemaResolver for MyCustomResolver {
    fn resolve(&self, root_schema: &Value, url: &Url, _original_reference: &str) -> Result<Arc<Value>, SchemaResolverError> {
        match url.scheme() {
            "http" | "https" => {
                Ok(Arc::new(json!({ "description": "an external schema" })))
            }
            _ => Err(anyhow!("scheme is not supported"))
        }
    }
}

let options = jsonschema::options().with_resolver(MyCustomResolver);

// New (0.21.0)
use jsonschema::{UriRef, Retrieve};

struct MyCustomRetriever;

impl Retrieve for MyCustomRetriever {
    fn retrieve(&self, uri: &UriRef<&str>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        match uri.scheme().map(|scheme| scheme.as_str()) {
            Some("http" | "https") => {
                Ok(json!({ "description": "an external schema" }))
            }
            _ => Err("scheme is not supported".into())
        }
    }
}

let options = jsonschema::options().with_retriever(MyCustomRetriever);
```

2. Update document handling:
   - Replace `with_document` with `with_resource`

Example:

```rust
// Old (0.20.x)
let options = jsonschema::options()
    .with_document("schema_id", schema_json);

// New (0.21.0)
use jsonschema::Resource;

let options = jsonschema::options()
    .with_resource("urn:schema_id", Resource::from_contents(schema_json)?);
```


## Upgrading from 0.19.x to 0.20.0

Draft-specific modules are now available:

   ```rust
   // Old (0.19.x)
   let validator = jsonschema::JSONSchema::options()
       .with_draft(jsonschema::Draft2012)
       .compile(&schema)
       .expect("Invalid schema");

   // New (0.20.0)
   let validator = jsonschema::draft202012::new(&schema)
       .expect("Invalid schema");
   ```

   Available modules: `draft4`, `draft6`, `draft7`, `draft201909`, `draft202012`

Use the new `options()` function for easier customization:

   ```rust
   // Old (0.19.x)
   let options = jsonschema::JSONSchema::options();

   // New (0.20.0)
   let options = jsonschema::options();
   ```

The following items have been renamed. While the old names are still supported in 0.20.0 for backward compatibility, it's recommended to update to the new names:

| Old Name (0.19.x) | New Name (0.20.0) |
|-------------------|-------------------|
| `CompilationOptions` | `ValidationOptions` |
| `JSONSchema` | `Validator` |
| `JSONPointer` | `JsonPointer` |
| `jsonschema::compile` | `jsonschema::validator_for` |
| `CompilationOptions::compile` | `ValidationOptions::build` |

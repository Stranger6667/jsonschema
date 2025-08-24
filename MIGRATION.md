# Migration Guide

## Upgrading from 0.32.x to 0.33.0 note

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


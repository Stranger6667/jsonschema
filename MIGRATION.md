# Migration Guide

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


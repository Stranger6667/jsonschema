use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::{Draft, Registry, Uri};
use serde_json::{Map, Value};
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::Arc,
};

struct FunctionInfo {
    name: String,
}

struct FunctionBody {
    body: TokenStream,
}

struct ResolvedRef {
    schema: Value,
    location: String,
    /// Base URI of the resolved schema (for resolving nested references)
    base_uri: Arc<Uri<String>>,
}

pub(crate) struct Codegen<'a> {
    schema: &'a Value,
    draft: Draft,
    registry: Option<Registry>,
    base_uri: Option<Arc<Uri<String>>>,
    /// Tracks the base URI context during schema compilation (for nested $refs)
    current_base_uri: RefCell<Option<Arc<Uri<String>>>>,
    location_to_function: RefCell<HashMap<String, FunctionInfo>>,
    function_bodies: RefCell<HashMap<String, FunctionBody>>,
    ref_counter: RefCell<usize>,
    seen: RefCell<HashSet<String>>,
}

impl<'a> Codegen<'a> {
    pub(crate) fn new(
        schema: &'a Value,
        draft: Draft,
        registry: Option<Registry>,
        base_uri: Option<Arc<Uri<String>>>,
    ) -> Self {
        Self {
            schema,
            draft,
            registry,
            base_uri,
            current_base_uri: RefCell::new(None),
            location_to_function: RefCell::new(HashMap::new()),
            function_bodies: RefCell::new(HashMap::new()),
            ref_counter: RefCell::new(0),
            seen: RefCell::new(HashSet::new()),
        }
    }
    /// Extract u64 from a JSON value, handling both integers and decimals like 2.0
    /// Draft 6+ allows integer-valued numbers (e.g., 2.0), Draft 4 does not
    fn value_as_u64(&self, value: &Value) -> Option<u64> {
        // Fast path: try integer first (most common case)
        if let Some(n) = value.as_u64() {
            return Some(n);
        }
        // Slow path: Draft 6+ supports integer-valued decimals
        if !matches!(self.draft, Draft::Draft4) {
            if let Some(f) = value.as_f64() {
                if f.fract() == 0.0 && f >= 0.0 && f <= u64::MAX as f64 {
                    return Some(f as u64);
                }
            }
        }
        None
    }

    /// Generate the full validator implementation.
    ///
    /// Optionally includes a recompile trigger for file-based schemas.
    pub(crate) fn generate(&self, recompile_trigger: &TokenStream) -> TokenStream {
        // If schema has external dependencies, generate a validator that always returns false
        if self.registry.is_none() {
            return quote! {
                pub fn is_valid(_instance: &serde_json::Value) -> bool {
                    #recompile_trigger
                    // Schema has external dependencies that cannot be resolved at compile time
                    false
                }

                pub fn validate(_instance: &serde_json::Value)
                    -> Result<(), jsonschema::ValidationError<'static>>
                {
                    Err(jsonschema::ValidationError::custom(
                        "Schema has external dependencies that cannot be resolved at compile time"
                    ))
                }
            };
        }

        let validation_expr = self.compile_schema(self.schema);

        // Generate helper functions for $ref as nested functions inside is_valid
        let helper_functions: Vec<TokenStream> = self
            .function_bodies
            .borrow()
            .iter()
            .map(|(name, body_info)| {
                let func_ident = format_ident!("{}", name);
                let body = &body_info.body;

                quote! {
                    #[inline]
                    fn #func_ident(instance: &serde_json::Value) -> bool {
                        #body
                    }
                }
            })
            .collect();

        quote! {
            // Helper functions for $ref at impl level
            #(#helper_functions)*

            pub fn is_valid(instance: &serde_json::Value) -> bool {
                #recompile_trigger
                #validation_expr
            }

            pub fn validate(instance: &serde_json::Value)
                -> Result<(), jsonschema::ValidationError<'static>>
            {
                if Self::is_valid(instance) {
                    Ok(())
                } else {
                    // TODO: Proper validation error
                    Err(jsonschema::ValidationError::custom("Schema validation failed"))
                }
            }
        }
    }
    /// Compile a schema into validation code.
    fn compile_schema(&self, schema: &Value) -> TokenStream {
        // TODO: Should generate errors too - maybe always generate a pair - for is_valid & validate?
        match schema {
            Value::Bool(true) => quote! { true },
            Value::Bool(false) => quote! { false },
            Value::Object(obj) => self.compile_object_schema(obj),
            _ => {
                // TODO: Return compilation error here
                quote! { true }
            }
        }
    }
    /// Compile an object schema.
    fn compile_object_schema(&self, schema: &Map<String, Value>) -> TokenStream {
        // Check for $ref first - in most drafts it stops other keywords
        if let Some(ref_value) = schema.get("$ref") {
            return self.compile_ref(ref_value);
        }

        // Check for $id and update base URI context if present
        let prev_base_uri = if let Some(id_value) = schema.get("$id") {
            if let Some(id_str) = id_value.as_str() {
                // Resolve the $id relative to current base URI
                let current_base = self
                    .current_base_uri
                    .borrow()
                    .as_ref()
                    .cloned()
                    .or_else(|| self.base_uri.clone());

                if let Some(base) = current_base {
                    if let Some(registry) = self.registry.as_ref() {
                        // Resolve the $id URI relative to current base
                        let resolver = registry.resolver((*base).clone());
                        if let Ok(resolved_id) = resolver.lookup(id_str) {
                            let new_base = resolved_id.resolver().base_uri().clone();
                            let prev = self.current_base_uri.borrow().clone();
                            *self.current_base_uri.borrow_mut() = Some(new_base);
                            prev
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // TODO: Thread draft-specific logic here

        // Check if we have a type constraint
        let has_type_constraint = schema.contains_key("type");

        // Check if we'll generate type-specific checks
        let typed = self.compile_typed(schema, has_type_constraint);
        let has_typed_checks = typed.is_some();

        // Collect type-agnostic keywords
        let mut untyped = Vec::new();

        // Only add universal type check if we don't have type-specific checks.
        // When we have type-specific checks with a type constraint,
        // the match statement handles type checking (single discriminant check).
        if has_type_constraint && !has_typed_checks {
            if let Some(value) = schema.get("type") {
                untyped.push(self.compile_type(value));
            }
        }
        if let Some(value) = schema.get("const") {
            untyped.push(self.compile_const(value));
        }
        if let Some(value) = schema.get("enum") {
            untyped.push(self.compile_enum(value));

            // TODO. validate for an array
        }
        if let Some(value) = schema.get("allOf") {
            untyped.push(self.compile_all_of(value));
        }
        if let Some(value) = schema.get("anyOf") {
            untyped.push(self.compile_any_of(value));
        }
        if let Some(value) = schema.get("oneOf") {
            untyped.push(self.compile_one_of(value));
        }
        if let Some(value) = schema.get("not") {
            untyped.push(self.compile_not(value));
        }
        if let Some(value) = schema.get("if") {
            if let Some(compiled) = self.compile_if_then_else(schema, value) {
                untyped.push(compiled);
            }
        }

        let mut all = untyped;
        if let Some(type_check) = typed {
            all.push(type_check);
        }

        let result = if all.is_empty() {
            // Empty schema - always valid
            quote! { true }
        } else {
            // Combine all checks
            quote! { ( #(#all)&&* ) }
        };

        // TODO: Use separate context
        // Restore previous base URI if we updated it for $id
        if prev_base_uri.is_some() {
            *self.current_base_uri.borrow_mut() = prev_base_uri;
        }

        result
    }
    /// Compile type-specific keywords.
    fn compile_typed(
        &self,
        schema: &Map<String, Value>,
        has_type_constraint: bool,
    ) -> Option<TokenStream> {
        // TODO: Add all keywords + check draft + vocabulary
        let has_string = schema.contains_key("minLength")
            || schema.contains_key("maxLength")
            || schema.contains_key("pattern")
            || schema.contains_key("format");

        let has_number = schema.contains_key("minimum")
            || schema.contains_key("maximum")
            || schema.contains_key("exclusiveMinimum")
            || schema.contains_key("exclusiveMaximum")
            || schema.contains_key("multipleOf");

        let has_array = schema.contains_key("minItems")
            || schema.contains_key("maxItems")
            || schema.contains_key("items")
            || schema.contains_key("uniqueItems")
            || schema.contains_key("additionalItems")
            || schema.contains_key("contains");

        let has_object = schema.contains_key("properties")
            || schema.contains_key("required")
            || schema.contains_key("minProperties")
            || schema.contains_key("maxProperties")
            || schema.contains_key("patternProperties")
            || schema.contains_key("additionalProperties")
            || schema.contains_key("dependencies")
            || schema.contains_key("propertyNames");

        if !has_string && !has_number && !has_array && !has_object {
            return None;
        }

        // Generate match arms for each type
        let mut match_arms = Vec::new();

        if has_string {
            let for_string = self.compile_for_string(schema);
            match_arms.push(quote! {
                serde_json::Value::String(s) => { #for_string }
            });
        }

        if has_number {
            let for_number = self.compile_for_number(schema);
            match_arms.push(quote! {
                serde_json::Value::Number(n) => { #for_number }
            });
        }

        if has_array {
            let for_array = self.compile_for_array(schema);
            match_arms.push(quote! {
                serde_json::Value::Array(arr) => { #for_array }
            });
        }

        if has_object {
            let for_object = self.compile_for_object(schema);
            match_arms.push(quote! {
                serde_json::Value::Object(obj) => { #for_object }
            });
        }

        // Default arm: check if we need to accept types without specific keywords (boolean, null)
        if has_type_constraint {
            // Get the type constraint
            if let Some(type_val) = schema.get("type") {
                // TODO: JsonTypeSet would work much better here
                // TODO: Recheck correctness here
                let mut additional_types = Vec::new();

                // Check if boolean or null are in the type constraint
                let has_boolean = match type_val {
                    Value::String(s) => s == "boolean",
                    Value::Array(arr) => arr.iter().any(|v| v.as_str() == Some("boolean")),
                    _ => false,
                };
                let has_null = match type_val {
                    Value::String(s) => s == "null",
                    Value::Array(arr) => arr.iter().any(|v| v.as_str() == Some("null")),
                    _ => false,
                };

                if has_boolean {
                    additional_types.push(quote! { serde_json::Value::Bool(_) });
                }
                if has_null {
                    additional_types.push(quote! { serde_json::Value::Null });
                }

                if !additional_types.is_empty() {
                    // Accept these additional types, reject everything else
                    match_arms.push(quote! { #(#additional_types)|* => true });
                }
            }
            match_arms.push(quote! { _ => false });
        } else {
            match_arms.push(quote! { _ => true });
        }

        Some(quote! {
            match instance {
                #(#match_arms),*
            }
        })
    }
    /// Compile string-specific keywords.
    fn compile_for_string(&self, schema: &Map<String, Value>) -> TokenStream {
        let min_length = schema.get("minLength").and_then(|v| self.value_as_u64(v));
        let max_length = schema.get("maxLength").and_then(|v| self.value_as_u64(v));
        let has_length_constraint = min_length.is_some() || max_length.is_some();

        let mut items = Vec::new();

        // Length checks - calculate once and reuse
        // TODO: u64 may be too narrow
        if let Some(value) = min_length {
            items.push(quote! { len >= #value as usize });
        }

        if let Some(value) = max_length {
            items.push(quote! { len <= #value as usize });
        }

        // TODO:
        //   - Use configured regex engine + its options
        if let Some(pattern) = schema.get("pattern").and_then(|v| v.as_str()) {
            // Try prefix optimization first
            if let Some(prefix) = jsonschema_core::regex::pattern_as_prefix(pattern) {
                let prefix: &str = prefix.as_ref();
                items.push(quote! { s.starts_with(#prefix) });
            } else if let Ok(pattern) = jsonschema_core::regex::to_rust_regex(pattern) {
                // Fall back to full regex for complex patterns
                items.push(quote! {
                    {
                        static PATTERN: std::sync::LazyLock<regex::Regex> =
                            std::sync::LazyLock::new(|| {
                                regex::Regex::new(#pattern).expect("Invalid regex")
                            });
                        PATTERN.is_match(s)
                    }
                });
            }
            // TODO: compile error on invalid regex
        }

        if let Some(compiled) = schema.get("format").and_then(|v| self.compile_format(v)) {
            items.push(compiled);
        }

        if items.is_empty() {
            // TODO: Is it actually possible??
            quote! { true }
        } else {
            let combined = quote! { ( #(#items)&&* ) };

            // If we have length checks, wrap in a block to calculate length once
            if has_length_constraint {
                quote! {
                    {
                        let len = s.chars().count();
                        #combined
                    }
                }
            } else {
                combined
            }
        }
    }
    /// Compile all number-specific keywords.
    fn compile_for_number(&self, schema: &Map<String, Value>) -> TokenStream {
        let mut items = Vec::new();
        // TODO: arbitrary precision support

        if matches!(self.draft, Draft::Draft4) {
            // Draft 4: exclusiveMinimum/Maximum are boolean modifiers
            let exclusive_min = schema
                .get("exclusiveMinimum")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let exclusive_max = schema
                .get("exclusiveMaximum")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            if let Some(value) = schema.get("minimum") {
                let op = if exclusive_min { ">" } else { ">=" };
                items.push(self.generate_numeric_check(op, value));
            }

            if let Some(value) = schema.get("maximum") {
                let op = if exclusive_max { "<" } else { "<=" };
                items.push(self.generate_numeric_check(op, value));
            }
        } else {
            // Draft 6+: standalone numeric values
            if let Some(value) = schema.get("minimum") {
                items.push(self.generate_numeric_check(">=", value));
            }
            if let Some(value) = schema.get("maximum") {
                items.push(self.generate_numeric_check("<=", value));
            }
            if let Some(value) = schema.get("exclusiveMinimum") {
                if !value.is_boolean() {
                    // Not a boolean (Draft 4 style)
                    items.push(self.generate_numeric_check(">", value));
                }
            }
            if let Some(value) = schema.get("exclusiveMaximum") {
                if !value.is_boolean() {
                    items.push(self.generate_numeric_check("<", value));
                }
            }
        }

        if let Some(value) = schema.get("multipleOf") {
            items.push(self.generate_multiple_of_check(value));
        }

        if items.is_empty() {
            // TODO: Is it actually possible??
            quote! { true }
        } else {
            quote! { ( #(#items)&&* ) }
        }
    }
    /// Compile all array-specific keywords.
    fn compile_for_array(&self, schema: &Map<String, Value>) -> TokenStream {
        let mut items = Vec::new();

        // TODO: coercing to usize??
        if let Some(value) = schema.get("minItems").and_then(|v| self.value_as_u64(v)) {
            items.push(quote! { arr.len() >= #value as usize });
        }
        // TODO: Compile error on invalid value

        if let Some(value) = schema.get("maxItems").and_then(|v| self.value_as_u64(v)) {
            items.push(quote! { arr.len() <= #value as usize });
        }

        if let Some(value) = schema.get("items") {
            let compiled = self.generate_items_check(value);
            items.push(compiled);
        }

        if let Some(compiled) = schema
            .get("uniqueItems")
            .and_then(|v| self.compile_unique_items(v))
        {
            items.push(compiled);
        }

        if let Some(compiled) =
            self.compile_additional_items(schema.get("additionalItems"), schema.get("items"))
        {
            items.push(compiled);
        }

        if let Some(compiled) = schema.get("contains").map(|v| self.compile_contains(v)) {
            items.push(compiled);
        }
        // TODO: unevaluatedItems

        if items.is_empty() {
            // TODO: Is it actually possible??
            quote! { true }
        } else {
            quote! { ( #(#items)&&* ) }
        }
    }
    /// Compile all object-specific keywords.
    fn compile_for_object(&self, schema: &Map<String, Value>) -> TokenStream {
        let mut items = Vec::new();

        if let Some(value) = schema
            .get("minProperties")
            .and_then(|v| self.value_as_u64(v))
        {
            items.push(quote! { obj.len() >= #value as usize });
        }

        if let Some(value) = schema
            .get("maxProperties")
            .and_then(|v| self.value_as_u64(v))
        {
            items.push(quote! { obj.len() <= #value as usize });
        }

        let required_fields: Vec<&str> = schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        // Properties validation via iteration (will be handled at the end)
        let properties_map = schema.get("properties").and_then(|v| v.as_object());

        // Partition required fields: those also in `properties` are tracked via bool
        // during iteration; those not in `properties` still need contains_key.
        let (required_in_props, required_only): (Vec<&str>, Vec<&str>) =
            if let Some(props) = properties_map {
                required_fields
                    .iter()
                    .copied()
                    .partition(|name| props.contains_key(*name))
            } else {
                (Vec::new(), required_fields.clone())
            };

        for name in &required_only {
            items.push(quote! { obj.contains_key(#name) });
        }

        if let Some(compiled) = schema
            .get("patternProperties")
            .and_then(|v| self.compile_pattern_properties(v))
        {
            items.push(compiled);
        }

        if let Some(compiled) = schema
            .get("dependencies")
            .and_then(|v| self.compile_dependencies(v))
        {
            items.push(compiled);
        }

        // Draft 6: propertyNames - all property names must validate against the schema
        if let Some(compiled) = schema
            .get("propertyNames")
            .map(|v| self.compile_property_names(v))
        {
            items.push(compiled);
        }
        // TODO: unevaluatedProperties, minContains, maxContains, etc

        let ap = schema.get("additionalProperties");
        let pp = schema.get("patternProperties");

        if let Some(properties) = properties_map {
            // For strict objects without patternProperties, a key-only precheck can fail fast
            // before potentially expensive per-value validation.
            let use_known_keys_precheck = matches!(ap, Some(Value::Bool(false)))
                && pp
                    .and_then(|value| value.as_object())
                    .map_or(true, |patterns| patterns.is_empty());

            // Merge additionalProperties coverage into the single properties iteration.
            // The wildcard arm body replaces `_ => true`, eliminating a separate key-coverage pass.
            let (wildcard_statics, wildcard_arm_body) = if use_known_keys_precheck {
                // Coverage is guaranteed by the precheck, so wildcard keys are unreachable.
                (Vec::new(), quote! { true })
            } else {
                self.compile_wildcard_arm(ap, pp)
            };
            let known_keys_precheck = if use_known_keys_precheck {
                self.compile_known_keys_precheck(properties)
            } else {
                quote! { true }
            };

            let mut match_arms = Vec::new();

            // Assign a unique bool variable per required-in-props field
            let tracked: Vec<(&str, proc_macro2::Ident)> = required_in_props
                .iter()
                .enumerate()
                .map(|(i, &name)| (name, format_ident!("__required_{}", i)))
                .collect();

            for (name, subschema) in properties {
                let compiled = self.compile_schema(subschema);
                if let Some((_, var)) = tracked.iter().find(|(n, _)| *n == name.as_str()) {
                    match_arms.push(quote! {
                        #name => { #var = true; #compiled }
                    });
                } else {
                    match_arms.push(quote! {
                        #name => #compiled
                    });
                }
            }

            let properties_check = if tracked.is_empty() {
                quote! {
                    {
                        #(#wildcard_statics)*
                        #known_keys_precheck && obj.iter().all(|(key, instance)| {
                            let key_str = key.as_str();
                            match key_str {
                                #(#match_arms,)*
                                _ => #wildcard_arm_body
                            }
                        })
                    }
                }
            } else {
                let var_decls = tracked
                    .iter()
                    .map(|(_, var)| quote! { let mut #var = false; });
                let var_checks = tracked.iter().map(|(_, var)| quote! { #var });
                quote! {
                    {
                        #(#wildcard_statics)*
                        #(#var_decls)*
                        #known_keys_precheck && obj.iter().all(|(key, instance)| {
                            let key_str = key.as_str();
                            match key_str {
                                #(#match_arms,)*
                                _ => #wildcard_arm_body
                            }
                        }) && #(#var_checks)&&*
                    }
                }
            };
            items.push(properties_check);
        } else {
            // No properties: use the standalone additionalProperties check when present.
            // Note: we do NOT push a `true` placeholder here, which avoids a spurious `&& true`.
            if let Some(compiled) =
                self.compile_additional_properties(ap, schema.get("properties"), pp)
            {
                items.push(compiled);
            }
        }

        if items.is_empty() {
            quote! { true }
        } else {
            quote! { ( #(#items)&&* ) }
        }
    }
    /// Compile the "type" keyword.
    fn compile_type(&self, value: &Value) -> TokenStream {
        match value {
            Value::String(ty) => self.generate_type_check(ty.as_str()),
            Value::Array(types) => {
                // TODO:
                //   - should generate `matches!(value, Value::String(_) | ...)` for efficiency
                //   - account for `number` + draft-specific logic
                let items = types
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(|type_name| self.generate_type_check(type_name));

                // TODO: compile error on empty array
                quote! { ( #(#items)||* ) }
            }
            _ => {
                // TODO: compilation error
                quote! { true }
            }
        }
    }
    /// Generate type check for a single type.
    fn generate_type_check(&self, value: &str) -> TokenStream {
        // TODO: Drop?
        match value {
            "string" => quote! { instance.is_string() },
            "number" => quote! { instance.is_number() },
            "integer" => {
                // TODO:
                //   - arbitrary precision
                //   - draft specific differences
                quote! {
                    match instance {
                        serde_json::Value::Number(n) => {
                            n.is_i64() || n.is_u64() || {
                                // Check if it's an f64 with no fractional part
                                n.as_f64().map_or(false, |f| f.fract() == 0.0)
                            }
                        }
                        _ => false
                    }
                }
            }
            "boolean" => quote! { instance.is_boolean() },
            "null" => quote! { instance.is_null() },
            "array" => quote! { instance.is_array() },
            "object" => quote! { instance.is_object() },
            _ => {
                // TODO: compile error
                quote! { true }
            }
        }
    }
    /// Compile the "const" keyword.
    fn compile_const(&self, value: &Value) -> TokenStream {
        match value {
            // Scalar constants can use direct checks without constructing serde_json::Value.
            Value::Null => quote! { instance.is_null() },
            Value::Bool(expected) => quote! { instance.as_bool() == Some(#expected) },
            Value::String(expected) => quote! { instance.as_str() == Some(#expected) },
            Value::Number(expected) => {
                let json = expected.to_string();
                quote! {
                    {
                        static EXPECTED: std::sync::LazyLock<serde_json::Number> =
                            std::sync::LazyLock::new(|| {
                                serde_json::from_str(#json)
                                    .expect("Failed to parse const number")
                            });
                        match instance {
                            serde_json::Value::Number(actual) => {
                                jsonschema::ext::cmp::equal_numbers(actual, &*EXPECTED)
                            }
                            _ => false,
                        }
                    }
                }
            }
            Value::Array(_) | Value::Object(_) => {
                let json = serde_json::to_string(value).expect("Failed to serialize const value");
                quote! {
                    {
                        static EXPECTED: std::sync::LazyLock<serde_json::Value> =
                            std::sync::LazyLock::new(|| {
                                serde_json::from_str(#json)
                                    .expect("Failed to parse const value")
                            });
                        jsonschema::ext::cmp::equal(instance, &*EXPECTED)
                    }
                }
            }
        }
    }
    /// Compile the "enum" keyword.
    fn compile_enum(&self, value: &Value) -> TokenStream {
        let Value::Array(variants) = value else {
            todo!("Proper error")
        };

        // Collect variants into per-type buckets using stable index constants.
        // Indices correspond to the 7 JSON types:
        //   0=Array, 1=Boolean, 2=Integer, 3=Null, 4=Number(float), 5=Object, 6=String
        const ARRAY_IDX: usize = 0;
        const BOOL_IDX: usize = 1;
        const INT_IDX: usize = 2;
        const NULL_IDX: usize = 3;
        const NUM_IDX: usize = 4;
        const OBJ_IDX: usize = 5;
        const STR_IDX: usize = 6;

        let mut by_type: [Vec<&Value>; 7] = Default::default();

        for variant in variants {
            let idx = match variant {
                Value::Null => NULL_IDX,
                Value::Bool(_) => BOOL_IDX,
                Value::Number(n) => {
                    // Properly detect integer vs float:
                    // Draft 6+ treats float-valued integers (e.g. 2.0) as integers too.
                    let is_integer = n.is_i64()
                        || n.is_u64()
                        || (!matches!(self.draft, Draft::Draft4)
                            && n.as_f64().map_or(false, |f| f.fract() == 0.0));
                    if is_integer {
                        INT_IDX
                    } else {
                        NUM_IDX
                    }
                }
                Value::String(_) => STR_IDX,
                Value::Array(_) => ARRAY_IDX,
                Value::Object(_) => OBJ_IDX,
            };
            by_type[idx].push(variant);
        }

        let mut match_arms = Vec::new();

        // Null: there is only one null value
        if !by_type[NULL_IDX].is_empty() {
            match_arms.push(quote! { serde_json::Value::Null => true });
        }

        // Boolean: compare the inner bool directly, no Value wrapping needed
        let booleans = &by_type[BOOL_IDX];
        if !booleans.is_empty() {
            let has_true = booleans.iter().any(|v| v.as_bool() == Some(true));
            let has_false = booleans.iter().any(|v| v.as_bool() == Some(false));
            let arm = match (has_true, has_false) {
                (true, true) => quote! { serde_json::Value::Bool(_) => true },
                (true, false) => quote! { serde_json::Value::Bool(b) => *b },
                (false, true) => quote! { serde_json::Value::Bool(b) => !*b },
                (false, false) => unreachable!(),
            };
            match_arms.push(arm);
        }

        // String: compare as &str without any Value wrapping, avoids LazyLock
        let strings = &by_type[STR_IDX];
        if !strings.is_empty() {
            let str_values: Vec<&str> = strings.iter().filter_map(|v| v.as_str()).collect();
            let arm = if str_values.len() == 1 {
                let s = str_values[0];
                quote! { serde_json::Value::String(s) => s.as_str() == #s }
            } else {
                quote! { serde_json::Value::String(s) => matches!(s.as_str(), #(#str_values)|*) }
            };
            match_arms.push(arm);
        }

        // Numbers (integers and floats combined): use jsonschema-aware comparison
        // to handle cross-type equality (e.g. 0 == 0.0).
        let int_variants = &by_type[INT_IDX];
        let num_variants = &by_type[NUM_IDX];
        if !int_variants.is_empty() || !num_variants.is_empty() {
            let all_numbers: Vec<Value> = int_variants
                .iter()
                .chain(num_variants.iter())
                .map(|v| (*v).clone())
                .collect();
            let numbers_json =
                serde_json::to_string(&all_numbers).expect("Failed to serialize number variants");
            match_arms.push(quote! {
                serde_json::Value::Number(_) => {
                    static NUMBER_VARIANTS: std::sync::LazyLock<Vec<serde_json::Value>> =
                        std::sync::LazyLock::new(|| {
                            serde_json::from_str::<Vec<serde_json::Value>>(#numbers_json)
                                .expect("Failed to parse number variants")
                        });
                    NUMBER_VARIANTS.iter().any(|v| jsonschema::ext::cmp::equal(v, instance))
                }
            });
        }

        // Arrays and objects: use jsonschema-aware comparison
        let array_variants = &by_type[ARRAY_IDX];
        let object_variants = &by_type[OBJ_IDX];
        let has_arrays = !array_variants.is_empty();
        let has_objects = !object_variants.is_empty();
        if has_arrays || has_objects {
            let complex: Vec<Value> = array_variants
                .iter()
                .chain(object_variants.iter())
                .map(|v| (*v).clone())
                .collect();
            let complex_json =
                serde_json::to_string(&complex).expect("Failed to serialize complex variants");
            let arm_pattern = match (has_arrays, has_objects) {
                (true, true) => {
                    quote! { serde_json::Value::Array(_) | serde_json::Value::Object(_) }
                }
                (true, false) => quote! { serde_json::Value::Array(_) },
                (false, true) => quote! { serde_json::Value::Object(_) },
                (false, false) => unreachable!(),
            };
            match_arms.push(quote! {
                #arm_pattern => {
                    static COMPLEX_VARIANTS: std::sync::LazyLock<Vec<serde_json::Value>> =
                        std::sync::LazyLock::new(|| {
                            serde_json::from_str::<Vec<serde_json::Value>>(#complex_json)
                                .expect("Failed to parse complex variants")
                        });
                    COMPLEX_VARIANTS.iter().any(|v| jsonschema::ext::cmp::equal(v, instance))
                }
            });
        }

        // Default: fast rejection for any type not present in the enum
        match_arms.push(quote! { _ => false });

        quote! {
            match instance {
                #(#match_arms),*
            }
        }
    }
    /// Generate numeric comparison for extracted Number value.
    fn generate_numeric_check(&self, operator: &str, limit: &Value) -> TokenStream {
        // TODO: Use proper enum instead of a string for op
        let op_token = match operator {
            ">=" => quote! { >= },
            "<=" => quote! { <= },
            ">" => quote! { > },
            "<" => quote! { < },
            _ => unreachable!(),
        };

        // TODO: Arbitrary precision
        let is_less_op = operator == "<" || operator == "<=";
        let is_greater_op = operator == ">" || operator == ">=";
        // TODO: use proper comparison that account for type differences

        if let Some(u) = limit.as_u64() {
            let neg_i64_result = is_less_op;
            quote! {
                if let Some(v) = n.as_u64() {
                    v #op_token #u
                } else if let Some(v) = n.as_i64() {
                    if v < 0 {
                        #neg_i64_result
                    } else {
                        (v as u64) #op_token #u
                    }
                } else if let Some(v) = n.as_f64() {
                    v #op_token (#u as f64)
                } else {
                    false
                }
            }
        } else if let Some(i) = limit.as_i64() {
            if i < 0 {
                let pos_u64_result = is_greater_op;
                quote! {
                    if let Some(v) = n.as_i64() {
                        v #op_token #i
                    } else if let Some(v) = n.as_u64() {
                        #pos_u64_result
                    } else if let Some(v) = n.as_f64() {
                        v #op_token (#i as f64)
                    } else {
                        false
                    }
                }
            } else {
                quote! {
                    if let Some(v) = n.as_i64() {
                        v #op_token #i
                    } else if let Some(v) = n.as_u64() {
                        v #op_token (#i as u64)
                    } else if let Some(v) = n.as_f64() {
                        v #op_token (#i as f64)
                    } else {
                        false
                    }
                }
            }
        } else if let Some(f) = limit.as_f64() {
            quote! {
                if let Some(v) = n.as_f64() {
                    v #op_token #f
                } else {
                    false
                }
            }
        } else {
            quote! { true }
        }
    }
    /// Generate multipleOf check for extracted Number value.
    fn generate_multiple_of_check(&self, value: &Value) -> TokenStream {
        // TODO: Arbitrary precision
        if let Some(multiple) = value.as_f64() {
            if multiple.fract() == 0.0 {
                // TODO: No truncation
                let multiple_i64 = multiple as i64;
                quote! {
                    if let Some(v) = n.as_u64() {
                        (v % #multiple_i64 as u64) == 0
                    } else if let Some(v) = n.as_i64() {
                        (v % #multiple_i64) == 0
                    } else if let Some(v) = n.as_f64() {
                        v.fract() == 0.0 && (v % #multiple) == 0.0
                    } else {
                        false
                    }
                }
            } else {
                quote! {
                    if let Some(v) = n.as_f64() {
                        if v == 0.0 {
                            true
                        } else {
                            let quotient = v / #multiple;
                            // Handle overflow: if quotient is infinite, check if v is an integer
                            // and if the reciprocal of the multiple is also an integer.
                            // An integer is a multiple of d iff 1/d is an integer.
                            if quotient.is_infinite() {
                                v.fract() == 0.0 && (1.0 / #multiple).fract() == 0.0
                            } else {
                                quotient.fract() == 0.0
                            }
                        }
                    } else {
                        false
                    }
                }
            }
        } else {
            quote! { true }
        }
    }
    /// Generate items check for extracted array value.
    fn generate_items_check(&self, value: &Value) -> TokenStream {
        if let Value::Array(schemas) = value {
            // Tuple validation - check each position
            let compiled = schemas.iter().enumerate().map(|(idx, schema)| {
                let validation = self.compile_schema(schema);
                quote! {
                    arr.get(#idx).map_or(true, |instance| #validation)
                }
            });
            // TODO: Compile error on empty array
            quote! { ( #(#compiled)&&* ) }
        } else {
            // TODO: check that it is an object
            let compiled = self.compile_schema(value);
            quote! {
                arr.iter().all(|instance| #compiled)
            }
        }
    }
    /// Compile the "uniqueItems" keyword.
    fn compile_unique_items(&self, value: &Value) -> Option<TokenStream> {
        if value.as_bool() == Some(true) {
            // Call the runtime's optimized is_unique helper
            Some(quote! {
                jsonschema::keywords_helpers::unique_items::is_unique(arr)
            })
        } else {
            None
        }
    }
    /// Compile the "additionalItems" keyword.
    fn compile_additional_items(
        &self,
        additional_items: Option<&Value>,
        items_schema: Option<&Value>,
    ) -> Option<TokenStream> {
        let additional_items_val = additional_items?;

        // Determine the tuple length from items schema
        let tuple_len = if let Some(Value::Array(items)) = items_schema {
            items.len()
        } else {
            // If items is not an array (tuple validation), additionalItems has no effect
            return None;
        };

        match additional_items_val {
            Value::Bool(false) => {
                // No additional items allowed beyond tuple length
                Some(quote! {
                    arr.len() <= #tuple_len
                })
            }
            Value::Bool(true) => {
                // All additional items are allowed
                None
            }
            schema => {
                // Additional items must match schema
                let schema_check = self.compile_schema(schema);
                Some(quote! {
                    arr.iter().skip(#tuple_len).all(|instance| #schema_check)
                })
            }
        }
    }
    /// Compile the "patternProperties" keyword.
    fn compile_pattern_properties(&self, value: &Value) -> Option<TokenStream> {
        let Value::Object(patterns) = value else {
            return None;
        };

        if patterns.is_empty() {
            return None;
        }

        let pattern_checks: Vec<_> = patterns
            .iter()
            .filter_map(|(pattern, schema)| {
                let schema_check = self.compile_schema(schema);

                // If the schema is trivially valid (always true), no check is needed
                if schema_check.to_string() == "true" {
                    return None;
                }

                if let Some(prefix) = jsonschema_core::regex::pattern_as_prefix(pattern) {
                    let prefix: &str = prefix.as_ref();
                    Some(quote! {
                        obj.iter()
                            .filter(|(key, _)| key.starts_with(#prefix))
                            .all(|(_, instance)| {
                                #schema_check
                            })
                    })
                } else if let Ok(pattern) = jsonschema_core::regex::to_rust_regex(pattern) {
                    Some(quote! {
                        {
                            static PATTERN: std::sync::LazyLock<regex::Regex> =
                                std::sync::LazyLock::new(|| {
                                    regex::Regex::new(#pattern).expect("Invalid regex pattern")
                                });
                            obj.iter()
                                .filter(|(key, _)| PATTERN.is_match(key))
                                .all(|(_, instance)| {
                                    #schema_check
                                })
                        }
                    })
                } else {
                    None
                }
            })
            .collect();

        if pattern_checks.is_empty() {
            None
        } else {
            Some(quote! {
                ( #(#pattern_checks)&&* )
            })
        }
    }
    /// Build the `_ =>` arm body for a properties match, merging `additionalProperties`
    /// coverage into a single iteration.
    ///
    /// Returns `(statics_to_emit, wildcard_arm_body)`.  Both assume `key_str: &str` and
    /// `instance: &Value` are already in scope at the call-site.
    fn compile_wildcard_arm(
        &self,
        additional_properties: Option<&Value>,
        pattern_properties: Option<&Value>,
    ) -> (Vec<TokenStream>, TokenStream) {
        // Split patternProperties into prefix-optimizable and regex-requiring patterns.
        // These are used to check whether a key (not matched by any named arm) is covered
        // by a pattern property — if so it is not considered "additional".
        let (prefixes, regex_patterns): (Vec<Cow<'_, str>>, Vec<Cow<'_, str>>) = pattern_properties
            .and_then(|v| v.as_object())
            .map(|obj| {
                let mut prefixes = Vec::new();
                let mut regex_patterns = Vec::new();
                for p in obj.keys() {
                    if let Some(prefix) = jsonschema_core::regex::pattern_as_prefix(p) {
                        prefixes.push(prefix);
                    } else if let Ok(regex) = jsonschema_core::regex::to_rust_regex(p) {
                        regex_patterns.push(regex);
                    }
                }
                (prefixes, regex_patterns)
            })
            .unwrap_or_default();

        let mut statics: Vec<TokenStream> = Vec::new();

        let prefix_check: Option<TokenStream> = match prefixes.as_slice() {
            [] => None,
            [p] => {
                let p: &str = p.as_ref();
                Some(quote! { key_str.starts_with(#p) })
            }
            _ => {
                let prefix_strs: Vec<&str> = prefixes.iter().map(Cow::as_ref).collect();
                statics.push(quote! {
                    static PATTERN_PREFIXES: &[&str] = &[#(#prefix_strs),*];
                });
                Some(quote! { PATTERN_PREFIXES.iter().any(|p| key_str.starts_with(p)) })
            }
        };
        let regex_check: Option<TokenStream> = if !regex_patterns.is_empty() {
            statics.push(quote! {
                static PATTERNS: std::sync::LazyLock<Vec<regex::Regex>> =
                    std::sync::LazyLock::new(|| {
                        vec![#(regex::Regex::new(#regex_patterns).expect("Invalid regex")),*]
                    });
            });
            Some(quote! { PATTERNS.iter().any(|p| p.is_match(key_str)) })
        } else {
            None
        };

        // Combine prefix and regex checks into a single pattern coverage expression.
        // Named match arms already act as the "known properties" filter, so KNOWN.contains()
        // is NOT needed here — any key reaching _ is by definition not in properties.
        let pattern_cover_check: Option<TokenStream> = match (prefix_check, regex_check) {
            (None, None) => None,
            (Some(p), None) => Some(p),
            (None, Some(r)) => Some(r),
            (Some(p), Some(r)) => Some(quote! { (#p) || (#r) }),
        };

        let arm_body = match additional_properties {
            // absent or true: any additional key is allowed
            None | Some(Value::Bool(true)) => quote! { true },
            Some(Value::Bool(false)) => match pattern_cover_check {
                // No patterns: any key reaching _ is disallowed
                None => quote! { false },
                // Covered by a pattern: allowed; otherwise disallowed
                Some(check) => check,
            },
            Some(schema) => {
                let schema_check = self.compile_schema(schema);
                if schema_check.to_string() == "true" {
                    // Schema is trivially valid — same as absent/true
                    quote! { true }
                } else {
                    match pattern_cover_check {
                        None => quote! { { #schema_check } },
                        Some(check) => quote! { (#check) || { #schema_check } },
                    }
                }
            }
        };

        (statics, arm_body)
    }

    /// Build a fast key-only precheck for strict objects (`additionalProperties: false`)
    /// with explicit `properties` and no `patternProperties`.
    fn compile_known_keys_precheck(&self, properties: &Map<String, Value>) -> TokenStream {
        let known_props: Vec<&str> = properties.keys().map(String::as_str).collect();
        if known_props.is_empty() {
            quote! { obj.is_empty() }
        } else {
            quote! {
                obj.keys().all(|key| {
                    matches!(key.as_str(), #(#known_props)|*)
                })
            }
        }
    }

    /// Compile the "additionalProperties" keyword.
    fn compile_additional_properties(
        &self,
        additional_properties: Option<&Value>,
        properties: Option<&Value>,
        pattern_properties: Option<&Value>,
    ) -> Option<TokenStream> {
        let additional_properties_val = additional_properties?;

        // Extract known property names from properties
        let known_props: Vec<&str> = properties
            .and_then(|v| v.as_object())
            .map(|obj| obj.keys().map(String::as_str).collect())
            .unwrap_or_default();

        // Split patternProperties into prefix-optimizable and regex-requiring
        let (prefixes, regex_patterns): (Vec<Cow<'_, str>>, Vec<Cow<'_, str>>) = pattern_properties
            .and_then(|v| v.as_object())
            .map(|obj| {
                let mut prefixes = Vec::new();
                let mut regex_patterns = Vec::new();
                for p in obj.keys() {
                    if let Some(prefix) = jsonschema_core::regex::pattern_as_prefix(p) {
                        prefixes.push(prefix);
                    } else if let Ok(regex) = jsonschema_core::regex::to_rust_regex(p) {
                        regex_patterns.push(regex);
                    }
                }
                (prefixes, regex_patterns)
            })
            .unwrap_or_default();

        // Build statics and per-condition fragments that will be assembled into the final
        // expression.  Prefix patterns with a single entry are inlined directly as
        // `starts_with` calls so no static slice or iterator overhead is emitted.
        let mut statics: Vec<TokenStream> = Vec::new();

        if !known_props.is_empty() {
            statics.push(quote! {
                static KNOWN: &[&str] = &[#(#known_props),*];
            });
        }
        let prefix_check: Option<TokenStream> = match prefixes.as_slice() {
            [] => None,
            [p] => {
                let p: &str = p.as_ref();
                Some(quote! { key_str.starts_with(#p) })
            }
            _ => {
                let prefix_strs: Vec<&str> = prefixes.iter().map(Cow::as_ref).collect();
                statics.push(quote! {
                    static PATTERN_PREFIXES: &[&str] = &[#(#prefix_strs),*];
                });
                Some(quote! { PATTERN_PREFIXES.iter().any(|p| key_str.starts_with(p)) })
            }
        };
        let regex_check: Option<TokenStream> = if !regex_patterns.is_empty() {
            statics.push(quote! {
                static PATTERNS: std::sync::LazyLock<Vec<regex::Regex>> =
                    std::sync::LazyLock::new(|| {
                        vec![#(regex::Regex::new(#regex_patterns).expect("Invalid regex")),*]
                    });
            });
            Some(quote! { PATTERNS.iter().any(|p| p.is_match(key_str)) })
        } else {
            None
        };

        match additional_properties_val {
            Value::Bool(false) => {
                // No additional properties allowed.
                // Build an OR of all "this key is covered" sub-expressions.
                let mut covered: Vec<TokenStream> = Vec::new();
                if let Some(check) = &prefix_check {
                    covered.push(check.clone());
                }
                if !known_props.is_empty() {
                    covered.push(quote! { KNOWN.contains(&key_str) });
                }
                if let Some(check) = &regex_check {
                    covered.push(check.clone());
                }

                if covered.is_empty() {
                    Some(quote! { obj.is_empty() })
                } else {
                    Some(quote! {
                        {
                            #(#statics)*
                            obj.keys().all(|key| {
                                let key_str = key.as_str();
                                #(#covered)||*
                            })
                        }
                    })
                }
            }
            Value::Bool(true) => None,
            schema => {
                // Additional properties must match schema.
                // Build an AND of all "this key is NOT covered" sub-expressions for the filter.
                let schema_check = self.compile_schema(schema);

                let mut excluded: Vec<TokenStream> = Vec::new();
                if let Some(check) = &prefix_check {
                    excluded.push(quote! { !(#check) });
                }
                if !known_props.is_empty() {
                    excluded.push(quote! { !KNOWN.contains(&key_str) });
                }
                if let Some(check) = &regex_check {
                    excluded.push(quote! { !(#check) });
                }

                if excluded.is_empty() {
                    Some(quote! {
                        obj.values().all(|instance| #schema_check)
                    })
                } else {
                    Some(quote! {
                        {
                            #(#statics)*
                            obj.iter()
                                .filter(|(key, _)| {
                                    let key_str = key.as_str();
                                    #(#excluded)&&*
                                })
                                .all(|(_, instance)| {
                                    #schema_check
                                })
                        }
                    })
                }
            }
        }
    }
    /// Compile the "dependencies" keyword.
    fn compile_dependencies(&self, value: &Value) -> Option<TokenStream> {
        let Value::Object(deps) = value else {
            return None;
        };

        if deps.is_empty() {
            return None;
        }

        let checks: Vec<_> = deps
            .iter()
            .map(|(prop, dep)| {
                match dep {
                    Value::Array(required_props) => {
                        // Property dependencies: if prop exists, all required_props must exist
                        let props: Vec<&str> =
                            required_props.iter().filter_map(|v| v.as_str()).collect();

                        if props.is_empty() {
                            // Empty array means no additional requirements
                            quote! { true }
                        } else {
                            quote! {
                                if obj.contains_key(#prop) {
                                    #(obj.contains_key(#props))&&*
                                } else {
                                    true
                                }
                            }
                        }
                    }
                    schema => {
                        // Schema dependencies: if prop exists, instance must validate against schema
                        // Handles both object schemas and boolean schemas (Draft 6+)
                        let schema_check = self.compile_schema(schema);
                        quote! {
                            if obj.contains_key(#prop) {
                                #schema_check
                            } else {
                                true
                            }
                        }
                    }
                }
            })
            .collect();

        if checks.is_empty() {
            None
        } else {
            Some(quote! {
                ( #(#checks)&&* )
            })
        }
    }
    /// Compile the "contains" keyword
    /// Array must contain at least one item matching the schema
    fn compile_contains(&self, value: &Value) -> TokenStream {
        let schema_check = self.compile_schema(value);
        quote! {
            arr.iter().any(|instance| #schema_check)
        }
    }
    /// Compile the "propertyNames" keyword
    /// All property names must validate against the schema
    fn compile_property_names(&self, value: &Value) -> TokenStream {
        // Property names are always strings, so when the schema only checks
        // string-specific keywords (optionally with `type: "string"`), we can
        // bind `s = key` directly and skip the Value::String(key.clone()) wrapping.
        if let Value::Object(schema) = value {
            let only_string_keywords = schema.iter().all(|(k, v)| {
                matches!(k.as_str(), "minLength" | "maxLength" | "pattern" | "format")
                    || (k == "type" && v.as_str() == Some("string"))
            });
            let has_string_keywords = schema.contains_key("minLength")
                || schema.contains_key("maxLength")
                || schema.contains_key("pattern")
                || schema.contains_key("format");
            if only_string_keywords && has_string_keywords {
                let string_check = self.compile_for_string(schema);
                return quote! {
                    obj.keys().all(|s| { #string_check })
                };
            }
        }
        let schema_check = self.compile_schema(value);
        quote! {
            obj.keys().all(|key| {
                (|instance: &serde_json::Value| #schema_check)(&serde_json::Value::String(key.clone()))
            })
        }
    }
    /// Check if formats should be validated by default for this draft.
    fn validates_formats_by_default(&self) -> bool {
        // TODO: It should be a method on ctx
        // Match runtime behavior exactly
        matches!(self.draft, Draft::Draft4 | Draft::Draft6 | Draft::Draft7)
    }
    /// Compile the "format" keyword.
    fn compile_format(&self, value: &Value) -> Option<TokenStream> {
        // Only validate if formats enabled by default for this draft
        if !self.validates_formats_by_default() {
            return None;
        }

        let format_name = value.as_str()?;

        // Map format names to validation functions
        let validation_call = match format_name {
            "date" => quote! { jsonschema::keywords_helpers::format::is_valid_date(s) },
            "date-time" => quote! { jsonschema::keywords_helpers::format::is_valid_datetime(s) },
            "time" => quote! { jsonschema::keywords_helpers::format::is_valid_time(s) },
            "duration" => quote! { jsonschema::keywords_helpers::format::is_valid_duration(s) },
            "email" => quote! { jsonschema::keywords_helpers::format::is_valid_email(s, None) },
            "idn-email" => {
                quote! { jsonschema::keywords_helpers::format::is_valid_idn_email(s, None) }
            }
            "hostname" => quote! { jsonschema::keywords_helpers::format::is_valid_hostname(s) },
            "idn-hostname" => {
                quote! { jsonschema::keywords_helpers::format::is_valid_idn_hostname(s) }
            }
            "ipv4" => quote! { jsonschema::keywords_helpers::format::is_valid_ipv4(s) },
            "ipv6" => quote! { jsonschema::keywords_helpers::format::is_valid_ipv6(s) },
            "uri" => quote! { jsonschema::keywords_helpers::format::is_valid_uri(s) },
            "uri-reference" => {
                quote! { jsonschema::keywords_helpers::format::is_valid_uri_reference(s) }
            }
            "iri" => quote! { jsonschema::keywords_helpers::format::is_valid_iri(s) },
            "iri-reference" => {
                quote! { jsonschema::keywords_helpers::format::is_valid_iri_reference(s) }
            }
            "uri-template" => {
                quote! { jsonschema::keywords_helpers::format::is_valid_uri_template(s) }
            }
            "json-pointer" => {
                quote! { jsonschema::keywords_helpers::format::is_valid_json_pointer(s) }
            }
            "relative-json-pointer" => {
                quote! { jsonschema::keywords_helpers::format::is_valid_relative_json_pointer(s) }
            }
            "uuid" => quote! { jsonschema::keywords_helpers::format::is_valid_uuid(s) },
            // Unknown or unsupported format - validation succeeds
            _ => return None,
        };

        Some(validation_call)
    }
    /// Compile the "allOf" keyword.
    fn compile_all_of(&self, value: &Value) -> TokenStream {
        // TODO: Compile error on wrong type
        if let Some(schemas) = value.as_array() {
            let compiled = schemas.iter().map(|schema| self.compile_schema(schema));
            // TODO: Compile error on empty array
            quote! { (#(#compiled)&&*) }
        } else {
            quote! { true }
        }
    }
    /// Compile the "anyOf" keyword.
    fn compile_any_of(&self, value: &Value) -> TokenStream {
        if let Some(schemas) = value.as_array() {
            let compiled = schemas.iter().map(|schema| self.compile_schema(schema));
            // TODO: Compile error on empty array
            quote! { (#(#compiled)||*) }
        } else {
            quote! { true }
        }
    }
    /// Compile the "oneOf" keyword.
    fn compile_one_of(&self, value: &Value) -> TokenStream {
        if let Some(schemas) = value.as_array() {
            if schemas.is_empty() {
                return quote! { false };
            }

            // Try to detect a discriminator-like key (required string const/enum) shared by branches.
            // When present, branch validation is gated by a cheap string equality/set-membership check.
            let discriminator_plan = self.one_of_discriminator_plan(schemas);

            let checks = schemas.iter().enumerate().map(|(idx, schema)| {
                let validation = self.compile_schema(schema);
                let branch_validation = match &discriminator_plan {
                    Some((_, branch_discriminators)) => {
                        if let Some(discriminator_values) = &branch_discriminators[idx] {
                            if discriminator_values.len() == 1 {
                                let discriminator_value = &discriminator_values[0];
                                quote! {
                                    {
                                        let __one_of_candidate = match __one_of_discriminator {
                                            Some(value) => value == #discriminator_value,
                                            None => true,
                                        };
                                        __one_of_candidate && { #validation }
                                    }
                                }
                            } else {
                                let allowed_values = discriminator_values.iter();
                                quote! {
                                    {
                                        let __one_of_candidate = match __one_of_discriminator {
                                            Some(value) => matches!(value, #(#allowed_values)|*),
                                            None => true,
                                        };
                                        __one_of_candidate && { #validation }
                                    }
                                }
                            }
                        } else {
                            validation
                        }
                    }
                    None => validation,
                };

                quote! {
                    if #branch_validation {
                        if matched {
                            false
                        } else {
                            matched = true;
                            true
                        }
                    } else {
                        true
                    }
                }
            });

            let discriminator_init = if let Some((discriminator_key, _)) = &discriminator_plan {
                quote! {
                    let __one_of_discriminator = match instance {
                        serde_json::Value::Object(obj) => obj.get(#discriminator_key).and_then(serde_json::Value::as_str),
                        _ => None,
                    };
                }
            } else {
                quote! {}
            };

            // We short-circuit as soon as a second branch validates.
            quote! {
                {
                    #discriminator_init
                    let mut matched = false;
                    ( #(#checks)&&* ) && matched
                }
            }
        } else {
            quote! { true }
        }
    }

    /// Build a discriminator plan for oneOf branches when they share a required
    /// string const/enum property (for example, `resourceType` in FHIR).
    fn one_of_discriminator_plan(
        &self,
        schemas: &[Value],
    ) -> Option<(String, Vec<Option<Vec<String>>>)> {
        let branch_discriminators: Vec<HashMap<String, Vec<String>>> = schemas
            .iter()
            .map(|schema| self.extract_required_string_discriminators_for_one_of(schema))
            .collect();

        // key -> (coverage_count, distinct_values, total_branch_cardinality)
        let mut stats: HashMap<String, (usize, HashSet<String>, usize)> = HashMap::new();
        for branch in &branch_discriminators {
            for (key, values) in branch {
                let entry = stats.entry(key.clone()).or_default();
                entry.0 += 1;
                entry.2 += values.len();
                for value in values {
                    entry.1.insert(value.clone());
                }
            }
        }

        let mut best: Option<(String, usize, usize, usize)> = None;
        for (key, (coverage, values, total_cardinality)) in stats {
            let distinct_values = values.len();
            // Need at least two covered branches and two distinct values to be useful.
            if coverage < 2 || distinct_values < 2 {
                continue;
            }
            match &best {
                None => best = Some((key, coverage, distinct_values, total_cardinality)),
                Some((_, best_coverage, best_distinct, best_total_cardinality)) => {
                    if coverage > *best_coverage
                        || (coverage == *best_coverage
                            && (distinct_values > *best_distinct
                                || (distinct_values == *best_distinct
                                    && total_cardinality < *best_total_cardinality)))
                    {
                        best = Some((key, coverage, distinct_values, total_cardinality));
                    }
                }
            }
        }

        let (key, _, _, _) = best?;
        let per_branch_values = branch_discriminators
            .iter()
            .map(|branch| branch.get(&key).cloned())
            .collect();
        Some((key, per_branch_values))
    }

    /// Extract `required` string const/enum discriminator properties from a branch schema.
    /// Resolves top-level $ref chains to make oneOf branch analysis effective.
    fn extract_required_string_discriminators_for_one_of(
        &self,
        schema: &Value,
    ) -> HashMap<String, Vec<String>> {
        let resolved = self.resolve_top_level_ref_for_one_of_analysis(schema);
        let Value::Object(obj) = resolved.as_ref() else {
            return HashMap::new();
        };

        let required: HashSet<&str> = obj
            .get("required")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let Some(properties) = obj.get("properties").and_then(Value::as_object) else {
            return HashMap::new();
        };

        let mut out = HashMap::new();
        for name in required {
            let Some(Value::Object(property_schema)) = properties.get(name) else {
                continue;
            };
            if let Some(Value::String(const_value)) = property_schema.get("const") {
                out.insert(name.to_string(), vec![const_value.clone()]);
                continue;
            }
            if let Some(Value::Array(enum_values)) = property_schema.get("enum") {
                let mut values: Vec<String> = enum_values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                // Only use this discriminator when all enum members are strings.
                if values.len() == enum_values.len() && !values.is_empty() {
                    values.sort_unstable();
                    values.dedup();
                    out.insert(name.to_string(), values);
                }
            }
        }
        out
    }

    /// Resolve a short top-level `$ref` chain for branch-shape analysis.
    fn resolve_top_level_ref_for_one_of_analysis<'b>(&self, schema: &'b Value) -> Cow<'b, Value> {
        let mut current = Cow::Borrowed(schema);
        for _ in 0..8 {
            let Value::Object(obj) = current.as_ref() else {
                break;
            };
            let Some(reference) = obj.get("$ref").and_then(Value::as_str) else {
                break;
            };
            let Ok(resolved) = self.resolve_ref(reference) else {
                break;
            };
            current = Cow::Owned(resolved.schema);
        }
        current
    }
    /// Compile the "not" keyword.
    fn compile_not(&self, value: &Value) -> TokenStream {
        let compiled = self.compile_schema(value);
        quote! { !(#compiled) }
    }

    /// Compile "if", "then", "else" keywords.
    fn compile_if_then_else(
        &self,
        parent: &Map<String, Value>,
        if_schema: &Value,
    ) -> Option<TokenStream> {
        let then_schema = parent.get("then");
        let else_schema = parent.get("else");

        match (then_schema, else_schema) {
            (Some(then_val), Some(else_val)) => {
                // if/then/else: if condition is true, validate with then, else validate with else
                let if_check = self.compile_schema(if_schema);
                let then_check = self.compile_schema(then_val);
                let else_check = self.compile_schema(else_val);
                Some(quote! {
                    if #if_check {
                        #then_check
                    } else {
                        #else_check
                    }
                })
            }
            (Some(then_val), None) => {
                // if/then: if condition is true, validate with then, else true
                let if_check = self.compile_schema(if_schema);
                let then_check = self.compile_schema(then_val);
                Some(quote! {
                    if #if_check {
                        #then_check
                    } else {
                        true
                    }
                })
            }
            (None, Some(else_val)) => {
                // if/else: if condition is true, return true, else validate with else
                let if_check = self.compile_schema(if_schema);
                let else_check = self.compile_schema(else_val);
                Some(quote! {
                    if #if_check {
                        true
                    } else {
                        #else_check
                    }
                })
            }
            (None, None) => None, // No then or else, nothing to do
        }
    }

    /// Compile a $ref keyword.
    fn compile_ref(&self, value: &Value) -> TokenStream {
        let Some(reference) = value.as_str() else {
            // TODO: Compile error
            // Invalid $ref - should be a string
            return quote! { false };
        };

        // TODO: Is it correct?
        let Ok(resolved) = self.resolve_ref(reference) else {
            // Can't resolve at compile time (external ref, missing definition, etc.)
            // Return false so validation fails at runtime with a clear error
            // TODO: Better compile-time error reporting for truly invalid schemas
            return quote! { false };
        };

        // Get or create function for this location, passing the resolved base URI
        let func_name =
            self.get_or_create_function(&resolved.location, &resolved.schema, &resolved.base_uri);
        let func_ident = format_ident!("{}", func_name);

        // Generate function call with Self:: prefix for impl-level method
        quote! { Self::#func_ident(instance) }
    }

    /// Resolve a reference using the Registry.
    fn resolve_ref(&self, reference: &str) -> Result<ResolvedRef, String> {
        // TODO: wtf, use proper error handling
        let registry = self
            .registry
            .as_ref()
            .ok_or_else(|| "No registry available".to_string())?;

        // Use current_base_uri if set (for nested schemas), otherwise use base_uri
        let base_uri = self
            .current_base_uri
            .borrow()
            .as_ref()
            .cloned()
            .or_else(|| self.base_uri.clone())
            .ok_or_else(|| "No base URI available".to_string())?;

        let resolver = registry.resolver((*base_uri).clone());
        let resolved = resolver
            .lookup(reference)
            .map_err(|e| format!("Failed to resolve {reference}: {e}"))?;

        // Get the resolved schema's base URI for resolving nested references
        let resolved_base_uri = resolved.resolver().base_uri().clone();

        // For local references (fragments), use the reference itself as the location key
        // For external references, use the resolved URI
        let location_key = if reference.starts_with('#') {
            // TODO: Is it correct?
            // Local reference - use the reference as-is for uniqueness
            format!("{base_uri}{reference}")
        } else {
            // External reference - use resolved URI
            resolved_base_uri.to_string()
        };
        let (contents, _, _) = resolved.into_inner();

        // TODO: Why clone?
        Ok(ResolvedRef {
            schema: contents.clone(),
            location: location_key,
            base_uri: resolved_base_uri,
        })
    }
    /// Get or create a function for a reference location.
    fn get_or_create_function(
        &self,
        location: &str,
        schema: &Value,
        schema_base_uri: &Arc<Uri<String>>,
    ) -> String {
        // Check if function already exists
        {
            let funcs = self.location_to_function.borrow();
            if let Some(func_info) = funcs.get(location) {
                return func_info.name.clone();
            }
        }

        // Create new function
        let func_id = {
            let mut counter = self.ref_counter.borrow_mut();
            let id = *counter;
            *counter += 1;
            id
        };
        let func_name = format!("validate_ref_{func_id}");

        // Store function info
        {
            let mut funcs = self.location_to_function.borrow_mut();
            funcs.insert(
                location.to_string(),
                FunctionInfo {
                    name: func_name.clone(),
                },
            );
        }

        // Check for recursion
        let is_recursive = self.seen.borrow().contains(location);

        if is_recursive {
            // Recursive - create stub that will be filled later
            // For now, just return true to avoid infinite recursion during compilation
            self.add_function_body(&func_name, quote! { true });
        } else {
            // Mark as seen and compile with the schema's base URI
            self.seen.borrow_mut().insert(location.to_string());

            // Save current base URI and set new one for this schema
            let prev_base_uri = self.current_base_uri.borrow().clone();
            *self.current_base_uri.borrow_mut() = Some(Arc::clone(schema_base_uri));

            let body = self.compile_schema(schema);

            // Restore previous base URI
            *self.current_base_uri.borrow_mut() = prev_base_uri;
            self.seen.borrow_mut().remove(location);

            self.add_function_body(&func_name, body);
        }

        func_name
    }

    /// Add a function body to the collection.
    fn add_function_body(&self, name: &str, body: TokenStream) {
        let mut bodies = self.function_bodies.borrow_mut();
        bodies.insert(name.to_string(), FunctionBody { body });
    }
}

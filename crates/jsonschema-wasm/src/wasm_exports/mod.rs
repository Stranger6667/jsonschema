mod retriever;

use crate::{
    errors::Error,
    options::{draft_from_id, Options, DRAFTS},
};
use serde::Serialize;
use serde_json::Value;
use wasm_bindgen::prelude::*;

fn to_js_err<E: std::fmt::Display>(e: E) -> JsValue {
    JsValue::from_str(&e.to_string())
}

// `serde_wasm_bindgen::to_value`'s default serializer renders JSON objects as ES `Map`s
// and leaks `arbitrary-precision`'s internal number token as a plain object. Round-trip
// through a JSON string instead: `serde_json::to_string` resolves the number token to a
// real numeric literal, and `JSON.parse` yields plain objects/arrays/numbers throughout.
fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    let s = serde_json::to_string(value).map_err(to_js_err)?;
    js_sys::JSON::parse(&s)
}

fn parse_options(options: JsValue) -> Result<Options, JsValue> {
    if options.is_undefined() || options.is_null() {
        Ok(Options::default())
    } else {
        serde_wasm_bindgen::from_value(options).map_err(to_js_err)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidateResult {
    valid: bool,
    errors: Vec<Error>,
    ms: f64,
}

#[derive(Serialize)]
struct DraftEntry {
    id: &'static str,
    label: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TransformResult {
    output: Value,
    ms: f64,
}

#[wasm_bindgen(skip_typescript)]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[wasm_bindgen(skip_typescript)]
pub fn drafts() -> Result<JsValue, JsValue> {
    let list: Vec<DraftEntry> = DRAFTS
        .iter()
        .map(|(id, label)| DraftEntry { id, label })
        .collect();
    to_js(&list)
}

fn build_options(
    options: &Options,
) -> Result<
    jsonschema::ValidationOptions<'static, std::sync::Arc<dyn jsonschema::AsyncRetrieve>>,
    JsValue,
> {
    let mut builder = jsonschema::async_options();
    if let Some(id) = options.draft.as_deref() {
        let draft = draft_from_id(id).ok_or_else(|| to_js_err(format!("unknown draft `{id}`")))?;
        builder = builder.with_draft(draft);
    }
    Ok(builder
        .should_validate_formats(options.format_assertions)
        .should_ignore_unknown_formats(options.ignore_unknown_formats)
        .with_retriever(retriever::FetchRetriever))
}

async fn build_validator(
    schema: &Value,
    options: &Options,
) -> Result<jsonschema::Validator, JsValue> {
    build_options(options)?
        .build(schema)
        .await
        .map_err(to_js_err)
}

#[wasm_bindgen(skip_typescript)]
pub async fn validate(
    schema: String,
    instance: String,
    options: JsValue,
) -> Result<JsValue, JsValue> {
    let options = parse_options(options)?;
    let schema: Value = serde_json::from_str(&schema).map_err(to_js_err)?;
    let instance: Value = serde_json::from_str(&instance).map_err(to_js_err)?;
    let t0 = js_sys::Date::now();
    let validator = build_validator(&schema, &options).await?;
    let errors: Vec<Error> = validator
        .iter_errors(&instance)
        .map(|e| Error::from(&e))
        .collect();
    let ms = js_sys::Date::now() - t0;
    to_js(&ValidateResult {
        valid: errors.is_empty(),
        errors,
        ms,
    })
}

#[wasm_bindgen(skip_typescript)]
pub async fn bundle(schema: String, options: JsValue) -> Result<JsValue, JsValue> {
    let options = parse_options(options)?;
    let schema: Value = serde_json::from_str(&schema).map_err(to_js_err)?;
    let t0 = js_sys::Date::now();
    let output = build_options(&options)?
        .bundle(&schema)
        .await
        .map_err(to_js_err)?;
    let ms = js_sys::Date::now() - t0;
    to_js(&TransformResult { output, ms })
}

#[wasm_bindgen(skip_typescript)]
pub async fn dereference(schema: String, options: JsValue) -> Result<JsValue, JsValue> {
    let options = parse_options(options)?;
    let schema: Value = serde_json::from_str(&schema).map_err(to_js_err)?;
    let t0 = js_sys::Date::now();
    let output = build_options(&options)?
        .dereference(&schema)
        .await
        .map_err(to_js_err)?;
    let ms = js_sys::Date::now() - t0;
    to_js(&TransformResult { output, ms })
}

#[wasm_bindgen(typescript_custom_section)]
const TS: &'static str = r#"
export type PathSegment = string | number;

export interface DraftEntry { id: string; label: string; }

export interface Options {
  draft?: string;
  formatAssertions?: boolean;
  ignoreUnknownFormats?: boolean;
}

export interface ValidationError {
  message: string;
  instancePath: PathSegment[];
  schemaPath: PathSegment[];
  kind: { type: string; [k: string]: unknown };
}

export interface ValidateResult { valid: boolean; errors: ValidationError[]; ms: number; }
export interface TransformResult { output: unknown; ms: number; }

export function version(): string;
export function drafts(): DraftEntry[];
export function validate(schema: string, instance: string, options?: Options): Promise<ValidateResult>;
export function bundle(schema: string, options?: Options): Promise<TransformResult>;
export function dereference(schema: string, options?: Options): Promise<TransformResult>;
"#;

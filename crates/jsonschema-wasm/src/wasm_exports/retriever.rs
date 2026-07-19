use jsonschema::AsyncRetrieve;
use referencing::Uri;
use serde_json::Value;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, Response};

pub(crate) struct FetchRetriever;

#[async_trait::async_trait(?Send)]
impl AsyncRetrieve for FetchRetriever {
    async fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let url = uri.as_str();
        let init = RequestInit::new();
        init.set_method("GET");
        let request = Request::new_with_str_and_init(url, &init).map_err(|e| js_err(&e))?;
        let window = web_sys::window().ok_or_else(|| boxed("no window"))?;
        let resp_value = JsFuture::from(window.fetch_with_request(&request))
            .await
            .map_err(|e| js_err(&e))?;
        let resp: Response = resp_value.dyn_into().map_err(|_| boxed("not a Response"))?;
        if !resp.ok() {
            return Err(boxed(&format!("fetch {url} failed: {}", resp.status())));
        }
        let text = JsFuture::from(resp.text().map_err(|e| js_err(&e))?)
            .await
            .map_err(|e| js_err(&e))?;
        let text = text.as_string().ok_or_else(|| boxed("response not text"))?;
        Ok(serde_json::from_str(&text)?)
    }
}

fn js_err(v: &wasm_bindgen::JsValue) -> Box<dyn std::error::Error + Send + Sync> {
    boxed(&format!("{v:?}"))
}

fn boxed(msg: &str) -> Box<dyn std::error::Error + Send + Sync> {
    msg.to_string().into()
}

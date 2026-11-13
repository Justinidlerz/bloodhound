//! HTTP requests run in the Node host; Rust retains redirects, retries and access checks.

use crate::loader::HttpResponse;
use crate::model::Options;
use napi::Status;
use napi::bindgen_prelude::{Function, Object, Promise};
use napi::threadsafe_function::ThreadsafeFunction;
use serde_json::json;
use std::sync::Arc;

pub type HostHttp = ThreadsafeFunction<String, Promise<String>, String, Status, false, true>;

pub fn from_options(options: &Object) -> napi::Result<Option<Arc<HostHttp>>> {
    options
        .get::<Function<String, Promise<String>>>("__bloodhoundHttp")?
        .map(|callback| {
            callback
                .build_threadsafe_function::<String>()
                .weak::<true>()
                .build()
                .map(Arc::new)
        })
        .transpose()
}

pub fn request(http: &HostHttp, url: &url::Url, options: &Options) -> Result<HttpResponse, String> {
    let host = &url[url::Position::BeforeHost..url::Position::AfterPort];
    let request = json!({
        "url": url.as_str(),
        "headers": options.request_headers.get(host),
        "timeout": options.budgets.resource_timeout_ms.max(1),
        "maxBytes": options.budgets.max_resource_bytes,
        "userAgent": concat!("bloodhound/", env!("CARGO_PKG_VERSION")),
    })
    .to_string();
    let response =
        futures::executor::block_on(async { http.call_async_catch(request).await?.await })
            .map_err(|error| error.reason)?;
    serde_json::from_str(&response).map_err(|error| format!("invalid-http-response: {error}"))
}

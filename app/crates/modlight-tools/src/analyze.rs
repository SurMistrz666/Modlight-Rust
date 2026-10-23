//! Analyze image (`AnalyzeImageService` and its providers): the picture and a prompt sent
//! to OpenAI (the Responses API or, "legacy", Chat Completions), Google Gemini or
//! OpenRouter with the user's own key, and the answer's text taken out. The requests are
//! built here as plain data (tests check them field by field), sent by [`send`] only when
//! the user runs the analysis or presses "Test connection" / "Load models".
//!
//! The keys are not part of these settings on disk: the app keeps them in the system's
//! credential store (Credential Manager on Windows, the Keychain on macOS; upstream
//! encrypted them with DPAPI inside the settings file).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use image::{ImageFormat, RgbaImage};
use modlight_core::l10n::{Arg, format, t, tools as tl};
use serde_json::{Value, json};

use crate::settings::{AiOptions, AiProvider};

/// `OpenAIModels`, `GeminiModels`, `OpenRouterModels` of the options window.
pub const OPENAI_MODELS: [&str; 5] = ["gpt-5.2", "gpt-5.1", "gpt-5", "gpt-5-mini", "gpt-5-nano"];
pub const GEMINI_MODELS: [&str; 4] = [
    "gemini-2.0-flash",
    "gemini-2.0-flash-lite",
    "gemini-1.5-flash",
    "gemini-1.5-pro",
];
pub const OPENROUTER_MODELS: [&str; 3] = [
    "openai/gpt-4o",
    "anthropic/claude-3.5-sonnet",
    "google/gemini-pro-1.5",
];
/// `ReasoningEfforts`, `VerbosityLevels`.
pub const REASONING_EFFORTS: [&str; 4] = ["minimal", "low", "medium", "high"];
pub const VERBOSITY_LEVELS: [&str; 3] = ["low", "medium", "high"];

/// `AnalyzeImageViewModel.PresetPrompts`.
pub fn preset_prompts() -> [&'static str; 4] {
    [
        t(tl::AnalyzeImageViewModel_What_is_in_this_image),
        t(tl::AnalyzeImageViewModel_Thoroughly_describe_this_image),
        t(tl::AnalyzeImageViewModel_Transcribe_image_text),
        t(tl::AnalyzeImageViewModel_Translate_text_into_English),
    ]
}

/// `LocalizedDisplayConverter` for the providers.
pub fn provider_name(provider: AiProvider) -> &'static str {
    t(match provider {
        AiProvider::OpenAI => tl::LocalizedDisplayConverter_OpenAI,
        AiProvider::Gemini => tl::LocalizedDisplayConverter_Gemini,
        AiProvider::OpenRouter => tl::LocalizedDisplayConverter_OpenRouter,
        AiProvider::OpenAILegacy => tl::LocalizedDisplayConverter_OpenAI_legacy,
    })
}

/// `LocalizedDisplayConverter` for the effort and verbosity levels (others as they are).
pub fn level_name(level: &str) -> String {
    match level {
        "minimal" => t(tl::LocalizedDisplayConverter_Minimal).to_string(),
        "low" => t(tl::LocalizedDisplayConverter_Low).to_string(),
        "medium" => t(tl::LocalizedDisplayConverter_Medium).to_string(),
        "high" => t(tl::LocalizedDisplayConverter_High).to_string(),
        other => other.to_string(),
    }
}

/// `OpenAPIKeyPage`.
pub fn api_key_page(provider: AiProvider) -> &'static str {
    match provider {
        AiProvider::OpenAI | AiProvider::OpenAILegacy => "https://platform.openai.com/api-keys",
        AiProvider::Gemini => "https://aistudio.google.com/app/apikey",
        AiProvider::OpenRouter => "https://openrouter.ai/keys",
    }
}

/// Where the providers answer. Upstream's addresses by default; tests point them at a
/// stub on 127.0.0.1 (the OpenAI one is also the user's "Custom base URL").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    pub openai: String,
    pub gemini: String,
    pub open_router: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            openai: "https://api.openai.com".into(),
            gemini: "https://generativelanguage.googleapis.com".into(),
            open_router: "https://openrouter.ai".into(),
        }
    }
}

/// The picture analysed: a file (`imagePath`) or a captured region (`imageData`).
#[derive(Clone, Debug)]
pub enum Picture {
    File(PathBuf),
    Image(Arc<RgbaImage>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// One request, ready to send. Its `Debug` hides the key (in the header or the address).
#[derive(Clone, PartialEq)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub bearer: Option<String>,
    /// A JSON body (`application/json; charset=utf-8`).
    pub body: Option<String>,
}

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let url = match self.url.find("key=") {
            Some(at) => format!("{}key=…", &self.url[..at]),
            None => self.url.clone(),
        };
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &url)
            .field("bearer", &self.bearer.as_ref().map(|_| "…"))
            .field("body_len", &self.body.as_ref().map(|b| b.len()))
            .finish()
    }
}

/// An answer: status code, reason phrase, body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub reason: String,
    pub body: String,
}

impl HttpResponse {
    pub fn success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// `EnsureSuccessStatusCode`'s message.
    fn status_error(&self) -> String {
        format!(
            "Response status code does not indicate success: {} ({}).",
            self.status, self.reason
        )
    }
}

/// `URLHelpers.CombineURL`: one slash between the two parts.
pub fn combine_url(url1: &str, url2: &str) -> String {
    if url1.is_empty() {
        return url2.to_string();
    }
    if url2.is_empty() {
        return url1.to_string();
    }
    let url1 = url1.strip_suffix('/').unwrap_or(url1);
    let url2 = url2.strip_prefix('/').unwrap_or(url2);
    format!("{url1}/{url2}")
}

fn openai_base<'a>(options: &'a AiOptions, endpoints: &'a Endpoints) -> &'a str {
    match options.openai_custom_url.as_deref() {
        Some(custom) if !custom.is_empty() => custom,
        _ => &endpoints.openai,
    }
}

/// The OpenAI address of `path`: the base (custom or default) unless it already ends
/// with the path.
fn openai_url(options: &AiOptions, endpoints: &Endpoints, path: &str) -> String {
    let url = openai_base(options, endpoints);
    if url.ends_with(path) {
        url.to_string()
    } else {
        combine_url(url, path)
    }
}

/// `OpenAIProvider`'s effort: GPT-5.x models know "none" instead of "minimal".
pub fn reasoning_effort(model: &str, effort: Option<&str>) -> String {
    if model.to_lowercase().starts_with("gpt-5.") {
        match effort {
            None => "none".into(),
            Some(e) if e.eq_ignore_ascii_case("minimal") => "none".into(),
            Some(e) => e.to_string(),
        }
    } else {
        effort.unwrap_or("minimal").to_string()
    }
}

fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// `ImageHelpers.LoadImage` / `ByteArrayToBitmap`.
fn decode(picture: &Picture) -> Result<RgbaImage, String> {
    match picture {
        Picture::File(path) => crate::images::load(path).map_err(|e| e.to_string()),
        Picture::Image(image) => Ok((**image).clone()),
    }
}

/// The picture as the provider takes it: OpenAI a JPEG (quality 90) data URI; Gemini and
/// OpenRouter the file's own bytes, or a captured region as PNG, in Base64.
pub fn image_payload(provider: AiProvider, picture: &Picture) -> Result<String, String> {
    match provider {
        AiProvider::OpenAI | AiProvider::OpenAILegacy => {
            let image = decode(picture)?;
            let jpeg =
                crate::images::encode(&image, ImageFormat::Jpeg).map_err(|e| e.to_string())?;
            Ok(format!("data:image/jpeg;base64,{}", base64(&jpeg)))
        }
        AiProvider::Gemini | AiProvider::OpenRouter => match picture {
            Picture::File(path) => std::fs::read(path)
                .map(|bytes| base64(&bytes))
                .map_err(|e| e.to_string()),
            Picture::Image(image) => Ok(base64(&crate::images::png(image))),
        },
    }
}

/// The prompt: empty means upstream's default question.
fn prompt(options: &AiOptions) -> String {
    if options.input.is_empty() {
        t(tl::AnalyzeImageViewModel_What_is_in_this_image).to_string()
    } else {
        options.input.clone()
    }
}

/// `IAIProvider.AnalyzeImage`'s request for the options' provider, with `image` from
/// [`image_payload`].
pub fn analyze_request(options: &AiOptions, image: &str, endpoints: &Endpoints) -> HttpRequest {
    let key = |k: &Option<String>| k.clone().unwrap_or_default();
    match options.provider {
        AiProvider::OpenAI => {
            let body = json!({
                "model": options.openai_model,
                "reasoning": {
                    "effort": reasoning_effort(&options.openai_model, Some(&options.openai_reasoning_effort)),
                },
                "input": [{
                    "role": "user",
                    "content": [
                        { "type": "input_text", "text": prompt(options) },
                        { "type": "input_image", "image_url": image },
                    ],
                }],
                "text": { "verbosity": options.openai_verbosity },
                "store": false,
            });
            HttpRequest {
                method: Method::Post,
                url: openai_url(options, endpoints, "/v1/responses"),
                bearer: Some(key(&options.openai_api_key)),
                body: Some(body.to_string()),
            }
        }
        AiProvider::OpenAILegacy => {
            let body = json!({
                "model": options.openai_model,
                "messages": [{
                    "role": "user",
                    "content": [
                        { "type": "text", "text": prompt(options) },
                        { "type": "image_url", "image_url": { "url": image } },
                    ],
                }],
            });
            HttpRequest {
                method: Method::Post,
                url: openai_url(options, endpoints, "/v1/chat/completions"),
                bearer: Some(key(&options.openai_api_key)),
                body: Some(body.to_string()),
            }
        }
        AiProvider::Gemini => {
            // Upstream sends the prompt as it is (no default) to Gemini and OpenRouter.
            let body = json!({
                "contents": [{
                    "parts": [
                        { "text": options.input },
                        { "inline_data": { "mime_type": "image/png", "data": image } },
                    ],
                }],
            });
            HttpRequest {
                method: Method::Post,
                url: format!(
                    "{}:generateContent?key={}",
                    combine_url(
                        &endpoints.gemini,
                        &format!(
                            "v1beta/models/{}",
                            modlight_core::tasks::naming::url_encode(&options.gemini_model)
                        )
                    ),
                    modlight_core::tasks::naming::url_encode(&key(&options.gemini_api_key))
                ),
                bearer: None,
                body: Some(body.to_string()),
            }
        }
        AiProvider::OpenRouter => {
            let body = json!({
                "model": options.open_router_model,
                "messages": [{
                    "role": "user",
                    "content": [
                        { "type": "text", "text": options.input },
                        { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{image}") } },
                    ],
                }],
                "max_tokens": 1024,
            });
            HttpRequest {
                method: Method::Post,
                url: combine_url(&endpoints.open_router, "api/v1/chat/completions"),
                bearer: Some(key(&options.open_router_api_key)),
                body: Some(body.to_string()),
            }
        }
    }
}

fn parse_json(body: &str) -> Result<Value, String> {
    serde_json::from_str(body).map_err(|e| e.to_string())
}

/// What a missing member of a `dynamic` answer threw.
const NULL_BINDING: &str = "Cannot perform runtime binding on a null reference";

/// The answer's text (`AnalyzeImage`'s result), or the error the window shows.
pub fn parse_analyze(provider: AiProvider, response: &HttpResponse) -> Result<String, String> {
    match provider {
        AiProvider::OpenAI => {
            if !response.success() {
                return Err(response.status_error());
            }
            let value = parse_json(&response.body)?;
            let outputs = value["output"].as_array().cloned().unwrap_or_default();
            for output in outputs {
                if let Some(text) = output["content"][0]["text"].as_str()
                    && !text.is_empty()
                {
                    return Ok(text.to_string());
                }
            }
            Ok(String::new())
        }
        AiProvider::OpenAILegacy => {
            if !response.success() {
                return Err(response.status_error());
            }
            let value = parse_json(&response.body)?;
            Ok(value["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or_default()
                .to_string())
        }
        AiProvider::Gemini | AiProvider::OpenRouter => {
            if !response.success() {
                let template = if provider == AiProvider::Gemini {
                    tl::GeminiProvider_API_error
                } else {
                    tl::OpenRouterProvider_API_error
                };
                return Err(format(t(template), &[Arg::from(response.body.as_str())]));
            }
            let value = parse_json(&response.body)?;
            let text = if provider == AiProvider::Gemini {
                &value["candidates"][0]["content"]["parts"][0]["text"]
            } else {
                &value["choices"][0]["message"]["content"]
            };
            match text {
                Value::String(text) => Ok(text.clone()),
                Value::Null if text_parent_exists(provider, &value) => Ok(String::new()),
                Value::Null => Err(NULL_BINDING.into()),
                other => Ok(other.to_string()),
            }
        }
    }
}

/// Whether the member holding the text is there (a null text is an empty answer; a
/// missing parent threw upstream).
fn text_parent_exists(provider: AiProvider, value: &Value) -> bool {
    if provider == AiProvider::Gemini {
        value["candidates"][0]["content"]["parts"][0].is_object()
    } else {
        value["choices"][0]["message"].is_object()
    }
}

/// `CreateModelsRequest`: the provider's model list, which also tests the key.
pub fn models_request(options: &AiOptions, endpoints: &Endpoints) -> Result<HttpRequest, String> {
    let key = |k: &Option<String>| {
        k.as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string)
    };
    match options.provider {
        AiProvider::OpenAI | AiProvider::OpenAILegacy => {
            let Some(key) = key(&options.openai_api_key) else {
                return Err(t(tl::AnalyzeImageService_Missing_OpenAI_API_key).into());
            };
            let base = match options.openai_custom_url.as_deref() {
                Some(custom) if !custom.trim().is_empty() => custom,
                _ => &endpoints.openai,
            };
            Ok(HttpRequest {
                method: Method::Get,
                url: combine_url(base, "v1/models"),
                bearer: Some(key),
                body: None,
            })
        }
        AiProvider::Gemini => {
            let Some(key) = key(&options.gemini_api_key) else {
                return Err(t(tl::AnalyzeImageService_Missing_Gemini_API_key).into());
            };
            Ok(HttpRequest {
                method: Method::Get,
                url: format!(
                    "{}?key={}",
                    combine_url(&endpoints.gemini, "v1beta/models"),
                    modlight_core::tasks::naming::url_encode(&key)
                ),
                bearer: None,
                body: None,
            })
        }
        AiProvider::OpenRouter => {
            let Some(key) = key(&options.open_router_api_key) else {
                return Err(t(tl::AnalyzeImageService_Missing_OpenRouter_API_key).into());
            };
            Ok(HttpRequest {
                method: Method::Get,
                url: combine_url(&endpoints.open_router, "api/v1/models"),
                bearer: Some(key),
                body: None,
            })
        }
    }
}

/// `TestConnectionAsync`'s verdict on the models answer.
pub fn connection_result(response: &HttpResponse) -> (bool, String) {
    if response.success() {
        return (true, t(tl::AnalyzeImageService_Connection_OK).to_string());
    }
    let mut summary = if response.reason.trim().is_empty() {
        response.status.to_string()
    } else {
        response.reason.clone()
    };
    if !response.body.trim().is_empty() {
        let chars: Vec<char> = response.body.chars().collect();
        summary.push_str(": ");
        if chars.len() > 200 {
            summary.extend(&chars[..200]);
            summary.push_str("...");
        } else {
            summary.push_str(&response.body);
        }
    }
    (false, summary)
}

/// `LoadModelsAsync`: the ids of `data`, sorted ignoring case.
pub fn parse_models(response: &HttpResponse) -> Result<Vec<String>, String> {
    if !response.success() {
        return Err(response.status_error());
    }
    let value = parse_json(&response.body)?;
    let Some(data) = value["data"].as_array() else {
        return Err("The given key was not present in the dictionary.".into());
    };
    let mut ids: Vec<String> = data
        .iter()
        .filter_map(|m| m["id"].as_str())
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string)
        .collect();
    ids.sort_by_key(|a| a.to_uppercase());
    Ok(ids)
}

/// `AnalyzeImageViewModel.AnalyzeAsync`'s result text (line breaks as the platform's).
pub fn result_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// `HttpClientFactory.Create()`: .NET's 100 second timeout, no caching.
pub fn client() -> Result<reqwest::Client, String> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::CACHE_CONTROL,
        reqwest::header::HeaderValue::from_static("no-cache"),
    );
    reqwest::Client::builder()
        .user_agent(modlight_core::components::net::user_agent())
        .default_headers(headers)
        .timeout(Duration::from_secs(100))
        .build()
        .map_err(|e| e.to_string())
}

/// Send a request and read the whole answer.
pub async fn send(client: &reqwest::Client, request: &HttpRequest) -> Result<HttpResponse, String> {
    let mut builder = match request.method {
        Method::Get => client.get(&request.url),
        Method::Post => client.post(&request.url),
    };
    if let Some(bearer) = &request.bearer {
        builder = builder.bearer_auth(bearer);
    }
    if let Some(body) = &request.body {
        builder = builder
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/json; charset=utf-8",
            )
            .body(body.clone());
    }
    // Errors without the address: it may carry the Gemini key.
    let response = builder
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| e.without_url().to_string())?;
    Ok(HttpResponse {
        status: status.as_u16(),
        reason: status.canonical_reason().unwrap_or_default().to_string(),
        body,
    })
}

/// `AnalyzeImageService.AnalyzeAsync`: build (off the async threads), send, read.
pub async fn analyze(
    client: &reqwest::Client,
    options: &AiOptions,
    picture: Picture,
    endpoints: &Endpoints,
) -> Result<String, String> {
    let provider = options.provider;
    let payload = {
        let picture = picture.clone();
        tokio::task::spawn_blocking(move || image_payload(provider, &picture))
            .await
            .map_err(|e| e.to_string())??
    };
    let request = analyze_request(options, &payload, endpoints);
    let response = send(client, &request).await?;
    parse_analyze(provider, &response)
}

/// `TestConnectionAsync`.
pub async fn test_connection(
    client: &reqwest::Client,
    options: &AiOptions,
    endpoints: &Endpoints,
) -> (bool, String) {
    let request = match models_request(options, endpoints) {
        Ok(request) => request,
        Err(error) => return (false, error),
    };
    match send(client, &request).await {
        Ok(response) => connection_result(&response),
        Err(error) => (false, error),
    }
}

/// `LoadModelsAsync`: OpenAI's list (the others have none to load).
pub async fn load_models(
    client: &reqwest::Client,
    options: &AiOptions,
    endpoints: &Endpoints,
) -> Result<Vec<String>, String> {
    if !matches!(
        options.provider,
        AiProvider::OpenAI | AiProvider::OpenAILegacy
    ) {
        return Ok(Vec::new());
    }
    let request = models_request(options, endpoints)?;
    let response = send(client, &request).await?;
    parse_models(&response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use modlight_core::l10n::{Language, set_thread_language};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;

    fn options(provider: AiProvider) -> AiOptions {
        AiOptions {
            provider,
            openai_api_key: Some("sk-test".into()),
            gemini_api_key: Some("g key".into()),
            open_router_api_key: Some("or-test".into()),
            input: "Describe".into(),
            ..AiOptions::default()
        }
    }

    fn picture() -> Picture {
        Picture::Image(Arc::new(RgbaImage::from_pixel(
            4,
            3,
            image::Rgba([200, 30, 30, 255]),
        )))
    }

    #[test]
    fn urls_and_efforts() {
        assert_eq!(
            combine_url("https://a.b/", "/v1/models"),
            "https://a.b/v1/models"
        );
        assert_eq!(combine_url("", "x"), "x");
        assert_eq!(combine_url("x", ""), "x");
        let mut o = options(AiProvider::OpenAI);
        let e = Endpoints::default();
        assert_eq!(
            openai_url(&o, &e, "/v1/responses"),
            "https://api.openai.com/v1/responses"
        );
        o.openai_custom_url = Some("http://127.0.0.1:9/v1/responses".into());
        assert_eq!(
            openai_url(&o, &e, "/v1/responses"),
            "http://127.0.0.1:9/v1/responses"
        );
        o.openai_custom_url = Some("http://127.0.0.1:9/proxy/".into());
        assert_eq!(
            openai_url(&o, &e, "/v1/chat/completions"),
            "http://127.0.0.1:9/proxy/v1/chat/completions"
        );
        assert_eq!(reasoning_effort("gpt-5.2", Some("minimal")), "none");
        assert_eq!(reasoning_effort("GPT-5.1", None), "none");
        assert_eq!(reasoning_effort("gpt-5.1", Some("high")), "high");
        assert_eq!(reasoning_effort("gpt-5-mini", Some("minimal")), "minimal");
        assert_eq!(reasoning_effort("gpt-5-mini", None), "minimal");
    }

    #[test]
    fn requests_per_provider() {
        set_thread_language(Some(Language::English));
        let e = Endpoints::default();

        let o = options(AiProvider::OpenAI);
        let image = image_payload(o.provider, &picture()).unwrap();
        assert!(image.starts_with("data:image/jpeg;base64,/9j/"));
        let r = analyze_request(&o, &image, &e);
        assert_eq!(r.method, Method::Post);
        assert_eq!(r.url, "https://api.openai.com/v1/responses");
        assert_eq!(r.bearer.as_deref(), Some("sk-test"));
        let body: Value = serde_json::from_str(r.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["model"], "gpt-5-mini");
        assert_eq!(body["reasoning"]["effort"], "minimal");
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["input"][0]["content"][0]["text"], "Describe");
        assert_eq!(body["input"][0]["content"][1]["type"], "input_image");
        assert_eq!(body["input"][0]["content"][1]["image_url"], image.as_str());
        assert!(body["input"][0]["content"][1].get("text").is_none());
        assert_eq!(body["text"]["verbosity"], "medium");
        assert_eq!(body["store"], false);

        let mut o = options(AiProvider::OpenAILegacy);
        o.input.clear();
        let r = analyze_request(&o, "data:x", &e);
        assert_eq!(r.url, "https://api.openai.com/v1/chat/completions");
        let body: Value = serde_json::from_str(r.body.as_deref().unwrap()).unwrap();
        assert_eq!(
            body["messages"][0]["content"][0]["text"],
            "What is in this image?"
        );
        assert_eq!(
            body["messages"][0]["content"][1]["image_url"]["url"],
            "data:x"
        );
        assert!(body.get("reasoning").is_none());

        let o = options(AiProvider::Gemini);
        let image = image_payload(o.provider, &picture()).unwrap();
        assert!(image.starts_with("iVBORw0KGgo"));
        let r = analyze_request(&o, &image, &e);
        assert_eq!(
            r.url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash-latest:generateContent?key=g%20key"
        );
        assert_eq!(r.bearer, None);
        assert!(!format!("{r:?}").contains("g%20key"));
        let body: Value = serde_json::from_str(r.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["contents"][0]["parts"][0]["text"], "Describe");
        assert_eq!(
            body["contents"][0]["parts"][1]["inline_data"]["mime_type"],
            "image/png"
        );
        assert_eq!(
            body["contents"][0]["parts"][1]["inline_data"]["data"],
            image.as_str()
        );

        let o = options(AiProvider::OpenRouter);
        let r = analyze_request(&o, "QUJD", &e);
        assert_eq!(r.url, "https://openrouter.ai/api/v1/chat/completions");
        assert_eq!(r.bearer.as_deref(), Some("or-test"));
        assert!(!format!("{r:?}").contains("or-test"));
        let body: Value = serde_json::from_str(r.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["model"], "google/gemini-flash-1.5");
        assert_eq!(
            body["messages"][0]["content"][1]["image_url"]["url"],
            "data:image/png;base64,QUJD"
        );
        assert_eq!(body["max_tokens"], 1024);
        set_thread_language(None);
    }

    #[test]
    fn file_bytes_go_as_they_are_to_gemini() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jpg");
        crate::images::save(&RgbaImage::new(3, 3), &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let payload = image_payload(AiProvider::Gemini, &Picture::File(path.clone())).unwrap();
        assert_eq!(payload, base64(&bytes));
        let payload = image_payload(AiProvider::OpenAI, &Picture::File(path)).unwrap();
        assert!(payload.starts_with("data:image/jpeg;base64,"));
        assert!(
            image_payload(
                AiProvider::OpenAI,
                &Picture::File(dir.path().join("none.png"))
            )
            .is_err()
        );
    }

    fn ok(body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            reason: "OK".into(),
            body: body.into(),
        }
    }

    #[test]
    fn answers_per_provider() {
        set_thread_language(Some(Language::English));
        let openai = ok(
            r#"{"id":"r","output":[{"type":"reasoning","content":[]},{"type":"message","content":[{"type":"output_text","text":"A red square."}]}]}"#,
        );
        assert_eq!(
            parse_analyze(AiProvider::OpenAI, &openai).unwrap(),
            "A red square."
        );
        assert_eq!(
            parse_analyze(AiProvider::OpenAI, &ok(r#"{"output":null}"#)).unwrap(),
            ""
        );
        let denied = HttpResponse {
            status: 401,
            reason: "Unauthorized".into(),
            body: "{}".into(),
        };
        assert_eq!(
            parse_analyze(AiProvider::OpenAI, &denied).unwrap_err(),
            "Response status code does not indicate success: 401 (Unauthorized)."
        );
        let legacy = ok(r#"{"choices":[{"message":{"role":"assistant","content":"Text"}}]}"#);
        assert_eq!(
            parse_analyze(AiProvider::OpenAILegacy, &legacy).unwrap(),
            "Text"
        );
        let gemini = ok(r#"{"candidates":[{"content":{"parts":[{"text":"G"}]}}]}"#);
        assert_eq!(parse_analyze(AiProvider::Gemini, &gemini).unwrap(), "G");
        assert_eq!(
            parse_analyze(AiProvider::Gemini, &ok("{}")).unwrap_err(),
            NULL_BINDING
        );
        let failed = HttpResponse {
            status: 400,
            reason: "Bad Request".into(),
            body: r#"{"error":"bad"}"#.into(),
        };
        assert_eq!(
            parse_analyze(AiProvider::Gemini, &failed).unwrap_err(),
            r#"Error from Gemini API: {"error":"bad"}"#
        );
        assert!(
            parse_analyze(AiProvider::OpenRouter, &failed)
                .unwrap_err()
                .ends_with(r#"{"error":"bad"}"#)
        );
        assert_eq!(
            parse_analyze(AiProvider::OpenRouter, &legacy).unwrap(),
            "Text"
        );
        assert!(parse_analyze(AiProvider::OpenAI, &ok("not json")).is_err());
        set_thread_language(None);
    }

    #[test]
    fn models_and_connection() {
        set_thread_language(Some(Language::English));
        let e = Endpoints::default();
        let mut o = options(AiProvider::OpenAI);
        o.openai_api_key = Some("  sk-test  ".into());
        let r = models_request(&o, &e).unwrap();
        assert_eq!(r.method, Method::Get);
        assert_eq!(r.url, "https://api.openai.com/v1/models");
        assert_eq!(r.bearer.as_deref(), Some("sk-test"));
        o.openai_api_key = None;
        assert_eq!(
            models_request(&o, &e).unwrap_err(),
            t(tl::AnalyzeImageService_Missing_OpenAI_API_key)
        );
        let r = models_request(&options(AiProvider::Gemini), &e).unwrap();
        assert_eq!(
            r.url,
            "https://generativelanguage.googleapis.com/v1beta/models?key=g%20key"
        );
        let r = models_request(&options(AiProvider::OpenRouter), &e).unwrap();
        assert_eq!(r.url, "https://openrouter.ai/api/v1/models");

        assert_eq!(
            connection_result(&ok("{}")),
            (true, "Connection OK.".to_string())
        );
        let long = HttpResponse {
            status: 403,
            reason: "Forbidden".into(),
            body: "x".repeat(250),
        };
        let (success, message) = connection_result(&long);
        assert!(!success);
        assert_eq!(message, format!("Forbidden: {}...", "x".repeat(200)));
        let bare = HttpResponse {
            status: 599,
            reason: String::new(),
            body: String::new(),
        };
        assert_eq!(connection_result(&bare).1, "599");

        let list = ok(r#"{"data":[{"id":"gpt-5"},{"id":"Babbage"},{"id":""},{"id":"dall-e-3"}]}"#);
        assert_eq!(
            parse_models(&list).unwrap(),
            ["Babbage", "dall-e-3", "gpt-5"]
        );
        assert!(parse_models(&ok("{}")).is_err());
        assert_eq!(result_text("a\r\nb\rc"), "a\nb\nc");
        set_thread_language(None);
    }

    /// A stub on 127.0.0.1 answering each request with the next canned answer and
    /// keeping what it was sent.
    fn stub(answers: Vec<(u16, &'static str)>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let kept = seen.clone();
        std::thread::spawn(move || {
            for (status, body) in answers {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                let mut reader = BufReader::new(stream);
                let mut head = String::new();
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    head.push_str(&line);
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut content = vec![0u8; length];
                let _ = reader.read_exact(&mut content);
                head.push_str(&String::from_utf8_lossy(&content));
                kept.lock().unwrap().push(head);
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let mut stream = reader.into_inner();
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (address, seen)
    }

    #[test]
    fn round_trip_with_a_local_stub() {
        set_thread_language(Some(Language::English));
        let (address, seen) = stub(vec![
            (
                200,
                r#"{"output":[{"content":[{"type":"output_text","text":"Stub says hi"}]}]}"#,
            ),
            (200, r#"{"data":[{"id":"b"},{"id":"A"}]}"#),
            (401, r#"{"error":{"message":"Incorrect API key"}}"#),
            (
                200,
                r#"{"candidates":[{"content":{"parts":[{"text":"Gemini stub"}]}}]}"#,
            ),
        ]);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let client = client().unwrap();
        let endpoints = Endpoints {
            openai: "https://unused.invalid".into(),
            gemini: address.clone(),
            open_router: "https://unused.invalid".into(),
        };
        let mut o = options(AiProvider::OpenAI);
        o.openai_custom_url = Some(address.clone());
        let text = runtime
            .block_on(analyze(&client, &o, picture(), &endpoints))
            .unwrap();
        assert_eq!(text, "Stub says hi");
        let models = runtime
            .block_on(load_models(&client, &o, &endpoints))
            .unwrap();
        assert_eq!(models, ["A", "b"]);
        let (success, message) = runtime.block_on(test_connection(&client, &o, &endpoints));
        assert!(!success);
        assert!(message.starts_with("Unauthorized: {\"error\""), "{message}");
        let g = options(AiProvider::Gemini);
        let text = runtime
            .block_on(analyze(&client, &g, picture(), &endpoints))
            .unwrap();
        assert_eq!(text, "Gemini stub");

        let seen = seen.lock().unwrap();
        assert!(seen[0].starts_with("POST /v1/responses HTTP/1.1"));
        assert!(seen[0].contains("authorization: Bearer sk-test"));
        assert!(seen[0].contains("content-type: application/json; charset=utf-8"));
        assert!(seen[0].contains("cache-control: no-cache"));
        assert!(seen[0].contains("\"input_image\""));
        assert!(seen[1].starts_with("GET /v1/models HTTP/1.1"));
        assert!(seen[3].starts_with(
            "POST /v1beta/models/gemini-1.5-flash-latest:generateContent?key=g%20key HTTP/1.1"
        ));
        assert!(!seen[3].contains("authorization"));
        set_thread_language(None);
    }
}

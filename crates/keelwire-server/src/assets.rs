//! Embedded web UI and built-in proprietary schemas.

use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../ui/dist/"]
struct UiAssets;

#[derive(RustEmbed)]
#[folder = "../keelwire-core/schemas/proprietary/"]
struct SchemaAssets;

pub fn builtin_schemas() -> Vec<(String, String)> {
    SchemaAssets::iter()
        .filter(|p| p.ends_with(".json"))
        .filter_map(|p| SchemaAssets::get(&p).map(|f| (p.to_string(), String::from_utf8_lossy(&f.data).to_string())))
        .collect()
}

pub async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match UiAssets::get(path) {
        Some(f) => serve(path, f.data.into_owned()),
        None if !path.starts_with("api/") && !path.contains('.') => match UiAssets::get("index.html") {
            Some(f) => serve("index.html", f.data.into_owned()),
            None => (StatusCode::NOT_FOUND, "UI not built").into_response(),
        },
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

fn serve(path: &str, data: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    Response::builder().header(header::CONTENT_TYPE, mime.as_ref()).header(header::CACHE_CONTROL, cache).body(Body::from(data)).unwrap()
}

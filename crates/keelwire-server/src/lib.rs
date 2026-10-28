//! Keelwire server: engine, REST/WebSocket API and embedded UI.
//! Used by the `keelwire` CLI (headless daemon) and the Tauri desktop app.

pub mod api;
pub mod assets;
pub mod boatfile;
pub mod capture;
pub mod dto;
pub mod engine;
pub mod extcap;
pub mod maretron_actions;
pub mod pdf;
pub mod report;
pub mod session;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;

pub use engine::{Engine, EngineConfig, TxMode};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug)]
pub struct ServerOptions {
    pub bind: SocketAddr,
    pub token: Option<String>,
    pub cors_any: bool,
}

/// Start the HTTP server. Returns the bound address (useful with port 0).
pub async fn serve(engine: Arc<Engine>, opts: ServerOptions) -> std::io::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let state = api::AppState { engine, token: opts.token.clone() };
    let mut app = api::router(state);
    if opts.cors_any {
        app = app.layer(tower_http::cors::CorsLayer::permissive());
    }
    let listener = tokio::net::TcpListener::bind(opts.bind).await?;
    let addr = listener.local_addr()?;
    let h = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("server error: {e}");
        }
    });
    Ok((addr, h))
}

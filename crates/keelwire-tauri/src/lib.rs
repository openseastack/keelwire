//! Keelwire app for desktop (Windows, macOS, Linux), iPhone/iPad and Android.
//!
//! Starts the same engine + HTTP/WebSocket server as the headless daemon on
//! 127.0.0.1 (random port) and shows it in a native web view, so every
//! platform runs identical code. On iPhone/iPad it also handles the app
//! lifecycle (iOS suspends apps that leave the screen) and provides the
//! share sheet and keep-awake (docs/IOS-SPEC.md).

use std::net::SocketAddr;
#[cfg(mobile)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use keelwire_server::{Engine, EngineConfig, ServerOptions};

static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();
static ADDR: OnceLock<SocketAddr> = OnceLock::new();
/// The app is on screen.
#[cfg(mobile)]
static FOREGROUND: AtomicBool = AtomicBool::new(true);

fn engine_config() -> EngineConfig {
    // A technician opens the app, so changes are allowed by default; they still
    // have to be unlocked in the app and relock after 15 minutes.
    // `--read-only` (or KEELWIRE_READ_ONLY=1) makes this install read-only.
    let allow_tx = !(std::env::args().any(|a| a == "--read-only") || std::env::var("KEELWIRE_READ_ONLY").map(|v| v != "0").unwrap_or(false));
    #[allow(unused_mut)]
    let mut cfg = EngineConfig { allow_tx, ..Default::default() };
    #[cfg(target_os = "android")]
    {
        // Android gives apps no HOME; use the app's private files folder
        // (/data/data/<identifier>/files). Moving finished sessions to shared
        // storage and a share sheet are the next Android steps.
        let files = std::path::PathBuf::from("/data/data/org.keelwire.app/files");
        cfg.data_dir = files.join("keelwire");
        cfg.sessions_dir = files.join("Vessels");
    }
    #[cfg(target_os = "ios")]
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        // iOS app container: settings and the open session stay private in
        // Library; finished sessions go to Documents, which the Files app shows
        // as "On My iPhone › Keelwire".
        cfg.data_dir = home.join("Library").join("Application Support").join("keelwire");
        cfg.sessions_dir = home.join("Documents").join("Vessels");
    }
    cfg
}

async fn serve(engine: Arc<Engine>, bind: SocketAddr) -> std::io::Result<SocketAddr> {
    let (addr, _h) = keelwire_server::serve(engine, ServerOptions { bind, token: None, cors_any: false }).await?;
    Ok(addr)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())).try_init();
    let rt: &'static tokio::runtime::Runtime = Box::leak(Box::new(tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime")));
    tauri::async_runtime::set(rt.handle().clone());

    let addr = rt.block_on(async move {
        let engine = Engine::new(engine_config());
        if let Ok(uri) = std::env::var("KEELWIRE_INTERFACE") {
            let _ = engine.connect(&uri);
        }
        // Windows / Linux pass a double-clicked boat file as an argument.
        if let Some(p) = std::env::args().skip(1).find(|a| a.to_ascii_lowercase().ends_with(".keelwire")) {
            if let Err(e) = engine.boat_open_path(std::path::Path::new(&p)) {
                tracing::warn!("{e}");
            }
        }
        ENGINE.set(engine.clone()).ok();
        serve(engine, "127.0.0.1:0".parse().unwrap()).await.expect("start embedded server")
    });
    ADDR.set(addr).ok();
    tracing::info!("embedded server on http://{addr}");

    tauri::Builder::default()
        .setup(move |app| {
            let url: tauri::Url = format!("http://{addr}/").parse().expect("url");
            let builder = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(url));
            #[cfg(desktop)]
            let builder = builder.title(concat!("Keelwire ", env!("CARGO_PKG_VERSION"))).inner_size(1440.0, 920.0).min_inner_size(900.0, 600.0);
            builder.build()?;
            #[cfg(target_os = "ios")]
            ios::setup(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Keelwire")
        .run(|_app, event| {
            match &event {
                // macOS and iOS deliver opened boat files as an Opened event.
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                tauri::RunEvent::Opened { urls } => {
                    for u in urls {
                        if let (Ok(p), Some(e)) = (u.to_file_path(), ENGINE.get()) {
                            if let Err(err) = e.boat_open_path(&p) {
                                tracing::warn!("{err}");
                            }
                        }
                    }
                }
                #[cfg(mobile)]
                tauri::RunEvent::WindowEvent { event: tauri::WindowEvent::Suspended, .. } => {
                    FOREGROUND.store(false, Ordering::Relaxed);
                    if let Some(e) = ENGINE.get() {
                        e.app_background(true);
                    }
                    #[cfg(target_os = "ios")]
                    keelwire_ios::set_keep_awake(false);
                }
                #[cfg(mobile)]
                tauri::RunEvent::WindowEvent { event: tauri::WindowEvent::Resumed, .. } => {
                    FOREGROUND.store(true, Ordering::Relaxed);
                    let handle = _app.clone();
                    tauri::async_runtime::spawn(async move { on_resume(handle).await });
                }
                _ => {}
            }
        });
}

/// Back on screen: make sure the in-app server still answers (iOS may close
/// listening sockets of suspended apps), then resume the session.
#[cfg(mobile)]
async fn on_resume(app: tauri::AppHandle) {
    use tauri::Manager;
    let (Some(engine), Some(addr)) = (ENGINE.get().cloned(), ADDR.get().copied()) else { return };
    let alive = tokio::time::timeout(std::time::Duration::from_millis(700), tokio::net::TcpStream::connect(addr)).await.map(|r| r.is_ok()).unwrap_or(false);
    if !alive {
        tracing::warn!("embedded server stopped answering while suspended; restarting on {addr}");
        match serve(engine.clone(), addr).await {
            Ok(_) => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.eval("location.reload()");
                }
            }
            Err(e) => tracing::error!("cannot restart the embedded server on {addr}: {e}"),
        }
    }
    engine.app_background(false);
}

#[cfg(target_os = "ios")]
mod ios {
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    use keelwire_server::engine::Platform;

    use super::{ENGINE, FOREGROUND};

    struct IosPlatform {
        app: tauri::AppHandle,
    }

    impl Platform for IosPlatform {
        fn share_files(&self, paths: Vec<PathBuf>) -> Result<(), String> {
            // UIKit must be driven from the main thread.
            self.app
                .run_on_main_thread(move || {
                    if let Err(e) = keelwire_ios::share_files(&paths) {
                        tracing::warn!("share: {e}");
                    }
                })
                .map_err(|e| e.to_string())
        }
    }

    pub fn setup(app: tauri::AppHandle) {
        let Some(engine) = ENGINE.get().cloned() else { return };
        let _ = engine.platform.set(Arc::new(IosPlatform { app: app.clone() }));
        // Keep the screen awake while a session records and the app is on screen.
        tauri::async_runtime::spawn(async move {
            let mut last: Option<bool> = None;
            loop {
                let want = FOREGROUND.load(Ordering::Relaxed) && engine.session_recording();
                if last != Some(want) {
                    last = Some(want);
                    let _ = app.run_on_main_thread(move || keelwire_ios::set_keep_awake(want));
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        });
    }
}

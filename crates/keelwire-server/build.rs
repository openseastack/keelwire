// Builds the web UI (ui/ -> ui/dist) so a plain `cargo build` produces one
// self-contained binary with the web interface and API inside it.
//
// * Runs `npm ci` (first time) and `npm run build` when the UI sources change.
// * Set KEELWIRE_SKIP_UI_BUILD=1 to skip (e.g. CI that builds the UI separately,
//   or a machine without Node.js); an existing ui/dist is then embedded as-is.
// * Without Node.js and without ui/dist, a placeholder page is embedded and a
//   cargo warning is printed; the API still works.
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui");
    let dist = ui.join("dist");

    for p in ["src", "public", "index.html", "package.json", "package-lock.json", "vite.config.ts", "tailwind.config.js"] {
        println!("cargo:rerun-if-changed={}", ui.join(p).display());
    }
    println!("cargo:rerun-if-changed=../keelwire-core/schemas/proprietary");
    println!("cargo:rerun-if-env-changed=KEELWIRE_SKIP_UI_BUILD");

    let skip = std::env::var_os("KEELWIRE_SKIP_UI_BUILD").is_some_and(|v| v != "0");
    if !skip && ui.join("package.json").exists() {
        match build_ui(&ui) {
            Ok(()) => {}
            Err(e) if dist.join("index.html").exists() => {
                println!("cargo:warning=web UI not rebuilt ({e}); embedding the existing ui/dist");
            }
            Err(e) => println!("cargo:warning=web UI not built ({e}); embedding a placeholder page"),
        }
    }

    if !dist.join("index.html").exists() {
        let _ = std::fs::create_dir_all(&dist);
        let _ = std::fs::write(
            dist.join("index.html"),
            "<!doctype html><meta charset=utf-8><title>Keelwire</title><body style=\"font-family:sans-serif;padding:2rem\"><h1>Keelwire</h1><p>This binary was built without the web UI (Node.js was not available). Install Node.js 20+ and rebuild. The API is available under <code>/api/v1</code>.</p>",
        );
    }
}

fn npm() -> PathBuf {
    PathBuf::from(if cfg!(windows) { "npm.cmd" } else { "npm" })
}

fn build_ui(ui: &Path) -> Result<(), String> {
    let run = |args: &[&str]| -> Result<(), String> {
        let st = Command::new(npm()).args(args).current_dir(ui).status().map_err(|e| format!("npm not found: {e}"))?;
        if st.success() { Ok(()) } else { Err(format!("`npm {}` failed", args.join(" "))) }
    };
    if !ui.join("node_modules").exists() {
        run(&["ci", "--no-audit", "--no-fund"])?;
    }
    run(&["run", "build"])
}

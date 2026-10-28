//! Re-render the reports of a saved session:
//! `cargo run -p keelwire-server --example render_report -- "<session folder>"`
fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("session folder"));
    let s: keelwire_server::session::Session = serde_json::from_slice(&std::fs::read(dir.join("session.json")).expect("session.json")).expect("valid session.json");
    let files: Vec<(String, u64, Option<String>)> = s.capture_files.iter().map(|f| (f.clone(), std::fs::metadata(dir.join(f)).map(|m| m.len()).unwrap_or(0), keelwire_server::session::sha256_file(&dir.join(f)))).collect();
    std::fs::write(dir.join(keelwire_server::report::OWNER_PDF), keelwire_server::report::owner_pdf(&s).unwrap()).unwrap();
    std::fs::write(dir.join(keelwire_server::report::TECH_PDF), keelwire_server::report::tech_pdf(&s, &files).unwrap()).unwrap();
    std::fs::write(dir.join(keelwire_server::report::TECH_HTML), keelwire_server::report::tech_html(&s, &files)).unwrap();
    println!("written to {}", dir.display());
}

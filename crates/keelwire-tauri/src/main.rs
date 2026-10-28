//! Desktop launcher; the app itself is `keelwire_app::run` (src/lib.rs).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    keelwire_app::run();
}

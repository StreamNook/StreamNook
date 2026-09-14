// Desktop binary entry point. All application logic lives in the library crate
// (`streamnook_lib::run`) so the exact same code drives the Tauri mobile
// (Android/iOS) builds, which load the library and call `run()` themselves.
//
// `windows_subsystem = "windows"` stays on the binary (a library has no
// subsystem) so release builds launch without a console window on Windows.
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

fn main() {
    streamnook_lib::run();
}

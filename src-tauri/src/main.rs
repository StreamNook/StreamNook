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
    // Linux runs on the Chromium Embedded Framework, which re-executes this
    // binary for its helper processes and takes the profile lock before any
    // plugin runs. `enter` sorts out which process this is (helper, second
    // launch, or the app) BEFORE anything below; see linux_cef.rs.
    #[cfg(target_os = "linux")]
    match streamnook_lib::linux_cef::enter() {
        streamnook_lib::linux_cef::Entry::Helper | streamnook_lib::linux_cef::Entry::Forwarded => {
            return;
        }
        streamnook_lib::linux_cef::Entry::NoDisplay => {
            eprintln!("{}", streamnook_lib::linux_cef::NO_DISPLAY_MESSAGE);
            std::process::exit(1);
        }
        streamnook_lib::linux_cef::Entry::Browser => {}
    }
    #[cfg(all(debug_assertions, desktop))]
    streamnook_lib::print_credential_if_asked();
    streamnook_lib::run();
}

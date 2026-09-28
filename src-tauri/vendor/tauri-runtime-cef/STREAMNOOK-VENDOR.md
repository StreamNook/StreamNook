# Vendored: tauri-runtime-cef

Upstream: <https://github.com/SableClient/tauri-runtime-cef>, commit
`fbd0f206576b983cb65b69a5a9aad2a73f4490a5` (Apache-2.0 OR MIT; both license
texts are kept beside this file). It is the Chromium Embedded Framework
runtime the Linux build of StreamNook runs on (`src/rt.rs`).

The changes from upstream:

- `Cargo.toml`: `cef` and `cef-dll-sys` pinned to `=151.8.1` (CEF 151.3.24,
  Chromium 151.0.7922.174) instead of `=150.2.1` (CEF 150.0.14). StreamNook
  ships its own CEF distribution of that exact version, built with
  proprietary codecs (H.264, AAC), which the stock distribution lacks; the
  crate pin and the distribution must move together because the runtime
  checks CEF's API hash at startup. The crate compiles against 151 without
  source changes.
- `src/cef_impl/client/command.rs` (new), `client/mod.rs`,
  `client/context_menu.rs`: a `CommandHandler` on every browser. Under
  Chrome style (everywhere but macOS) Chrome runs its own accelerator table
  inside an app window: Ctrl+W closed the main window, F5 and Ctrl+R
  reloaded the app, Ctrl+H/J/U/T/P opened Chrome's history, downloads,
  view-source, new-tab and print windows, Ctrl+Shift+J opened DevTools with
  DevTools off. `on_chrome_command` now handles every Chrome command as a
  no-op except an allowlist: clipboard and editing (cut, copy, paste, paste
  and match style, delete, undo, redo, select all, spelling replacements,
  the copy items of the link and image menus), plus the DevTools commands
  when DevTools are enabled for the webview. Commands are matched by `IDC_`
  name through `cef_id_for_command_id_name`, never by number. The same
  allowlist prunes Chrome's context menu (it replaces the old "drop the last
  item when DevTools are off" rule), and the app menu, page-action icons and
  toolbar buttons report hidden. The keyboard handler is unchanged.
- `src/policy.rs`, `client/life_span.rs`, `cef_impl/request_handler.rs`,
  `client/mod.rs`: `PopupRequest` carries `opener_url`, the URL of the frame
  that asked for the popup, so the app's popup policy can tell its own pages
  (whose links go to the system browser) from remote pages loaded in the
  same webview. The request handler also implements `on_open_urlfrom_tab`
  and puts new-tab and new-window links through the same policy: Chromium
  routes a `target="_blank"` anchor (and a middle or ctrl click) there, not
  to `on_before_popup`, so without it those links reached no policy and
  did nothing.
- `src/webview.rs` (`devtools_initialization_script_source`): the CDP
  document-start copy of the initialization scripts skips the native
  custom-scheme form (`tauri://localhost/…`) as it already skipped
  `http://tauri.localhost/…`. Both forms are served by the runtime's scheme
  handlers, which inject the same scripts into the HTML, so every app
  document ran them twice and the second run threw `Cannot redefine
  property: postMessage / metadata / __TAURI_PATTERN__`.

Everything else (source, scripts, examples, the Nix flake) is as upstream.
The crate's own `Cargo.lock` is dropped: the app's lock is the one that
counts for a path dependency.

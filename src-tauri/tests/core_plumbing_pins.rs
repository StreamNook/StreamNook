//! Source pins for the core plumbing of Autopilot's background mode: the
//! settings import and its lock, the main teardown, the lock-free inner bodies
//! and the locked commands, the registry's temp-file save, and the host-side
//! calls into `background_mode`. Pinned by source because the order and the
//! locks are the whole rule. It also pins the one deep-link door: the two
//! deep-link loops, the link slot and its commands, and the notice stamps.

const DROPS: &str = include_str!("../src/commands/drops.rs");
const SETTINGS: &str = include_str!("../src/commands/settings.rs");
const SETTINGS_EXTRAS: &str = include_str!("../src/commands/settings_extras.rs");
const STREAMING: &str = include_str!("../src/commands/streaming.rs");
const MULTI_NOOK: &str = include_str!("../src/commands/multi_nook.rs");
const WATCH_START: &str = include_str!("../src/services/watch_start.rs");
const WATCH_SESSION: &str = include_str!("../src/services/watch_session.rs");
const KICK_ACCOUNT: &str = include_str!("../src/services/providers/kick_account.rs");
const REGISTRY: &str = include_str!("../src/plugin_host/registry.rs");
const HOST_MOD: &str = include_str!("../src/plugin_host/mod.rs");
const PROCESS: &str = include_str!("../src/plugin_host/process.rs");
const BROKER: &str = include_str!("../src/plugin_host/broker.rs");
const LIB: &str = include_str!("../src/lib.rs");
const NOTICES: &str = include_str!("../src/commands/notification_mirror.rs");
const EVENT_NAMES: &str = include_str!("event_names.txt");

fn norm(src: &str) -> String {
    src.replace("\r\n", "\n")
}

/// The body of the top-level function whose signature starts with `sig`, up to
/// the closing brace at the start of a line.
fn body(src: &str, sig: &str) -> String {
    let src = norm(src);
    let start = src.find(sig).unwrap_or_else(|| panic!("{sig} not found"));
    let rest = &src[start..];
    let end = rest.find("\n}\n").unwrap_or_else(|| panic!("{sig}: no end"));
    rest[..end].to_string()
}

/// The body of a method inside an impl block (closing brace indented four).
fn method(src: &str, sig: &str) -> String {
    let src = norm(src);
    let start = src.find(sig).unwrap_or_else(|| panic!("{sig} not found"));
    let rest = &src[start..];
    let end = rest.find("\n    }\n").unwrap_or_else(|| panic!("{sig}: no end"));
    rest[..end].to_string()
}

fn at(b: &str, needle: &str) -> usize {
    b.find(needle).unwrap_or_else(|| panic!("`{needle}` missing:\n{b}"))
}

/// Every needle appears, each after the one before it.
fn in_order(b: &str, needles: &[&str]) {
    let mut last = 0;
    for needle in needles {
        let found = b[last..]
            .find(needle)
            .unwrap_or_else(|| panic!("`{needle}` missing or out of order (after byte {last}):\n{b}"));
        last += found + needle.len();
    }
}

/// The first line of the function body that is neither blank nor a comment.
fn first_statement(b: &str) -> String {
    let open = at(b, ") {\n") + ") {\n".len();
    b[open..]
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("//"))
        .expect("a statement")
        .to_string()
}

/// The text from the first `from` up to the first `to` after it.
fn between<'a>(src: &'a str, from: &str, to: &str) -> &'a str {
    let start = src.find(from).unwrap_or_else(|| panic!("`{from}` not found"));
    let rest = &src[start..];
    let end = rest[from.len()..]
        .find(to)
        .unwrap_or_else(|| panic!("`{to}` not found after `{from}`"));
    &rest[..from.len() + end]
}

#[test]
fn import_callers_hold_the_drops_lock() {
    let lock = "let _write = crate::commands::drops::DROPS_SETTINGS_WRITE.lock().await;";
    let callers = [
        (SETTINGS, "pub async fn import_settings_text("),
        (SETTINGS, "pub async fn import_settings("),
        (SETTINGS_EXTRAS, "pub async fn restore_settings_backup("),
    ];
    for (src, sig) in callers {
        let b = body(src, sig);
        let held = at(&b, lock);
        let merged = at(&b, "apply_imported_settings(");
        let applied = at(&b, "crate::commands::drops::apply_drops_settings(");
        assert!(held < merged, "{sig}: the lock is taken before the import");
        assert!(merged < applied, "{sig}: the drops hand-off follows the import");
        assert!(
            b.contains("apply_drops_settings(&app, &state, imported.drops, None).await"),
            "{sig}: the hand-off is awaited with the merged drops settings and no override"
        );
        assert!(b.contains("let imported = "), "{sig}: the merged settings are bound");
        assert!(!b.contains("let _ = "), "{sig}: nothing is thrown away");
    }
    // The save itself never takes the lock; the two drops commands do.
    let apply = body(DROPS, "pub async fn apply_drops_settings(");
    assert!(!apply.contains("DROPS_SETTINGS_WRITE"), "apply_drops_settings never locks");
    assert!(DROPS.contains("pub(crate) static DROPS_SETTINGS_WRITE:"));
    for sig in ["pub async fn update_drops_settings(", "pub async fn patch_drops_settings("] {
        let b = body(DROPS, sig);
        in_order(
            &b,
            &["let _write = DROPS_SETTINGS_WRITE.lock().await;", "apply_drops_settings(&app, &state,"],
        );
    }
}

#[test]
fn drops_saves_keep_told_backend_owned_and_tell_background_mode() {
    let b = body(DROPS, "pub async fn apply_drops_settings(");
    in_order(
        &b,
        &[
            "next.told_keep_running = told_override.unwrap_or(previous.told_keep_running);",
            "drops_service.update_settings(next.clone()).await;",
            "state.plugin_host.push_drops_settings();",
            "#[cfg(desktop)]\n    crate::services::background_mode::on_drops_settings_saved(app, &previous, &next);",
        ],
    );
}

#[test]
fn the_main_teardown_steps_are_guarded_and_in_order() {
    let b = body(WATCH_START, "pub(crate) async fn stop_for_closed_main(");
    assert_eq!(first_statement(&b), "let _aim = EARNING_AIM.lock().await;");
    in_order(
        &b,
        &[
            "let _aim = EARNING_AIM.lock().await;",
            "if !still_current() {\n        return;\n    }",
            "GRID_EXIT_DEFERRED.store(false, Ordering::Release);",
            "STOPPED.store(true, Ordering::Release);",
            "supersede();",
            "if let Err(e) = streaming::stop_stream_if(&still_current).await {",
            "if still_current() {\n        crate::commands::multi_nook::stop_all_for_closed_main(&app, &still_current).await;\n    }",
            "if still_current() {\n        watch_session::close_grid_now(&app).await;\n    }",
            "if end_session {\n        watch_session::end_session(app.clone(), &still_current).await;\n    }",
            "if still_current() {\n        crate::services::sleep_timer::cancel(&app);\n    }",
            "if still_current() {\n        crate::services::watch_minutes::set_source(crate::services::watch_minutes::MAIN, None);\n    }",
            "if still_current() {\n        crate::commands::drops::stop_drops_monitoring_inner(&app).await;\n    }",
            "if still_current() {\n        streaming::clear_active_channels(&app.state::<AppState>()).await;\n    }",
        ],
    );
    // No shared wrapper: the two multi-nook steps are separate guarded statements.
    assert!(
        !b.contains("stop_all_for_closed_main(&app, &still_current).await;\n        watch_session::close_grid_now"),
        "close_grid_now has its own guard"
    );
    assert!(!b.contains("watch_session::stop("), "the session is ended, never stopped");
    assert!(!b.contains("let _ = "), "no step's result is thrown away");
    // stop_stream_if re-checks at its top, after the YouTube await before the
    // TikTok relay, and right before the server goes down.
    let stop = body(STREAMING, "pub async fn stop_stream_if(");
    in_order(
        &stop,
        &[
            ") -> Result<(), String> {\n    if !still_current() {\n        return Ok(());\n    }\n    // Every close of a live stream",
            "crate::services::youtube_dash::stop(crate::services::stream_server::SOLO_STREAM_ID).await;\n    if !still_current() {\n        return Ok(());\n    }\n    // Same for the TikTok relay",
            "crate::services::tiktok_relay::stop(crate::services::stream_server::SOLO_STREAM_ID);\n    if !still_current() {\n        return Ok(());\n    }\n    StreamServer::stop().await",
        ],
    );
    // The wrapper keeps its own unguarded meaning: always current.
    assert!(norm(STREAMING).contains("    stop_stream_if(&|| true).await\n}"));
    assert!(stop.starts_with("pub async fn stop_stream_if(still_current: &(impl Fn() -> bool + Sync))"));
}

#[test]
fn close_grid_now_moves_no_earning_and_the_grid_paths_use_clear_grid() {
    let close = body(WATCH_SESSION, "pub async fn close_grid_now(");
    assert!(close.contains("    clear_grid();\n"));
    assert!(!close.contains("earning_after_grid("));
    assert!(!close.contains("set_grid_focus("));
    in_order(
        &close,
        &["clear_grid();", "crate::services::watch_drops::poke();", "aim_watch_socket(app).await;", "refresh_raid_fallback(app);"],
    );
    let focus = body(WATCH_SESSION, "pub fn set_grid_focus(");
    assert!(focus.contains("let seq = clear_grid();"), "the close branch clears through clear_grid");
}

#[test]
fn the_earning_paths_take_the_lock_and_look_for_main() {
    // earning_after_grid: the line after its lock.
    let after = body(WATCH_START, "pub async fn earning_after_grid(");
    let lines: Vec<&str> = after.lines().collect();
    let lock = lines.iter().position(|l| *l == "    let _aim = EARNING_AIM.lock().await;").expect("lock line");
    assert_eq!(
        lines[lock + 1],
        "    if !crate::services::window_visibility::window_open(app, \"main\") { return; }"
    );
    // The grid-focus task: lock after the lookup, main check, sequence compare, then the aim.
    let focus = body(WATCH_SESSION, "pub fn set_grid_focus(");
    let lock = at(&focus, "let _aim = crate::services::watch_start::earning_aim().await;");
    let after_lock = &focus[lock..];
    in_order(
        after_lock,
        &[
            "let _aim = crate::services::watch_start::earning_aim().await;",
            "if !crate::services::window_visibility::window_open(&app, \"main\") {\n            return;\n        }",
            "GRID_SEQ.load(Ordering::Acquire) != seq",
            "monitor_watched_channel(",
            "register_active_channel(&channel.id)",
        ],
    );
    assert!(
        focus[..lock].contains("get_user_by_login(&login)"),
        "the id lookup runs before the lock is taken"
    );
    // follow holds the lock and registers through the lock-free inner.
    let follow = body(WATCH_START, "async fn follow(");
    assert!(follow.contains("streaming::register_active_channel_inner(row.user_id.clone(), &state).await;"));
    assert!(!follow.contains("streaming::register_active_channel("));
}

#[test]
fn stop_all_for_closed_main_ends_the_grids_sockets() {
    let b = body(
        MULTI_NOOK,
        "pub async fn stop_all_for_closed_main(app: &AppHandle, still_current: &(impl Fn() -> bool + Sync)) {",
    );
    in_order(
        &b,
        &[
            "if let Err(e) = stop_all_multi_nooks().await {",
            "    if !still_current() {\n        return;\n    }\n    crate::services::multi_nook_raids::set_channels(app, vec![]);\n    crate::services::multi_nook_meta::set_channels(app, vec![]);",
        ],
    );
}

#[test]
fn the_locked_commands_take_the_lock_and_check_main() {
    let register = body(STREAMING, "pub async fn register_active_channel(");
    assert!(register.contains("app: crate::rt::AppHandle,"));
    in_order(
        &register,
        &[
            "let _aim = crate::services::watch_start::earning_aim().await;",
            "#[cfg(desktop)]\n    if !crate::services::window_visibility::window_open(&app, \"main\") {\n        return Ok(());\n    }",
            "register_active_channel_inner(channel_id, &state).await",
        ],
    );
    let inner = body(STREAMING, "pub async fn register_active_channel_inner(");
    assert!(!inner.contains("earning_aim"), "the inner takes no lock");
    let start = body(DROPS, "pub async fn start_drops_monitoring(");
    assert!(start.contains("app_handle: AppHandle,"));
    in_order(
        &start,
        &[
            "let _aim = crate::services::watch_start::earning_aim().await;",
            "#[cfg(desktop)]\n    if !crate::services::window_visibility::window_open(&app_handle, \"main\") {\n        return Ok(());\n    }",
            "monitor_watched_channel(&app_handle, channel_id, channel_name).await;",
        ],
    );
}

fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `word` appears in `line` as a whole identifier.
fn has_word(line: &str, word: &str) -> bool {
    let bytes = line.as_bytes();
    let mut from = 0;
    while let Some(i) = line[from..].find(word) {
        let start = from + i;
        let end = start + word.len();
        let before = start == 0 || !is_ident(bytes[start - 1]);
        let after = end >= bytes.len() || !is_ident(bytes[end]);
        if before && after {
            return true;
        }
        from = end;
    }
    false
}

#[test]
fn the_locked_commands_have_no_rust_callers() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    assert!(files.len() > 100, "the walk found the sources");
    let mut stray = Vec::new();
    let (mut register_def, mut register_handler, mut start_def, mut start_handler) = (0, 0, 0, 0);
    for path in files {
        let name = path.to_string_lossy().replace('\\', "/");
        let text = norm(&std::fs::read_to_string(&path).expect("read source"));
        for (n, line) in text.lines().enumerate() {
            let t = line.trim();
            if t.starts_with("//") {
                continue;
            }
            if has_word(t, "register_active_channel") {
                if t.contains(".register_active_channel(") {
                    // The websocket service's method, not the command.
                } else if name.ends_with("src/commands/streaming.rs") && t == "pub async fn register_active_channel(" {
                    register_def += 1;
                } else if name.ends_with("src/lib.rs") && t == "register_active_channel," {
                    register_handler += 1;
                } else if name.ends_with("src/services/channel_points_websocket_service.rs")
                    && t.starts_with("pub async fn register_active_channel(&self")
                {
                    // The service's own method definition.
                } else {
                    stray.push(format!("{name}:{}: {t}", n + 1));
                }
            }
            if has_word(t, "start_drops_monitoring") {
                if name.ends_with("src/commands/drops.rs") && t == "pub async fn start_drops_monitoring(" {
                    start_def += 1;
                } else if name.ends_with("src/lib.rs") && t == "start_drops_monitoring," {
                    start_handler += 1;
                } else {
                    stray.push(format!("{name}:{}: {t}", n + 1));
                }
            }
        }
    }
    assert!(stray.is_empty(), "a locked command is called from Rust (use the inner instead):\n{}", stray.join("\n"));
    assert_eq!((register_def, register_handler, start_def, start_handler), (1, 1, 1, 1));
}

#[test]
fn registry_save_writes_a_temp_file_and_renames() {
    let save = body(REGISTRY, "pub fn save(registry: &Registry)");
    in_order(
        &save,
        &[
            "let tmp = PathBuf::from(tmp_name);",
            "std::fs::write(&tmp, json)?;",
            "rename_with_retries(&tmp, &path)",
            "#[cfg(desktop)]\n    crate::services::background_mode::note_registry(registry);",
        ],
    );
    assert!(!save.contains("std::fs::write(&path"), "the registry file is never written directly");
    let rename = body(REGISTRY, "fn rename_with_retries(");
    assert!(rename.contains("std::fs::rename(from, to)"));
    assert!(rename.contains("std::thread::sleep(Duration::from_millis(50));"));
    assert!(norm(REGISTRY).contains("const RENAME_RETRIES: u32 = 5;"));
    let load = body(REGISTRY, "pub fn load() -> Result<Registry>");
    assert!(load.contains("#[cfg(desktop)]\n    crate::services::background_mode::note_registry(&registry);"));
}

#[test]
fn the_host_side_tells_background_mode() {
    // The supervisor, at both places a provider's handle is removed.
    let process = norm(PROCESS);
    let notes = process
        .matches("host.running.write().await.remove(&plugin_id);")
        .count();
    assert_eq!(notes, 2);
    assert_eq!(
        process
            .matches(
                "host.running.write().await.remove(&plugin_id);\n        // Whatever status this provider last sent no longer holds.\n        #[cfg(desktop)]\n        crate::services::background_mode::note_provider_stopped(&plugin_id);"
            )
            .count()
            + process
                .matches(
                    "host.running.write().await.remove(&plugin_id);\n    #[cfg(desktop)]\n    crate::services::background_mode::note_provider_stopped(&plugin_id);"
                )
                .count(),
        2
    );
    assert!(process.contains("\"host_session\": super::host_session_id(),"));
    // The broker: the notice and the provider's status.
    let notify = body(BROKER, "pub async fn handle_host_method(");
    assert!(notify.contains(
        "crate::services::background_mode::deliver_plugin_notice(app, name, text).await;"
    ));
    let status = body(BROKER, "pub fn handle_set_status(");
    assert!(status.contains(
        "#[cfg(desktop)]\n    if slot == DROPS_STATUS_SLOT {\n        crate::services::background_mode::note_drops_status(&record.id, &value);\n    }"
    ));
    assert!(norm(BROKER).contains("const DROPS_STATUS_SLOT: &str = \"drops.status\";"));
}

#[test]
fn the_drops_push_is_serialized_and_carries_session_and_generation() {
    let host = norm(HOST_MOD);
    let push = method(HOST_MOD, "    pub async fn push_drops_settings(&self) {");
    in_order(
        &push,
        &[
            "let _push = self.push_lock.lock().await;",
            "crate::services::evolution::evolution_bases().await;",
            "crate::services::evolution::locked_stages(&self.app).await;",
            "state.drops_service.lock().await.get_settings().await;",
            "let generation = self.push_generation.fetch_add(1, Ordering::AcqRel) + 1;",
            "\"host_session\": host_session_id(),",
            "\"generation\": generation,",
        ],
    );
    assert!(host.contains("pub async fn startup(&self, loaded: Option<Registry>) {"));
    assert!(host.contains("pub fn host_session_id() -> &'static str {"));
    // With a registry handed in, startup neither loads nor migrates it.
    let startup = method(HOST_MOD, "    pub async fn startup(");
    assert!(!startup.contains("registry::load()"));
    assert!(startup.contains("None => match load_prepared() {"));
}

#[test]
fn the_visible_kick_window_is_a_user_window() {
    let src = norm(KICK_ACCOUNT);
    assert!(src.contains("format!(\"kick-sync-shown-{}\", SEQ.fetch_add(1, Ordering::Relaxed))"));
    assert!(src.contains("format!(\"kick-sync-{}\", SEQ.fetch_add(1, Ordering::Relaxed))"));
}

#[test]
fn both_deep_link_loops_route_store_and_emit_the_stamp() {
    // Fails without the change: the loops matched a badge, then a channel, kept
    // them in two slots (a desktop channel link not at all) and emitted two events.
    let lib = norm(LIB);
    let warm = between(
        &lib,
        "app.deep_link().on_open_url(move |event| {",
        "if let Ok(Some(urls)) = app.deep_link().get_current() {",
    );
    let cold = between(
        &lib,
        "if let Ok(Some(urls)) = app.deep_link().get_current() {",
        "// Create and manage the background service",
    );
    // The notice check stays first; then the one routed arm: store, show (warm
    // only), emit the stamp the store returned.
    in_order(
        warm,
        &[
            "commands::notification_mirror::is_notice_url(&url)",
            "continue;",
            "inbound::arrival_ms(&url)",
            "inbound::without_stamp(&url)",
            "inbound::route(&url)",
            ".store(link, at)",
            "show_main_window(&watch_handle);",
            "\"streamnook:link\", stamp",
            "continue;",
        ],
    );
    assert!(warm.contains("#[cfg(desktop)]\n                            show_main_window(&watch_handle);"));
    in_order(
        cold,
        &[
            "commands::notification_mirror::is_notice_url(&url)",
            "continue;",
            "inbound::arrival_ms(&url)",
            "inbound::without_stamp(&url)",
            "inbound::route(&url)",
            ".store(link, at)",
            "\"streamnook:link\", stamp",
            "continue;",
        ],
    );
    assert!(!cold.contains("show_main_window"), "the window is still being created on a cold start");
    for (name, text) in [("warm", warm), ("cold", cold)] {
        assert_eq!(text.matches("inbound::route(&url)").count(), 1, "{name}: one routed arm");
        assert_eq!(text.matches(".emit(").count(), 1, "{name}: one emit, whatever the kind");
        assert!(
            !text.contains("parse_watch_link(") && !text.contains("badge_link::parse("),
            "{name}: the loops never parse a kind themselves"
        );
    }
}

#[test]
fn the_link_slot_is_imported_at_module_scope_and_managed_outside_the_ios_fence() {
    // Fails without the change: there was no `PendingLink`; the two old slots
    // were the only ones managed.
    let lib = norm(LIB);
    let import = "\nuse services::inbound_link::{self as inbound, PendingLink};\n";
    assert_eq!(lib.matches(import).count(), 1, "one module-scope import, never inside a function");
    assert!(
        lib.find(import).unwrap() < lib.find("pub fn run() {").expect("run()"),
        "the import sits above `run`, with the file's other top-level imports"
    );
    in_order(
        &lib,
        &[
            "app.manage(PendingLink::default());",
            "#[cfg(not(target_os = \"ios\"))]\n            {\n                use tauri_plugin_deep_link::DeepLinkExt;\n\n                let watch_handle",
        ],
    );
}

#[test]
fn the_link_commands_read_the_slot_through_try_state() {
    // Fails without the change: none of these commands existed. `try_state`
    // makes an unmanaged slot a no-op instead of a panic.
    let lib = norm(LIB);
    for sig in ["fn take_pending_link(", "fn peek_pending_link_stamp(", "fn link_superseded("] {
        assert!(body(&lib, sig).contains("try_state::<PendingLink>()"), "{sig} reads the slot with try_state");
    }
    assert!(body(&lib, "fn take_pending_link(").contains("slot.take()"));
    let peek = body(&lib, "fn peek_pending_link_stamp(");
    assert!(peek.contains("slot.peek_stamp()"));
    assert!(!peek.contains("take("), "a peek never clears the slot");
    assert!(body(&lib, "fn link_superseded(").contains("slot.drop_if_older(seen.saturating_add(1))"));
    assert!(lib.contains("#[cfg(desktop)]\n#[tauri::command]\nfn peek_pending_link_stamp("));
    assert!(lib.contains("#[cfg(desktop)]\n#[tauri::command]\nfn link_superseded("));
    assert!(
        body(&lib, "fn category_share_link(")
            .contains("share_link_for(&id, &name, &services::share_link::day_tag())")
    );
    assert!(body(&lib, "fn resolve_category_link(").contains("services::category_link::resolve(&id).await"));
    assert!(body(&lib, "fn inbound_link(").contains("and_then(inbound::route)"));
}

#[test]
fn a_notice_click_takes_its_stamp_on_arrival_and_the_page_reports_it() {
    // Fails without the change: no click took a stamp and `notice_opened` did
    // not exist, so a link stored before a click opened over it.
    let nm = norm(NOTICES);
    let on_activated = body(&nm, "pub fn on_activated(");
    in_order(
        &on_activated,
        &[
            "Use::PageAction(id) => {",
            "next_arrival()",
            "activate(app, &id, seq)",
            "Use::Data(action) => {",
            "next_arrival()",
            "run_action(app, action, seq)",
        ],
    );
    assert_eq!(on_activated.matches("next_arrival()").count(), 2, "a reply and an ignored click take no stamp");
    let open = body(&nm, "pub fn open_mirrored_notification(");
    assert_eq!(first_statement(&open), "let seq = next_arrival();");
    in_order(&open, &["activate(&app, &id, seq)", "run_action(&app, action, Some(seq))"]);
    let opened = body(&nm, "pub fn notice_opened(");
    assert!(opened.contains("try_state::<PendingLink>()"));
    assert!(opened.contains("slot.drop_if_older(seq)"));
    assert!(nm.contains("static PENDING: Lazy<Mutex<Option<(NoticeAction, u64)>>>"));
    assert!(nm.contains("emit_to(\"main\", ACTIVATE_EVENT, payload)"));
}

#[test]
fn the_notice_machinery_keeps_its_shape() {
    // A guard, not coverage: the page-knows set and the one-field `PageAction`
    // stay exactly as they were, so a click is never cleared on a guess.
    let nm = norm(NOTICES);
    assert!(nm.contains("    PageAction(String),\n"));
    assert!(nm.contains("fn remember_page_id(id: &str) {"));
    let mut uses = 0;
    for line in nm.lines() {
        if let Some(at) = line.find("PageAction(") {
            uses += 1;
            let inside = line[at + "PageAction(".len()..].split(')').next().unwrap_or("");
            assert!(!inside.contains(','), "PageAction stays one field: {line}");
        }
    }
    assert!(uses >= 4, "the scan found the declaration and its uses ({uses})");
}

#[test]
fn the_link_event_is_listed_and_the_old_ones_are_not() {
    // Fails without the change: `streamnook:link` was not in the list and the two
    // old events were.
    let names: Vec<&str> = EVENT_NAMES
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    assert!(names.len() > 90, "the list was read");
    assert!(names.contains(&"streamnook:link"));
    for dead in [concat!("streamnook:", "watch"), concat!("streamnook:", "badge")] {
        assert!(!names.contains(&dead), "{dead} is no longer emitted");
    }
}

/// Every file under `dir` whose extension is one of `extensions`, found
/// recursively.
fn files_with(dir: &std::path::Path, extensions: &[&str], out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            files_with(&path, extensions, out);
        } else if path.extension().is_some_and(|e| extensions.iter().any(|x| e == *x)) {
            out.push(path);
        }
    }
}

/// `path: needle` for every needle that appears in a file's text.
fn scan(files: &[std::path::PathBuf], needles: &[&str]) -> Vec<String> {
    let mut hits = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for needle in needles {
            if text.contains(*needle) {
                hits.push(format!("{}: {needle}", path.display()));
            }
        }
    }
    hits
}

#[test]
fn the_old_watch_and_badge_door_is_gone_from_the_core() {
    // A guard, not coverage: two slots, two take commands and two events became
    // one slot, one take command and one event. The needles are built with
    // `concat!`, so this file never spells them, and the scan covers the Rust
    // sources, the permission files and the event list, never this tests folder.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    files_with(&root.join("src"), &["rs"], &mut files);
    files_with(&root.join("permissions"), &["toml"], &mut files);
    files.push(root.join("tests").join("event_names.txt"));
    assert!(files.len() > 100, "the walk found the core's sources ({} files)", files.len());
    let dead = [
        concat!("Pending", "WatchLink"),
        concat!("Pending", "BadgeLink"),
        concat!("take_pending_", "watch_link"),
        concat!("take_pending_", "badge_link"),
        concat!("streamnook:", "watch"),
        concat!("streamnook:", "badge"),
    ];
    let hits = scan(&files, &dead);
    assert!(hits.is_empty(), "the old link door is back:\n{}", hits.join("\n"));
    // Positive control: the same walk finds what replaced them, so a wrong path
    // or an empty glob fails here instead of passing.
    for found in ["PendingLink", "take_pending_link", "streamnook:link"] {
        assert!(!scan(&files, &[found]).is_empty(), "the scan finds {found}");
    }
}

#[test]
fn the_replaced_notice_and_link_machinery_stays_gone() {
    // A guard, not coverage. Rust owns the stamp order now, so no page-side
    // generation counter, notice-clears-link flag or boot gate may come back.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let page = root.join("..").join("src");
    let mut files = Vec::new();
    files_with(&root.join("src"), &["rs"], &mut files);
    files_with(&page, &["ts", "tsx"], &mut files);
    assert!(files.len() > 500, "the walk found both source trees ({} files)", files.len());
    let dead = [
        concat!("notice_clears_", "link"),
        concat!("forget_page_", "ids"),
        concat!("bump", "LinkGeneration"),
        concat!("run", "Activated"),
        concat!("run", "NoticeAction"),
        concat!("link", "Generation"),
        concat!("create", "BootGate"),
    ];
    let hits = scan(&files, &dead);
    assert!(hits.is_empty(), "replaced machinery is back:\n{}", hits.join("\n"));
    for gone in ["noticeActions.ts", "noticeActions.test.ts"] {
        assert!(!page.join("utils").join(gone).exists(), "src/utils/{gone} stays deleted");
    }
    // Positive controls: one needle in each tree that must be found.
    assert!(!scan(&files, &["startLinkDrain"]).is_empty(), "the scan reads the page sources");
    assert!(!scan(&files, &["fn remember_page_id("]).is_empty(), "the scan reads the Rust sources");
}

#[test]
fn the_shells_no_longer_name_the_old_link_door() {
    // A guard, not coverage: the desktop shell, the phone shell and the badge
    // opener once drained the two old slots by these names.
    let page = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("src");
    let shells = [
        page.join("App.tsx"),
        page.join("mobile").join("boot").join("useMobileBoot.ts"),
        page.join("utils").join("badgeLinkOpen.ts"),
    ];
    let dead = [
        concat!("take_pending_", "watch_link"),
        concat!("take_pending_", "badge_link"),
        concat!("streamnook:", "watch"),
        concat!("streamnook:", "badge"),
    ];
    let hits = scan(&shells, &dead);
    assert!(hits.is_empty(), "a shell names the old link door again:\n{}", hits.join("\n"));
    // Positive controls: each shell reads the one door.
    assert!(!scan(&shells[..1], &["startLinkDrain"]).is_empty(), "App.tsx drains through startLinkDrain");
    assert!(!scan(&shells[1..2], &["take_pending_link"]).is_empty(), "the phone shell takes the one slot");
}

#[test]
fn the_iphone_sources_never_gain_the_old_link_guard() {
    // A guard, not coverage. The desktop tree carries no `ios/` folder; the
    // phone tree runs this check.
    let ios = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("ios");
    if !ios.exists() {
        return;
    }
    let mut files = Vec::new();
    for dir in ["Sources", "Tests"] {
        if ios.join(dir).exists() {
            files_with(&ios.join(dir), &["swift"], &mut files);
        }
    }
    assert!(!files.is_empty(), "no Swift sources under ios/");
    let hits = scan(&files, &[concat!("link", "Gen")]);
    assert!(hits.is_empty(), "the old link guard is back:\n{}", hits.join("\n"));
    assert!(!scan(&files, &["import "]).is_empty(), "positive control: the scan reads Swift text");
}

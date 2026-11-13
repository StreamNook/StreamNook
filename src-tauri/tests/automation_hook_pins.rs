//! Where the account automations are wired into existing code. Pinned by
//! source, because a removed hook passes silently: the engine is a no-op
//! until it hears from them.

const IRC: &str = include_str!("../src/services/irc_service.rs");
const LIB: &str = include_str!("../src/lib.rs");
const SETTINGS: &str = include_str!("../src/commands/settings.rs");
const BACKUPS: &str = include_str!("../src/services/settings_backups.rs");
const OUTWARD: &str = include_str!("../src/services/account_automations/outward.rs");
const APP_STORE: &str = include_str!("../src/services/account_store.rs");

fn norm(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// The text from `sig` to the first line that is exactly `end`.
fn between(src: &str, sig: &str, end: &str) -> String {
    let src = norm(src);
    let start = src.find(sig).unwrap_or_else(|| panic!("{sig} not found"));
    let rest = &src[start..];
    rest[..rest
        .find(end)
        .unwrap_or_else(|| panic!("no end after {sig}"))]
        .to_string()
}

#[test]
fn the_lane_notice_and_clearchat_reach_the_engine() {
    let lane = between(IRC, "fn side_effect_lane()", "\n}\n");
    assert!(lane.contains("hand_to_engine(&se);"));
    assert!(
        lane.find("emit_chat_message").unwrap() < lane.find("hand_to_engine(&se);").unwrap(),
        "after the plugin fan-out"
    );
    let hand = between(IRC, "fn hand_to_engine(", "\n}\n");
    assert!(hand.contains("account_automations::on_chat(&se.msg, se.dropped)"));
    let src = norm(IRC);
    let notice = src.find("chat_burst::note_notice(&ch, id);").unwrap();
    assert!(src[notice..notice + 600].contains("account_automations::on_notice(&ch, id, text)"));
    assert!(src.contains(
        "account_automations::on_clearchat(ch, clear.target_user_id.as_deref(), seconds)"
    ));
}

#[test]
fn every_claims_clear_resets_the_engine_and_the_ensured_flag() {
    let src = norm(IRC);
    // The fresh start's seed, `stop` and `stop_irc_only`: three clears, three resets.
    assert_eq!(
        src.matches("account_automations::on_claims_reset();")
            .count(),
        3
    );
    let seed = between(IRC, "consumers.clear();", "on_claims_reset();");
    assert!(
        seed.contains("ensured_clear_all();")
            && seed.contains("ensured_set(&channel.to_lowercase())")
    );
    for sig in [
        "pub async fn stop() -> Result<()>",
        "async fn stop_irc_only()",
    ] {
        let body = between(IRC, sig, "\n    }\n");
        let clear = body
            .find("get_channel_consumers().lock().await.clear();")
            .unwrap();
        let reset = body
            .find("account_automations::on_claims_reset();")
            .unwrap();
        assert!(
            clear < reset && body.contains("ensured_clear_all();"),
            "{sig}"
        );
    }
    assert!(between(IRC, "async fn part_channel(", "\n    }\n").contains("ensured_clear(key);"));
    assert!(
        between(IRC, "async fn part_unclaimed(", "\n    }\n").contains("ensured_keep_only(keep);")
    );
}

#[test]
fn a_window_claim_is_noted_for_the_engine_and_the_start_is_bridge_aware() {
    assert!(between(IRC, "pub async fn join_channel(", "\n    }\n")
        .contains("account_automations::note_window_claim(&key)"));
    let start = between(IRC, "pub async fn start(", "\n    }\n");
    assert!(
        start.contains("bridge_needs_rebuild(")
            && start.contains("Self::ensure_local_ws_bridge().await?;")
    );
    assert!(
        start.find("bridge_needs_rebuild(").unwrap() < start.find("let existing_port").unwrap()
    );
    assert!(start.contains("ensured_set(&key);"));
    let dead = between(
        IRC,
        "WS bridge task is dead; rebuilding bridge",
        "handle.take();",
    );
    assert!(dead.contains("close_page_sockets();"));
}

#[test]
fn the_engine_label_is_not_a_window_and_its_leave_keeps_a_warm_up_room() {
    assert!(between(IRC, "fn held_by_another_window(", "\n}\n").contains("w != ENGINE_LABEL"));
    let leave = between(IRC, "pub async fn leave_channel(", "\n    }\n");
    assert!(leave.contains("engine_leave_keeps_room(window, ensured_has(&key))"));
    assert!(
        leave.find("engine_leave_keeps_room").unwrap()
            < leave.find("Self::part_channel(&key)").unwrap()
    );
}

#[test]
fn the_window_hooks_and_the_exit_flush_are_in_lib() {
    let flush = between(LIB, "pub(crate) fn flush_debounced_stores()", "\n}\n");
    assert!(flush.contains("services::account_automations::flush();"));
    let src = norm(LIB);
    assert!(src.contains(
        "services::badge_feed::start(app_handle.clone());\n            // Account automations"
    ));
    assert!(src.contains("services::account_automations::init(&app_handle);"));
    // Destroyed: adopt before release, both in one task; the window is forgotten.
    let adopt = src
        .find("services::account_automations::adopt_window_chats(&gone).await;")
        .unwrap();
    let release = src
        .find("IrcService::release_window_claims(&gone, None).await;")
        .unwrap();
    assert!(adopt < release);
    assert!(src.contains("services::account_automations::forget_window(&label);"));
    // Close to tray: the lifetime is read before the spawn, the task adopts,
    // stands down when main was reopened, then tags, tells the page and hides.
    let close = between(
        LIB,
        "api.prevent_close();",
        "services::background_mode::refresh_tray(&app_handle);",
    );
    let order = [
        "let lifetime = services::background_mode::main_lifetime();",
        "tauri::async_runtime::spawn(",
        "services::account_automations::adopt_main_chats().await;",
        "if services::background_mode::main_lifetime() != lifetime {",
        "return;",
        "note_keep_alive_close(&app_handle);",
        "emit(\"main-hiding-to-tray\"",
        "main_win.hide()",
        "note_main_hidden_to_tray();",
    ];
    let mut at = 0;
    for needle in order {
        at += close[at..]
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} missing or out of order"));
    }
}

#[test]
fn every_restore_path_pauses() {
    assert!(norm(SETTINGS).contains("pause_restored(&mut imported.automations)"));
    let apply = between(SETTINGS, "pub(crate) fn apply_imported_settings(", "\n}\n");
    assert!(
        apply.find("state.settings.lock()").unwrap() < apply.find("pause_restored(").unwrap(),
        "inside the settings lock"
    );
    assert!(
        apply.find("pause_restored(").unwrap()
            < apply.find("*state_settings = imported.clone();").unwrap()
    );
    assert!(norm(BACKUPS).contains("pause_restored(&mut settings.automations)"));
    let restored = between(
        LIB,
        "if services::install_marker::reconcile(&data, &mut loaded) {",
        "write_settings_to_disk_sync",
    );
    assert!(restored.contains("pause_restored(&mut loaded.automations)"));
}

#[test]
fn outward_names_primary_only_inside_its_one_line_primary_id() {
    let src = norm(OUTWARD);
    // Drop the real `primary_id` (its signature to the closing brace; the
    // trait's bodiless declaration comes first, so find the one with a body),
    // then comments.
    let start = src
        .find("fn primary_id(&self) -> Option<String> {\n        real_only();")
        .expect("the real primary_id");
    let end = start + src[start..].find("\n    }\n").expect("primary_id ends");
    let without = format!("{}{}", &src[..start], &src[end..]);
    let code: String = without
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        code.matches("primary(").count(),
        0,
        "outward.rs reads the registry's main only through primary_id"
    );
    let primary_id = &src[start..end];
    assert!(
        primary_id.lines().filter(|l| !l.trim().is_empty()).count() <= 3,
        "primary_id stays a one-line delegation"
    );
}

#[test]
fn claims_and_releases_send_the_verified_bearer_not_post_json() {
    let src = norm(OUTWARD);
    assert!(
        !src.contains("post_json("),
        "outward.rs names no post_json("
    );
    assert!(src.contains("post_json_with_bearer("));
}

#[test]
fn account_files_carry_the_engine_hooks_and_no_door_changed() {
    assert!(norm(APP_STORE).contains("account_automations::on_accounts_changed();"));
}

#[test]
fn engine_modules_name_no_p2_door_outside_outward() {
    const FORBIDDEN: &[&str] = &[
        "update_chat_settings_as",
        "update_chat_settings_engine",
        "create_stream_marker_engine",
        "modify_channel_info_engine",
        "search_categories_engine",
        "send_chat_announcement_engine",
        "send_shoutout_engine",
        "pin_chat_message_engine",
        "timeout_user_engine",
        "delete_chat_message_engine",
        "moderate_token_bound",
        "stream_manager",
        "slash_dispatch::dispatch",
        "AutomationBackend",
        "slash_backend",
        "AccountStore::primary(",
        "get_token",
    ];
    let count = |line: &str, c: char| line.chars().filter(|x| *x == c).count() as i32;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/services/account_automations");
    let (mut files, mut total) = (0, 0);
    for entry in std::fs::read_dir(&dir).expect("the engine folder").flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".rs") || name == "outward.rs" || name == "testkit.rs" {
            continue;
        }
        let text = std::fs::read_to_string(entry.path()).expect("a source file");
        let (mut skip_depth, mut after_cfg_test, mut scanned) = (0i32, false, 0usize);
        for (n, line) in text.lines().enumerate() {
            let code = line.trim_start();
            // Inside a `#[cfg(test)] mod x { .. }` block: skip it by brace depth, then go on
            // scanning (mod.rs declares `testkit` at line 41 and the engine code goes on after).
            if skip_depth > 0 {
                skip_depth += count(line, '{') - count(line, '}');
                continue;
            }
            if code.starts_with("#[cfg(test)]") {
                after_cfg_test = true;
                continue;
            }
            if after_cfg_test {
                after_cfg_test = false;
                if code.starts_with("mod ") || code.starts_with("pub mod ") || code.starts_with("pub(crate) mod ") {
                    skip_depth = (count(line, '{') - count(line, '}')).max(0);
                }
                continue; // a test-only item: its first line is not engine code
            }
            if code.starts_with("//") {
                continue;
            }
            scanned += 1;
            for word in FORBIDDEN {
                assert!(!line.contains(word), "{name}:{}: `{word}` belongs in outward.rs only", n + 1);
            }
        }
        assert!(scanned >= 8, "{name}: the pin scanned only {scanned} lines (a test module swallowed it?)");
        files += 1;
        total += scanned;
    }
    assert!(files >= 12 && total >= 3000, "the walk saw {files} files and {total} lines");
}

#[test]
fn p2_hooks_stay_in_place() {
    let inbox = include_str!("../src/services/whisper_inbox.rs");
    assert!(inbox.contains("account_automations::engine()") && inbox.contains("on_whisper("), "record_incoming hands every whisper to the engine");
    let send = include_str!("../src/services/whisper_send.rs");
    assert!(send.matches("book_recipient(").count() >= 2, "run_queued_classified and cold_reply both book");
    assert!(send.contains("fn send_in_classified") && send.contains("fn run_queued_classified"));
}

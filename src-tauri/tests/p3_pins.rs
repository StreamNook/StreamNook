//! P3 pins: the hooks, the registrations and the Android speech wiring.

use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &str) -> String {
    // Fold CRLF so `\n}\n` and line patterns match on an autocrlf checkout.
    fs::read_to_string(root().join(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .replace("\r\n", "\n")
}

/// The body of `name` up to the next top-level item, from a source text.
fn body<'a>(src: &'a str, signature: &str) -> &'a str {
    let start = src
        .find(signature)
        .unwrap_or_else(|| panic!("no {signature}"));
    let rest = &src[start..];
    let end = rest.find("\n}\n").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

fn toml_names(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix('"')
                .and_then(|l| l.split('"').next())
                .map(str::to_string)
        })
        .collect()
}

#[test]
fn the_manifest_names_the_tts_service_before_the_application() {
    let manifest = read("gen/android/app/src/main/AndroidManifest.xml");
    let queries = manifest.find("<queries>").expect("a <queries> element");
    assert!(manifest.contains("android.intent.action.TTS_SERVICE"));
    assert!(
        queries < manifest.find("<application").unwrap(),
        "<queries> is a child of <manifest>, before <application>"
    );
}

#[test]
fn speak_android_is_android_only_and_in_the_mobile_permission_file_alone() {
    let mobile = toml_names(&read("permissions/mobile-commands.toml"));
    let app = toml_names(&read("permissions/app-commands.toml"));
    assert!(mobile.iter().any(|n| n == "speak_android"));
    assert!(
        !app.iter().any(|n| n == "speak_android"),
        "a name is in exactly one of the two files"
    );
    let lib = read("src/lib.rs");
    assert!(lib.contains("#[cfg(target_os = \"android\")]\nmod speech_plugin;"));
    assert!(lib.contains(
        "#[cfg(target_os = \"android\")]\n    let builder = builder.plugin(speech_plugin::init());"
    ));
    assert!(
        lib.contains("#[cfg(target_os = \"android\")]\n            speech_plugin::speak_android,")
    );
    let plugin = read("src/speech_plugin.rs");
    assert!(plugin.contains("#![cfg(target_os = \"android\")]"));
    assert!(
        plugin.contains("spawn_blocking")
            && plugin.contains("run_mobile_plugin")
            && plugin.contains("register_android_plugin")
    );
    let ios_parity = read("tests/ios_invoke_parity.rs");
    assert!(
        !ios_parity.contains("speak_android"),
        "an Android-only command is outside iOS parity and needs no exemption"
    );
}

#[test]
fn the_reward_list_is_registered_on_every_target() {
    let lib = read("src/lib.rs");
    assert!(lib.contains("commands::account_automations::automations_own_rewards,"));
    assert!(toml_names(&read("permissions/app-commands.toml"))
        .iter()
        .any(|n| n == "automations_own_rewards"));
    assert!(!toml_names(&read("permissions/mobile-commands.toml"))
        .iter()
        .any(|n| n == "automations_own_rewards"));
}

#[test]
fn the_speak_event_is_registered_and_sent_by_literal() {
    assert!(read("tests/event_names.txt")
        .lines()
        .any(|l| l == "automations://speak"));
    assert!(read("src/services/account_automations/local.rs").contains("\"automations://speak\""));
}

#[test]
fn every_p3_hook_is_in_place() {
    let whisper = read("src/services/whisper_service.rs");
    let handler = body(&whisper, "fn handle_text(");
    let route = handler
        .find("TopicKind::from_sub_type")
        .expect("frames routed by subscription type");
    assert!(
        route < handler.find("event.get(\"whisper\")").unwrap(),
        "own topics before the whisper parse"
    );
    assert!(handler.contains("account_automations::on_own_topic(kind, event)"));
    assert!(
        handler.contains("own_revoked(kind"),
        "a revocation becomes that topic's status"
    );
    let state = read("src/services/channel_state.rs");
    assert!(
        state.matches("account_automations::on_live_edge(").count() >= 2,
        "both channel_state writers"
    );
    assert!(state.contains("live_edge(\n                    s.answered,"));
    assert!(read("src/services/account_automations/reconcile.rs").contains("hold_own_topics"));
    assert!(
        read("src/services/account_automations/reconcile.rs").contains("set_automation_highlights")
    );
    assert!(read("src/services/chat_rules.rs").contains("automation_highlight(facts)"));
}

#[test]
fn a_session_reconnect_keeps_the_subscriptions_and_a_new_session_subscribes_afresh() {
    let whisper = read("src/services/whisper_service.rs");
    let run = body(&whisper, "async fn run_session(");
    let subscribed = run.find("subscribe_to_whispers(").unwrap();
    let new_subs = run
        .find("OwnSubs::new(")
        .expect("a session's own subscriptions");
    assert!(
        subscribed < new_subs,
        "built only after the whisper subscription went through"
    );
    assert_eq!(run.matches("OwnSubs::new(").count(), 1);
    // The migration arm moves the session id and touches no own subscription.
    let moved = &run[run.find("moved = async").unwrap()..];
    let moved = &moved[..moved.find("continue;").unwrap()];
    assert!(
        !moved.contains("own.sync") && !moved.contains("own.release"),
        "a session_reconnect re-subscribes nothing"
    );
}

#[test]
fn speak_never_books_claims_or_posts() {
    let exec = read("src/services/account_automations/executor.rs");
    let speak = body(&exec, "async fn speak_action(");
    for forbidden in [
        "book(",
        "claims::",
        "chat_send",
        "outward_action(",
        "engine.claims",
        "engine.outward",
    ] {
        assert!(
            !speak.contains(forbidden),
            "speak_action must not name {forbidden}"
        );
    }
    assert!(
        speak.contains("outward_text_ok"),
        "the chatter-text rule applies to what is read aloud"
    );
    let routing = read("src/services/account_automations/speak.rs");
    for forbidden in ["chat_send", "claim_post", "send_message"] {
        assert!(
            !routing.contains(forbidden),
            "nothing is ever posted to chat: {forbidden}"
        );
    }
}

#[test]
fn the_reward_picker_and_the_topics_are_helix_not_gql() {
    for path in [
        "src/services/account_automations/own_topics.rs",
        "src/services/account_automations/speak.rs",
        "src/speech_plugin.rs",
    ] {
        let text = read(path).to_ascii_lowercase();
        assert!(
            !text.contains("graphql") && !text.contains("gql"),
            "{path}: a GQL call needs a private watchdog manifest entry"
        );
    }
    let twitch = read("src/services/twitch_service.rs");
    let rewards = body(&twitch, "pub async fn custom_rewards_engine(");
    assert!(
        rewards.contains("channel_points/custom_rewards")
            && !rewards.to_ascii_lowercase().contains("gql")
    );
}

#[test]
fn only_the_door_names_the_reward_helper_and_no_engine_module_reads_a_token() {
    let dir = root().join("src/services/account_automations");
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name == "testkit.rs" {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        // The tests module may name anything; the pin reads the production text.
        let production = text.split("#[cfg(test)]\nmod tests").next().unwrap_or("");
        if name != "outward.rs" {
            assert!(
                !production.contains("custom_rewards_engine"),
                "{name} names the reward helper"
            );
            assert!(!production.contains("get_token"), "{name} reads a token");
        }
    }
}

#[test]
fn the_android_plugin_never_asks_for_audio_focus() {
    let kotlin = read("gen/android/app/src/main/java/app/streamnook/SpeechPlugin.kt");
    for forbidden in [
        "requestAudioFocus",
        "AudioFocusRequest",
        "abandonAudioFocus",
    ] {
        assert!(!kotlin.contains(forbidden), "{forbidden}");
    }
    assert!(kotlin.contains("QUEUE_ADD") && kotlin.contains("TextToSpeech"));
}

const MAIN_ACTIVITY: &str = "gen/android/app/src/main/java/app/streamnook/MainActivity.kt";

/// The text from `from` up to the first `to` after it.
fn cut<'a>(src: &'a str, from: &str, to: &str) -> &'a str {
    let start = src.find(from).unwrap_or_else(|| panic!("no {from}"));
    let rest = &src[start..];
    let end = rest[from.len()..]
        .find(to)
        .unwrap_or_else(|| panic!("no {to:?} after {from}"));
    &rest[..from.len() + end]
}

/// Every needle appears, each after the one before it.
fn in_order(text: &str, needles: &[&str]) {
    let mut at = 0;
    for needle in needles {
        let found = text[at..]
            .find(needle)
            .unwrap_or_else(|| panic!("`{needle}` missing or out of order"));
        at += found + needle.len();
    }
}

#[test]
fn a_link_intent_is_stamped_before_the_plugins_see_it() {
    // Fails if a link reaches `super` (and so the deep-link plugin) without its arrival time.
    let kotlin = read(MAIN_ACTIVITY);
    let rule = cut(&kotlin, "private fun isForwardedLink(", "\n\n");
    for needle in [
        "Intent.ACTION_VIEW",
        "\"org.chromium.arc.intent.action.VIEW\"",
        "intent.data != null",
    ] {
        assert!(rule.contains(needle), "isForwardedLink spells {needle}");
    }
    let stamp = cut(&kotlin, "private fun stampedLink(", "\n  }\n");
    in_order(
        stamp,
        &["SystemClock.elapsedRealtime()", "setDataAndType(", "encodedFragment(\"sn-at=$at\")"],
    );
    let warm = cut(&kotlin, "override fun onNewIntent(", "\n  }\n");
    in_order(
        warm,
        &[
            "if (isForwardedLink(intent)) {",
            "val stamped = stampedLink(intent)",
            "setIntent(stamped)",
            "super.onNewIntent(stamped)",
            "return",
            "super.onNewIntent(intent)",
        ],
    );
    let create = cut(&kotlin, "override fun onCreate(", "super.onCreate(savedInstanceState)");
    in_order(
        create,
        &[
            "FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY",
            "setIntent(Intent(intent).apply { data = null",
            "isForwardedLink(intent)",
            "setIntent(stamped)",
        ],
    );
}

#[test]
fn every_tap_stash_is_written_with_its_time_under_the_tap_lock() {
    // Fails if a stash goes back to a per-activity field, or is written without its time or outside the lock.
    let kotlin = read(MAIN_ACTIVITY);
    assert_eq!(kotlin.matches("private val tapLock = Any()").count(), 1);
    let companion = cut(&kotlin, "  companion object {", "\n  }\n");
    assert!(companion.contains("private val tapLock = Any()"), "one lock for the process");
    for name in ["pendingChannel", "pendingMention", "pendingWhisper"] {
        assert!(companion.contains(&format!("private var {name}: String? = null")), "{name}");
        assert!(companion.contains(&format!("private var {name}At: Long = 0L")), "{name}At");
        assert!(!kotlin.contains(&format!("@Volatile private var {name}")), "no per-activity {name}");
    }
    let lines: Vec<&str> = kotlin.lines().collect();
    let mut writes = 0;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        for name in ["pendingChannel", "pendingMention", "pendingWhisper"] {
            if !t.starts_with(&format!("{name} = ")) || t.ends_with("= null") {
                continue;
            }
            writes += 1;
            assert!(
                lines[i + 1].trim().starts_with(&format!("{name}At = ")),
                "line {}: {name} is written with its time",
                i + 1
            );
            let opener = lines[..i].iter().rev().find(|l| {
                let l = l.trim_start();
                l.contains("synchronized(tapLock)") || l.starts_with("fun ") || l.contains(" fun ")
            });
            assert!(
                opener.is_some_and(|l| l.contains("synchronized(tapLock)")),
                "line {}: {name} is written under tapLock",
                i + 1
            );
        }
    }
    assert_eq!(writes, 6, "three warm stashes and three cold ones");
}

#[test]
fn the_bridge_answers_the_page_from_the_shared_stashes() {
    // Fails if a name the page calls is missing from InsetsBridge, or a tap is read and cleared outside the lock.
    let kotlin = read(MAIN_ACTIVITY);
    let bridge = cut(&kotlin, "inner class InsetsBridge {", "\n  }\n");
    for name in [
        "consumePendingTap",
        "consumePendingChannelIf",
        "consumePendingMentionIf",
        "consumePendingWhisperIf",
        "bootNowMs",
        "isPipClosing",
    ] {
        assert!(
            bridge.contains(&format!("@JavascriptInterface\n    fun {name}(")),
            "{name} is a bridge method"
        );
    }
    assert!(bridge.contains("fun bootNowMs(): Long = SystemClock.elapsedRealtime()"));
    assert!(bridge.contains("fun isPipClosing(): Boolean = pipClosedPending"));
    for name in [
        "consumePendingChannel",
        "consumePendingMention",
        "consumePendingWhisper",
        "consumePendingTap",
        "consumePendingChannelIf",
        "consumePendingMentionIf",
        "consumePendingWhisperIf",
    ] {
        let body = cut(bridge, &format!("fun {name}("), "\n    }\n");
        assert!(body.contains("synchronized(tapLock)"), "{name} reads under tapLock");
    }
}

#[test]
fn a_dismissed_pip_window_is_marked_before_sn_pip_and_a_finished_owner_clears_the_taps() {
    // Fails if sn:pip goes out before the dismissal mark, or a finishing activity leaves taps behind.
    let kotlin = read(MAIN_ACTIVITY);
    let pip = cut(&kotlin, "override fun onPictureInPictureModeChanged(", "\n  }\n");
    in_order(
        pip,
        &[
            "val closing = !isInPictureInPictureMode && lifecycle.currentState == Lifecycle.State.CREATED",
            "if (closing) pipClosedPending = true",
            "new CustomEvent('sn:pip'",
            "if (closing) {",
            "sn:pip-closed",
        ],
    );
    assert_eq!(pip.matches("pipClosedPending = true").count(), 1);
    let stash = cut(&kotlin, "val coldAt = SystemClock.elapsedRealtime()", "\n    }\n");
    in_order(stash, &["synchronized(tapLock) {", "owner = this", "if (!restored) {"]);
    let destroy = cut(&kotlin, "override fun onDestroy() {", "unregisterReceiver(muteReceiver)");
    in_order(
        destroy,
        &[
            "synchronized(tapLock) {",
            "if (isFinishing && owner === this) clearTapsLocked()",
            "if (owner === this) owner = null",
        ],
    );
}

//! P3 pins: the hooks, the registrations and the Android speech wiring.

use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &str) -> String {
    fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
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

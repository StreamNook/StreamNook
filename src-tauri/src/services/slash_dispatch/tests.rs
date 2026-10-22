//! The dispatcher against a fake app: every command call, notice and toast is
//! recorded, so each case checks exactly what a typed line would do. Ported
//! from the page's own dispatcher tests, with the Helix answers as fixtures.

use super::*;
use std::sync::Mutex as StdMutex;

type Answer = Box<dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync>;

struct Fake {
    calls: StdMutex<Vec<(String, Value)>>,
    notices: StdMutex<Vec<(String, String)>>,
    me: StdMutex<Option<Me>>,
    role: StdMutex<Role>,
    settings: StdMutex<Value>,
    answer: StdMutex<Answer>,
    open: StdMutex<bool>,
    history: StdMutex<Vec<SweepEntry>>,
    latest: StdMutex<HashMap<String, String>>,
    whisper: StdMutex<Option<(String, String, String)>>,
}

/// Helix and app answers the cases share.
fn default_answer(cmd: &str, _args: &Value) -> Result<Value, String> {
    match cmd {
        "get_user_by_login" => Ok(json!({ "id": "7", "login": "bob", "display_name": "Bob" })),
        "get_active_poll" => Ok(json!({ "id": "poll-9" })),
        "chat_send_paced" => Ok(json!({ "is_sent": true, "drop_reason": null })),
        "run_chat_burst" => Ok(json!({ "message": "Spam started: 2 messages at 1000 ms apart. /spam stop ends it", "status": null })),
        c if c.starts_with("slash_") => Ok(json!("ok")),
        _ => Ok(Value::Null),
    }
}

impl Fake {
    fn new() -> Arc<Fake> {
        Arc::new(Fake {
            calls: StdMutex::new(Vec::new()),
            notices: StdMutex::new(Vec::new()),
            me: StdMutex::new(Some(Me { id: "1".into(), login: "me".into(), display_name: "Me".into() })),
            role: StdMutex::new(Role::Moderator),
            settings: StdMutex::new(json!({ "chat_commands": { "user_commands": [] }, "show_mod_logs": false })),
            answer: StdMutex::new(Box::new(default_answer)),
            open: StdMutex::new(true),
            history: StdMutex::new(Vec::new()),
            latest: StdMutex::new(HashMap::new()),
            whisper: StdMutex::new(None),
        })
    }
    fn answer(&self, f: impl Fn(&str, &Value) -> Result<Value, String> + Send + Sync + 'static) {
        *self.answer.lock().unwrap() = Box::new(f);
    }
    fn calls(&self, cmd: &str) -> Vec<Value> {
        self.calls.lock().unwrap().iter().filter(|(c, _)| c == cmd).map(|(_, a)| a.clone()).collect()
    }
    fn notices(&self) -> Vec<(String, String)> {
        self.notices.lock().unwrap().clone()
    }
    fn login_as(&self, login: &str) {
        *self.me.lock().unwrap() = Some(Me { id: "1".into(), login: login.into(), display_name: login.into() });
    }
}

impl Backend for Fake {
    fn invoke<'a>(&'a self, cmd: &'a str, args: Value) -> BoxFut<'a, Result<Value, String>> {
        self.calls.lock().unwrap().push((cmd.to_string(), args.clone()));
        let answer = self.answer.lock().unwrap();
        let out = (*answer)(cmd, &args);
        drop(answer);
        Box::pin(async move { out })
    }
    fn me(&self) -> Option<Me> {
        self.me.lock().unwrap().clone()
    }
    fn role(&self, _channel: &str) -> Role {
        *self.role.lock().unwrap()
    }
    fn settings(&self) -> Value {
        self.settings.lock().unwrap().clone()
    }
    fn last_whisper(&self) -> Option<(String, String, String)> {
        self.whisper.lock().unwrap().clone()
    }
    fn history(&self, _channel: &str, since_ms: i64) -> Vec<SweepEntry> {
        self.history.lock().unwrap().iter().filter(|e| e.ts_ms >= since_ms).cloned().collect()
    }
    fn latest_message_id(&self, _channel: &str, login: &str) -> Option<String> {
        self.latest.lock().unwrap().get(&login.to_lowercase()).cloned()
    }
    fn is_open<'a>(&'a self, _channel: &'a str) -> BoxFut<'a, bool> {
        let open = *self.open.lock().unwrap();
        Box::pin(async move { open })
    }
    fn post_notice<'a>(&'a self, channel: &'a str, text: String) -> BoxFut<'a, ()> {
        self.notices.lock().unwrap().push((channel.to_string(), text));
        Box::pin(async {})
    }
    fn now_ms(&self) -> i64 {
        1_000_000_000
    }
    fn new_id(&self) -> String {
        "new-id".to_string()
    }
}

fn ctx(fake: &Arc<Fake>, channel: &str, platform: Platform) -> Ctx {
    Ctx { b: fake.clone(), channel: channel.to_string(), broadcaster_id: "42".to_string(), platform }
}

async fn run_in(fake: &Arc<Fake>, channel: &str, line: &str) -> Outcome {
    dispatch(ctx(fake, channel, Platform::Desktop), line.to_string(), 0).await
}

async fn run(fake: &Arc<Fake>, line: &str) -> Outcome {
    run_in(fake, "chan", line).await
}

fn toast(o: &Outcome, kind: ToastKind) -> Vec<String> {
    o.toasts.iter().filter(|t| t.kind == kind).map(|t| t.text.clone()).collect()
}

// ───────────────────────────── table and aliases ─────────────────────────────

#[test]
fn aliases_map_onto_one_implementation() {
    assert_eq!(resolve_alias("b"), "ban");
    assert_eq!(resolve_alias("u"), "unban");
    assert_eq!(resolve_alias("p"), "purge");
    assert_eq!(resolve_alias("sub"), "subscribers");
    assert_eq!(resolve_alias("suboff"), "subscribersoff");
    assert_eq!(resolve_alias("r9kbeta"), "uniquechat");
    assert_eq!(resolve_alias("deletepoll"), "cancelpoll");
    assert_eq!(resolve_alias("rejoin"), "reconnect");
    assert_eq!(resolve_alias("unbanall"), "massunban");
    assert_eq!(resolve_alias("ban"), "ban");
    assert_eq!(find_builtin("B").map(|b| b.name), Some("ban"));
}

#[test]
fn aliases_never_collide_with_a_name_or_each_other() {
    let names: Vec<&str> = BUILTINS.iter().map(|b| b.name).collect();
    let mut seen = std::collections::HashSet::new();
    for b in BUILTINS {
        for a in b.aliases {
            assert!(!names.contains(a), "{a} is also a command name");
            assert!(seen.insert(*a), "{a} is listed twice");
        }
    }
    let unique: std::collections::HashSet<_> = names.iter().collect();
    assert_eq!(unique.len(), names.len());
}

#[test]
fn the_names_custom_commands_yield_to_match_the_page_list() {
    // The settings page flags a custom command that shares a name; it reads
    // its own copy of this list.
    let page = include_str!("../../../../src/utils/chatCommands.ts");
    let start = page.find("BUILT_IN_FIRST_TRIGGERS: ReadonlySet<string> = new Set([").expect("page list");
    let end = page[start..].find("]);").map(|e| start + e).expect("page list end");
    let names: Vec<&str> = page[start..end].split('\'').skip(1).step_by(2).collect();
    assert_eq!(names, BUILT_IN_FIRST, "the custom-command precedence list drifted from src/utils/chatCommands.ts");
}

#[tokio::test]
async fn every_listed_command_is_handled_bare_and_with_its_examples() {
    let fake = Fake::new();
    fake.login_as("chan");
    let mut lines = Vec::new();
    for b in BUILTINS.iter().filter(|b| dev_build() || !b.dev_only()) {
        if matches!(b.name, "me" | "spam" | "pyramid" | "timer" | "chain" | "foreach" | "runin") {
            continue;
        }
        lines.push(format!("/{}", b.name));
        lines.extend(b.aliases.iter().map(|a| format!("/{a}")));
        lines.extend(b.examples.iter().map(|e| e.to_string()));
    }
    for line in lines {
        if line.starts_with("/timer") || line.starts_with("/chain") || line.starts_with("/foreach") || line.starts_with("/runin") {
            continue;
        }
        let o = run(&fake, &line).await;
        assert!(o.handled, "{line} was not handled");
        assert!(o.clear_draft, "{line} keeps the draft");
    }
    assert!(fake.calls("send_chat_message").is_empty());
}

#[tokio::test]
async fn me_goes_to_chat_as_typed() {
    let fake = Fake::new();
    let o = run(&fake, "/me waves").await;
    assert!(!o.handled);
    assert!(!o.clear_draft);
    assert!(!run(&fake, "/nosuchcommand x").await.handled);
}

#[tokio::test]
async fn uptime_says_offline_only_when_twitch_says_so() {
    let fake = Fake::new();
    fake.answer(|cmd, _| if cmd == "get_stream" { Err("Not logged in".into()) } else { Ok(Value::Null) });
    let failed = run(&fake, "/uptime").await;
    assert!(failed.notices.is_empty());
    assert_eq!(toast(&failed, ToastKind::Error), ["Couldn't check whether this channel is live: Not logged in"]);
    fake.answer(|_, _| Ok(Value::Null));
    assert_eq!(run(&fake, "/uptime").await.notices, ["This channel is offline."]);
    fake.answer(|cmd, _| {
        if cmd == "get_stream" {
            Ok(json!({ "user_name": "Chan", "started_at": "2001-09-09T01:46:40Z" }))
        } else {
            Ok(Value::Null)
        }
    });
    assert!(run(&fake, "/uptime").await.notices[0].starts_with("Chan has been live for"));
}

#[tokio::test]
async fn chatters_emoteinfo_and_openurl_show_the_real_failure() {
    let fake = Fake::new();
    fake.answer(|cmd, _| match cmd {
        "get_channel_chatters" | "emote_counts" | "open_url" => Err("Twitch answered HTTP 500".into()),
        _ => Ok(Value::Null),
    });
    assert_eq!(toast(&run(&fake, "/chatters").await, ToastKind::Error), ["Couldn't count the chatters: Twitch answered HTTP 500"]);
    assert_eq!(toast(&run(&fake, "/emoteinfo").await, ToastKind::Error), ["Couldn't read this channel's emotes: Twitch answered HTTP 500"]);
    assert_eq!(toast(&run(&fake, "/openurl example.com").await, ToastKind::Error), ["Couldn't open example.com: Twitch answered HTTP 500"]);
}

#[tokio::test]
async fn a_failed_disconnect_is_shown() {
    let fake = Fake::new();
    fake.answer(|cmd, _| if cmd == "stop_chat" { Err("socket busy".into()) } else { Ok(Value::Null) });
    assert_eq!(toast(&run(&fake, "/disconnect").await, ToastKind::Error), ["Couldn't disconnect from chat: socket busy"]);
}

#[tokio::test]
async fn a_failed_lookup_is_never_called_not_found() {
    let fake = Fake::new();
    fake.answer(|cmd, _| if cmd == "get_user_by_login" { Err("User 'ghost' not found".into()) } else { Ok(Value::Null) });
    assert_eq!(toast(&run(&fake, "/usercard ghost").await, ToastKind::Error), ["User ghost not found"]);
    fake.answer(|cmd, _| if cmd == "get_user_by_login" { Err("error sending request".into()) } else { Ok(Value::Null) });
    assert_eq!(toast(&run(&fake, "/usercard bob").await, ToastKind::Error), ["Couldn't look up bob: error sending request"]);
}

#[tokio::test]
async fn ending_a_poll_says_why_it_could_not() {
    let fake = Fake::new();
    // A moderator: Twitch takes this only from the owner, so it is refused up front.
    let mod_try = run(&fake, "/endpoll").await;
    assert_eq!(toast(&mod_try, ToastKind::Error), ["/endpoll: only the broadcaster can end a poll on Twitch"]);
    assert!(fake.calls("get_active_poll").is_empty());

    *fake.role.lock().unwrap() = Role::Broadcaster;
    // A lookup Twitch refused is an error, never "no poll".
    fake.answer(|cmd, _| if cmd == "get_active_poll" { Err("you are not a moderator in this channel".into()) } else { Ok(Value::Null) });
    let refused = run(&fake, "/cancelpoll").await;
    assert_eq!(toast(&refused, ToastKind::Error), ["Couldn't read the running poll: you are not a moderator in this channel"]);
    // No poll really running.
    fake.answer(|_, _| Ok(Value::Null));
    let none = run(&fake, "/endpoll").await;
    assert_eq!(toast(&none, ToastKind::Error), ["No poll is running right now"]);
}

#[tokio::test]
async fn mods_and_vips_run_here_and_never_reach_chat() {
    let fake = Fake::new();
    fake.answer(|cmd, a| match cmd {
        "slash_role_list" => Ok(json!([format!("{} of this channel", a["role"].as_str().unwrap_or_default())])),
        _ => Ok(Value::Null),
    });
    for line in ["/mods", "/vips"] {
        let o = run(&fake, line).await;
        assert!(o.handled && o.send.is_none(), "{line} must run here");
        assert_eq!(o.notices.len(), 1);
    }
    assert!(fake.calls("send_chat_message").is_empty() && fake.calls("chat_send_paced").is_empty());
}

#[tokio::test]
async fn a_failed_mods_list_is_an_error_never_sent() {
    let fake = Fake::new();
    fake.answer(|cmd, _| if cmd == "slash_role_list" { Err("GQL request failed".into()) } else { Ok(Value::Null) });
    let o = run(&fake, "/mods").await;
    assert!(o.handled && o.send.is_none());
    assert!(o.notices.is_empty(), "no \"There are no moderators\" line");
    assert_eq!(toast(&o, ToastKind::Error), ["Command failed: GQL request failed"]);
    assert!(fake.calls("send_chat_message").is_empty());
}

#[tokio::test]
async fn aliases_make_the_same_call_as_the_full_name() {
    let fake = Fake::new();
    run(&fake, "/b bob spamming").await;
    run(&fake, "/ban bob spamming").await;
    let bans = fake.calls("ban_user");
    assert_eq!(bans.len(), 2);
    assert_eq!(bans[0], bans[1]);
    assert_eq!(bans[0], json!({ "broadcasterId": "42", "targetUserId": "7", "duration": null, "reason": "spamming" }));
}

#[tokio::test]
async fn ban_toasts_the_typed_name_and_an_unknown_user() {
    let fake = Fake::new();
    let o = run(&fake, "/ban @Bob").await;
    assert_eq!(toast(&o, ToastKind::Success), vec!["Banned bob"]);
    assert_eq!(fake.calls("get_user_by_login"), vec![json!({ "login": "bob" })]);
    fake.answer(|cmd, a| if cmd == "get_user_by_login" { Err("not found".into()) } else { default_answer(cmd, a) });
    let o = run(&fake, "/ban ghost").await;
    assert_eq!(toast(&o, ToastKind::Error), vec!["User ghost not found"]);
}

#[tokio::test]
async fn purge_times_out_for_one_second() {
    let fake = Fake::new();
    run(&fake, "/p bob").await;
    run(&fake, "/purge bob rude").await;
    assert_eq!(
        fake.calls("ban_user"),
        vec![
            json!({ "broadcasterId": "42", "targetUserId": "7", "duration": 1, "reason": null }),
            json!({ "broadcasterId": "42", "targetUserId": "7", "duration": 1, "reason": "rude" }),
        ]
    );
}

#[tokio::test]
async fn sub_turns_on_sub_only_chat() {
    let fake = Fake::new();
    let o = run(&fake, "/sub").await;
    assert_eq!(fake.calls("update_chat_settings"), vec![json!({ "broadcasterId": "42", "settings": { "subscriber_mode": true } })]);
    assert_eq!(toast(&o, ToastKind::Success), vec!["Subscribers-only mode enabled"]);
}

#[tokio::test]
async fn slow_and_followers_read_their_lengths() {
    let fake = Fake::new();
    let o = run(&fake, "/slow").await;
    assert_eq!(toast(&o, ToastKind::Success), vec!["Slow mode enabled (30s)"]);
    run(&fake, "/slow 10").await;
    run(&fake, "/followers 1d").await;
    run(&fake, "/followers").await;
    run(&fake, "/followers 2mo").await;
    let calls = fake.calls("update_chat_settings");
    assert_eq!(calls[1]["settings"], json!({ "slow_mode": true, "slow_mode_wait_time": 10 }));
    assert_eq!(calls[2]["settings"]["follower_mode_duration"], json!(1440));
    assert_eq!(calls[3]["settings"]["follower_mode_duration"], json!(0));
    assert_eq!(calls[4]["settings"]["follower_mode_duration"], json!(86_400));
}

#[tokio::test]
async fn deletepoll_archives_the_running_poll() {
    let fake = Fake::new();
    // Polls are the channel owner's to end.
    *fake.role.lock().unwrap() = Role::Broadcaster;
    let o = run(&fake, "/deletepoll").await;
    assert_eq!(fake.calls("end_poll"), vec![json!({ "broadcasterId": "42", "pollId": "poll-9", "status": "ARCHIVED" })]);
    assert_eq!(toast(&o, ToastKind::Success), vec!["Poll canceled"]);
}

#[tokio::test]
async fn vote_followage_and_gift_pass_their_arguments() {
    let fake = Fake::new();
    run(&fake, "/vote 2").await;
    assert_eq!(fake.calls("slash_vote"), vec![json!({ "channelId": "42", "choice": "2" })]);
    run(&fake, "/followage alice otherchan").await;
    run(&fake, "/followed").await;
    assert_eq!(
        fake.calls("slash_followage"),
        vec![json!({ "channel": "otherchan", "user": "alice" }), json!({ "channel": "chan", "user": null })]
    );
    let o = run(&fake, "/gift 5").await;
    assert_eq!(
        fake.calls("open_subscribe_window"),
        vec![json!({ "channelLogin": "chan", "title": "Gift subs to chan", "provider": "twitch" })]
    );
    assert_eq!(o.notices, vec!["Opened chan's sub page. Choose \"Gift a sub\" and pick 5. Payment happens on Twitch."]);
}

#[tokio::test]
async fn automod_aliases_choose_the_verb() {
    let fake = Fake::new();
    run(&fake, "/automod_approve abc").await;
    run(&fake, "/automod deny").await;
    assert_eq!(
        fake.calls("slash_automod"),
        vec![
            json!({ "channel": "chan", "messageId": "abc", "allow": true }),
            json!({ "channel": "chan", "messageId": null, "allow": false }),
        ]
    );
}

#[tokio::test]
async fn reply_sends_through_the_composer_with_the_parent() {
    let fake = Fake::new();
    fake.answer(|cmd, a| if cmd == "slash_last_message" { Ok(json!({ "id": "m-5", "display_name": "Bob" })) } else { default_answer(cmd, a) });
    let o = run(&fake, "/reply @bob good point").await;
    assert_eq!(o.send.as_deref(), Some("good point"));
    assert_eq!(o.reply_to.as_deref(), Some("m-5"));
}

#[tokio::test]
async fn missing_arguments_show_usage_instead_of_reaching_chat() {
    let fake = Fake::new();
    let o = run(&fake, "/ban").await;
    assert!(o.handled);
    assert_eq!(toast(&o, ToastKind::Info), vec!["Usage: /ban <username> [reason]"]);
    assert_eq!(o.notices, vec!["Usage: /ban <username> [reason]"]);
    let o = run(&fake, "/purge").await;
    assert_eq!(toast(&o, ToastKind::Info), vec!["Usage: /purge <username> [reason]"]);
    assert!(o.notices.is_empty());
    let o = run(&fake, "/w bob").await;
    assert_eq!(toast(&o, ToastKind::Info), vec!["Usage: /w <username> <message>"]);
    assert!(fake.calls("ban_user").is_empty());
}

#[tokio::test]
async fn editor_asks_for_confirmation_before_granting() {
    let fake = Fake::new();
    fake.login_as("editroom");
    let o = run_in(&fake, "editroom", "/editor bob").await;
    assert!(fake.calls("slash_editor").is_empty());
    assert_eq!(
        o.notices,
        vec!["Editors can change your stream title, category, ads and more. Make bob an editor? Run the same command again within 20 seconds to confirm."]
    );
    run_in(&fake, "editroom", "/editor bob").await;
    assert_eq!(fake.calls("slash_editor"), vec![json!({ "broadcasterId": "42", "target": "bob", "add": true })]);
}

#[tokio::test]
async fn broadcaster_only_commands_refuse_viewers_before_calling_twitch() {
    let fake = Fake::new();
    let o = run(&fake, "/raidsend").await;
    assert_eq!(toast(&o, ToastKind::Error), vec!["/raidsend is for the broadcaster of this channel"]);
    run(&fake, "/massunban").await;
    assert!(fake.calls("slash_raid_now").is_empty());
    assert!(fake.calls("slash_mass_unban_start").is_empty());
}

#[tokio::test]
async fn chain_runs_each_step_in_order() {
    let fake = Fake::new();
    run(&fake, "/chain /slow 10 | /slowoff").await;
    assert_eq!(
        fake.calls("update_chat_settings"),
        vec![
            json!({ "broadcasterId": "42", "settings": { "slow_mode": true, "slow_mode_wait_time": 10 } }),
            json!({ "broadcasterId": "42", "settings": { "slow_mode": false } }),
        ]
    );
}

#[tokio::test]
async fn foreach_fills_in_each_item() {
    let fake = Fake::new();
    run(&fake, "/foreach alice,bob /shoutout $").await;
    assert_eq!(fake.calls("send_shoutout").len(), 2);
    let o = run(&fake, "/foreach a,b /shoutout").await;
    assert_eq!(toast(&o, ToastKind::Error), vec!["Put $ where each item goes, for example /foreach a,b /shoutout $"]);
}

#[tokio::test]
async fn say_sends_text_without_running_it() {
    let fake = Fake::new();
    let o = run(&fake, "/say !discord").await;
    assert_eq!(o.send.as_deref(), Some("!discord"));
    assert!(fake.calls("chat_send_paced").is_empty());
}

#[tokio::test]
async fn shrug_lurk_and_squishy_send_text() {
    let fake = Fake::new();
    assert_eq!(run(&fake, "/shrug").await.send.as_deref(), Some("¯\\_(ツ)_/¯"));
    assert_eq!(run(&fake, "/shrug no idea").await.send.as_deref(), Some("no idea ¯\\_(ツ)_/¯"));
    let lurk = run(&fake, "/lurk").await.send.unwrap();
    assert!(lurk.strip_prefix("/me ").is_some_and(|l| crate::services::slash_commands::LURK_LINES.contains(&l)), "{lurk}");
    let lurk = run(&fake, "/lurk dinner").await.send.unwrap();
    assert!(lurk.starts_with("/me ") && lurk.ends_with(": dinner"), "{lurk}");
    assert!(run(&fake, "/squishy").await.send.unwrap().starts_with("notsquishY WHEN"));
}

#[tokio::test]
async fn a_provider_room_runs_only_commands_that_need_no_twitch_api() {
    let fake = Fake::new();
    let o = run_in(&fake, "kick:somechan", "/echo hi").await;
    assert_eq!(o.notices, vec!["hi"]);
    assert!(!run_in(&fake, "kick:somechan", "/ban bob").await.handled);
    // /me is Twitch's: another platform gets the words.
    let lurk = run_in(&fake, "kick:somechan", "/lurk").await.send.unwrap();
    assert!(crate::services::slash_commands::LURK_LINES.contains(&lurk.as_str()), "{lurk}");
    assert!(!run_in(&fake, "kick:somechan", "/me waves").await.handled);
}

// ───────────────────────────── scripts and confirmations ─────────────────────────────

#[tokio::test]
async fn a_lone_massunban_asks_before_it_starts_and_a_second_confirms() {
    let fake = Fake::new();
    fake.login_as("unbanroom");
    run_in(&fake, "unbanroom", "/massunban").await;
    assert!(fake.calls("slash_mass_unban_start").is_empty());
    let o = run_in(&fake, "unbanroom", "/massunban").await;
    assert_eq!(fake.calls("slash_mass_unban_start"), vec![json!({ "broadcasterId": "42", "channel": "unbanroom" })]);
    assert_eq!(o.notices, vec!["Mass unban started, about three a second. /massunban stop ends it."]);
}

#[tokio::test]
async fn a_script_cannot_confirm_on_its_own() {
    let fake = Fake::new();
    fake.login_as("scriptroom");
    run_in(&fake, "scriptroom", "/chain /massunban | /massunban").await;
    assert!(fake.calls("slash_mass_unban_start").is_empty());
    assert!(fake.notices().iter().any(|(ch, n)| ch == "scriptroom" && n.contains("can't run from /chain, /foreach, /timer or /runin")));
    run_in(&fake, "scriptroom", "/foreach x,y /editor bob $").await;
    assert!(fake.calls("slash_editor").is_empty());
}

#[tokio::test]
async fn me_inside_a_chain_still_reaches_chat_paced() {
    let fake = Fake::new();
    run(&fake, "/chain /me waves | /me dances").await;
    assert_eq!(
        fake.calls("chat_send_paced"),
        vec![
            json!({ "channel": "chan", "channelId": "42", "text": "/me waves" }),
            json!({ "channel": "chan", "channelId": "42", "text": "/me dances" }),
        ]
    );
}

#[tokio::test]
async fn a_timer_sends_to_the_room_it_was_set_in() {
    let fake = Fake::new();
    let o = run_in(&fake, "timerroom", "/timer 1s /say hi").await;
    assert_eq!(o.notices, vec!["Runs in 1s: /say hi  (/timer stop cancels)"]);
    let o = run_in(&fake, "timerroom", "/timer 1s hello").await;
    assert!(o.send.is_none());
    tokio::time::sleep(std::time::Duration::from_millis(1600)).await;
    let mut sent: Vec<Value> = fake.calls("chat_send_paced");
    sent.sort_by_key(|v| v["text"].as_str().unwrap_or_default().to_string());
    assert_eq!(
        sent,
        vec![
            json!({ "channel": "timerroom", "channelId": "42", "text": "hello" }),
            json!({ "channel": "timerroom", "channelId": "42", "text": "hi" }),
        ]
    );
}

#[tokio::test]
async fn timer_stop_cancels_what_waits_in_the_room() {
    let fake = Fake::new();
    run_in(&fake, "stoproom", "/timer 1h hello").await;
    run_in(&fake, "stoproom", "/timer 2h again").await;
    let o = run_in(&fake, "stoproom", "/timer stop").await;
    assert_eq!(o.notices, vec!["Canceled 2 timers."]);
    let o = run_in(&fake, "stoproom", "/timer stop").await;
    assert_eq!(o.notices, vec!["No timers are waiting here."]);
    let o = run_in(&fake, "stoproom", "/timer 25h x").await;
    assert_eq!(toast(&o, ToastKind::Error), vec!["A timer can wait up to 24 hours"]);
}

#[tokio::test]
async fn a_refused_send_to_a_chat_that_is_not_open_shows_a_toast() {
    let fake = Fake::new();
    *fake.open.lock().unwrap() = false;
    fake.answer(|cmd, a| match cmd {
        "get_user_by_login" => Ok(json!({ "id": "9", "login": "other", "display_name": "Other" })),
        "chat_send_paced" => Err("Chat is in emote-only mode, so text messages would not be accepted".into()),
        _ => default_answer(cmd, a),
    });
    let o = run(&fake, "/runin other hello there").await;
    assert_eq!(fake.calls("chat_send_paced"), vec![json!({ "channel": "other", "channelId": "9", "text": "hello there" })]);
    assert_eq!(
        toast(&o, ToastKind::Error),
        vec!["Not sent to other: Chat is in emote-only mode, so text messages would not be accepted"]
    );
    assert!(fake.notices().is_empty());
}

#[tokio::test]
async fn reconnect_says_so_after_the_reload() {
    let fake = Fake::new();
    let o = run(&fake, "/reconnect").await;
    assert_eq!(toast(&o, ToastKind::Info), vec!["Reconnecting chat..."]);
    assert_eq!(o.ui, Some(Ui::ReloadChat));
    assert_eq!(o.notices, vec!["Chat reconnected."]);
}

#[tokio::test]
async fn spam_hands_the_raw_arguments_to_rust_and_prints_the_reply() {
    let fake = Fake::new();
    let o = run_in(&fake, "somechan", "/SPAM 20  two  spaces --delay 200").await;
    assert_eq!(
        fake.calls("run_chat_burst"),
        vec![json!({ "kind": "spam", "args": " 20  two  spaces --delay 200", "channelLogin": "somechan", "channelId": "42", "announceEnd": false })]
    );
    assert!(o.notices[0].starts_with("Spam started"));
    assert!(o.send.is_none());
    fake.answer(|_, _| Err("A spam is already running here (3/20). /spam stop ends it".into()));
    let phone = dispatch(ctx(&fake, "c", Platform::Ios), "/pyramid stop".to_string(), 0).await;
    assert_eq!(toast(&phone, ToastKind::Error), vec!["A spam is already running here (3/20). /spam stop ends it"]);
    assert_eq!(fake.calls("run_chat_burst")[1]["announceEnd"], json!(true));
}

// ───────────────────────────── arguments ─────────────────────────────

#[tokio::test]
async fn timeout_reads_a_length_with_a_unit() {
    let fake = Fake::new();
    run(&fake, "/timeout @bob 10m").await;
    run(&fake, "/timeout bob 1h30m too loud").await;
    run(&fake, "/timeout bob 90").await;
    run(&fake, "/timeout bob calm down").await;
    let got: Vec<(Value, Value)> = fake.calls("ban_user").iter().map(|c| (c["duration"].clone(), c["reason"].clone())).collect();
    assert_eq!(
        got,
        vec![
            (json!(600), Value::Null),
            (json!(5400), json!("too loud")),
            (json!(90), Value::Null),
            (json!(600), json!("calm down")),
        ]
    );
    assert_eq!(fake.calls("get_user_by_login")[0], json!({ "login": "bob" }));
}

#[test]
fn delays_take_weeks_days_hours_minutes_and_seconds() {
    assert_eq!(parse_delay("30s"), Some(30_000));
    assert_eq!(parse_delay("1h30m"), Some(5_400_000));
    assert_eq!(parse_delay("45"), Some(45_000));
    assert_eq!(parse_delay("2d"), Some(2 * 86_400_000));
    assert_eq!(parse_delay("1w1d"), Some(8 * 86_400_000));
    assert_eq!(parse_delay("10x"), None);
    assert_eq!(parse_delay("soon"), None);
    assert_eq!(parse_delay(""), None);
    assert_eq!(parse_delay("1m1h"), None);
}

#[test]
fn followers_lengths_are_minutes() {
    assert_eq!(followers_minutes("10m"), Some(10));
    assert_eq!(followers_minutes("2h"), Some(120));
    assert_eq!(followers_minutes("1d"), Some(1440));
    assert_eq!(followers_minutes("1w"), Some(10_080));
    assert_eq!(followers_minutes("1mo"), Some(43_200));
    assert_eq!(followers_minutes("15"), Some(15));
    assert_eq!(followers_minutes("soon"), Some(0));
    assert_eq!(followers_minutes("xm"), None);
}

#[tokio::test]
async fn purge_and_logs_strip_the_at() {
    let fake = Fake::new();
    run(&fake, "/purge @bob").await;
    run(&fake, "/logs @bob").await;
    assert_eq!(fake.calls("get_user_by_login"), vec![json!({ "login": "bob" })]);
    assert_eq!(fake.calls("slash_recent_messages"), vec![json!({ "channel": "chan", "login": "bob", "limit": 10 })]);
}

#[test]
fn setting_values_read_as_json_first() {
    assert_eq!(parse_setting_value("15"), json!(15));
    assert_eq!(parse_setting_value("true"), json!(true));
    assert_eq!(parse_setting_value("hello there"), json!("hello there"));
    assert_eq!(js_stringify(&json!({ "a": 15.0, "b": [1.5, 2.0] })), "{\"a\":15,\"b\":[1.5,2]}");
}

#[tokio::test]
async fn setting_get_set_and_reset() {
    let fake = Fake::new();
    *fake.settings.lock().unwrap() = json!({ "chat_design": { "font_size": 14.0, "other": true } });
    fake.answer(|cmd, a| {
        if cmd == "get_default_settings" {
            Ok(json!({ "chat_design": { "font_size": 13.0 } }))
        } else {
            default_answer(cmd, a)
        }
    });
    assert_eq!(run(&fake, "/setting get chat_design.font_size").await.notices, vec!["chat_design.font_size = 14"]);
    let o = run(&fake, "/setting set chat_design.font_size 15").await;
    assert_eq!(o.notices, vec!["chat_design.font_size = 15"]);
    assert_eq!(fake.calls("patch_settings")[0], json!({ "patch": { "chat_design": { "font_size": 15, "other": true } } }));
    let o = run(&fake, "/setting set chat_design.font_size big").await;
    assert_eq!(toast(&o, ToastKind::Error), vec!["chat_design.font_size takes a number, not a string"]);
    let o = run(&fake, "/setting reset chat_design.font_size").await;
    assert_eq!(o.notices, vec!["chat_design.font_size = 13"]);
    let o = run(&fake, "/setting get nope.nothing").await;
    assert_eq!(toast(&o, ToastKind::Error), vec!["There is no setting called nope.nothing"]);
}

// ───────────────────────────── custom commands ─────────────────────────────

fn with_commands(fake: &Arc<Fake>, commands: Value) {
    fake.settings.lock().unwrap()["chat_commands"]["user_commands"] = commands;
}

#[tokio::test]
async fn a_saved_custom_p_keeps_running_the_saved_text() {
    let fake = Fake::new();
    with_commands(&fake, json!([{ "trigger": "p", "expansion": "PogChamp {1}", "enabled": true, "require_slash": true }]));
    let o = run(&fake, "/p nice").await;
    assert!(fake.calls("ban_user").is_empty());
    assert_eq!(o.send.as_deref(), Some("PogChamp nice"));
}

#[tokio::test]
async fn a_saved_custom_lurk_wins_and_a_custom_ban_never_fires() {
    let fake = Fake::new();
    with_commands(
        &fake,
        json!([
            { "trigger": "lurk", "expansion": "brb, lurking", "enabled": true, "require_slash": true },
            { "trigger": "ban", "expansion": "banana", "enabled": true, "require_slash": true },
        ]),
    );
    assert_eq!(run(&fake, "/lurk").await.send.as_deref(), Some("brb, lurking"));
    let o = run(&fake, "/ban bob").await;
    assert!(o.send.is_none());
    assert_eq!(fake.calls("ban_user").len(), 1);
}

#[tokio::test]
async fn custom_commands_report_missing_and_empty_expansions() {
    let fake = Fake::new();
    with_commands(
        &fake,
        json!([
            { "trigger": "hug", "expansion": "hugs {1} and {2}", "enabled": true },
            { "trigger": "blank", "expansion": "{1+}", "enabled": true },
        ]),
    );
    let o = run(&fake, "/hug").await;
    assert_eq!(toast(&o, ToastKind::Error), vec!["/hug expects arguments {1}, {2}"]);
    let o = run(&fake, "/hug bob").await;
    assert_eq!(toast(&o, ToastKind::Error), vec!["/hug expects argument {2}"]);
    let o = run(&fake, "/blank").await;
    assert_eq!(toast(&o, ToastKind::Error), vec!["/blank expects argument {1}"]);
}

#[tokio::test]
async fn the_template_reads_the_room_it_was_typed_in() {
    let fake = Fake::new();
    with_commands(&fake, json!([{ "trigger": "where", "expansion": "{user} in {channel} ({channel.id}) playing {stream.game}", "enabled": true }]));
    fake.answer(|cmd, a| {
        if cmd == "get_stream" && a["login"] == "otherroom" {
            Ok(json!({ "user_id": "77", "user_login": "otherroom", "user_name": "OtherRoom", "game_name": "Chess", "title": "t", "started_at": "2001-09-09T01:00:00Z" }))
        } else {
            default_answer(cmd, a)
        }
    });
    let o = dispatch(
        Ctx { b: fake.clone(), channel: "otherroom".into(), broadcaster_id: "77".into(), platform: Platform::Desktop },
        "/where".into(),
        0,
    )
    .await;
    assert_eq!(o.send.as_deref(), Some("Me in OtherRoom (77) playing Chess"));
}

#[tokio::test]
async fn a_custom_command_that_names_a_command_runs_it() {
    let fake = Fake::new();
    with_commands(&fake, json!([{ "trigger": "tt", "expansion": "/timeout {1} 600 spam", "enabled": true, "require_slash": true }]));
    let o = run(&fake, "/tt bob").await;
    assert!(o.send.is_none(), "never sent to chat as text");
    assert_eq!(
        fake.calls("ban_user"),
        vec![json!({ "broadcasterId": "42", "targetUserId": "7", "duration": 600, "reason": "spam" })]
    );
}

#[tokio::test]
async fn a_custom_command_for_a_confirmed_command_confirms_when_typed_again() {
    let fake = Fake::new();
    // Its own room and chatter: confirmations are process-wide and the
    // /editor test above confirms "editroom|editor:bob" in parallel.
    fake.login_as("edalias");
    with_commands(&fake, json!([{ "trigger": "ed", "expansion": "/editor {1}", "enabled": true, "require_slash": true }]));
    let o = run_in(&fake, "edalias", "/ed carol").await;
    assert!(fake.calls("slash_editor").is_empty());
    assert_eq!(
        o.notices,
        vec!["Editors can change your stream title, category, ads and more. Make carol an editor? Run the same command again within 20 seconds to confirm."]
    );
    run_in(&fake, "edalias", "/ed carol").await;
    assert_eq!(fake.calls("slash_editor"), vec![json!({ "broadcasterId": "42", "target": "carol", "add": true })]);
}

#[tokio::test]
async fn a_custom_command_that_names_itself_expands_once() {
    let fake = Fake::new();
    with_commands(&fake, json!([{ "trigger": "loop", "expansion": "/loop", "enabled": true, "require_slash": true }]));
    let o = run(&fake, "/loop").await;
    assert_eq!(o.send.as_deref(), Some("/loop"));
}

#[tokio::test]
async fn plain_text_triggers_expand_and_may_start_a_command() {
    let fake = Fake::new();
    with_commands(
        &fake,
        json!([
            { "trigger": "!hi", "expansion": "hello {1}", "enabled": true, "require_slash": false },
            { "trigger": "!clear", "expansion": "/clear", "enabled": true, "require_slash": false },
            { "trigger": "gg", "expansion": "good game", "enabled": true, "require_slash": false, "also_match_suffix": true },
        ]),
    );
    assert_eq!(run(&fake, "!hi bob").await.send.as_deref(), Some("hello bob"));
    let o = run(&fake, "!hi").await;
    assert_eq!(toast(&o, ToastKind::Error), vec!["Trigger \"!hi\" expects argument {1}"]);
    run(&fake, "!clear").await;
    assert_eq!(fake.calls("clear_chat").len(), 1);
    assert_eq!(run(&fake, "that was gg").await.send.as_deref(), Some("good game"));
    assert!(!run(&fake, "just chatting").await.handled);
}

#[test]
fn templates_expand_in_one_pass() {
    let ctx = TemplateContext {
        user_name: "Me".into(),
        user_id: "1".into(),
        channel_name: "Chan".into(),
        channel_id: "42".into(),
        stream_title: "Title".into(),
        stream_game: "Game".into(),
        stream_uptime: "1h 2m".into(),
        args: vec!["a".into(), "b".into(), "c".into()],
    };
    let ex = |t: &str| expand_user_command(t, &ctx);
    assert_eq!(ex("{1} {2} {3}").text, "a b c");
    assert_eq!(ex("{2+}").text, "b c");
    assert_eq!(ex("{*}").text, "a b c");
    assert_eq!(ex("{{1}} and }}").text, "{1} and }");
    assert_eq!(ex("{user} {user.id} {channel} {Channel.Name} {channel.id}").text, "Me 1 Chan Chan 42");
    assert_eq!(ex("{stream.title}/{stream.game}/{stream.uptime}").text, "Title/Game/1h 2m");
    assert_eq!(ex("{unknown.thing} {1}").text, "{unknown.thing} a");
    // An inserted value is never read as a placeholder again.
    let tricky = TemplateContext { args: vec!["{2}".into(), "x".into()], ..ctx.clone() };
    assert_eq!(expand_user_command("{1}", &tricky).text, "{2}");
    let ex4 = expand_user_command("hi {4}  there {5+} {10}", &ctx);
    assert_eq!(ex4.text, "hi there");
    // Sorted the way the page sorted them: as text.
    assert_eq!(ex4.missing_args, vec![10, 4, 5]);
    assert_eq!(expand_user_command("  x \t\t y  ", &ctx).text, "x y");
}

#[test]
fn uptime_reads_hours_and_minutes() {
    let start = "2001-09-09T01:46:40Z"; // 1_000_000_000_000 ms
    assert_eq!(format_uptime(Some(start), 1_000_000_000_000 + 2 * 3_600_000 + 14 * 60_000), "2h 14m");
    assert_eq!(format_uptime(Some(start), 1_000_000_000_000 + 5 * 60_000), "5m");
    assert_eq!(format_uptime(Some(start), 1_000_000_000_000), "0m");
    assert_eq!(format_uptime(None, 0), "");
}

// ───────────────────────────── platforms ─────────────────────────────

#[tokio::test]
async fn phones_refuse_desktop_only_commands_and_the_builders() {
    let fake = Fake::new();
    for line in ["/proc notepad", "/popup", "/poll", "/prediction", "/screenshot"] {
        let o = dispatch(ctx(&fake, "chan", Platform::Android), line.to_string(), 0).await;
        let typed = line.split_whitespace().next().unwrap();
        assert_eq!(toast(&o, ToastKind::Info), vec![format!("{typed} works in the desktop app")], "{line}");
        assert!(o.ui.is_none());
    }
    let o = run(&fake, "/poll").await;
    assert_eq!(o.ui, Some(Ui::OpenPollComposer { channel: "chan".into(), composer: "poll".into() }));
}

#[tokio::test]
async fn phones_get_a_profile_sheet_and_a_local_clear() {
    let fake = Fake::new();
    let o = dispatch(ctx(&fake, "chan", Platform::Ios), "/usercard @Bob".into(), 0).await;
    assert_eq!(o.ui, Some(Ui::OpenUserCard { user_id: "7".into(), login: "bob".into(), display_name: "Bob".into() }));
    let o = dispatch(ctx(&fake, "chan", Platform::Ios), "/user".into(), 0).await;
    assert_eq!(toast(&o, ToastKind::Info), vec!["Usage: /user <user>"]);
    let o = dispatch(ctx(&fake, "chan", Platform::Ios), "/clearchat".into(), 0).await;
    assert_eq!(o.ui, Some(Ui::ClearLocalChat));
    assert_eq!(o.notices, vec!["Cleared messages in this chat (visual only)."]);
}

#[test]
fn help_on_a_phone_leaves_out_desktop_only_commands() {
    let phone = help_lines(None, true).join(" ");
    let desktop = help_lines(None, false).join(" ");
    assert!(desktop.contains("/proc"));
    assert!(!phone.contains("/proc"));
    assert!(!phone.contains("/screenshot"));
    assert!(phone.contains("/spam"));
    assert_eq!(help_lines(Some("proc"), true), vec!["/proc works in the desktop app."]);
    let ban = help_lines(Some("b"), false);
    assert_eq!(ban[0], "/ban <username> [reason]: Permanently ban a user");
    assert!(ban.iter().any(|l| l == "Also works as /b"));
    assert_eq!(help_lines(Some("nope"), false), vec!["There is no /nope command. /help lists them all."]);
    assert!(help_lines(None, false).len() > 2);
}

// ───────────────────────────── /r, /pin, /nuke, /remind ─────────────────────────────

#[tokio::test]
async fn r_replies_to_the_last_whisper() {
    let fake = Fake::new();
    let o = run(&fake, "/r hi").await;
    assert_eq!(toast(&o, ToastKind::Info), vec!["No whisper to reply to yet"]);
    *fake.whisper.lock().unwrap() = Some(("bob".into(), "Bob".into(), "7".into()));
    let o = run(&fake, "/r").await;
    assert_eq!(toast(&o, ToastKind::Info), vec!["Usage: /r <message>  (replies to Bob)"]);
    let o = run(&fake, "/r see you").await;
    assert_eq!(fake.calls("send_whisper"), vec![json!({ "toUserId": "7", "message": "see you" })]);
    assert_eq!(toast(&o, ToastKind::Success), vec!["Whisper sent to Bob"]);
}

const PIN_ID: &str = "0f8a2c4e-1b3d-4e5f-8a9b-0c1d2e3f4a5b";

#[test]
fn pin_arguments_resolve_to_a_message_or_a_send() {
    let latest = |login: &str| match login.to_lowercase().as_str() {
        "bob" => Some("bob-msg".to_string()),
        "alice" => Some("alice-msg".to_string()),
        _ => None,
    };
    let r = |text: &str| resolve_pin_args(&text.split(' ').collect::<Vec<_>>(), latest);
    assert_eq!(r(PIN_ID), Ok(PinTarget::Message { message_id: PIN_ID.into(), duration_seconds: None }));
    assert_eq!(r(&format!("{PIN_ID} 5m")), Ok(PinTarget::Message { message_id: PIN_ID.into(), duration_seconds: Some(300) }));
    assert_eq!(r("@bob"), Ok(PinTarget::Message { message_id: "bob-msg".into(), duration_seconds: None }));
    assert_eq!(r("Alice 90s"), Ok(PinTarget::Message { message_id: "alice-msg".into(), duration_seconds: Some(90) }));
    assert_eq!(r("@carol"), Err("No recent message from carol in this chat".to_string()));
    assert_eq!(r("welcome to the stream"), Ok(PinTarget::Send { text: "welcome to the stream".into() }));
    assert_eq!(r("hello"), Ok(PinTarget::Send { text: "hello".into() }));
    assert_eq!(r("back in 5m"), Ok(PinTarget::Send { text: "back in 5m".into() }));
    assert_eq!(r("bob is on fire"), Ok(PinTarget::Send { text: "bob is on fire".into() }));
    assert_eq!(r("@bob nice play"), Ok(PinTarget::Send { text: "@bob nice play".into() }));
    assert!(resolve_pin_args(&[], latest).is_err());
    assert!(resolve_pin_args(&[""], latest).is_err());
}

#[tokio::test]
async fn pin_pins_a_message_or_sends_one_to_pin() {
    let fake = Fake::new();
    fake.latest.lock().unwrap().insert("bob".into(), "bob-msg".into());
    let o = run(&fake, "/pin @bob 5m").await;
    assert_eq!(fake.calls("pin_chat_message"), vec![json!({ "broadcasterId": "42", "messageId": "bob-msg", "durationSeconds": 300 })]);
    assert_eq!(toast(&o, ToastKind::Success), vec!["Pinned for 300s"]);
    let o = run(&fake, "/pin giveaway at noon").await;
    assert_eq!(o.send.as_deref(), Some("giveaway at noon"));
    assert!(o.pin_after_send);
    *fake.role.lock().unwrap() = Role::Viewer;
    let o = run(&fake, "/pin hi").await;
    assert_eq!(toast(&o, ToastKind::Error), vec!["/pin is a moderator-only command"]);
}

fn sweep_entry(id: &str, user: &str, text: &str, ts: i64) -> SweepEntry {
    SweepEntry { id: id.into(), user_id: user.into(), login: user.into(), display: user.to_uppercase(), content: text.into(), ts_ms: ts }
}

#[tokio::test]
async fn a_nuke_with_failed_bans_says_so() {
    let fake = Fake::new();
    *fake.history.lock().unwrap() = vec![
        sweep_entry("1", "a", "buy followers", 999_990_000),
        sweep_entry("2", "b", "buy followers now", 999_995_000),
    ];
    fake.answer(|cmd, args| {
        if cmd == "ban_user" && args["targetUserId"] == "b" { Err("they are already banned".into()) } else { Ok(Value::Null) }
    });
    let o = run_in(&fake, "nukefail", "/nuke buy followers ban 1m").await;
    assert!(toast(&o, ToastKind::Success).is_empty(), "no clean-sweep toast");
    assert_eq!(toast(&o, ToastKind::Error), ["/nuke: 1 of 2 user(s) failed: they are already banned"]);
}

#[tokio::test]
async fn nuke_sweeps_the_window_and_undo_lifts_the_bans() {
    let fake = Fake::new();
    *fake.history.lock().unwrap() = vec![
        sweep_entry("1", "a", "buy followers", 999_990_000),
        sweep_entry("2", "b", "hello", 999_995_000),
        sweep_entry("3", "a", "BUY FOLLOWERS", 999_996_000),
        sweep_entry("4", "c", "old buy followers", 900_000_000),
    ];
    let o = run_in(&fake, "nukeroom", "/nuke buy followers ban 1m").await;
    assert_eq!(toast(&o, ToastKind::Success), vec!["/nuke: 2 message(s), 1 user(s)"]);
    assert_eq!(o.notices, vec!["/nuke pattern \"buy followers\" → ban on 2 message(s) from 1 user(s)."]);
    assert_eq!(fake.calls("ban_user"), vec![json!({ "broadcasterId": "42", "targetUserId": "a", "duration": null, "reason": "/nuke" })]);
    let o = run_in(&fake, "nukeroom", "/undo").await;
    assert_eq!(fake.calls("unban_user"), vec![json!({ "broadcasterId": "42", "targetUserId": "a" })]);
    assert_eq!(o.notices, vec!["/undo reversed nuke on \"buy followers\": unbanned 1/1 user(s)."]);
    let o = run_in(&fake, "nukeroom", "/undo").await;
    assert_eq!(o.notices, vec!["/undo: no nuke to undo on this channel."]);
    let o = run_in(&fake, "nukeroom", "/nuke spam").await;
    assert_eq!(toast(&o, ToastKind::Error), vec!["usage: /nuke <pattern> <action> <past[:future]>  e.g. /nuke spam ban 5m:1m"]);
    let o = run_in(&fake, "nukeroom", "/nuke stop").await;
    assert_eq!(toast(&o, ToastKind::Info), vec!["/nuke: nothing running on this channel"]);
    *fake.role.lock().unwrap() = Role::Viewer;
    let o = run_in(&fake, "nukeroom", "/nuke x ban 1m").await;
    assert_eq!(toast(&o, ToastKind::Error), vec!["/nuke is a moderator-only command"]);
}

#[tokio::test]
async fn remind_creates_lists_and_manages_reminders() {
    let fake = Fake::new();
    let o = run(&fake, "/remind every 20m Drink water x3").await;
    let calls = fake.calls("patch_settings");
    let patch = &calls[0]["patch"]["reminders"]["reminders"];
    assert_eq!(patch[0]["trigger"], json!("interval"));
    assert_eq!(patch[0]["interval_seconds"], json!(1200));
    assert_eq!(patch[0]["message"], json!("Drink water"));
    assert_eq!(patch[0]["repeat_count"], json!(3));
    assert_eq!(patch[0]["channel_login"], json!("chan"));
    assert_eq!(patch[0]["id"], json!("new-id"));
    assert_eq!(o.notices, vec!["Reminder set: every 20m ×3 in #chan (now #1). /remind list to manage."]);

    *fake.settings.lock().unwrap() = json!({ "reminders": { "reminders": [patch[0].clone()] } });
    let o = run(&fake, "/remind list").await;
    assert_eq!(o.notices[0], "Your reminders (1):");
    assert_eq!(o.notices[1], "#1 [on] every 20m ×3 · #chan · \"Drink water\"");
    let o = run(&fake, "/remind disable 1").await;
    assert_eq!(o.notices, vec!["Reminder #1 disabled."]);
    let o = run(&fake, "/remind remove 4").await;
    assert_eq!(o.notices, vec!["There's no reminder #4. Run /remind list to see them."]);
    let o = run(&fake, "/remind clear").await;
    assert_eq!(o.notices, vec!["Cleared 1 reminder."]);
    let o = run(&fake, "/remind every soon hi").await;
    assert_eq!(toast(&o, ToastKind::Info), vec!["How often? e.g. /remind every 20m or every 30s <message>"]);
    run(&fake, "/remind now hello x2").await;
    assert_eq!(fake.calls("post_reminder"), vec![json!({ "channel": "chan", "channelId": "42", "message": "hello", "repeat": 2 })]);
}

#[tokio::test]
async fn favorite_adds_once_and_says_when_nothing_changes() {
    let fake = Fake::new();
    *fake.settings.lock().unwrap() = json!({ "favorite_streamers": [], "favorite_channels": [] });
    let o = run(&fake, "/favorite bob").await;
    assert_eq!(toast(&o, ToastKind::Success), vec!["Bob added to favorites"]);
    let calls = fake.calls("patch_settings");
    let patch = &calls[0]["patch"];
    assert_eq!(patch["favorite_streamers"], json!(["7"]));
    assert_eq!(patch["favorite_channels"][0]["channel"], json!("bob"));
    assert_eq!(fake.calls("refresh_favorites"), vec![json!({ "key": "7" })]);
    *fake.settings.lock().unwrap() = json!({ "favorite_streamers": ["7"] });
    let o = run(&fake, "/favorite bob").await;
    assert_eq!(toast(&o, ToastKind::Info), vec!["Bob is already a favorite"]);
}

#[tokio::test]
async fn emoteinfo_counts_per_provider() {
    let fake = Fake::new();
    fake.answer(|cmd, a| {
        if cmd == "emote_counts" {
            Ok(json!({ "twitch": 1200, "bttv": 3, "7tv": 40, "ffz": 0 }))
        } else {
            default_answer(cmd, a)
        }
    });
    let o = run(&fake, "/emoteinfo").await;
    assert_eq!(o.notices, vec!["Emotes loaded for chan: Twitch 1,200, BTTV 3, 7TV 40, FFZ 0. /refresh fetches them again."]);
    let o = run(&fake, "/seventv").await;
    assert_eq!(o.notices, vec!["Emotes loaded for chan: 7TV 40. /refresh fetches them again."]);
}

#[test]
fn seventv_answers_are_read() {
    assert_eq!(seventv_emote_id("https://7tv.app/emotes/01F6MQ33FG000FFJ97ZB8MWV52"), Some("01F6MQ33FG000FFJ97ZB8MWV52".into()));
    assert_eq!(seventv_emote_id("01F6MQ33FG000FFJ97ZB8MWV52"), Some("01F6MQ33FG000FFJ97ZB8MWV52".into()));
    assert_eq!(seventv_emote_id("peepo"), None);
    let data = json!({ "users": { "me": {
        "id": "s1",
        "mainConnection": { "platform": "TWITCH", "platformId": "1", "platformUsername": "me", "platformDisplayName": "Me" },
        "style": { "activeEmoteSetId": "set-me" },
        "editorFor": [
            { "state": "ACCEPTED", "permissions": { "emoteSet": { "manage": true } },
              "user": { "id": "s2", "mainConnection": { "platform": "TWITCH", "platformId": "42", "platformUsername": "chan", "platformDisplayName": "Chan" }, "style": { "activeEmoteSetId": "set-chan" } } },
            { "state": "ACCEPTED", "permissions": { "emoteSet": { "manage": false } },
              "user": { "id": "s3", "mainConnection": { "platform": "TWITCH", "platformId": "43", "platformUsername": "ro", "platformDisplayName": "Ro" }, "style": {} } }
        ]
    } } });
    let channels = seventv_editable(&data);
    assert_eq!(channels.len(), 3);
    assert!(channels[0].manage_emotes);
    assert_eq!(channels[1].twitch_id.as_deref(), Some("42"));
    assert_eq!(channels[1].active_set.as_deref(), Some("set-chan"));
    assert!(!channels[2].manage_emotes);
    assert_eq!(channels[2].active_set, None);
}

#[tokio::test]
async fn seventv_add_finds_the_emote_and_adds_it_to_the_room_set() {
    let fake = Fake::new();
    fake.answer(|cmd, a| {
        if cmd != "seventv_graphql_authed" {
            return default_answer(cmd, a);
        }
        Ok(match a["operationName"].as_str().unwrap_or("") {
            "EditableChannels" => json!({ "data": { "users": { "me": {
                "id": "s1", "mainConnection": { "platform": "TWITCH", "platformId": "1", "platformUsername": "me" }, "style": {},
                "editorFor": [{ "state": "ACCEPTED", "permissions": { "emoteSet": { "manage": true } },
                    "user": { "id": "s2", "mainConnection": { "platform": "TWITCH", "platformId": "42", "platformUsername": "chan", "platformDisplayName": "Chan" }, "style": { "activeEmoteSetId": "set-chan" } } }]
            } } } }),
            "SearchEmotes" => json!({ "data": { "emotes": { "search": { "items": [{ "id": "e1", "defaultName": "peepoHey" }] } } } }),
            "AddEmote" => json!({ "data": {} }),
            _ => json!({ "errors": [{ "message": "nope" }] }),
        })
    });
    let o = run(&fake, "/7tv add peepohey").await;
    assert_eq!(o.notices, vec!["Added peepoHey to Chan's 7TV emotes."]);
    let add = fake.calls("seventv_graphql_authed").into_iter().find(|c| c["operationName"] == "AddEmote").unwrap();
    assert_eq!(add["variables"], json!({ "setId": "set-chan", "emote": { "emoteId": "e1" }, "zeroWidth": null, "override": null }));
    let o = run(&fake, "/7tv").await;
    assert_eq!(toast(&o, ToastKind::Info), vec!["Usage: /7tv <search|add|remove|rename> ..."]);
}

#[test]
fn chatter_counts_read_either_answer() {
    assert_eq!(count_chatters(&json!({ "total": 12 })), Some(12));
    assert_eq!(count_chatters(&json!({ "data": [1, 2, 3] })), Some(3));
    assert_eq!(count_chatters(&json!([{ "moderators": [1], "viewers": [1, 2] }])), Some(3));
    assert_eq!(count_chatters(&json!("x")), None);
}

#[test]
fn colors_are_names_or_hex() {
    assert!(is_color("orange"));
    assert!(is_color("#ff8800"));
    assert!(is_color("ff8800"));
    assert!(is_color("rgb(1, 2, 3)"));
    assert!(!is_color("notacolor"));
}

#[test]
fn notices_are_local_notice_frames() {
    let v: Value = serde_json::from_str(&notice_frame("chan", "hello")).unwrap();
    assert_eq!(v, json!({ "type": "NOTICE", "channel": "chan", "msg_id": "streamnook_notice", "message": "hello" }));
}

#[test]
fn the_outcome_serializes_in_the_documented_shape() {
    let mut o = Outcome::handled();
    o.ok("Banned bob");
    o.ui = Some(Ui::OpenUserCard { user_id: "7".into(), login: "bob".into(), display_name: "Bob".into() });
    assert_eq!(
        serde_json::to_value(&o).unwrap(),
        json!({
            "handled": true, "send": null, "clear_draft": true,
            "toasts": [{ "text": "Banned bob", "kind": "success" }],
            "notices": [],
            "ui": { "kind": "open_user_card", "user_id": "7", "login": "bob", "display_name": "Bob" }
        })
    );
    let mut pin = Outcome::handled();
    pin.send = Some("x".into());
    pin.pin_after_send = true;
    pin.reply_to = Some("m".into());
    let v = serde_json::to_value(&pin).unwrap();
    assert_eq!(v["pin_after_send"], json!(true));
    assert_eq!(v["reply_to"], json!("m"));
}

#[tokio::test]
async fn moderation_toasts_use_the_shared_words() {
    let fake = Fake::new();
    let o = run(&fake, "/timeout bob 10m").await;
    assert_eq!(toast(&o, ToastKind::Success), vec!["Timed out bob for 10m"]);
    let o = run(&fake, "/untimeout bob").await;
    assert_eq!(toast(&o, ToastKind::Success), vec!["Removed the timeout on bob"]);
    let o = run(&fake, "/unban bob").await;
    assert_eq!(toast(&o, ToastKind::Success), vec!["Unbanned bob"]);
    // Past Twitch's 14 days the timeout is what Twitch will really apply.
    run(&fake, "/timeout bob 30d").await;
    assert_eq!(fake.calls("ban_user").last().unwrap()["duration"], json!(1_209_600));
}

#[tokio::test]
async fn mods_and_vips_read_the_signed_in_role_list() {
    let fake = Fake::new();
    fake.answer(|cmd, a| match cmd {
        "slash_role_list" => Ok(json!([format!("role {}", a["role"].as_str().unwrap_or_default())])),
        _ => default_answer(cmd, a),
    });
    let o = run(&fake, "/mods").await;
    assert_eq!(fake.calls("slash_role_list"), vec![json!({ "channel": "chan", "role": "mods" })]);
    assert!(serde_json::to_string(&o).unwrap().contains("role mods"));
    run(&fake, "/vips").await;
    assert_eq!(fake.calls("slash_role_list")[1]["role"], json!("vips"));
}

//! Tauri boundary for chat slash commands. Each command validates what was
//! typed, does the work in `services::slash_commands` (or an existing
//! service), and returns the line(s) the chat shows. The page dispatches only.

use crate::models::settings::AppState;
use crate::rt::AppHandle;
use crate::services::slash_commands as sc;
use crate::services::twitch_service::{ChannelRole, TwitchService};
use tauri::{Emitter, State};

fn login_arg(raw: &str, what: &str) -> Result<String, String> {
    sc::clean_login(raw).ok_or_else(|| format!("\"{}\" is not a Twitch {what} name", raw.trim()))
}

/// Run a line typed in a room's composer: a slash command (or a plain-text
/// trigger saved in Settings). `channel` is the room's bare lowercase Twitch
/// login (or a provider room key such as `kick:slug`, where only commands that
/// need no Twitch API run), `broadcaster_id` its Twitch user id, `platform`
/// "desktop", "android" or "ios". The answer is described in
/// `services::slash_dispatch`.
#[tauri::command]
pub async fn run_slash_command(
    app: AppHandle,
    window: crate::rt::Window,
    channel: String,
    broadcaster_id: String,
    text: String,
    depth: Option<u32>,
    platform: Option<String>,
) -> Result<crate::services::slash_dispatch::Outcome, String> {
    use crate::services::slash_dispatch::{dispatch, Ctx, Platform};
    let room = channel.trim().trim_start_matches('#');
    let room = if room.contains(':') { room.to_string() } else { room.to_lowercase() };
    let ctx = Ctx {
        b: std::sync::Arc::new(crate::commands::slash_backend::TauriBackend { app, window: window.label().to_string() }),
        channel: room,
        broadcaster_id: broadcaster_id.trim().to_string(),
        platform: Platform::parse(platform.as_deref().unwrap_or("desktop")),
    };
    Ok(dispatch(ctx, text, depth.unwrap_or(0)).await)
}

/// What `/nuke <args>` would hit in `channel` right now, for the count shown
/// while it is typed. None while the line does not parse yet.
#[derive(serde::Serialize)]
pub struct NukePreview {
    pub messages: usize,
    pub users: usize,
    pub action: String,
}

#[tauri::command]
pub fn nuke_preview(channel: String, args: String) -> Option<NukePreview> {
    let parsed = crate::services::nuke::parse_nuke_args(&args).ok()?;
    let cutoff = chrono::Utc::now().timestamp_millis() - parsed.past_seconds as i64 * 1000;
    let entries = crate::services::chat_history::ChatHistory::since(&channel, cutoff);
    let (messages, affected) = crate::services::nuke::sweep(&entries, &parsed, cutoff);
    Some(NukePreview { messages, users: affected.len(), action: parsed.action.describe() })
}

/// How many `/nuke` windows are still catching new messages in `channel`.
#[tauri::command]
pub fn nuke_active(channel: String) -> usize {
    crate::services::nuke::active_count(&channel)
}

/// End every running `/nuke` window in `channel`. Returns how many.
#[tauri::command]
pub fn nuke_stop(channel: String) -> usize {
    crate::services::nuke::stop(&channel)
}

/// `/followage [user] [channel]`.
#[tauri::command]
pub async fn slash_followage(channel: String, user: Option<String>) -> Result<String, String> {
    let channel = login_arg(&channel, "channel")?;
    let user = match user.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => Some(login_arg(u, "user")?),
        None => None,
    };
    sc::followage(&channel, user.as_deref()).await
}

/// `/follows [channel]`.
#[tauri::command]
pub async fn slash_follower_count(channel: String) -> Result<String, String> {
    sc::follower_count(&login_arg(&channel, "channel")?).await
}

/// `/accountage [user]`.
#[tauri::command]
pub async fn slash_account_age(user: String) -> Result<String, String> {
    sc::account_age(&login_arg(&user, "user")?).await
}

/// `/viewers [channel]`.
#[tauri::command]
pub async fn slash_viewer_count(channel: String) -> Result<String, String> {
    sc::viewer_count(&login_arg(&channel, "channel")?).await
}

/// `/founders [channel]`.
#[tauri::command]
pub async fn slash_founders(channel: String) -> Result<Vec<String>, String> {
    sc::founders(&login_arg(&channel, "channel")?).await
}

/// `/namehistory <user | user id>`.
#[tauri::command]
pub async fn slash_name_history(user: String) -> Result<Vec<String>, String> {
    let user = user.trim().trim_start_matches('@').to_string();
    if user.chars().all(|c| c.is_ascii_digit()) && !user.is_empty() {
        return sc::name_history(&user).await;
    }
    sc::name_history(&login_arg(&user, "user")?).await
}

/// `/rules [channel]`.
#[tauri::command]
pub async fn slash_chat_rules(channel: String) -> Result<Vec<String>, String> {
    sc::chat_rules(&login_arg(&channel, "channel")?).await
}

/// `/mods` and `/vips`: the channel's moderators or VIPs.
#[tauri::command]
pub async fn slash_role_list(state: State<'_, AppState>, channel: String, role: String) -> Result<Vec<String>, String> {
    let role = match role.as_str() {
        "mods" => ChannelRole::Moderators,
        "vips" => ChannelRole::Vips,
        other => return Err(format!("\"{other}\" is not a role list")),
    };
    sc::role_list(&state.twitch_auth, &login_arg(&channel, "channel")?, role).await
}

/// `/sharedchat`: the channel's Shared Chat session, if any.
#[tauri::command]
pub async fn slash_shared_chat(broadcaster_id: String, channel: String) -> Result<Vec<String>, String> {
    if broadcaster_id.is_empty() {
        return Err("No channel open".into());
    }
    sc::shared_chat(&broadcaster_id, &channel).await
}

/// `/raidbrowser`: followed channels that are live now.
#[tauri::command]
pub async fn slash_raid_candidates(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let (_, own) = sc::me().await?;
    let streams = TwitchService::get_followed_streams(&state)
        .await
        .map_err(|e| format!("Could not load the channels you follow: {e}"))?;
    Ok(sc::raid_candidate_lines(&streams, &own, 10))
}

/// `/raidsend`: send the pending raid now instead of waiting out the countdown.
#[tauri::command]
pub async fn slash_raid_now(broadcaster_id: String) -> Result<(), String> {
    sc::raid_now(&broadcaster_id).await
}

/// `/leadmod` and `/unleadmod`.
#[tauri::command]
pub async fn slash_lead_mod(broadcaster_id: String, target: String, assign: bool) -> Result<String, String> {
    let login = login_arg(&target, "user")?;
    let user = sc::user_by_login(&login)
        .await?
        .ok_or_else(|| format!("There is no Twitch user named {login}"))?;
    let id = user["id"].as_str().unwrap_or_default();
    sc::lead_mod(&broadcaster_id, id, assign).await?;
    let name = user["display_name"].as_str().unwrap_or(&login);
    Ok(if assign {
        format!("{name} is now a lead moderator.")
    } else {
        format!("{name} is no longer a lead moderator.")
    })
}

/// `/editor` and `/uneditor`.
#[tauri::command]
pub async fn slash_editor(broadcaster_id: String, target: String, add: bool) -> Result<String, String> {
    let login = login_arg(&target, "user")?;
    sc::editor(&broadcaster_id, &login, add).await?;
    Ok(if add {
        format!("{login} is now an editor of your channel.")
    } else {
        format!("{login} is no longer an editor of your channel.")
    })
}

/// The poll `channel_id` shows right now (the poll banner's first read when a
/// chat opens), followed until it ends so the banner and `/vote` work in any
/// open chat, with or without PubSub for that channel. None when no poll.
#[tauri::command]
pub async fn watch_channel_poll(channel_id: String) -> Result<Option<serde_json::Value>, String> {
    sc::watch_poll_now(channel_id.trim(), None).await
}

/// `/vote <number | option>` in the poll running in `channel_id`.
#[tauri::command]
pub async fn slash_vote(channel_id: String, choice: String) -> Result<String, String> {
    // PubSub fills the cache only for the watched stream's channel; any other
    // chat asks Twitch.
    let poll = match sc::active_poll(&channel_id) {
        Some(p) => p,
        None => {
            sc::watch_poll_now(&channel_id, Some("poll-updated")).await?;
            sc::active_poll(&channel_id).ok_or("No poll is running in this channel")?
        }
    };
    // One free vote per poll: say which one already counts.
    if let Some(voted) = sc::my_poll_vote(&poll.poll_id) {
        let title = poll.choices.iter().find(|(id, _)| *id == voted).map(|(_, t)| t.as_str()).unwrap_or("an option");
        return Err(format!("You already voted for \"{title}\" in this poll."));
    }
    let (choice_id, title) = sc::pick_choice(&poll, &choice)?;
    crate::commands::drops::vote_on_poll(poll.poll_id.clone(), choice_id, channel_id).await?;
    Ok(format!("You voted for \"{title}\"."))
}

/// The newest message `login` sent in `channel` that the app still holds.
#[tauri::command]
pub async fn slash_last_message(
    channel: String,
    login: String,
) -> Result<Option<crate::services::chat_history::SearchHit>, String> {
    let login = login_arg(&login, "user")?;
    tokio::task::spawn_blocking(move || sc::last_message_of(&channel, &login))
        .await
        .map_err(|e| e.to_string())
}

/// `/lurk [note]`: a random lurk line, as a `/me` action when `action` is set
/// (Twitch) and plain words otherwise.
#[tauri::command]
pub fn slash_lurk_line(note: Option<String>, action: bool) -> String {
    sc::lurk(note.as_deref(), action)
}

/// `/delete <message id | @user>`.
#[tauri::command]
pub async fn slash_delete_message(broadcaster_id: String, channel: String, target: String) -> Result<String, String> {
    let target = target.trim().to_string();
    if target.is_empty() {
        return Err("Usage: /delete <message id or @user>".into());
    }
    let looks_like_id = target.len() >= 30 && target.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    let (id, whose) = if looks_like_id {
        (target.clone(), None)
    } else {
        let login = login_arg(&target, "user")?;
        let ch = channel.clone();
        let l = login.clone();
        let hit = tokio::task::spawn_blocking(move || sc::last_message_of(&ch, &l))
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("No recent message from {login} in this chat"))?;
        (hit.id, Some(hit.display_name))
    };
    TwitchService::delete_chat_message(&broadcaster_id, &id)
        .await
        .map_err(|e| sc::delete_refusal_text(&e.to_string()))?;
    Ok(match whose {
        Some(name) => format!("Deleted {name}'s last message."),
        None => "Message deleted.".into(),
    })
}

/// `/logs [user]`: their newest messages in this chat, oldest of them first.
#[tauri::command]
pub async fn slash_recent_messages(channel: String, login: String, limit: Option<usize>) -> Result<Vec<String>, String> {
    let login = login_arg(&login, "user")?;
    let limit = limit.unwrap_or(10).clamp(1, 50);
    let l = login.clone();
    let hits = tokio::task::spawn_blocking(move || sc::recent_messages_of(&channel, &l, limit))
        .await
        .map_err(|e| e.to_string())?;
    if hits.is_empty() {
        return Ok(vec![format!("No messages from {login} in this chat yet.")]);
    }
    let mut out = vec![format!("Last {} messages from {} here:", hits.len(), hits[0].display_name)];
    for h in hits.iter().rev() {
        let at = chrono::DateTime::from_timestamp_millis(h.ts_ms)
            .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
            .unwrap_or_default();
        out.push(format!("{at} {}", h.content));
    }
    Ok(out)
}

/// `/massunban`: unban everyone in your own channel. Progress arrives as
/// `mass-unban-progress` events.
#[tauri::command]
pub async fn slash_mass_unban_start(app: AppHandle, broadcaster_id: String) -> Result<(), String> {
    sc::start_mass_unban(broadcaster_id, move |p| {
        let _ = app.emit(sc::MASS_UNBAN_EVENT, p);
    })
    .await
}

/// `/massunban stop`. False when nothing was running.
#[tauri::command]
pub fn slash_mass_unban_stop(broadcaster_id: String) -> bool {
    sc::stop_mass_unban(&broadcaster_id)
}

/// `/lowtrust`: monitored and restricted users seen here, with their messages.
#[tauri::command]
pub async fn slash_low_trust(channel: String) -> Result<Vec<String>, String> {
    sc::low_trust_lines(&channel.to_ascii_lowercase()).await
}

/// `/automod allow|deny [message id]`: the newest held message when no id.
#[tauri::command]
pub async fn slash_automod(channel: String, message_id: Option<String>, allow: bool) -> Result<String, String> {
    let held = crate::services::automod_queue::AutomodQueue::list(&channel.to_ascii_lowercase());
    let pick = match message_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => held
            .iter()
            .find(|m| m.message_id == id)
            .cloned()
            .ok_or("That message is not waiting for review here")?,
        None => held
            .iter()
            .filter(|m| m.status == "held")
            .max_by_key(|m| m.held_at_ms)
            .cloned()
            .ok_or("No messages are waiting for review here")?,
    };
    TwitchService::resolve_automod_message(&pick.message_id, allow)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "{} {}'s message: \"{}\"",
        if allow { "Allowed" } else { "Denied" },
        pick.user_name,
        pick.text
    ))
}

/// `/addstreamhighlight [comment]`.
#[tauri::command]
pub async fn slash_add_stream_highlight(channel: String, comment: Option<String>) -> Result<String, String> {
    let channel = login_arg(&channel, "channel")?;
    sc::add_stream_highlight(&channel, comment.as_deref().unwrap_or("")).await
}

fn open_path(path: &std::path::Path) -> Result<(), String> {
    if cfg!(target_os = "ios") {
        return Err("Opening files is not available on iPhone".into());
    }
    let program = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(path.as_os_str())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open {}: {e}", path.display()))
}

/// `/openstreamhighlights`: open the highlights file.
#[tauri::command]
pub async fn slash_open_stream_highlights() -> Result<String, String> {
    let path = sc::highlights_file()?;
    if !path.exists() {
        return Err("No highlights yet. Add one with /addstreamhighlight".into());
    }
    open_path(&path)?;
    Ok(path.display().to_string())
}

/// `/exporttext <file> <text>`: append the text to a file in StreamNook's
/// exports folder. Returns the file's path.
#[tauri::command]
pub async fn slash_export_text(file: String, text: String) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("Usage: /exporttext <file> <text>".into());
    }
    sc::export_text(&file, &text).map(|p| p.display().to_string())
}

/// `/openfile [file]`: show an exported file selected in its folder, or open
/// the exports folder. A file is only ever revealed, never opened: opening
/// hands it to whatever program the system runs for its ending.
#[cfg(not(target_os = "ios"))]
#[tauri::command]
pub async fn slash_open_export(file: Option<String>) -> Result<String, String> {
    let dir = sc::exports_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    match file.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) => {
            let name = sc::safe_export_name(f).ok_or(sc::EXPORT_NAME_RULE)?;
            let p = dir.join(name);
            if !p.is_file() {
                return Err(format!("There is no {} in the exports folder", p.file_name().unwrap_or_default().to_string_lossy()));
            }
            tauri_plugin_opener::reveal_item_in_dir(&p).map_err(|e| format!("Could not show {}: {e}", p.display()))?;
            Ok(p.display().to_string())
        }
        None => {
            open_path(&dir)?;
            Ok(dir.display().to_string())
        }
    }
}

#[cfg(target_os = "ios")]
#[tauri::command]
pub async fn slash_open_export(_file: Option<String>) -> Result<String, String> {
    Err(crate::ios_ffi::not_wired("/openfile"))
}

/// `/streamlink [quality]`: open the channel in Streamlink, when installed.
#[tauri::command]
pub async fn slash_open_streamlink(channel: String, quality: Option<String>) -> Result<String, String> {
    if !cfg!(desktop) {
        return Err("Streamlink is a desktop program".into());
    }
    let channel = login_arg(&channel, "channel")?;
    let quality = quality
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .unwrap_or("best")
        .to_string();
    if !quality.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ',' | '_' | '-')) {
        return Err("A quality looks like best, 720p60 or audio_only".into());
    }
    let program = sc::find_on_path("streamlink")
        .ok_or("Streamlink is not installed (or not on PATH). Get it from streamlink.github.io")?;
    std::process::Command::new(program)
        .arg(format!("https://www.twitch.tv/{channel}"))
        .arg(&quality)
        .spawn()
        .map_err(|e| format!("Could not start Streamlink: {e}"))?;
    Ok(format!("Opening {channel} ({quality}) in Streamlink."))
}

/// `/raw <line>`: write one IRC line to the chat connection.
#[tauri::command]
pub async fn slash_send_raw_irc(line: String) -> Result<(), String> {
    let line = sc::check_raw_line(&line)?;
    crate::services::irc_service::IrcService::send_raw_line(&line)
        .await
        .map_err(|e| e.to_string())
}

/// `/fakemsg <IRC line>`: show a made-up message in your own chat only.
/// Development builds only, so nobody can stage a convincing fake message in
/// the released app.
#[tauri::command]
pub async fn slash_inject_fake_message(line: String) -> Result<(), String> {
    if !cfg!(debug_assertions) {
        return Err("/fakemsg is only in development builds".into());
    }
    let line = line.trim().to_string();
    if line.is_empty() || line.contains(['\r', '\n']) {
        return Err("Usage: /fakemsg <one IRC PRIVMSG line>".into());
    }
    let parsed = crate::services::irc_service::IrcService::parse_historical_messages(vec![line]).await;
    let msg = parsed
        .into_iter()
        .next()
        .ok_or("That line is not a chat message Twitch would send (a PRIVMSG with tags)")?;
    crate::services::providers::publish_chat_message(&msg).await;
    Ok(())
}

/// `/proc <program> [args]`: run a program on this computer after you confirm
/// it in a system dialog. No shell is involved.
#[cfg(not(target_os = "ios"))]
#[tauri::command]
pub async fn slash_run_program(app: AppHandle, command_line: String) -> Result<String, String> {
    if !cfg!(desktop) {
        return Err("/proc only runs on the desktop app".into());
    }
    let words = sc::split_command_line(&command_line);
    let Some((program, args)) = words.split_first() else {
        return Err("Usage: /proc <program> [arguments]".into());
    };
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .message(format!(
            "A chat command wants to run this program on your computer:\n\n{}\n\nOnly run it if you typed it yourself and trust it.",
            command_line.trim()
        ))
        .title("Run a program?")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom("Run".into(), "Cancel".into()))
        .show(move |ok| {
            let _ = tx.send(ok);
        });
    if !rx.await.unwrap_or(false) {
        return Err("Canceled".into());
    }
    let child = std::process::Command::new(program)
        .args(args)
        .spawn()
        .map_err(|e| format!("Could not start {program}: {e}"))?;
    Ok(format!("Started {program} (process {}).", child.id()))
}

#[cfg(target_os = "ios")]
#[tauri::command]
pub async fn slash_run_program(_app: AppHandle, _command_line: String) -> Result<String, String> {
    Err("/proc only runs on the desktop app".into())
}

/// `/copy <text>`: put text on the clipboard.
#[cfg(not(target_os = "ios"))]
#[tauri::command]
pub fn slash_copy_text(app: AppHandle, text: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

#[cfg(target_os = "ios")]
#[tauri::command]
pub fn slash_copy_text(_app: AppHandle, text: String) -> Result<(), String> {
    crate::ios_ffi::copy_text(&text);
    Ok(())
}

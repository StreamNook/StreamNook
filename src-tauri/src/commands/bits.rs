//! Bits in the chat box: the Bits section of the insert menu, sending a cheer
//! the viewer confirmed, and Twitch's Get Bits page. The rules and the Twitch
//! calls live in `services::bits`; these only shape what the page sees.

use serde::Serialize;
use tauri::{Emitter, State};

use crate::models::settings::AppState;
use crate::rt::AppHandle;
use crate::services::bits::{self, CheerOutcome};
use crate::services::irc_service::{channel_cheermote_set, IrcService};
use crate::services::twitch_web_gql::web_session_token;

/// One cheermote the viewer can cheer with, and the amounts to offer.
#[derive(Serialize)]
pub struct CheermoteChoice {
    /// As Twitch writes it ("Cheer"); the inserted word is prefix + amount.
    pub prefix: String,
    pub custom: bool,
    pub tiers: Vec<CheermoteAmount>,
}

#[derive(Serialize)]
pub struct CheermoteAmount {
    pub bits: u32,
    pub url: String,
    pub color: String,
}

/// What the Bits section shows for a channel.
#[derive(Serialize)]
pub struct BitsState {
    pub enabled: bool,
    pub cheer_min: u32,
    pub emote_min: u32,
    /// The range the picker offers for one cheermote: tiers below
    /// `pick_min` cannot be cheered alone here, and a typed amount stays
    /// between the two.
    pub pick_min: u32,
    pub pick_max: u32,
    /// Ready-made amounts, ascending, starting at `pick_min`.
    pub quick_amounts: Vec<u32>,
    /// None when the twitch.tv session could not be read.
    pub balance: Option<i64>,
    /// Channel cheermotes first, then Twitch's own, in Twitch's order.
    pub cheermotes: Vec<CheermoteChoice>,
    /// Twitch's animated Cheer gems by amount, ascending, for drawing any
    /// amount of bits (the balance, the Bits tab) the way Twitch does.
    pub icons: Vec<CheermoteAmount>,
}

/// The wallet's Bits tab for `channel_login` (bare login) / `channel_id`. One
/// public settings lookup (cached) and the cached balance; the channel's
/// cheermotes come from the set chat already holds, fetched here only if the
/// channel was never joined.
#[tauri::command]
pub async fn get_bits_state(
    state: State<'_, AppState>,
    channel_login: String,
    channel_id: String,
) -> Result<BitsState, String> {
    let key = channel_login.trim().trim_start_matches('#').to_ascii_lowercase();
    let ch = bits::channel_bits(&key).await?;
    if !ch.enabled {
        return Ok(BitsState {
            enabled: false,
            cheer_min: ch.cheer_min,
            emote_min: ch.emote_min,
            pick_min: ch.smallest_single(),
            pick_max: bits::MAX_PER_CHEERMOTE,
            quick_amounts: vec![],
            balance: None,
            cheermotes: vec![],
            icons: vec![],
        });
    }
    if channel_cheermote_set(&key).is_none() && !channel_id.is_empty() {
        IrcService::fetch_and_store_cheermotes(key.clone(), channel_id.clone()).await;
    }
    // The balance the wallet already holds; read once per account, then kept
    // current by each cheer's answer and the Get Bits close.
    let balance = bits::known_balance(&state.twitch_auth).await;
    let mut cheermotes: Vec<(u32, CheermoteChoice)> = channel_cheermote_set(&key)
        .map(|set| {
            set.values()
                .filter_map(|tiers| {
                    let first = tiers.first()?;
                    let amounts: Vec<CheermoteAmount> = tiers
                        .iter()
                        .filter(|t| t.can_cheer)
                        .map(|t| CheermoteAmount { bits: t.min_bits, url: t.url.clone(), color: t.color.clone() })
                        .collect();
                    (!amounts.is_empty()).then(|| {
                        (
                            first.order,
                            CheermoteChoice { prefix: first.prefix.clone(), custom: first.custom, tiers: amounts },
                        )
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let quick_amounts = ch.quick_amounts(cheermotes.iter().flat_map(|(_, c)| c.tiers.iter().map(|t| t.bits)));
    cheermotes.sort_by(|(ao, a), (bo, b)| b.custom.cmp(&a.custom).then(ao.cmp(bo)).then(a.prefix.cmp(&b.prefix)));
    let icons = channel_cheermote_set(&key)
        .and_then(|set| {
            set.get(bits::BITS_ICON_PREFIX).map(|tiers| {
                tiers
                    .iter()
                    .map(|t| CheermoteAmount { bits: t.min_bits, url: t.url.clone(), color: t.color.clone() })
                    .collect()
            })
        })
        .unwrap_or_default();
    Ok(BitsState {
        enabled: true,
        cheer_min: ch.cheer_min,
        emote_min: ch.emote_min,
        pick_min: ch.smallest_single(),
        pick_max: bits::MAX_PER_CHEERMOTE,
        quick_amounts,
        balance,
        cheermotes: cheermotes.into_iter().map(|(_, c)| c).collect(),
        icons,
    })
}

/// Check a message for cheers: live while the viewer types (the cheer bar),
/// and again after Get Bits. `None` when it does not cheer in that channel.
/// Uses the cached balance, so typing never waits on Twitch.
#[tauri::command]
pub async fn check_cheer(
    state: State<'_, AppState>,
    channel_login: String,
    channel_id: String,
    text: String,
) -> Result<Option<bits::CheerCheck>, String> {
    Ok(bits::typed_cheer(&state.twitch_auth, &text, &channel_login, Some(&channel_id), true).await)
}

/// Send a cheer the viewer confirmed. Every rule is checked again here, with a
/// fresh balance, before anything is spent: the page's copy may be stale.
#[tauri::command]
pub async fn send_cheer(
    app: AppHandle,
    state: State<'_, AppState>,
    channel_login: String,
    channel_id: String,
    text: String,
    continue_anyway: Option<bool>,
) -> Result<CheerOutcome, String> {
    let key = channel_login.trim().trim_start_matches('#').to_ascii_lowercase();
    let set = channel_cheermote_set(&key).ok_or("This channel's cheermotes haven't loaded yet. Try again in a moment.")?;
    let cheers = bits::scan(&text, &set);
    if cheers.count == 0 {
        return Err("There's no cheer in this message.".to_string());
    }
    let ch = bits::channel_bits(&key).await?;
    if !ch.enabled {
        return Err("This channel has bits turned off.".to_string());
    }
    let check = bits::check_for(&state.twitch_auth, &text, &key, &channel_id, &set, &cheers, &ch, true, true).await;
    if let Some(problem) = check.problems.first() {
        return Ok(CheerOutcome {
            sent: false,
            balance: check.balance,
            problem: Some(problem.clone()),
            needs_bits: check.needs_bits,
            ..Default::default()
        });
    }
    let token = web_session_token(&state.twitch_auth).await?;
    let continue_anyway = continue_anyway.unwrap_or(false);
    // Twitch refuses a cheer without its security token (IntegrityCheckFailed,
    // nothing spent). Mint one, and if Twitch refuses a cached one, mint again
    // and send once more: the refused send never reached the bits.
    let integrity = match crate::services::twitch_integrity::get().await {
        Ok(i) => Some(i),
        Err(e) => {
            log::warn!("[Bits] no integrity token: {e}");
            None
        }
    };
    let mut outcome = bits::send(&token, &channel_id, &text, cheers.total, continue_anyway, integrity.as_ref()).await?;
    if outcome.integrity_refused && integrity.is_some() {
        crate::services::twitch_integrity::invalidate();
        match crate::services::twitch_integrity::get().await {
            Ok(fresh) => outcome = bits::send(&token, &channel_id, &text, cheers.total, continue_anyway, Some(&fresh)).await?,
            Err(e) => log::warn!("[Bits] fresh integrity token failed: {e}"),
        }
    }
    if let Some(balance) = outcome.balance {
        let _ = app.emit("bits-balance-changed", serde_json::json!({ "balance": balance }));
    }
    Ok(outcome)
}

const GET_BITS_URL: &str = "https://www.twitch.tv/bits";

/// Twitch's own bits page in the signed-in panel, like Subscribe: buying
/// happens on Twitch and never touches the app. The balance is read again when
/// the panel closes (`refresh_balance_after_purchase`).
#[cfg(desktop)]
#[tauri::command]
pub fn open_bits_window(app: AppHandle) -> Result<String, String> {
    let label = format!("bits-{}", chrono::Utc::now().timestamp_millis());
    crate::commands::twitch::emit_overlay_open_with(&app, &label, GET_BITS_URL, "panel", None)?;
    Ok(label)
}

/// The phone's Get Bits: the same page in the login overlay.
#[cfg(target_os = "android")]
#[tauri::command]
pub async fn open_bits_window(app: AppHandle) -> Result<String, String> {
    crate::twitch_login_plugin::open_overlay(&app, GET_BITS_URL, "Get Bits")?;
    Ok("bits".to_string())
}

/// A Get Bits panel closed: read the balance again and tell every window.
#[cfg(desktop)]
pub fn refresh_balance_after_purchase(app: &AppHandle) {
    use tauri::Manager;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let Ok(token) = web_session_token(&state.twitch_auth).await else { return };
        if let Ok((_, balance)) = bits::fetch_balance(&token).await {
            let _ = app.emit("bits-balance-changed", serde_json::json!({ "balance": balance }));
        }
    });
}

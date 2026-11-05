//! StreamNook's own cosmetics (member badges, Atmospheres), the profile
//! surfaces built from every cosmetic source, and the writes that change what a
//! member wears. Each is a thin door onto `services::cosmetics_registry`,
//! `services::member_theme` and `services::profile_view`.

use crate::services::cosmetics_registry::{self as registry, Atmosphere, MemberIds, Snapshot};
use crate::services::member_link::{self, PageLinkAnswer};
use crate::services::member_theme::{self as themes, MemberTheme, ProfilePrefs};
use crate::services::profile_view::{self, ExternalPicker, OwnIdentity, ProfileSheet, StreamNookPicker};
use tauri::command;

/// Everything the pages' synchronous registry reads need.
#[command]
pub async fn sn_registry_snapshot() -> Snapshot {
    registry::snapshot()
}

/// Re-reads the member, cosmetics and Atmosphere registries (focus, resume, a
/// fresh grant). Idempotent; a call during a load queues one more pass.
#[command]
pub async fn sn_refresh_registries() {
    registry::refresh().await;
}

/// The Atmosphere catalog with each look as layers, for native renderers.
#[command]
pub async fn sn_atmospheres() -> Vec<Atmosphere> {
    registry::ready(std::time::Duration::from_secs(5)).await;
    registry::atmospheres()
}

#[command]
pub async fn get_profile_prefs(user_id: String) -> ProfilePrefs {
    themes::profile_prefs(&user_id).await
}

/// The accolades a member earned; an error (not an empty list) when the read
/// failed.
#[command]
pub async fn get_accolades(user_id: String) -> Result<Vec<String>, String> {
    profile_view::accolades(&user_id).await
}

#[command]
pub async fn is_stripe_subscriber(user_id: String) -> bool {
    profile_view::stripe_subscriber(&user_id).await
}

/// A member's resolved theme (None for a non-member). Answers at once from the
/// cache; a member not seen yet costs one read.
#[command]
pub async fn get_member_theme(member_id: String) -> Option<MemberTheme> {
    themes::resolve(&member_id).await
}

/// Marks the viewer's own accounts so their theme picks paint without the wear
/// gate and are kept for the next cold launch.
#[command]
pub async fn register_own_member_themes(member_ids: Vec<String>) {
    themes::register_own(&member_ids);
}

/// Wears a StreamNook badge, or with None the plain mark. The error is the
/// server's words when it gave any, else "Could not equip that badge.".
#[command]
pub async fn equip_streamnook_badge(user_id: String, slug: Option<String>) -> Result<(), String> {
    let equipped = registry::equip_badge(&user_id, slug.as_deref())
        .await
        .map_err(|e| e.unwrap_or_else(|| "Could not equip that badge.".to_string()));
    // Their sheets show what they wear, whether or not this write landed.
    profile_view::bump_member_epoch(&user_id);
    equipped?;
    if let Some(theme) = themes::peek(&user_id) {
        crate::services::chat_row_view::post_member_frame(&theme);
    }
    Ok(())
}

/// Sets the profile theme (`tier`, `paint`, an Atmosphere id, the Cologne id).
/// Fails with "Could not apply that atmosphere." when the write did not land.
#[command]
pub async fn set_profile_theme(user_id: String, theme: String) -> Result<(), String> {
    themes::register_own(std::slice::from_ref(&user_id));
    let result = themes::set_profile_theme(&user_id, &theme).await.map_err(|e| {
        log::warn!("[SnCosmetics] theme write failed: {e}");
        "Could not apply that atmosphere.".to_string()
    });
    profile_view::bump_member_epoch(&user_id);
    result
}

/// Shows or hides the member's theme on their chat rows; whether it saved.
#[command]
pub async fn set_chat_atmosphere(user_id: String, show: bool) -> bool {
    themes::register_own(std::slice::from_ref(&user_id));
    let saved = themes::set_chat_atmosphere(&user_id, show).await;
    profile_view::bump_member_epoch(&user_id);
    saved
}

/// The user profile sheet for one person in one room. `tz_offset_min` is the
/// viewer's offset from UTC in minutes, east positive (the phones pass the
/// device's; None reads the platform's own zone). `open_id` names this
/// presentation of the sheet: its roast line changes once per presentation, and
/// None never changes it. Both are optional so older callers keep working.
#[command]
#[allow(clippy::too_many_arguments)]
pub async fn user_profile_sheet(
    user_id: String,
    login: String,
    display_name: String,
    color: Option<String>,
    channel_id: Option<String>,
    channel_login: Option<String>,
    tz_offset_min: Option<i32>,
    open_id: Option<String>,
) -> ProfileSheet {
    profile_view::profile_sheet(
        &user_id,
        &login,
        &display_name,
        color.as_deref(),
        channel_id.as_deref(),
        channel_login.as_deref(),
        tz_offset_min,
        open_id.as_deref(),
    )
    .await
}

/// The sheet as last built for this person in this room, without asking
/// anything. `open_id` is the presentation (see `user_profile_sheet`).
#[command]
pub async fn user_profile_sheet_cached(user_id: String, channel_id: Option<String>, open_id: Option<String>) -> Option<ProfileSheet> {
    profile_view::cached_sheet(&user_id, channel_id.as_deref(), open_id.as_deref())
}

/// The viewer's own identity header, built fresh.
#[command]
pub async fn own_identity_header(user_id: String, login: String, display_name: String) -> OwnIdentity {
    profile_view::own_identity(&user_id, &login, &display_name).await
}

/// The header as saved last time, for the first frame of the You tab.
#[command]
pub async fn own_identity_header_cached(user_id: String) -> Option<OwnIdentity> {
    profile_view::cached_own_identity(&user_id)
}

/// The StreamNook half of the Cosmetics screen.
#[command]
pub async fn cosmetics_picker_state(user_id: String) -> StreamNookPicker {
    profile_view::streamnook_picker(&user_id).await
}

/// The 7TV and Twitch half of the Cosmetics screen.
#[command]
pub async fn external_cosmetics_state(user_id: String, login: String) -> ExternalPicker {
    profile_view::external_picker(&user_id, &login).await
}

/// The StreamNook page link for a member's card, for the desktop (the phones get
/// it on the sheet). `login` is the member's own Twitch login for `member_id`.
/// States: `member` (with the link), `not_member`, `not_ready` (the member list
/// has not loaded, or the login is not known yet; ask again when either lands).
#[command]
pub async fn streamnook_profile_link(member_id: String, login: String) -> PageLinkAnswer {
    member_link::link_answer(&member_id, &login)
}

/// Something that changes what a member's sheet shows was written outside Rust
/// (an accolade grant): the next sheet build reads it.
#[command]
pub async fn profile_changed(user_id: String) {
    profile_view::bump_member_epoch(&user_id);
}

/// Who is a member, with the registry version and whether the list has loaded:
/// the ids alone, for a phone that only needs to know who to reserve a page link
/// for (the snapshot carries the whole catalog and every grant).
#[command]
pub async fn sn_member_ids() -> MemberIds {
    registry::member_ids()
}

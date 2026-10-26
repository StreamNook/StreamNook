//! Twitch's Drops approval, asked for when an action needs it.
//!
//! Follow, channel points and drops all run on the Android-client grant the
//! Drops sign-in stores (`DropsAuthService`), not on the main sign-in. Someone
//! who never connected Drops, or whose grant was cleared by a sign-out, used to
//! get a bare failure. Here the platform's sign-in sheet is opened at Twitch's
//! authorize page instead (an account that approved before passes straight
//! through, with no consent screen), the grant is collected, and the action
//! runs again. One body for desktop and both phones, like the Kick sign-in.

use std::time::{Duration, Instant};

use anyhow::anyhow;

use crate::services::drops_auth_service::DropsAuthService;

/// A Twitch action's answer when the grant is missing or revoked.
pub const NEEDS_DROPS_AUTH: &str = "NEEDS_DROPS_AUTH";
/// The person closed the approval sheet. Nothing failed, so callers stay quiet.
pub const APPROVAL_CLOSED: &str = "APPROVAL_CLOSED";

const POLL: Duration = Duration::from_millis(400);
/// The iPhone's sheet reports closed just before its grant is saved.
const SETTLE: Duration = Duration::from_millis(500);
/// Long enough to sign in from scratch if the web session had lapsed.
const WINDOW: Duration = Duration::from_secs(300);

/// One approval at a time. A second caller waits, then finds the grant stored
/// (or asks again, since its own tap asked for it).
static GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Ask Twitch for the approval in the sign-in sheet. True once the grant is
/// stored (at once when it already is), false when the sheet was closed.
pub async fn ensure() -> anyhow::Result<bool> {
    let _one = GATE.lock().await;
    if DropsAuthService::is_authenticated().await {
        return Ok(true);
    }
    let app = crate::services::providers::app_handle()
        .ok_or_else(|| anyhow!("app handle not available for the Twitch approval"))?;
    let start = DropsAuthService::token_epoch();
    sheet::open(&app, &DropsAuthService::authorize_url()).map_err(|e| anyhow!(e))?;
    let outcome = wait(&app, start).await;
    sheet::close(&app);
    let approved = outcome?;
    log::info!("[TwitchApproval] {}", if approved { "approved" } else { "sheet closed" });
    // The desktop overlay and the iPhone sheet announce from their own
    // capture; on Android the capture is the loop in `wait`.
    #[cfg(target_os = "android")]
    {
        use tauri::Emitter;
        if approved {
            let _ = app.emit("drops-login-complete", ());
        }
    }
    Ok(approved)
}

async fn stored_since(start: u64) -> bool {
    DropsAuthService::token_epoch() != start && DropsAuthService::is_authenticated().await
}

async fn wait(app: &crate::rt::AppHandle, start: u64) -> anyhow::Result<bool> {
    let deadline = Instant::now() + WINDOW;
    loop {
        tokio::time::sleep(POLL).await;
        // Open flag FIRST, then the grant. The sheet keeps the grant before it
        // closes, so once a closed sheet is seen here, the take below sees any
        // grant that closed it (the same order as the Kick relay).
        let open = sheet::is_open(app);
        let token = sheet::take_drops_token(app);
        if !token.is_empty() {
            DropsAuthService::store_access_token(token).await?;
        }
        if stored_since(start).await {
            return Ok(true);
        }
        if !open {
            tokio::time::sleep(SETTLE).await;
            return Ok(stored_since(start).await);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
    }
}

/// Run a Twitch action that rides on the approval. When it answers
/// `NEEDS_DROPS_AUTH`, ask for the approval once and run it again.
pub async fn run<T, Fut>(action: impl FnMut() -> Fut) -> Result<T, String>
where
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    run_with(action, ensure).await
}

async fn run_with<T, Fut, A, AFut>(mut action: impl FnMut() -> Fut, approve: A) -> Result<T, String>
where
    Fut: std::future::Future<Output = anyhow::Result<T>>,
    A: FnOnce() -> AFut,
    AFut: std::future::Future<Output = anyhow::Result<bool>>,
{
    match action().await {
        Err(e) if e.to_string() == NEEDS_DROPS_AUTH => {
            if !approve().await.map_err(|e| e.to_string())? {
                return Err(APPROVAL_CLOSED.to_string());
            }
            action().await.map_err(|e| e.to_string())
        }
        other => other.map_err(|e| e.to_string()),
    }
}

/// `dismiss_login_overlay` calls this when the drops overlay goes away, by
/// completion or by the person closing it.
#[cfg(desktop)]
pub fn sheet_dismissed() {
    sheet::OPEN.store(false, std::sync::atomic::Ordering::SeqCst);
}

/// Desktop: the React-chromed `drops-login` overlay in the main window. Its
/// window is mounted by the frontend after the open event, so whether it is up
/// is tracked here rather than asked of the window manager.
#[cfg(desktop)]
mod sheet {
    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::rt::AppHandle;

    const LABEL: &str = "drops-login";
    pub(super) static OPEN: AtomicBool = AtomicBool::new(false);

    pub fn open(app: &AppHandle, url: &str) -> Result<(), String> {
        // A follow from a popout or with the app in the tray: the overlay lives
        // in the main window, so bring it forward first.
        crate::show_main_window(app);
        OPEN.store(true, Ordering::SeqCst);
        crate::commands::twitch::emit_overlay_open_with(app, LABEL, url, "fullbody", None)
    }

    pub fn is_open(_app: &AppHandle) -> bool {
        OPEN.load(Ordering::SeqCst)
    }

    /// `report_login_popup_url` stores the grant itself on desktop.
    pub fn take_drops_token(_app: &AppHandle) -> String {
        String::new()
    }

    pub fn close(app: &AppHandle) {
        crate::commands::twitch::dismiss_login_overlay(app, LABEL);
    }
}

/// Phones: the same Rust-driven sheet the Kick sign-in uses.
#[cfg(mobile)]
mod sheet {
    #[cfg(target_os = "ios")]
    use crate::services::ios_sheet as overlay;
    #[cfg(target_os = "android")]
    use crate::twitch_login_plugin as overlay;

    use crate::rt::AppHandle;

    pub fn open(app: &AppHandle, url: &str) -> Result<(), String> {
        overlay::open_overlay(app, url, "Approve on Twitch")
    }

    pub fn is_open(app: &AppHandle) -> bool {
        overlay::overlay_is_open(app)
    }

    pub fn take_drops_token(app: &AppHandle) -> String {
        overlay::take_drops_token(app)
    }

    pub fn close(app: &AppHandle) {
        overlay::close_overlay(app)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[tokio::test]
    async fn a_missing_approval_is_asked_for_and_the_action_runs_again() {
        let runs = Cell::new(0);
        let out = run_with(
            || {
                runs.set(runs.get() + 1);
                let first = runs.get() == 1;
                async move { if first { Err(anyhow!(NEEDS_DROPS_AUTH)) } else { Ok(7) } }
            },
            || async { Ok(true) },
        )
        .await;
        assert_eq!(out, Ok(7));
        assert_eq!(runs.get(), 2);
    }

    #[tokio::test]
    async fn a_closed_sheet_is_reported_and_the_action_is_not_retried() {
        let runs = Cell::new(0);
        let out: Result<(), String> = run_with(
            || {
                runs.set(runs.get() + 1);
                async { Err(anyhow!(NEEDS_DROPS_AUTH)) }
            },
            || async { Ok(false) },
        )
        .await;
        assert_eq!(out, Err(APPROVAL_CLOSED.to_string()));
        assert_eq!(runs.get(), 1);
    }

    #[tokio::test]
    async fn other_errors_pass_through_without_asking() {
        let asked = Cell::new(false);
        let out: Result<(), String> = run_with(
            || async { Err(anyhow!("GQL follow failed (HTTP 500)")) },
            || {
                asked.set(true);
                async { Ok(true) }
            },
        )
        .await;
        assert_eq!(out, Err("GQL follow failed (HTTP 500)".to_string()));
        assert!(!asked.get());
    }
}

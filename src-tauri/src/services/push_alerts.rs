//! Instant go-live alerts on both phones: this phone's registration with the
//! push server (`notify.streamnook.app`).
//!
//! The server holds, per device, the ids of the channels whose go-lives it
//! should push, and pushes `stream.online` to every device holding that id.
//! Which channels those are is decided here, in one place for both phones:
//! the channels the viewer follows, minus the ones muted for live alerts, and
//! none at all while the master switch or the Live channels switch is off. An
//! empty list keeps the device registered with nothing to send, so turning a
//! switch back on needs no new token.
//!
//! The device differs per phone. Android hands over its FCM token through the
//! `fcm_token` file Kotlin writes when Play services answers; the iPhone hands
//! over its APNs token, environment and topic with `push_set_apns_device`.
//! Android also keeps its own Instant alerts switch (`push_notifications`): off
//! drops the device from the server and leaves the scheduled poll to deliver.
//! The iPhone's poll (`services::live_poll`) is a background refresh iOS runs
//! when it chooses, a fallback only, so push stays its main lane and that
//! switch is not read there.
//!
//! Every change that alters the list (a follow, an unfollow, a mute, the
//! switches) re-registers on a short debounce, so a run of quick changes costs
//! one request.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The push lane's server. Registration is authed by the viewer's own Twitch
/// token; the server validates it against Twitch and stores only the user id,
/// the device token and the channel ids it was handed.
#[cfg(mobile)]
const NOTIFY_SERVER: &str = "https://notify.streamnook.app";

/// Where this phone's pushes go.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PushDevice {
    /// Firebase Cloud Messaging (Android).
    Fcm { token: String },
    /// Apple Push Notification service (iPhone): `env` is `sandbox` for
    /// development-signed builds and `production` for TestFlight and the App
    /// Store; `topic` is the app's own bundle id.
    Apns { token: String, env: String, topic: String },
}

impl PushDevice {
    /// An APNs device from what the iPhone reports, or why it cannot be one.
    pub fn apns(token: &str, env: &str, topic: &str) -> Result<Self, String> {
        let token = token.trim().to_ascii_lowercase();
        if token.is_empty() || token.len() % 2 != 0 || !token.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("The device token is not hex.".to_string());
        }
        if !matches!(env, "sandbox" | "production") {
            return Err(format!("Unknown push environment {env}."));
        }
        let topic = topic.trim();
        if topic.is_empty() {
            return Err("The push topic is empty.".to_string());
        }
        Ok(Self::Apns { token, env: env.to_string(), topic: topic.to_string() })
    }

    /// The `/register` body. An FCM device also carries the original
    /// `fcm_token` key, so the body reads the same to a server that predates
    /// `kind` and to one that knows it.
    pub fn register_body(&self, follows: &[String]) -> Value {
        let mut body = self.identity();
        body["follows"] = json!(follows);
        body
    }

    /// The `/unregister` body.
    pub fn unregister_body(&self) -> Value {
        self.identity()
    }

    fn identity(&self) -> Value {
        match self {
            Self::Fcm { token } => json!({ "kind": "fcm", "token": token, "fcm_token": token }),
            Self::Apns { token, env, topic } => json!({ "kind": "apns", "token": token, "env": env, "topic": topic }),
        }
    }
}

/// The settings that decide what the server holds for this phone. Muted logins
/// are kept lowercased and sorted, so two reads of the same list compare equal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlertInputs {
    /// The master notifications switch.
    pub enabled: bool,
    /// Notify me about > Live channels.
    pub live: bool,
    /// Android's Instant alerts switch.
    pub push: bool,
    pub muted: Vec<String>,
}

impl AlertInputs {
    /// From a `live_notifications` object as saved. A switch that was never
    /// saved reads as on, matching the serde defaults on the settings struct.
    pub fn from_json(notif: Option<&Value>) -> Self {
        let flag = |name: &str| notif.and_then(|n| n.get(name)).and_then(Value::as_bool).unwrap_or(true);
        let mut muted: Vec<String> = notif
            .and_then(|n| n.get("muted_live_channels"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        muted.sort();
        muted.dedup();
        Self { enabled: flag("enabled"), live: flag("show_live_notifications"), push: flag("push_notifications"), muted }
    }

    pub fn from_settings(settings: &crate::models::settings::Settings) -> Self {
        Self::from_json(serde_json::to_value(&settings.live_notifications).ok().as_ref())
    }
}

/// What the server should hold for this phone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Not registered at all (Android with Instant alerts off).
    Drop,
    /// Registered; with the followed channels when `alerting`, else with none.
    Hold { alerting: bool },
}

pub fn plan(platform: &str, inputs: &AlertInputs) -> Plan {
    if platform == "android" && !inputs.push {
        return Plan::Drop;
    }
    let alerting = inputs.enabled && inputs.live && crate::services::alert_types::works_on("live", platform);
    Plan::Hold { alerting }
}

/// The channel ids to hand the server, from the followed channels as
/// `(id, login)`: every followed channel whose login is not muted, each once,
/// in follow order. `None` drops the device instead.
pub fn follow_list(platform: &str, inputs: &AlertInputs, follows: &[(String, String)]) -> Option<Vec<String>> {
    match plan(platform, inputs) {
        Plan::Drop => None,
        Plan::Hold { alerting: false } => Some(Vec::new()),
        Plan::Hold { alerting: true } => {
            let mut seen = std::collections::HashSet::new();
            Some(
                follows
                    .iter()
                    .filter(|(id, login)| {
                        !id.is_empty() && !inputs.muted.iter().any(|m| m.eq_ignore_ascii_case(login.trim()))
                    })
                    .filter(|(id, _)| seen.insert(id.clone()))
                    .map(|(id, _)| id.clone())
                    .collect(),
            )
        }
    }
}

/// The phone side: the device, the debounce and the commands. Phones only.
#[cfg(mobile)]
pub mod server {
    use super::*;
    use crate::services::cache_service;
    use crate::services::twitch_service::TwitchService;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// How long a run of changes settles before the one re-register.
    const DEBOUNCE: Duration = Duration::from_secs(3);
    #[cfg(target_os = "ios")]
    /// The iPhone reports its token on every launch and return to the app; the
    /// same device registered this recently is not sent again. Follows made
    /// outside StreamNook reach the server on the next send after this.
    const RESEND_AFTER: Duration = Duration::from_secs(6 * 60 * 60);
    /// Followed channels are paged 100 at a time, up to this many pages.
    const FOLLOW_PAGES: usize = 20;
    const APNS_FILE: &str = "apns_device.json";

    /// Bumped by every send and every scheduled re-register; a debounced one
    /// that finds it moved stands down.
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    /// The inputs the last settings save left, compared on the next.
    static INPUTS: Mutex<Option<AlertInputs>> = Mutex::new(None);
    /// The device the server last accepted, and when.
    static LAST_SENT: Mutex<Option<(PushDevice, Instant)>> = Mutex::new(None);

    /// The settings as the app loaded them at launch, before any save.
    pub fn seed(settings: &crate::models::settings::Settings) {
        if let Ok(mut inputs) = INPUTS.lock() {
            *inputs = Some(AlertInputs::from_settings(settings));
        }
    }

    /// After every settings save: re-register when a switch or the mute list
    /// moved.
    pub fn refresh(settings: &crate::models::settings::Settings) {
        let next = AlertInputs::from_settings(settings);
        let changed = match INPUTS.lock() {
            Ok(mut inputs) => {
                let changed = inputs.as_ref() != Some(&next);
                *inputs = Some(next);
                changed
            }
            Err(_) => false,
        };
        if changed {
            schedule();
        }
    }

    /// Re-register once things settle (a follow, an unfollow, a settings change).
    pub fn schedule() {
        let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(DEBOUNCE).await;
            if GENERATION.load(Ordering::SeqCst) != generation {
                return;
            }
            match sync().await {
                Ok(outcome) => log::info!("[PushAlerts] re-registered: {outcome}"),
                Err(e) => log::warn!("[PushAlerts] re-register failed: {e}"),
            }
        });
    }

    fn inputs() -> AlertInputs {
        if let Some(inputs) = INPUTS.lock().ok().and_then(|i| i.clone()) {
            return inputs;
        }
        let saved = cache_service::get_app_data_dir()
            .ok()
            .and_then(|d| std::fs::read_to_string(d.join("settings.json")).ok())
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .unwrap_or(Value::Null);
        AlertInputs::from_json(saved.get("live_notifications"))
    }

    /// This phone's device, when it has reported one.
    fn device() -> Option<PushDevice> {
        let dir = cache_service::get_app_data_dir().ok()?;
        if cfg!(target_os = "ios") {
            let text = std::fs::read_to_string(dir.join(APNS_FILE)).ok()?;
            serde_json::from_str::<PushDevice>(&text).ok()
        } else {
            let token = std::fs::read_to_string(dir.join("fcm_token")).ok()?;
            let token = token.trim();
            (!token.is_empty()).then(|| PushDevice::Fcm { token: token.to_string() })
        }
    }

    /// Every followed Twitch channel as `(id, login)`, paged.
    async fn followed() -> Result<Vec<(String, String)>, String> {
        let mut follows = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..FOLLOW_PAGES {
            let (page, next) = TwitchService::get_all_followed_channels(100, cursor.take())
                .await
                .map_err(|e| e.to_string())?;
            let empty = page.is_empty();
            follows.extend(page.into_iter().map(|c| (c.user_id, c.user_login)));
            match next {
                Some(next) if !empty => cursor = Some(next),
                _ => break,
            }
        }
        Ok(follows)
    }

    async fn post(path: &str, body: &Value) -> Result<(), String> {
        let user_token = TwitchService::get_token().await.map_err(|e| e.to_string())?;
        let resp = crate::services::http::client()
            .clone()
            .post(format!("{NOTIFY_SERVER}{path}"))
            .header(reqwest::header::AUTHORIZATION, format!("OAuth {user_token}"))
            .json(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("{path} failed: {status}"));
        }
        Ok(())
    }

    /// Puts the server in step with this phone now: the follow list the rules
    /// give, or no registration at all. "no-token" is a normal outcome: the
    /// device has not reported one yet, and registers when it does.
    async fn sync() -> Result<String, String> {
        GENERATION.fetch_add(1, Ordering::SeqCst);
        let Some(device) = device() else {
            return Ok("no-token".to_string());
        };
        let inputs = inputs();
        let follows = match plan(crate::services::alert_types::PLATFORM, &inputs) {
            Plan::Hold { alerting: true } => followed().await?,
            _ => Vec::new(),
        };
        match follow_list(crate::services::alert_types::PLATFORM, &inputs, &follows) {
            Some(ids) => {
                post("/register", &device.register_body(&ids)).await?;
                // The iPhone's poll leaves these to the push once pushes arrive.
                #[cfg(target_os = "ios")]
                crate::services::live_poll::poll::note_registered(Some(&ids));
                if let Ok(mut last) = LAST_SENT.lock() {
                    *last = Some((device, Instant::now()));
                }
                Ok(format!("registered:{}", ids.len()))
            }
            None => {
                post("/unregister", &device.unregister_body()).await?;
                #[cfg(target_os = "ios")]
                crate::services::live_poll::poll::note_registered(None);
                if let Ok(mut last) = LAST_SENT.lock() {
                    *last = None;
                }
                Ok("unregistered".to_string())
            }
        }
    }

    /// Register this phone with the push server now. Called by the shells
    /// after sign-in and when a switch turns on; re-running is cheap (the
    /// server replaces the follow set wholesale).
    #[tauri::command]
    pub async fn push_register() -> Result<String, String> {
        sync().await
    }

    /// Drop this phone from the push server.
    #[tauri::command]
    pub async fn push_unregister() -> Result<(), String> {
        GENERATION.fetch_add(1, Ordering::SeqCst);
        let Some(device) = device() else {
            return Ok(());
        };
        post("/unregister", &device.unregister_body()).await?;
        #[cfg(target_os = "ios")]
        crate::services::live_poll::poll::note_registered(None);
        if let Ok(mut last) = LAST_SENT.lock() {
            *last = None;
        }
        Ok(())
    }

    /// The iPhone's APNs device, as the system reported it: the token in hex,
    /// `sandbox` or `production`, and the app's bundle id as the topic. Kept
    /// for every later re-register, then registered at once unless the same
    /// device was registered a short while ago.
    #[cfg(target_os = "ios")]
    #[tauri::command]
    pub async fn push_set_apns_device(token: String, env: String, topic: String) -> Result<String, String> {
        let device = PushDevice::apns(&token, &env, &topic)?;
        let recent = LAST_SENT
            .lock()
            .ok()
            .and_then(|last| last.as_ref().map(|(sent, at)| *sent == device && at.elapsed() < RESEND_AFTER))
            .unwrap_or(false);
        let path = cache_service::get_app_data_dir().map_err(|e| e.to_string())?.join(APNS_FILE);
        let stored = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<PushDevice>(&s).ok());
        if stored.as_ref() != Some(&device) {
            let json = serde_json::to_string(&device).map_err(|e| e.to_string())?;
            std::fs::write(&path, json).map_err(|e| e.to_string())?;
        }
        if recent {
            return Ok("unchanged".to_string());
        }
        sync().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn follows() -> Vec<(String, String)> {
        [("1", "alpha"), ("2", "Bravo"), ("3", "charlie"), ("2", "bravo"), ("", "ghost")]
            .iter()
            .map(|(id, login)| (id.to_string(), login.to_string()))
            .collect()
    }

    fn inputs(enabled: bool, live: bool, push: bool, muted: &[&str]) -> AlertInputs {
        let notif = json!({
            "enabled": enabled,
            "show_live_notifications": live,
            "push_notifications": push,
            "muted_live_channels": muted,
        });
        AlertInputs::from_json(Some(&notif))
    }

    #[test]
    fn every_followed_channel_alerts_once_in_follow_order() {
        let on = inputs(true, true, true, &[]);
        for platform in ["android", "ios"] {
            assert_eq!(follow_list(platform, &on, &follows()), Some(vec!["1".into(), "2".into(), "3".into()]), "{platform}");
        }
    }

    #[test]
    fn muted_channels_are_left_out_whatever_their_case() {
        let some = inputs(true, true, true, &["BRAVO", " charlie "]);
        assert_eq!(follow_list("ios", &some, &follows()), Some(vec!["1".into()]));
        assert_eq!(follow_list("android", &some, &follows()), Some(vec!["1".into()]));
    }

    #[test]
    fn master_or_live_off_keeps_the_device_with_nothing_to_send() {
        for (enabled, live) in [(false, true), (true, false), (false, false)] {
            let off = inputs(enabled, live, true, &[]);
            assert_eq!(follow_list("ios", &off, &follows()), Some(vec![]));
            assert_eq!(follow_list("android", &off, &follows()), Some(vec![]));
            assert_eq!(plan("ios", &off), Plan::Hold { alerting: false });
        }
    }

    #[test]
    fn instant_alerts_off_drops_an_android_device_and_is_not_read_on_iphone() {
        let no_push = inputs(true, true, false, &[]);
        assert_eq!(plan("android", &no_push), Plan::Drop);
        assert_eq!(follow_list("android", &no_push, &follows()), None);
        assert_eq!(follow_list("ios", &no_push, &follows()), Some(vec!["1".into(), "2".into(), "3".into()]));
    }

    #[test]
    fn both_phones_send_go_live_alerts() {
        assert!(crate::services::alert_types::works_on("live", "ios"));
        assert!(crate::services::alert_types::works_on("live", "android"));
    }

    #[test]
    fn unsaved_switches_read_as_on() {
        let fresh = AlertInputs::from_json(None);
        assert_eq!(fresh, AlertInputs { enabled: true, live: true, push: true, muted: vec![] });
        let defaults = AlertInputs::from_settings(&crate::models::settings::Settings::default());
        assert_eq!(defaults, fresh);
    }

    #[test]
    fn muted_lists_compare_by_content() {
        assert_eq!(inputs(true, true, true, &["b", "A"]), inputs(true, true, true, &["a", "B", "a"]));
        assert_ne!(inputs(true, true, true, &["a"]), inputs(true, true, true, &[]));
    }

    #[test]
    fn apns_body_is_the_contract() {
        let device = PushDevice::apns("ABCDEF0123", "sandbox", "XTL-C84UQRPAQ2.app.streamnook").unwrap();
        assert_eq!(
            device.register_body(&["123".into(), "456".into()]),
            json!({
                "kind": "apns",
                "token": "abcdef0123",
                "env": "sandbox",
                "topic": "XTL-C84UQRPAQ2.app.streamnook",
                "follows": ["123", "456"],
            })
        );
        assert_eq!(device.unregister_body(), json!({ "kind": "apns", "token": "abcdef0123", "env": "sandbox", "topic": "XTL-C84UQRPAQ2.app.streamnook" }));
        assert_eq!(device.register_body(&[])["follows"], json!([]));
    }

    #[test]
    fn fcm_body_still_reads_as_the_old_android_body() {
        let device = PushDevice::Fcm { token: "fcm-abc".into() };
        let body = device.register_body(&["9".into()]);
        assert_eq!(body["kind"], "fcm");
        assert_eq!(body["token"], "fcm-abc");
        // What the server before `kind` reads: {fcm_token, follows}.
        assert_eq!(body["fcm_token"], "fcm-abc");
        assert_eq!(body["follows"], json!(["9"]));
        assert!(body.get("env").is_none() && body.get("topic").is_none());
        let gone = device.unregister_body();
        assert_eq!((gone["kind"].as_str(), gone["token"].as_str(), gone["fcm_token"].as_str()), (Some("fcm"), Some("fcm-abc"), Some("fcm-abc")));
        assert!(gone.get("follows").is_none());
    }

    #[test]
    fn apns_devices_are_checked() {
        assert!(PushDevice::apns("", "sandbox", "app.streamnook").is_err());
        assert!(PushDevice::apns("xyz1", "sandbox", "app.streamnook").is_err());
        assert!(PushDevice::apns("abc", "sandbox", "app.streamnook").is_err());
        assert!(PushDevice::apns("ab12", "development", "app.streamnook").is_err());
        assert!(PushDevice::apns("ab12", "production", " ").is_err());
        assert!(PushDevice::apns("ab12", "production", "app.streamnook").is_ok());
    }

    #[test]
    fn a_stored_device_reads_back() {
        let device = PushDevice::apns("ab12", "production", "app.streamnook").unwrap();
        let text = serde_json::to_string(&device).unwrap();
        assert_eq!(serde_json::from_str::<PushDevice>(&text).unwrap(), device);
    }
}

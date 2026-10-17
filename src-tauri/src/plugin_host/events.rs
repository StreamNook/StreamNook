//! Background event emitters: the watch tick and the followed-live feed.
//! Both are lazy: nothing polls and nothing is emitted unless at least one
//! running plugin subscribed to the event. Stream lifecycle events
//! (on_stream_start, on_stream_stop, on_channel_change) are pushed by the
//! frontend through the plugins_report_stream_event command instead.

use log::debug;
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use super::HostInner;

pub fn start_background_emitters(host: Arc<HostInner>) {
    // Watch tick: nominally every 60 seconds while anything subscribes.
    {
        let host = host.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                if host.shutting_down.load(Ordering::SeqCst) {
                    break;
                }
                if !host.any_hook("on_watch_tick").await {
                    continue;
                }
                let active = host.active_channel.read().await.clone();
                // `credential_epoch` changes whenever the drops token is
                // replaced or removed; a plugin holding a copy asks again.
                let params = json!({
                    "active_channel_id": active.map(|c| c.channel_id),
                    "ts": chrono::Utc::now().to_rfc3339(),
                    "credential_epoch": crate::services::drops_auth_service::DropsAuthService::token_epoch(),
                });
                host.emit_event("on_watch_tick", params).await;
            }
        });
    }

    // Followed-live: refreshed every 2 minutes while anything subscribes.
    {
        let host = host.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(120));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                if host.shutting_down.load(Ordering::SeqCst) {
                    break;
                }
                if !host.any_hook("on_followed_live").await {
                    continue;
                }
                match super::broker::fetch_followed_live(&host).await {
                    Ok(channels) => {
                        host.emit_event("on_followed_live", json!({ "channels": channels }))
                            .await;
                    }
                    Err(e) => {
                        debug!("[PluginHost] followed-live refresh failed: {e}");
                    }
                }
            }
        });
    }
}

//! The engine's one long-lived task.
//!
//! Every request (a settings save, an account change, the app coming to the
//! front, a window claiming an adopted room, recovery finishing) marks one
//! `watch` channel; the loop reads the newest mark, so requests never pile
//! up and two passes never run at once. Each pass compiles the rules, then
//! makes the engine's chat claims, liveness watches and badge work match
//! what the rules want. It is the only owner of the engine's chat claims and
//! watch refs.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::AbortHandle;
use tokio::sync::watch;

use crate::models::automations::{ChannelScope, RunState, MAX_ENGINE_CHATS};

use super::compile::Compiled;
use super::own_topics::{self, TopicKind};
use super::provider_facts::{KICK, TIKTOK, YOUTUBE};
use super::{lock, now_ms, Engine};

/// Chat start attempts: 5 s, doubling to 5 min, reset on success and on an
/// account save.
#[derive(Debug)]
pub(crate) struct Backoff {
    first: Duration,
    cap: Duration,
    wait: Duration,
    next: Option<Instant>,
}

impl Backoff {
    pub(crate) fn new(first: Duration, cap: Duration) -> Backoff {
        Backoff { first, cap, wait: first, next: None }
    }

    pub(crate) fn ready(&self, now: Instant) -> bool {
        self.next.is_none_or(|at| now >= at)
    }

    pub(crate) fn failed(&mut self, now: Instant) {
        self.next = Some(now + self.wait);
        self.wait = (self.wait * 2).min(self.cap);
    }

    pub(crate) fn reset(&mut self) {
        self.wait = self.first;
        self.next = None;
    }
}

/// A running rule listens in any open chat.
pub(crate) fn any_open(compiled: &Compiled) -> bool {
    compiled.running().any(|r| matches!(r.rule.only_if.channels, ChannelScope::AnyOpen {}))
}

/// A running rule listens in the chat of this device's current watch.
pub(crate) fn watching(compiled: &Compiled) -> bool {
    // A rule with `only_while_open` (every converted "current channel" reminder) never joins the
    // watched chat: it posts only where a window already holds it (§ 4.2, D281).
    compiled
        .running()
        .any(|r| matches!(r.rule.only_if.channels, ChannelScope::Watching {}) && !r.rule.only_if.only_while_open)
}

/// What the loop keeps between passes.
pub(crate) struct State {
    /// Rooms this task holds under the engine's label.
    pub(crate) joined: BTreeSet<String>,
    /// Live-only watches held, by login.
    refs: BTreeSet<String>,
    /// Chats of other platforms this task holds under the engine's label, by
    /// chat key (`kick:alpha`).
    pub(crate) provider_joined: BTreeSet<String>,
    /// P3: the own-channel topics last handed to the whisper session.
    own: Option<(String, BTreeSet<TopicKind>)>,
    pub(crate) backoff: Backoff,
    last_run: RunState,
    gallery_next: Option<Instant>,
    badge_task: Option<AbortHandle>,
}

impl State {
    pub(crate) fn new(engine: &Engine) -> State {
        let t = engine.timings;
        State {
            joined: BTreeSet::new(),
            refs: BTreeSet::new(),
            provider_joined: BTreeSet::new(),
            own: None,
            backoff: Backoff::new(t.chat_start_first, t.chat_start_cap),
            last_run: RunState::Off,
            gallery_next: None,
            badge_task: None,
        }
    }

    fn next_wake(&self) -> Option<Instant> {
        [self.backoff.next, self.gallery_next].into_iter().flatten().min()
    }
}

/// Start the loop on the engine's spawner; the handle stops it.
pub(crate) fn spawn(engine: Arc<Engine>, rx: watch::Receiver<u64>) -> AbortHandle {
    engine.spawn_abortable(run(engine.clone(), rx))
}

/// The loop: a pass, then sleep until the next mark or the next timer.
pub(crate) async fn run(engine: Arc<Engine>, mut rx: watch::Receiver<u64>) {
    let mut st = State::new(&engine);
    loop {
        rx.borrow_and_update();
        pass(&engine, &mut st).await;
        let wake = st.next_wake();
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() {
                    break;
                }
            }
            _ = sleep_until(wake) => {}
        }
    }
    if let Some(task) = st.badge_task.take() {
        task.abort();
    }
}

async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at.into()).await,
        None => std::future::pending::<()>().await,
    }
}

/// One pass. Crate-visible so tests drive it step by step.
pub(crate) async fn pass(engine: &Arc<Engine>, st: &mut State) {
    let now = Instant::now();
    let (claims_reset, reset_backoff, adopt) = {
        let mut rec = lock(&engine.rec);
        (
            std::mem::take(&mut rec.claims_reset),
            std::mem::take(&mut rec.reset_backoff),
            std::mem::take(&mut rec.adopt),
        )
    };
    if claims_reset {
        // IrcService dropped every claim: nothing is held any more.
        st.joined.clear();
    }
    if reset_backoff {
        st.backoff.reset();
    }

    let compiled = engine.recompile().await;
    let foreground = engine.foreground();
    let run = compiled.run;
    if run == RunState::Off && st.last_run != RunState::Off {
        engine.executor.bump();
    }
    st.last_run = run;
    let active = foreground && run != RunState::Off;
    let open = any_open(&compiled);

    // Adoption first, so a closing window's rooms never PART.
    for (label, done) in adopt {
        if active && open {
            adopt_rooms(engine, st, &label).await;
        }
        let _ = done.send(());
    }

    // Adopted rooms go when a window claimed them after the adoption, or
    // when no rule listens in any open chat any more.
    {
        let mut rec = lock(&engine.rec);
        let claims = std::mem::take(&mut rec.window_claims);
        let keep = open && active;
        rec.adopted.retain(|room, stamp| keep && claims.get(room).is_none_or(|at| at <= stamp));
    }

    // Wanted rooms: the rules' own chats, the watched chat, adopted rooms.
    let mut wanted: Vec<String> = Vec::new();
    if active {
        for login in &compiled.needed_chats {
            push_unique(&mut wanted, login);
        }
        if watching(&compiled) {
            if let Some(login) = engine.local.current_watch() {
                push_unique(&mut wanted, &login);
            }
        }
        let adopted: Vec<String> = lock(&engine.rec).adopted.keys().cloned().collect();
        for room in adopted {
            push_unique(&mut wanted, &room);
        }
    }
    wanted.truncate(MAX_ENGINE_CHATS);
    let wanted_set: BTreeSet<String> = wanted.iter().cloned().collect();

    for room in st.joined.difference(&wanted_set).cloned().collect::<Vec<_>>() {
        engine.local.chat_leave(&room).await;
        st.joined.remove(&room);
    }
    let missing: Vec<String> = wanted.iter().filter(|r| !st.joined.contains(*r)).cloned().collect();
    join_rooms(engine, st, &missing, now).await;
    provider_pass(engine, st, &compiled, active).await;
    engine.set_kept(st.joined.iter().chain(st.provider_joined.iter()).cloned().collect());

    // Liveness watches for only-live rules (their room ids looked up once).
    let wanted_refs: BTreeSet<String> = if active {
        compiled.only_live_chats.iter().chain(compiled.live_watch.iter()).map(|l| l.to_lowercase()).collect()
    } else {
        BTreeSet::new()
    };
    for login in st.refs.difference(&wanted_refs).cloned().collect::<Vec<_>>() {
        engine.local.unwatch_live(&login).await;
        st.refs.remove(&login);
    }
    for login in wanted_refs.difference(&st.refs).cloned().collect::<Vec<_>>() {
        match engine.outward.room_id(&login).await {
            Ok(room) => {
                engine.local.watch_live(&login, &room).await;
                st.refs.insert(login);
            }
            Err(e) => log::debug!("[Automations] no room id for {login}: {e}"),
        }
    }

    // P3: the own-channel topics the main account's rules need, handed to the
    // whisper session (which subscribes them while its socket is up). Nothing
    // is held while Off or in the background; the held text of each rule is
    // read back from the session's state.
    let main = engine.main_id();
    let want = if active { own_topics::wanted(&compiled, main.as_deref()) } else { None };
    if want != st.own {
        match &want {
            Some((owner, kinds)) => engine.local.hold_own_topics(owner, kinds).await,
            None => {
                if let Some((owner, _)) = &st.own {
                    engine.local.release_own_topics(owner).await;
                }
            }
        }
        st.own = want;
    }
    engine.set_own_held(if active {
        own_topics::rule_holds(&compiled, main.as_deref(), &engine.local.own_topic_state())
    } else {
        std::collections::HashMap::new()
    });
    // P3: the Highlight actions of the running Message rules.
    crate::services::chat_rules::set_automation_highlights(if active { compiled.highlights.clone() } else { Vec::new() });

    // The badge batcher's clock and the gallery.
    if active && !compiled.badge_rules.is_empty() {
        if st.badge_task.is_none() {
            st.badge_task = Some(spawn_batcher(engine.clone()));
        }
        if st.gallery_next.is_none_or(|at| now >= at) {
            st.gallery_next = Some(match engine.local.refresh_gallery().await {
                Ok(()) => now + engine.timings.gallery_refresh,
                Err(e) => {
                    log::debug!("[Automations] gallery refresh failed: {e}");
                    now + engine.timings.gallery_retry
                }
            });
        }
    } else {
        if let Some(task) = st.badge_task.take() {
            task.abort();
        }
        st.gallery_next = None;
    }

    engine.announce_status();
}

/// Keep the other platforms' chats the running rules listen in (and the ones
/// a closing window left to the engine): open the missing, let go of the
/// surplus. Twitch's rooms count first against the 50; a chat that fails to
/// open is tried again on the next pass (passes follow events, not a timer).
async fn provider_pass(engine: &Arc<Engine>, st: &mut State, compiled: &Compiled, active: bool) {
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    if active {
        wanted.extend(compiled.needed_provider_chats.iter().cloned());
        wanted.extend(lock(&engine.rec).adopted.keys().filter(|room| room.contains(':')).cloned());
    }
    let room = MAX_ENGINE_CHATS.saturating_sub(st.joined.len());
    let wanted: BTreeSet<String> = wanted.into_iter().take(room).collect();
    for key in st.provider_joined.difference(&wanted).cloned().collect::<Vec<_>>() {
        if let Some((provider, channel)) = key.split_once(':') {
            engine.local.provider_leave(provider, channel).await;
        }
        st.provider_joined.remove(&key);
    }
    for key in wanted.difference(&st.provider_joined).cloned().collect::<Vec<_>>() {
        let Some((provider, channel)) = key.split_once(':') else { continue };
        match engine.local.provider_join(provider, channel).await {
            Ok(()) => {
                st.provider_joined.insert(key);
            }
            Err(e) => log::debug!("[Automations] join {key} failed: {e}"),
        }
    }
}

/// Adopt every chat of another platform that `label` holds and no other label
/// holds, as `adopt_rooms` does for Twitch's rooms.
async fn adopt_provider_rooms(engine: &Arc<Engine>, st: &mut State, label: &str) {
    for provider in [KICK, YOUTUBE, TIKTOK] {
        for channel in engine.local.provider_held_by(provider, label).await {
            if engine.local.provider_held_by_other_than(provider, &channel, label).await {
                continue;
            }
            let key = crate::services::providers::key::make_key(provider, &channel);
            if st.provider_joined.contains(&key) || engine.local.provider_join(provider, &channel).await.is_ok() {
                st.provider_joined.insert(key.clone());
                lock(&engine.rec).adopted.insert(key, now_ms());
            }
        }
    }
}

fn push_unique(list: &mut Vec<String>, room: &str) {
    let key = room.trim_start_matches('#').to_lowercase();
    if !key.is_empty() && !list.contains(&key) {
        list.push(key);
    }
}

/// Claim `rooms`; when the shared connection is down, start it with the
/// first (never while recovery rebuilds it, and on the backoff).
async fn join_rooms(engine: &Arc<Engine>, st: &mut State, rooms: &[String], now: Instant) {
    let mut rest = rooms.iter();
    if !rooms.is_empty() && !engine.local.chat_connected().await {
        if engine.local.recovery_cycling() || !st.backoff.ready(now) {
            return;
        }
        let Some(first) = rest.next() else { return };
        engine.starting.store(true, Ordering::SeqCst);
        let started = engine.local.chat_start(first).await;
        engine.starting.store(false, Ordering::SeqCst);
        match started {
            Ok(()) => {
                st.backoff.reset();
                st.joined.insert(first.clone());
            }
            Err(e) => {
                log::debug!("[Automations] chat start failed: {e}");
                st.backoff.failed(now);
                return;
            }
        }
    }
    for room in rest {
        match engine.local.chat_join(room).await {
            Ok(()) => {
                st.joined.insert(room.clone());
            }
            Err(e) => log::debug!("[Automations] join {room} failed: {e}"),
        }
    }
}

/// Adopt every room `label` holds that no other label (besides the engine)
/// holds, stamping the adoption time.
async fn adopt_rooms(engine: &Arc<Engine>, st: &mut State, label: &str) {
    adopt_provider_rooms(engine, st, label).await;
    for room in engine.local.chats_held_by(label).await {
        let holders = engine.local.holders(&room).await;
        if holders.iter().any(|h| h != label && h != super::AUTOMATIONS_LABEL) {
            continue;
        }
        if st.joined.contains(&room) || engine.local.chat_join(&room).await.is_ok() {
            st.joined.insert(room.clone());
            lock(&engine.rec).adopted.insert(room, now_ms());
        }
    }
}

/// The batcher's clock: the open batch closes when its time is up.
fn spawn_batcher(engine: Arc<Engine>) -> AbortHandle {
    let task = engine.clone();
    engine.spawn_abortable(async move {
        loop {
            tokio::time::sleep(task.timings.batcher_tick).await;
            task.tick_batcher();
        }
    })
}

#[cfg(test)]
mod tests {
    use super::super::testkit::*;
    use super::*;

    #[tokio::test]
    async fn refreshes_a_b_a_end_with_a_and_leak_nothing() {
        let kit = Kit::new();
        let mut st = State::new(&kit.engine);
        kit.settings(RunState::On, vec![these_rule("ra", &["alpha"], true)]).await;
        pass(&kit.engine, &mut st).await;
        kit.settings(RunState::On, vec![these_rule("rb", &["beta"], true)]).await;
        pass(&kit.engine, &mut st).await;
        kit.settings(RunState::On, vec![these_rule("ra", &["alpha"], true)]).await;
        pass(&kit.engine, &mut st).await;
        assert_eq!(kit.local.joined(), set(&["alpha"]));
        assert_eq!(kit.local.watched(), set(&["alpha"]));
        assert_eq!(st.joined, set(&["alpha"]));
    }

    #[tokio::test]
    async fn a_claims_reset_joins_again() {
        let kit = Kit::new();
        let mut st = State::new(&kit.engine);
        kit.settings(RunState::On, vec![these_rule("ra", &["alpha"], false)]).await;
        pass(&kit.engine, &mut st).await;
        assert_eq!(kit.local.join_calls("alpha"), 1);
        kit.local.clear_claims();
        super::super::on_claims_reset_on(&kit.engine);
        pass(&kit.engine, &mut st).await;
        assert_eq!(kit.local.join_calls("alpha"), 2);
        assert!(kit.local.joined().contains("alpha"));
    }

    #[tokio::test]
    async fn a_claims_reset_during_its_own_start_is_ignored() {
        let kit = Kit::new();
        kit.engine.starting.store(true, Ordering::SeqCst);
        super::super::on_claims_reset_on(&kit.engine);
        assert!(!lock(&kit.engine.rec).claims_reset);
        kit.engine.starting.store(false, Ordering::SeqCst);
        super::super::on_claims_reset_on(&kit.engine);
        assert!(lock(&kit.engine.rec).claims_reset);
    }

    #[tokio::test]
    async fn the_chat_start_uses_the_engine_label_and_backs_off() {
        let kit = Kit::new();
        kit.local.set_connected(false);
        kit.local.fail_starts(true);
        let mut st = State::new(&kit.engine);
        kit.settings(RunState::On, vec![these_rule("ra", &["alpha", "beta"], false)]).await;
        pass(&kit.engine, &mut st).await;
        pass(&kit.engine, &mut st).await;
        pass(&kit.engine, &mut st).await;
        assert_eq!(kit.local.start_calls(), 1, "one attempt per backoff step");
        // An account save resets the backoff.
        kit.engine.accounts_changed();
        pass(&kit.engine, &mut st).await;
        assert_eq!(kit.local.start_calls(), 2);
        // No start while recovery rebuilds the connection.
        kit.local.set_cycling(true);
        kit.engine.accounts_changed();
        pass(&kit.engine, &mut st).await;
        assert_eq!(kit.local.start_calls(), 2);
        kit.local.set_cycling(false);
        kit.local.fail_starts(false);
        kit.engine.accounts_changed();
        pass(&kit.engine, &mut st).await;
        assert_eq!(kit.local.start_calls(), 3);
        assert_eq!(kit.local.last_start_label(), Some(super::super::AUTOMATIONS_LABEL.to_string()));
        assert!(kit.local.joined().contains("alpha") && kit.local.joined().contains("beta"));
    }

    #[tokio::test]
    async fn a_stream_chat_joined_but_not_claimed_is_left_alone() {
        let kit = Kit::new();
        kit.local.hold("main", "stream");
        let mut st = State::new(&kit.engine);
        kit.settings(RunState::On, vec![these_rule("ra", &["alpha"], false)]).await;
        pass(&kit.engine, &mut st).await;
        assert!(kit.local.holders_now("stream").contains(&"main".to_string()));
        assert_eq!(kit.local.leave_calls("stream"), 0);
    }

    #[tokio::test]
    async fn off_and_the_background_let_go_of_everything() {
        let kit = Kit::new();
        let mut st = State::new(&kit.engine);
        kit.settings(RunState::On, vec![these_rule("ra", &["alpha"], true)]).await;
        pass(&kit.engine, &mut st).await;
        kit.settings(RunState::Off, vec![these_rule("ra", &["alpha"], true)]).await;
        pass(&kit.engine, &mut st).await;
        assert!(kit.local.joined().is_empty());
        assert!(kit.local.watched().is_empty());
        kit.settings(RunState::On, vec![these_rule("ra", &["alpha"], true)]).await;
        pass(&kit.engine, &mut st).await;
        kit.engine.presence.store(false, Ordering::SeqCst);
        pass(&kit.engine, &mut st).await;
        assert!(kit.local.joined().is_empty());
        assert!(kit.local.watched().is_empty());
    }

    #[tokio::test]
    async fn a_failed_recovery_rebuild_requests_a_pass_that_starts_chat() {
        let kit = Kit::new();
        kit.local.set_connected(false);
        kit.local.set_cycling(true);
        let mut st = State::new(&kit.engine);
        kit.settings(RunState::On, vec![these_rule("ra", &["alpha"], false)]).await;
        pass(&kit.engine, &mut st).await;
        assert_eq!(kit.local.start_calls(), 0);
        // Recovery's Flight::drop clears the flag and requests a refresh.
        kit.local.set_cycling(false);
        pass(&kit.engine, &mut st).await;
        assert_eq!(kit.local.start_calls(), 1);
    }

    #[tokio::test]
    async fn kick_chats_are_joined_on_their_platform_and_left_when_no_longer_wanted() {
        let kit = Kit::new();
        kit.connect_kick().await;
        let mut st = State::new(&kit.engine);
        kit.settings(RunState::On, vec![kick_these_rule("rk", &["alpha"])]).await;
        pass(&kit.engine, &mut st).await;
        assert!(kit.local.joined().contains("kick:alpha"));
        assert_eq!(kit.local.join_calls("kick:alpha"), 1);
        assert_eq!(kit.local.start_calls(), 0, "no Twitch connection is started for a Kick chat");
        assert!(st.provider_joined.contains("kick:alpha") && st.joined.is_empty());
        pass(&kit.engine, &mut st).await;
        assert_eq!(kit.local.join_calls("kick:alpha"), 1, "a second pass joins nothing again");
        kit.settings(RunState::On, vec![]).await;
        pass(&kit.engine, &mut st).await;
        assert!(kit.local.joined().is_empty());
        assert_eq!(kit.local.leave_calls("kick:alpha"), 1);
        assert!(st.provider_joined.is_empty());
    }

    #[tokio::test]
    async fn a_kick_rule_with_no_connected_account_joins_nothing() {
        let kit = Kit::new();
        let mut st = State::new(&kit.engine);
        kit.settings(RunState::On, vec![kick_these_rule("rk", &["alpha"])]).await;
        pass(&kit.engine, &mut st).await;
        assert!(kit.local.joined().is_empty(), "a held rule listens nowhere");
        kit.connect_kick().await;
        pass(&kit.engine, &mut st).await;
        assert!(kit.local.joined().contains("kick:alpha"));
    }

    #[tokio::test]
    async fn off_and_the_background_let_go_of_the_kick_chats_too() {
        let kit = Kit::new();
        kit.connect_kick().await;
        let mut st = State::new(&kit.engine);
        kit.settings(RunState::On, vec![kick_these_rule("rk", &["alpha"])]).await;
        pass(&kit.engine, &mut st).await;
        assert!(kit.local.joined().contains("kick:alpha"));
        kit.engine.presence.store(false, Ordering::SeqCst);
        pass(&kit.engine, &mut st).await;
        assert!(kit.local.joined().is_empty());
        kit.engine.presence.store(true, Ordering::SeqCst);
        pass(&kit.engine, &mut st).await;
        assert!(kit.local.joined().contains("kick:alpha"));
        kit.settings(RunState::Off, vec![kick_these_rule("rk", &["alpha"])]).await;
        pass(&kit.engine, &mut st).await;
        assert!(kit.local.joined().is_empty());
    }

    #[tokio::test]
    async fn twitch_rooms_and_other_platform_chats_share_the_fifty() {
        let kit = Kit::new();
        kit.connect_kick().await;
        let mut st = State::new(&kit.engine);
        let twitch: Vec<String> = (0..25).map(|i| format!("room{i}")).collect();
        let twitch_refs: Vec<&str> = twitch.iter().map(String::as_str).collect();
        let kick: Vec<String> = (0..30).map(|i| format!("chan{i}")).collect();
        let kick_refs: Vec<&str> = kick.iter().map(String::as_str).collect();
        kit.settings(RunState::On, vec![these_rule("rt", &twitch_refs, false), kick_these_rule("rk", &kick_refs)]).await;
        pass(&kit.engine, &mut st).await;
        assert_eq!(st.joined.len(), 25);
        assert_eq!(st.provider_joined.len(), 25, "the rest of the 50");
        assert_eq!(kit.local.joined().len(), 50);
    }

    #[tokio::test]
    async fn a_notify_rule_on_youtube_joins_its_chat_on_that_platform() {
        let kit = Kit::new();
        kit.outward.set_platform_identity("youtube", "UCme", "me");
        let mut rule = kick_these_rule("ry", &["alpha"]);
        rule["send_as"] = serde_json::json!({ "provider": "twitch", "id": main_id(), "login": "me", "name": "Me" });
        rule["only_if"]["platforms"] = serde_json::json!(["youtube"]);
        rule["only_if"]["channels"] =
            serde_json::json!({ "kind": "these", "list": [{ "id": "c-1", "key": "youtube:UCchannel", "room_id": "", "name": "UCchannel" }] });
        rule["actions"] = serde_json::json!([{ "kind": "notify", "id": "a-1", "title": "T", "body": "B", "toast": true, "system": false, "sound": null, "volume": 1.0 }]);
        let mut st = State::new(&kit.engine);
        kit.settings(RunState::On, vec![rule]).await;
        pass(&kit.engine, &mut st).await;
        assert_eq!(kit.local.join_calls("youtube:uchannel"), 1, "joined through the YouTube adapter (it keys lowercase)");
        assert!(st.provider_joined.contains("youtube:UCchannel"));
        assert_eq!(kit.local.start_calls(), 0);
    }

    #[tokio::test]
    async fn a_closing_window_hands_over_a_kick_chat_nobody_else_holds() {
        let kit = Kit::new();
        kit.connect_kick().await;
        kit.settings(RunState::On, vec![kick_any_open_rule("rk")]).await;
        kit.local.hold("main", "kick:solo");
        kit.local.hold("main", "kick:shared");
        kit.local.hold("multichat-1", "kick:shared");
        let rec = kit.spawn_reconcile();
        super::super::adopt_window_chats_on(&kit.engine, "main").await;
        assert!(kit.local.joined().contains("kick:solo"));
        assert!(!kit.local.joined().contains("kick:shared"), "another window holds it");
        assert!(lock(&kit.engine.rec).adopted.contains_key("kick:solo"));
        rec.abort();
    }

    #[test]
    fn backoff_doubles_to_its_cap_and_resets() {
        let t0 = Instant::now();
        let mut b = Backoff::new(Duration::from_secs(5), Duration::from_secs(300));
        assert!(b.ready(t0));
        let mut waits = Vec::new();
        for _ in 0..8 {
            b.failed(t0);
            waits.push(b.next.unwrap() - t0);
        }
        assert_eq!(waits[0], Duration::from_secs(5));
        assert_eq!(waits[1], Duration::from_secs(10));
        assert_eq!(*waits.last().unwrap(), Duration::from_secs(300));
        b.reset();
        assert!(b.ready(t0));
    }
}

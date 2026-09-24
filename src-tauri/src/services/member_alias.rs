//! Which StreamNook member, if any, is behind a Kick or YouTube chat identity.
//!
//! Every StreamNook cosmetic is filed under a Twitch user id. Chat identifies
//! people per platform. This owns the translation between the two, for the whole
//! app: one cache, one network path, shared by the main window, every MultiChat
//! popout and the phone, rather than each webview resolving for itself.
//!
//! **It asks about the chatters it has actually seen, never for the whole
//! mapping.** A file listing every claim would be a machine-joinable
//! Twitch/Kick/YouTube identity graph for the entire membership, which is a far
//! stronger thing to hand out than "this person wears our badge". The endpoint is
//! shaped to refuse that, and so is this.
//!
//! A NEGATIVE answer is cached like any other. Most chatters are not members, so
//! if "nobody" were the uncached path this would re-ask for almost every chatter
//! on every platform, forever. Negatives are dropped only when something real
//! changes: the local user linking or unlinking an account.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const API_BASE: &str = "https://streamnook.app";

/// Matches the server's cap, so a batch is never silently truncated.
const MAX_KEYS_PER_REQUEST: usize = 200;

/// How long a POSITIVE answer is trusted. A claim changes when somebody connects
/// or disconnects an account, which is rare, and the local user's own changes
/// invalidate directly rather than waiting this out.
const POSITIVE_TTL: Duration = Duration::from_secs(900);

/// Upper bound on tracked keys. Sized against the chat user map (8000 on
/// desktop) so a long session in a busy channel cannot grow this without limit.
const MAX_ENTRIES: usize = 8000;

/// The whole point of a circuit breaker here: this endpoint shares a daily
/// request quota with checkout and Twitch sign-in, so a bug in the caching above
/// must not be able to spend it. After this many consecutive failures the
/// service stops asking until the cooldown passes.
const FAILURE_LIMIT: u32 = 5;
const BREAKER_COOLDOWN: Duration = Duration::from_secs(300);

struct Entry {
    member_id: Option<String>,
    stored_at: Instant,
}

struct State {
    entries: HashMap<String, Entry>,
    consecutive_failures: u32,
    breaker_opened_at: Option<Instant>,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| {
        Mutex::new(State {
            entries: HashMap::new(),
            consecutive_failures: 0,
            breaker_opened_at: None,
        })
    })
}

#[derive(Deserialize)]
struct ResolveResponse {
    #[serde(default)]
    resolved: HashMap<String, String>,
}

/// A chat key this service can answer for. Twitch needs no translation, and
/// anything malformed is refused here rather than becoming a request.
fn is_resolvable(key: &str) -> bool {
    let Some((provider, id)) = key.split_once(':') else {
        return false; // bare = Twitch, already a member id
    };
    if provider != "kick" && provider != "youtube" {
        return false;
    }
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn breaker_is_open(st: &mut State) -> bool {
    match st.breaker_opened_at {
        Some(at) if at.elapsed() < BREAKER_COOLDOWN => true,
        Some(_) => {
            // Cooled off: let exactly one attempt through and judge by it.
            st.breaker_opened_at = None;
            st.consecutive_failures = 0;
            false
        }
        None => false,
    }
}

/// Drop the oldest entries once the map is over its bound. Cheap and rare: this
/// only runs on a batch that actually grew the map past the cap.
fn evict_if_needed(entries: &mut HashMap<String, Entry>) {
    if entries.len() <= MAX_ENTRIES {
        return;
    }
    let mut by_age: Vec<(String, Instant)> = entries
        .iter()
        .map(|(k, e)| (k.clone(), e.stored_at))
        .collect();
    by_age.sort_by_key(|(_, at)| *at);
    let overflow = entries.len() - MAX_ENTRIES;
    for (key, _) in by_age.into_iter().take(overflow) {
        entries.remove(&key);
    }
}

/// What a resolve call could and could not answer.
///
/// "No claim" and "could not find out" are different answers and must never
/// share a representation. A key absent from `resolved` and absent from
/// `unresolved` has no claim, and the caller may stop asking. A key in
/// `unresolved` was not answered at all — the request failed, the breaker is
/// open, or the batch was over the cap — and the caller must ask again. Folding
/// the second into the first is how a single network blip would leave members
/// looking like strangers for the rest of a session.
#[derive(serde::Serialize, Default, Debug, PartialEq)]
pub struct ResolveOutcome {
    pub resolved: HashMap<String, String>,
    pub unresolved: Vec<String>,
}

/// Resolve a batch of chat keys to the members behind them.
///
/// Answers from cache where it can and asks about the rest in one request.
pub async fn resolve(keys: Vec<String>) -> ResolveOutcome {
    let mut out = ResolveOutcome::default();
    let mut unknown: Vec<String> = Vec::new();

    {
        let Ok(mut st) = state().lock() else {
            out.unresolved = keys.into_iter().filter(|k| is_resolvable(k)).collect();
            return out;
        };
        for key in keys {
            if !is_resolvable(&key) {
                continue;
            }
            match st.entries.get(&key) {
                Some(e) if e.member_id.is_none() => {
                    // A cached "nobody". Held until something real changes.
                }
                Some(e) if e.stored_at.elapsed() < POSITIVE_TTL => {
                    if let Some(id) = &e.member_id {
                        out.resolved.insert(key, id.clone());
                    }
                }
                _ => unknown.push(key),
            }
        }
        unknown.sort();
        unknown.dedup();
        // Over the server's cap: not asked this time, so not answered.
        if unknown.len() > MAX_KEYS_PER_REQUEST {
            out.unresolved.extend(unknown.split_off(MAX_KEYS_PER_REQUEST));
        }
        if unknown.is_empty() {
            return out;
        }
        if breaker_is_open(&mut st) {
            out.unresolved.extend(unknown);
            return out;
        }
    }

    let url = format!(
        "{API_BASE}/api/v1/cosmetics/resolve?keys={}",
        unknown
            .iter()
            .map(|k| urlencoding::encode(k).into_owned())
            .collect::<Vec<_>>()
            .join(",")
    );

    let fetched = match crate::services::http::client()
        .get(&url)
        .timeout(Duration::from_secs(8))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => resp.json::<ResolveResponse>().await.ok(),
        Ok(resp) => {
            log::warn!("[MemberAlias] resolve HTTP {}", resp.status());
            None
        }
        Err(e) => {
            log::warn!("[MemberAlias] resolve request failed: {e}");
            None
        }
    };

    let Ok(mut st) = state().lock() else {
        out.unresolved.extend(unknown);
        return out;
    };

    let Some(body) = fetched else {
        st.consecutive_failures += 1;
        if st.consecutive_failures >= FAILURE_LIMIT && st.breaker_opened_at.is_none() {
            st.breaker_opened_at = Some(Instant::now());
            log::warn!(
                "[MemberAlias] {FAILURE_LIMIT} consecutive failures; pausing lookups for {}s",
                BREAKER_COOLDOWN.as_secs()
            );
        }
        // Nothing cached, and every key reported back as unanswered: a failure is
        // not evidence that somebody is not a member.
        out.unresolved.extend(unknown);
        return out;
    };

    st.consecutive_failures = 0;
    let now = Instant::now();
    for key in unknown {
        let member_id = body.resolved.get(&key).cloned();
        if let Some(id) = &member_id {
            out.resolved.insert(key.clone(), id.clone());
        }
        st.entries.insert(
            key,
            Entry {
                member_id,
                stored_at: now,
            },
        );
    }
    evict_if_needed(&mut st.entries);
    out
}

/// Forget everything, so the next sighting of each chatter re-asks.
///
/// Called when the local user links or unlinks an account: their own claim just
/// changed, and a negative cached a moment ago would otherwise keep their badge
/// off their own messages until the app restarted.
pub fn invalidate() {
    if let Ok(mut st) = state().lock() {
        st.entries.clear();
        st.consecutive_failures = 0;
        st.breaker_opened_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_non_twitch_platforms_are_resolvable() {
        // A bare id is already a Twitch member id and needs no translation.
        assert!(!is_resolvable("249031143"));
        assert!(is_resolvable("kick:12345"));
        assert!(is_resolvable("youtube:UCabcDEF_-123"));
        // 7TV has no TikTok platform and neither do we.
        assert!(!is_resolvable("tiktok:12345"));
        assert!(!is_resolvable("twitch:249031143"));
    }

    #[test]
    fn malformed_keys_never_become_requests() {
        assert!(!is_resolvable("kick:"));
        assert!(!is_resolvable("kick:has space"));
        assert!(!is_resolvable("kick:has,comma"));
        // Bounded: keys become cache entries on both sides of the wire.
        assert!(!is_resolvable(&format!("kick:{}", "a".repeat(65))));
        assert!(is_resolvable(&format!("kick:{}", "a".repeat(64))));
    }

    #[test]
    fn youtube_ids_keep_their_case() {
        // UCabc and UCABC are different channels; nothing here may fold them.
        assert!(is_resolvable("youtube:UCabc"));
        assert!(is_resolvable("youtube:UCABC"));
    }

    #[tokio::test]
    async fn an_open_breaker_reports_keys_as_unresolved_not_absent() {
        // Trip the breaker directly rather than through a real failing request.
        {
            let mut st = state().lock().unwrap();
            st.entries.clear();
            st.breaker_opened_at = Some(Instant::now());
        }
        let out = resolve(vec!["kick:123".into(), "youtube:UCabc".into(), "249031143".into()]).await;
        // Not answered is not the same as no claim: both come back to be asked again.
        let mut unresolved = out.unresolved.clone();
        unresolved.sort();
        assert_eq!(unresolved, vec!["kick:123".to_string(), "youtube:UCabc".to_string()]);
        assert!(out.resolved.is_empty());
        // The bare Twitch id was never resolvable, so it is in neither list.
        assert!(!out.unresolved.contains(&"249031143".to_string()));
        invalidate();
    }

    #[test]
    fn eviction_keeps_the_newest() {
        let mut entries: HashMap<String, Entry> = HashMap::new();
        let base = Instant::now();
        for i in 0..(MAX_ENTRIES + 10) {
            entries.insert(
                format!("kick:{i}"),
                Entry {
                    member_id: None,
                    stored_at: base + Duration::from_millis(i as u64),
                },
            );
        }
        evict_if_needed(&mut entries);
        assert_eq!(entries.len(), MAX_ENTRIES);
        // The ten oldest went; the newest stayed.
        assert!(!entries.contains_key("kick:0"));
        assert!(entries.contains_key(&format!("kick:{}", MAX_ENTRIES + 9)));
    }
}

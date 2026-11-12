use super::*;
use crate::services::twitch_smiley_style::{begin_test, test_wear, TEST_LOCK};
use crate::services::twitch_smilies::catalog_rows;

impl EmoteService {
    /// Caches `set` under `key` as it is.
    pub(crate) async fn test_cache_set(&self, key: &str, set: EmoteSet) {
        self.cache.write().await.put(
            key.to_string(),
            CachedEmoteSet {
                set,
                timestamp: SystemTime::now(),
                seven_tv_ok: true,
            },
        );
    }

    /// The Twitch emote ids cached under `key`.
    pub(crate) async fn test_cached_ids(&self, key: &str) -> Vec<String> {
        self.cache
            .read()
            .await
            .peek(key)
            .map(|c| c.set.twitch.iter().map(|e| e.id.clone()).collect())
            .unwrap_or_default()
    }
}

fn kappa() -> Emote {
    let mut e = crate::services::twitch_smilies::face_row("25", "Kappa");
    e.emote_type = Some("globals".to_string());
    e
}

fn robot_set() -> EmoteSet {
    let mut set = EmoteSet::new();
    set.twitch = catalog_rows("robots").unwrap();
    set.twitch.push(kappa());
    set
}

fn sorted_ids(list: &[Emote]) -> Vec<String> {
    let mut ids: Vec<String> = list.iter().map(|e| e.id.clone()).collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn a_cached_put_after_a_switch_wears_the_switched_set() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    // A fetch began while the account wore the robots; the switch lands before it caches.
    test_wear("42", "monkeys");
    let service = EmoteService::new();
    let mut fetched = robot_set();
    {
        let mut cache = service.cache.write().await;
        put_dressed(
            &mut cache,
            "c1".to_string(),
            &mut fetched,
            SystemTime::now(),
            true,
        );
    }
    let cached = service.test_cached_ids("c1").await;
    assert!(
        cached.contains(&"499".to_string()) && cached.contains(&"483".to_string()),
        "{cached:?}"
    );
    assert!(
        !cached.contains(&"1".to_string()) && !cached.contains(&"9".to_string()),
        "{cached:?}"
    );
    assert!(cached.contains(&"25".to_string()), "Kappa stays");
    // The set handed back to the caller is the set that was cached.
    let mut cached_sorted = cached;
    cached_sorted.sort();
    assert_eq!(sorted_ids(&fetched.twitch), cached_sorted);
}

#[tokio::test]
async fn cached_sets_follow_a_later_switch() {
    let _lock = TEST_LOCK.lock().await;
    let _t = begin_test();
    let service = EmoteService::new();
    service.test_cache_set("c1", robot_set()).await;
    service.normalize_cached_smilies().await;
    assert!(
        service
            .test_cached_ids("c1")
            .await
            .contains(&"1".to_string()),
        "no style is known yet"
    );
    test_wear("42", "purple");
    service.normalize_cached_smilies().await;
    let cached = service.test_cached_ids("c1").await;
    assert!(
        cached.contains(&"440".to_string()) && !cached.contains(&"1".to_string()),
        "{cached:?}"
    );
}

#[test]
fn cached_user_id_follows_the_token() {
    let cache = Some(("token-a".to_string(), "100".to_string()));
    assert_eq!(cached_id_for(&cache, "token-a").as_deref(), Some("100"));
    assert_eq!(
        cached_id_for(&cache, "token-b"),
        None,
        "another account's token never borrows the id"
    );
    assert_eq!(cached_id_for(&None, "token-a"), None);
}

#[test]
fn helix_refusals_are_typed() {
    assert_eq!(helix_failure_outcome(401), HelixOutcome::Refused);
    assert_eq!(helix_failure_outcome(403), HelixOutcome::Refused);
    for status in [400, 404, 429, 500, 503] {
        assert_eq!(
            helix_failure_outcome(status),
            HelixOutcome::Failed,
            "{status}"
        );
    }
}

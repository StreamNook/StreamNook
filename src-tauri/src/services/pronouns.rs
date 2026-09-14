//! Pronouns from the community service Chatterino uses (pronouns.alejo.io,
//! v1 API). Opt-in (`user_card.show_pronouns`); one small GET per unique
//! login, cached for six hours including misses, bounded to 2000 entries.
//! The id-to-label map ("hehim" -> "he/him") is fetched once a day.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const USER_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const MAP_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const USER_CAP: usize = 2000;
const BASE: &str = "https://api.pronouns.alejo.io/v1";

#[derive(Deserialize)]
struct PronounDef {
    subject: String,
    #[serde(default)]
    object: String,
    #[serde(default)]
    singular: bool,
}

#[derive(Deserialize)]
struct UserPronouns {
    #[serde(default)]
    pronoun_id: Option<String>,
    #[serde(default)]
    alt_pronoun_id: Option<String>,
}

struct Cache {
    map: HashMap<String, String>,
    map_fetched: Option<Instant>,
    users: HashMap<String, (Instant, Option<String>)>,
}

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

fn cache() -> &'static Mutex<Cache> {
    CACHE.get_or_init(|| {
        Mutex::new(Cache {
            map: HashMap::new(),
            map_fetched: None,
            users: HashMap::new(),
        })
    })
}

fn label_for(map: &HashMap<String, String>, id: &str) -> Option<String> {
    map.get(id).cloned()
}

async fn ensure_map() {
    let stale = cache()
        .lock()
        .map(|c| c.map_fetched.map(|t| t.elapsed() > MAP_TTL).unwrap_or(true))
        .unwrap_or(true);
    if !stale {
        return;
    }
    let client = crate::services::http::client().clone();
    let Ok(resp) = client
        .get(format!("{}/pronouns", BASE))
        .header("User-Agent", format!("StreamNook/{}", env!("CARGO_PKG_VERSION")))
        .send()
        .await
    else {
        return;
    };
    let Ok(defs) = resp.json::<HashMap<String, PronounDef>>().await else {
        return;
    };
    let mut map = HashMap::with_capacity(defs.len());
    for (id, d) in defs {
        let label = if d.singular || d.object.is_empty() {
            d.subject
        } else {
            format!("{}/{}", d.subject, d.object)
        };
        map.insert(id, label);
    }
    if let Ok(mut c) = cache().lock() {
        c.map = map;
        c.map_fetched = Some(Instant::now());
    }
}

pub struct Pronouns;

impl Pronouns {
    /// Display label ("she/her", "he/they") or None when the user set none.
    pub async fn for_login(login: &str) -> Option<String> {
        let key = login.trim().to_lowercase();
        if key.is_empty() {
            return None;
        }
        if let Ok(c) = cache().lock() {
            if let Some((at, val)) = c.users.get(&key) {
                if at.elapsed() < USER_TTL {
                    return val.clone();
                }
            }
        }
        ensure_map().await;
        let client = crate::services::http::client().clone();
        let fetched: Option<String> = match client
            .get(format!("{}/users/{}", BASE, key))
            .header("User-Agent", format!("StreamNook/{}", env!("CARGO_PKG_VERSION")))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let user = resp.json::<UserPronouns>().await.ok();
                let c = cache().lock().ok();
                match (user, c) {
                    (Some(u), Some(c)) => {
                        let primary = u.pronoun_id.as_deref().and_then(|id| label_for(&c.map, id));
                        let alt = u.alt_pronoun_id.as_deref().and_then(|id| label_for(&c.map, id));
                        match (primary, alt) {
                            (Some(p), Some(a)) => {
                                // "he/him" + "they/them" -> "he/they", the site's own shorthand.
                                let ps = p.split('/').next().unwrap_or(&p).to_string();
                                let as_ = a.split('/').next().unwrap_or(&a).to_string();
                                Some(format!("{}/{}", ps, as_))
                            }
                            (Some(p), None) => Some(p),
                            _ => None,
                        }
                    }
                    _ => None,
                }
            }
            _ => None, // 404 (no pronouns set) and network errors both cache as none
        };
        if let Ok(mut c) = cache().lock() {
            if c.users.len() >= USER_CAP {
                // Drop the oldest quarter; cheap and rare.
                let mut entries: Vec<(String, Instant)> = c.users.iter().map(|(k, (t, _))| (k.clone(), *t)).collect();
                entries.sort_by_key(|(_, t)| *t);
                for (k, _) in entries.into_iter().take(USER_CAP / 4) {
                    c.users.remove(&k);
                }
            }
            c.users.insert(key, (Instant::now(), fetched.clone()));
        }
        fetched
    }
}

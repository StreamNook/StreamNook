//! The formats credentials had before the vault, read only to migrate them.

use cookie_store::CookieStore;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Every credential file, by name or by prefix for the per-account ones, and
/// the fixed XOR key it was obfuscated with before the vault.
const XOR_KEYS: &[(&str, &[u8])] = &[
    (".twitch_token", b"StreamNookTokenKey2024"),
    (".twitch_account_", b"StreamNookTokenKey2024"),
    (".twitch_drops_token", b"StreamNookDropsKey2024"),
    // Also the per-account `.seventv_token_<twitch id>` files.
    (".seventv_token", b"StreamNook7TVKey2024"),
    (".modroom_token", b"StreamNookModRoomKey2026"),
    (".kick_token", b"StreamNookKickKey2026"),
    (".youtube_session", b"StreamNookYouTubeKey2026"),
    (".tiktok_session", b"StreamNookTikTokKey2026"),
];

/// The cookie-store files that mirrored the Twitch and Drops tokens, and the
/// credential file each one mirrored. The app set its own token cookies with no
/// lifetime and the store saved only persistent cookies, so these rarely held a
/// token on disk; a persistent `auth-token` cookie in one is still plaintext, so
/// it is imported and the file deleted.
pub(super) const COOKIE_JARS: &[(&str, &str)] = &[
    ("cookies.json", ".twitch_token"),
    ("cookies_drops.json", ".twitch_drops_token"),
];

/// The XOR key a credential file used, or `None` for a file that is not a
/// credential. Temp files a write left behind match their credential's entry.
pub(super) fn xor_key(file_name: &str) -> Option<&'static [u8]> {
    XOR_KEYS
        .iter()
        .find(|(prefix, _)| file_name.starts_with(prefix))
        .map(|(_, key)| *key)
}

pub(super) fn decode(raw: &[u8], key: &[u8]) -> Vec<u8> {
    raw.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}

/// The token a cookie mirror holds, as the JSON its credential file stores
/// (`access_token`, `refresh_token`, `expires_at`), or `None` without one.
pub(super) fn jar_token(path: &Path) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    let store = CookieStore::load_json(BufReader::new(file)).ok()?;
    let get = |name: &str| {
        store
            .get("twitch.tv", "/", name)
            .map(|c| c.value().to_string())
    };
    let access_token = get("auth-token").filter(|t| !t.is_empty())?;
    let token = serde_json::json!({
        "access_token": access_token,
        "refresh_token": get("refresh-token").unwrap_or_default(),
        "expires_at": get("token-expires-at")
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0),
    });
    serde_json::to_vec(&token).ok()
}

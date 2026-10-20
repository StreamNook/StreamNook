//! The watcher's request signature, as the notify Worker checks it.
//!
//! `X-SN-Watcher-Ts` is Unix seconds in decimal. `X-SN-Watcher-Sig` is the
//! lowercase hex of HMAC-SHA256 keyed with the secret's UTF-8 bytes (the hex
//! string itself, not decoded), over `ts + method + path + body` with no
//! separators: method uppercase, path without host or query, body the exact
//! bytes sent (empty for a GET). The Worker refuses a timestamp more than
//! 5 minutes off.

use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const TS_HEADER: &str = "X-SN-Watcher-Ts";
pub const SIG_HEADER: &str = "X-SN-Watcher-Sig";

/// The signature for one request.
pub fn signature(secret: &str, ts: u64, method: &str, path: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes a key of any length");
    mac.update(ts.to_string().as_bytes());
    mac.update(method.as_bytes());
    mac.update(path.as_bytes());
    mac.update(body);
    hex(&mac.finalize().into_bytes())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0xf) as usize] as char);
    }
    out
}

/// Seconds since the Unix epoch.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_a_reference_hmac() {
        // RFC 4231 test case 2 (key "Jefe"): the HMAC and hex encoding are right.
        let mut mac = Hmac::<Sha256>::new_from_slice(b"Jefe").unwrap();
        mac.update(b"what do ya want for nothing?");
        assert_eq!(
            hex(&mac.finalize().into_bytes()),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn signs_ts_method_path_and_body_in_that_order() {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"secret").unwrap();
        mac.update(b"1700000000POST/watcher/mentions[]");
        let expected = hex(&mac.finalize().into_bytes());
        assert_eq!(signature("secret", 1_700_000_000, "POST", "/watcher/mentions", b"[]"), expected);
        assert_ne!(signature("secret", 1_700_000_000, "GET", "/watcher/mentions", b"[]"), expected);
    }
}

//! A webview's own browsing data store, on every desktop platform.
//!
//! On Windows and Linux a webview's store is its `data_directory`: a WebView2
//! user data folder, a WebKitGTK context per folder. macOS ignores that folder,
//! and a webview there shares WebKit's one default store with every other, the
//! main window included, unless it is given a data store identifier: the macOS
//! way to a persistent store of its own, which WebKit has from macOS 14. On
//! macOS 11 to 13 a webview given one stays in the default store (wry falls
//! back), which is where every webview there lived anyway.
//!
//! Only for webviews that hold no account. A sign-in's pages stay in the default
//! store on macOS, where every Mac user's session already is; moving them would
//! sign everyone out once. `services::sign_in_profile` clears only the
//! platform's cookies there instead.

use sha2::{Digest, Sha256};

/// The identifier of the store that belongs to `name`, for
/// `WebviewWindowBuilder::data_store_identifier`, which only macOS reads.
///
/// The same name always gives the same store, across launches and installs. A
/// name must therefore never change: a new one starts an empty store and
/// strands the old, as renaming a profile folder would elsewhere.
pub fn own_store(name: &str) -> [u8; 16] {
    let digest = Sha256::digest(format!("streamnook webview store: {name}").as_bytes());
    let mut id = [0u8; 16];
    id.copy_from_slice(&digest[..16]);
    // Well formed as a name-based UUID (version 8, RFC 4122 variant). WebKit
    // takes any 16 bytes; this only keeps them a valid UUID.
    id[6] = (id[6] & 0x0f) | 0x80;
    id[8] = (id[8] & 0x3f) | 0x80;
    id
}

#[cfg(test)]
mod tests {
    use super::own_store;

    #[test]
    fn a_name_always_gives_the_same_store_and_another_name_another() {
        assert_eq!(own_store("kick-resolver"), own_store("kick-resolver"));
        assert_ne!(own_store("kick-resolver"), own_store("tiktok-feed"));
        assert_ne!(own_store("tiktok-feed"), own_store("youtube-potoken"));
    }

    /// Pinned: every Mac user's stores are found by these bytes, so a change to
    /// the derivation strands all of them at once.
    #[test]
    fn the_derivation_never_changes() {
        assert_eq!(
            own_store("kick-resolver"),
            [
                0x17, 0x01, 0xef, 0xc2, 0x7d, 0x5c, 0x8e, 0x4b, 0xa5, 0xb9, 0x1e, 0x24, 0x06,
                0xa4, 0x5a, 0x33
            ]
        );
    }

    #[test]
    fn the_identifier_is_a_well_formed_uuid() {
        let id = own_store("youtube-potoken");
        assert_eq!(id[6] >> 4, 8, "version 8");
        assert_eq!(id[8] >> 6, 0b10, "RFC 4122 variant");
    }
}

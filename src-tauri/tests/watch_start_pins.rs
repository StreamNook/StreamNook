//! Where a watch start takes the watch session (src/services/watch_start.rs).
//! Pinned by source, because the order is the whole rule: a start that awaited
//! after its last ticket check takes the session only while it is still the
//! latest, so it never supersedes a newer start.

const SRC: &str = include_str!("../src/services/watch_start.rs");

/// The body of the top-level function whose signature starts with `sig`.
fn body(sig: &str) -> String {
    let src = SRC.replace("\r\n", "\n");
    let start = src.find(sig).unwrap_or_else(|| panic!("{sig} not found"));
    let rest = &src[start..];
    rest[..rest.find("\n}\n").expect("function end")].to_string()
}

#[test]
fn the_session_is_taken_only_by_the_latest_start() {
    for sig in ["async fn follow(", "async fn follow_provider(", "pub async fn open_room("] {
        let b = body(sig);
        assert!(b.contains("watch_session::start_current("), "{sig} takes the session only while current");
        assert!(!b.contains("watch_session::start("), "{sig} never supersedes a newer start");
    }
}

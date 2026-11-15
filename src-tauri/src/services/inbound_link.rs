//! The one inbound-link type: what a `streamnook:` or streamnook.app link
//! means (`route`), the slot that holds one until the page takes it
//! (`PendingLink`), and the counter that orders links against notice clicks.
//!
//! Both deep-link loops in `lib.rs` match on `route`, store the result, and
//! tell the page with one event that carries the stored link's stamp. The
//! iPhone's parse-only door wraps `route` too, so the order lives in one place.
//!
//! Nothing here sends anything to Twitch, and no comment may spell a Twitch
//! sender call: the manifest scan reads comments too.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::{badge_link, category_link, share_link};

/// What a link opens. The wire shape is `{"kind":"watch","key":"xqc"}`,
/// `{"kind":"badge","setId":"clover","version":"3"}` or
/// `{"kind":"category","provider":"twitch","id":"509658"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum InboundLink {
    /// A stream, as `startStream` takes it: a Twitch login or `provider:channel`.
    Watch {
        key: String,
    },
    Badge {
        set_id: String,
        version: String,
    },
    Category {
        provider: String,
        id: String,
    },
}

/// A link taken from the slot. `at` is when it was stored, in milliseconds on
/// the device's boot clock (not the wall clock, which can step), the same
/// clock a phone's native shell stamps taps with. It is 0 where there is no
/// such clock. The slot's stamp is not part of this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TakenLink {
    pub link: InboundLink,
    pub at: u64,
}

struct Held {
    link: InboundLink,
    at: u64,
    stamp: u64,
}

/// The next number of a counter: 1, then 2, and so on. 0 never comes out, so
/// 0 can mean "nothing seen".
fn next_from(counter: &AtomicU64) -> u64 {
    counter.fetch_add(1, Ordering::SeqCst) + 1
}

/// Link stores and notice clicks share this counter, so "the click came after
/// the link" is a plain comparison of two numbers.
static ARRIVAL: AtomicU64 = AtomicU64::new(0);

/// The next arrival number. Starts at 1.
pub fn next_arrival() -> u64 {
    next_from(&ARRIVAL)
}

/// Milliseconds on the device's boot clock. Hosts without that clock, which
/// is every build for now, answer 0: the desktop ignores the time, and the
/// iPhone never stores a link.
pub fn boot_ms() -> u64 {
    0
}

/// Seconds and nanoseconds from a clock read, as whole milliseconds. A
/// negative reading is 0.
pub fn ms_from_timespec(sec: i64, nsec: i64) -> u64 {
    let ms = sec.saturating_mul(1000).saturating_add(nsec / 1_000_000);
    u64::try_from(ms).unwrap_or(0)
}

/// The one link waiting for the page, if any.
///
/// A new link replaces whatever is held: the newest link wins. Each stored
/// link is stamped from `next_arrival()` while the slot's lock is held, so two
/// stores can never leave the older stamp in the slot.
#[derive(Default)]
pub struct PendingLink(Mutex<Option<Held>>);

impl PendingLink {
    /// Hold `link`, replacing any held one. `at_ms` is the time it arrived
    /// (see `boot_ms`). Returns the stamp the link took, which the caller
    /// sends to the page.
    pub fn store(&self, link: InboundLink, at_ms: u64) -> u64 {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let stamp = next_arrival();
        *slot = Some(Held {
            link,
            at: at_ms,
            stamp,
        });
        stamp
    }

    /// Take the held link out of the slot.
    pub fn take(&self) -> Option<TakenLink> {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        slot.take().map(|held| TakenLink {
            link: held.link,
            at: held.at,
        })
    }

    /// The held link's stamp, leaving the link in place.
    pub fn peek_stamp(&self) -> Option<u64> {
        let slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        slot.as_ref().map(|held| held.stamp)
    }

    /// Clear the slot when the held link's stamp is lower than `seq`: a link
    /// that arrived before that moment is dropped, one that arrived after it
    /// stays. The compare happens under the same lock a store takes.
    pub fn drop_if_older(&self, seq: u64) {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|held| held.stamp < seq) {
            *slot = None;
        }
    }
}

/// Whether a `streamnook://category/...` or `streamnook://badge/...` link
/// carries a path segment. Such a link that no parser accepted is refused
/// outright: read as a stream it would open a channel named "category" or
/// "badge". Without a segment the host is a plain channel name, as it is for
/// released apps.
fn names_a_refused_segment(url: &url::Url) -> bool {
    url.scheme() == "streamnook"
        && url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("category") || host.eq_ignore_ascii_case("badge")
        })
        && url
            .path_segments()
            .is_some_and(|mut segments| segments.any(|s| !s.is_empty()))
}

/// What a link opens, or None when it opens nothing. A badge link wins, then a
/// category link, then a stream link, so `streamnook://category/509658` is a
/// category and not a channel named "category".
pub fn route(url: &url::Url) -> Option<InboundLink> {
    if let Some(badge) = badge_link::parse(url) {
        return Some(InboundLink::Badge {
            set_id: badge.set_id,
            version: badge.version,
        });
    }
    if let Some(category) = category_link::parse(url) {
        return Some(InboundLink::Category {
            provider: category.provider.to_string(),
            id: category.id,
        });
    }
    if names_a_refused_segment(url) {
        return None;
    }
    share_link::openable(url).map(|link| InboundLink::Watch {
        key: link.stream_key(),
    })
}

/// Held by every test that touches `ARRIVAL` or a slot: the counter is shared
/// by the whole test binary.
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::sync::{Arc, MutexGuard};

    const LINK_FIXTURE: &str = include_str!("../../tests/fixtures/inbound_link.json");

    fn lock() -> MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn fixture() -> Value {
        serde_json::from_str(LINK_FIXTURE).expect("the link fixture is JSON")
    }

    fn url(link: &str) -> url::Url {
        url::Url::parse(link).expect("test url parses")
    }

    fn watch(key: &str) -> InboundLink {
        InboundLink::Watch {
            key: key.to_string(),
        }
    }

    fn category(id: &str) -> InboundLink {
        InboundLink::Category {
            provider: "twitch".to_string(),
            id: id.to_string(),
        }
    }

    fn badge(set_id: &str, version: &str) -> InboundLink {
        InboundLink::Badge {
            set_id: set_id.to_string(),
            version: version.to_string(),
        }
    }

    /// The source above the test module, with LF line ends on any checkout.
    fn live_source() -> String {
        let all = include_str!("inbound_link.rs").replace("\r\n", "\n");
        all.split("#[cfg(test)]")
            .next()
            .expect("the file has a live part")
            .to_string()
    }

    fn method_body(src: &str, signature: &str) -> String {
        let start = src
            .find(signature)
            .unwrap_or_else(|| panic!("`{signature}` is in the source"));
        let rest = &src[start..];
        let end = rest
            .find("\n    }\n")
            .unwrap_or_else(|| panic!("`{signature}` ends"));
        rest[..end].to_string()
    }

    fn in_order(body: &str, needles: &[&str]) {
        let mut at = 0;
        for needle in needles {
            let found = body[at..]
                .find(needle)
                .unwrap_or_else(|| panic!("`{needle}` is missing or out of order"));
            at += found + needle.len();
        }
    }

    // route

    #[test]
    fn a_badge_link_routes_in_every_form() {
        // Fails if the badge parser is skipped or its result is not carried over.
        assert_eq!(
            route(&url("streamnook:///badge/clover")),
            Some(badge("clover", "1"))
        );
        assert_eq!(
            route(&url("streamnook:///badge/clover/3")),
            Some(badge("clover", "3"))
        );
        assert_eq!(
            route(&url("streamnook://badge/clover/2")),
            Some(badge("clover", "2"))
        );
        assert_eq!(
            route(&url("streamnook://BADGE/Clover")),
            Some(badge("clover", "1"))
        );
    }

    #[test]
    fn a_category_link_routes_in_every_form() {
        // Fails if the host form falls through to a stream named "category".
        assert_eq!(
            route(&url("streamnook:///category/509658")),
            Some(category("509658"))
        );
        assert_eq!(
            route(&url("streamnook://category/509658")),
            Some(category("509658"))
        );
        assert_eq!(
            route(&url("https://streamnook.app/c/509658/just-chatting?d=1114")),
            Some(category("509658"))
        );
    }

    #[test]
    fn a_stream_link_routes_to_its_key() {
        // Fails if the stream key is not the `startStream` key.
        assert_eq!(route(&url("streamnook://xqc")), Some(watch("xqc")));
        assert_eq!(route(&url("streamnook://watch/xqc")), Some(watch("xqc")));
        assert_eq!(
            route(&url("streamnook://watch/kick/xqc")),
            Some(watch("kick:xqc"))
        );
        assert_eq!(
            route(&url("https://streamnook.app/w/xqc?d=1114")),
            Some(watch("xqc"))
        );
    }

    #[test]
    fn a_bare_category_or_badge_host_stays_a_channel() {
        // Fails if released apps' reading of these two links changes.
        assert_eq!(
            route(&url("streamnook://category")),
            Some(watch("category"))
        );
        assert_eq!(route(&url("streamnook://badge")), Some(watch("badge")));
    }

    #[test]
    fn a_refused_segment_is_nothing_never_a_channel() {
        // Fails if a category or badge host form with a segment no parser accepts opens a channel.
        for refused in [
            "streamnook://category/abc",
            "streamnook://category/kick/1",
            "streamnook://category/1234567890123456",
            "streamnook://badge/!!!",
            "streamnook://badge/a/b/c",
            "streamnook:///category/abc",
            "streamnook:///category/kick/1",
            "https://example.com/c/509658",
            "streamnook://watch",
        ] {
            assert_eq!(route(&url(refused)), None, "{refused}");
        }
    }

    #[test]
    fn every_route_of_the_shared_fixture_holds() {
        // Fails if the fixture's url/link pairs drift from `route`: Swift's fake core answers from the same list.
        let fixture = fixture();
        let routes = fixture["routes"].as_array().expect("routes is a list");
        assert!(routes.len() >= 10, "the fixture lists routes");
        for entry in routes {
            let text = entry["url"].as_str().expect("a route has a url");
            let got = serde_json::to_value(route(&url(text))).unwrap();
            assert_eq!(got, entry["link"], "{text}");
        }
    }

    #[test]
    fn no_route_depends_on_a_build_target() {
        // Fails if a listed route names a platform that only some builds can watch: the list must mean the same everywhere.
        let fixture = fixture();
        for entry in fixture["routes"].as_array().expect("routes is a list") {
            let text = entry["url"].as_str().unwrap().to_ascii_lowercase();
            assert!(
                !text.contains("youtube") && !text.contains("tiktok"),
                "{text}"
            );
        }
    }

    // the wire shapes, against the one shared fixture

    #[test]
    fn the_links_match_the_shared_fixture() {
        // Fails if a kind, a field name or its case moves: the page and Swift read the same file.
        let fixture = fixture();
        let links = &fixture["inboundLink"];
        assert_eq!(serde_json::to_value(watch("xqc")).unwrap(), links["watch"]);
        assert_eq!(
            serde_json::to_value(watch("kick:xqc")).unwrap(),
            links["watchKick"]
        );
        assert_eq!(
            serde_json::to_value(badge("clover", "3")).unwrap(),
            links["badge"]
        );
        assert_eq!(
            serde_json::to_value(category("509658")).unwrap(),
            links["category"]
        );
        let cases = links.as_object().expect("inboundLink is an object");
        assert_eq!(cases.len(), 4);
        for (name, case) in cases {
            let read: InboundLink =
                serde_json::from_value(case.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(&serde_json::to_value(read).unwrap(), case, "{name}");
        }
    }

    #[test]
    fn a_taken_link_matches_the_shared_fixture() {
        // Fails if `at` or `link` is renamed, or the stamp leaks into the taken shape.
        let _guard = lock();
        let fixture = fixture();
        let slot = PendingLink::default();
        slot.store(watch("xqc"), 86_400_000);
        let taken = slot.take().expect("a link was held");
        assert_eq!(serde_json::to_value(&taken).unwrap(), fixture["taken"]);
        let read: TakenLink = serde_json::from_value(fixture["taken"].clone()).unwrap();
        assert_eq!(read, taken);
    }

    // the counter

    #[test]
    fn a_local_counter_starts_at_one() {
        // Fails if the counter hands out 0 or repeats a number.
        let counter = AtomicU64::new(0);
        assert_eq!(next_from(&counter), 1);
        assert_eq!(next_from(&counter), 2);
    }

    #[test]
    fn arrivals_only_go_up() {
        // Fails if two arrivals can share a number.
        let _guard = lock();
        let first = next_arrival();
        let second = next_arrival();
        assert!(first >= 1);
        assert_eq!(second, first + 1);
    }

    // the slot (stamps are compared relative to the first one a case takes, since other tests share the counter)

    #[test]
    fn a_store_returns_its_stamp_and_a_newer_store_has_a_higher_one() {
        // Fails if `store` returns a stale number or does not stamp from the shared counter.
        let _guard = lock();
        let slot = PendingLink::default();
        let s = slot.store(watch("a"), 1);
        assert!(s >= 1);
        assert_eq!(slot.store(watch("b"), 2), s + 1);
        assert_eq!(slot.peek_stamp(), Some(s + 1));
    }

    #[test]
    fn peeking_a_stamp_leaves_the_link_in_the_slot() {
        // Fails if `peek_stamp` takes the link, or answers on an empty slot.
        let _guard = lock();
        let slot = PendingLink::default();
        assert_eq!(slot.peek_stamp(), None);
        let s = slot.store(watch("a"), 1);
        assert_eq!(slot.peek_stamp(), Some(s));
        assert_eq!(slot.peek_stamp(), Some(s));
        assert!(slot.take().is_some());
        assert_eq!(slot.peek_stamp(), None);
    }

    #[test]
    fn a_new_link_replaces_the_held_one() {
        // Fails if the slot keeps the older link or queues both.
        let _guard = lock();
        let slot = PendingLink::default();
        slot.store(category("509658"), 10);
        slot.store(watch("xqc"), 20);
        assert_eq!(slot.take().map(|t| t.link), Some(watch("xqc")));
        assert_eq!(slot.take(), None);
        slot.store(category("509658"), 10);
        slot.store(badge("clover", "3"), 20);
        assert_eq!(slot.take().map(|t| t.link), Some(badge("clover", "3")));
    }

    #[test]
    fn taking_from_an_empty_slot_is_nothing() {
        // Fails if `take` invents a link.
        let _guard = lock();
        assert_eq!(PendingLink::default().take(), None);
    }

    #[test]
    fn a_taken_link_carries_the_time_it_was_given() {
        // Fails if `at` is read from a clock, or a replacing store keeps the old time.
        let _guard = lock();
        let slot = PendingLink::default();
        slot.store(watch("a"), 1500);
        assert_eq!(slot.take().map(|t| t.at), Some(1500));
        slot.store(watch("a"), 1500);
        slot.store(watch("b"), 2500);
        assert_eq!(slot.take().map(|t| t.at), Some(2500));
    }

    #[test]
    fn a_link_stored_before_a_click_is_dropped() {
        // Fails if `drop_if_older` keeps a link that arrived before the click.
        let _guard = lock();
        let slot = PendingLink::default();
        let s = slot.store(watch("a"), 1);
        let click = next_arrival();
        assert_eq!(click, s + 1);
        slot.drop_if_older(click);
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn a_link_stored_after_a_click_stays() {
        // Fails if `drop_if_older` clears a link that arrived after the click.
        let _guard = lock();
        let slot = PendingLink::default();
        let click = next_arrival();
        let s = slot.store(watch("a"), 1);
        assert_eq!(s, click + 1);
        slot.drop_if_older(click);
        assert_eq!(slot.take().map(|t| t.link), Some(watch("a")));
    }

    #[test]
    fn dropping_on_an_empty_slot_does_nothing() {
        // Fails if `drop_if_older` panics or stores something on an empty slot.
        let _guard = lock();
        let slot = PendingLink::default();
        slot.drop_if_older(u64::MAX);
        assert_eq!(slot.peek_stamp(), None);
    }

    #[test]
    fn a_page_that_heard_of_a_link_drops_it_and_not_the_next_one() {
        // Fails if the drop compares with `<=` where the page passes `seen + 1`: a link stored after the action must survive.
        let _guard = lock();
        let slot = PendingLink::default();
        let seen = slot.store(watch("a"), 1);
        slot.drop_if_older(seen.saturating_add(1));
        assert_eq!(slot.take(), None);
        let after = slot.store(watch("b"), 2);
        assert_eq!(after, seen + 1);
        slot.drop_if_older(seen.saturating_add(1));
        assert_eq!(slot.take().map(|t| t.link), Some(watch("b")));
    }

    #[test]
    fn two_stores_at_once_leave_the_higher_stamp() {
        // A smoke test: concurrent stores never panic and leave the higher stamp. The ordering itself
        // is proved by the source pin on the lock and by the step-by-step tests above.
        let _guard = lock();
        for _ in 0..200 {
            let slot = Arc::new(PendingLink::default());
            let a = {
                let slot = Arc::clone(&slot);
                std::thread::spawn(move || slot.store(watch("a"), 1))
            };
            let b = {
                let slot = Arc::clone(&slot);
                std::thread::spawn(move || slot.store(watch("b"), 2))
            };
            let (sa, sb) = (a.join().unwrap(), b.join().unwrap());
            assert_eq!(slot.peek_stamp(), Some(sa.max(sb)));
            let held = slot.take().expect("a link is held").link;
            assert_eq!(held, if sa > sb { watch("a") } else { watch("b") });
        }
    }

    #[test]
    fn a_drop_racing_a_store_never_clears_a_newer_link() {
        // A smoke test: a drop running beside a store never panics or clears the newer link. The
        // compare's order is proved by the source pin on the lock and by the step-by-step tests.
        let _guard = lock();
        for _ in 0..200 {
            let slot = Arc::new(PendingLink::default());
            let seq = next_arrival();
            let dropper = {
                let slot = Arc::clone(&slot);
                std::thread::spawn(move || slot.drop_if_older(seq))
            };
            let storer = {
                let slot = Arc::clone(&slot);
                std::thread::spawn(move || slot.store(watch("new"), 1))
            };
            dropper.join().unwrap();
            let stamp = storer.join().unwrap();
            assert!(stamp > seq);
            assert_eq!(slot.take().map(|t| t.link), Some(watch("new")));
        }
    }

    #[test]
    fn the_lock_is_taken_before_the_stamp_and_before_the_compare() {
        // Fails if `store` stamps outside the lock or `drop_if_older` compares outside it.
        let live = live_source();
        in_order(
            &method_body(&live, "pub fn store("),
            &[".lock(", "next_arrival()"],
        );
        in_order(
            &method_body(&live, "pub fn drop_if_older("),
            &[".lock(", "stamp < seq"],
        );
    }

    // the boot clock

    #[test]
    fn milliseconds_come_from_seconds_and_nanoseconds() {
        // Fails if the math is off by a unit or a negative reading wraps.
        assert_eq!(ms_from_timespec(0, 0), 0);
        assert_eq!(ms_from_timespec(86_400, 500_000_000), 86_400_500);
        assert_eq!(ms_from_timespec(1, 999_999_999), 1999);
        assert_eq!(ms_from_timespec(-1, 0), 0);
        assert_eq!(ms_from_timespec(-1, 999_000_000), 0);
        assert_eq!(ms_from_timespec(i64::MAX, 0), i64::MAX as u64);
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn a_host_without_the_boot_clock_answers_zero() {
        // Fails if a host build starts reporting a time nothing there can compare.
        assert_eq!(boot_ms(), 0);
    }

    // sender text

    #[test]
    fn this_file_sends_no_twitch_gql() {
        // Fails if a sender call or its name appears here, comments included: the manifest scan reads them.
        let live = live_source();
        assert!(live.contains("pub fn route("), "the live part was read");
        for sender in [
            concat!("gql.twitch", ".tv"),
            concat!("twitch_web_gql", "::post"),
            concat!("gql_query", "("),
            concat!("gql_public", "_read("),
            concat!("gql_web", "_query("),
        ] {
            assert!(
                !live.contains(sender),
                "`{sender}` is spelled in inbound_link.rs"
            );
        }
    }
}

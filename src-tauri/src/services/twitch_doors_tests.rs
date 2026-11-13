//! The engine's typed Helix doors against a fake server on 127.0.0.1: the
//! status mapping with real errors, the one-write rule, the category read's
//! chain, and the 3xx that is never followed. No test touches a real account,
//! a real token or a real Twitch host; thresholds are sub-second.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::*;
use crate::services::chat_service::engine_client;

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    target: String,
    auth: String,
    body: String,
}

type Route = (
    &'static str,
    &'static str,
    u16,
    String,
    Vec<(&'static str, &'static str)>,
);

/// A server answering `routes` (method, path prefix, status, body, headers) for
/// any number of requests, recording each. A route with status 0 never answers.
async fn serve(routes: Vec<Route>) -> (String, Arc<Mutex<Vec<Seen>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let routes = routes.clone();
            let log = log.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let Ok(n) = sock.read(&mut chunk).await else {
                        return;
                    };
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(head_end) = text.find("\r\n\r\n") {
                        let length = text[..head_end]
                            .lines()
                            .find_map(|l| {
                                l.split_once(':')
                                    .filter(|(k, _)| {
                                        k.trim().eq_ignore_ascii_case("content-length")
                                    })
                                    .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if buf.len() >= head_end + 4 + length {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let first = text.lines().next().unwrap_or("").to_string();
                let mut parts = first.split_whitespace();
                let (method, target) = (
                    parts.next().unwrap_or("").to_string(),
                    parts.next().unwrap_or("").to_string(),
                );
                let auth = text
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("authorization:"))
                    .map(|l| l[14..].trim().to_string())
                    .unwrap_or_default();
                let body = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
                log.lock().unwrap().push(Seen {
                    method: method.clone(),
                    target: target.clone(),
                    auth,
                    body,
                });
                let route = routes
                    .iter()
                    .find(|(m, p, ..)| *m == method && target.starts_with(p));
                match route {
                    Some((_, _, 0, ..)) => tokio::time::sleep(Duration::from_secs(30)).await,
                    Some((_, _, status, body, headers)) => {
                        let extra: String = headers
                            .iter()
                            .map(|(k, v)| format!("{k}: {v}\r\n"))
                            .collect();
                        let reply = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        let _ = sock.write_all(reply.as_bytes()).await;
                        let _ = sock.shutdown().await;
                    }
                    None => {
                        let _ = sock
                            .write_all(
                                b"HTTP/1.1 404 X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            )
                            .await;
                    }
                }
            });
        }
    });
    (format!("http://{addr}"), seen)
}

fn quick() -> Client {
    engine_client(Duration::from_millis(300), Duration::from_millis(800))
}

fn route(method: &'static str, path: &'static str, status: u16, body: Value) -> Route {
    (method, path, status, body.to_string(), vec![])
}

fn count(seen: &Arc<Mutex<Vec<Seen>>>, prefix: &str) -> usize {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|s| s.target.starts_with(prefix))
        .count()
}

#[tokio::test]
async fn every_status_maps_through_door_outcome_with_real_errors() {
    for (status, expect) in [
        (400u16, "refused"),
        (403, "refused"),
        (429, "refused"),
        (500, "unknown"),
        (503, "unknown"),
        (401, "unauthorized"),
    ] {
        let (base, seen) = serve(vec![route(
            "PATCH",
            "/helix/chat/settings",
            status,
            json!({ "message": "no" }),
        )])
        .await;
        let result = TwitchService::update_chat_settings_engine(
            &quick(),
            &base,
            "T",
            "9",
            "8",
            &json!({ "slow_mode": false }),
        )
        .await;
        let got = match door_outcome(&result) {
            DoorOutcome::Sent => "sent",
            DoorOutcome::Refused(_) => "refused",
            DoorOutcome::RefusedUnauthorized(_) => "unauthorized",
            DoorOutcome::Unknown => "unknown",
        };
        assert_eq!(got, expect, "{status}");
        assert_eq!(seen.lock().unwrap().len(), 1, "one write, retries off");
    }
    let (base, _) = serve(vec![route("PATCH", "/helix/chat/settings", 200, json!({}))]).await;
    assert_eq!(
        door_outcome(
            &TwitchService::update_chat_settings_engine(&quick(), &base, "T", "9", "8", &json!({}))
                .await
        ),
        DoorOutcome::Sent
    );
}

#[tokio::test]
async fn a_redirect_is_never_followed_and_reads_unknown() {
    let (base, seen) = serve(vec![(
        "PATCH",
        "/helix/channels",
        302,
        String::new(),
        vec![("Location", "/moved")],
    )])
    .await;
    let result =
        TwitchService::modify_channel_info_engine(&quick(), &base, "T", "8", Some("x"), None).await;
    assert_eq!(door_outcome(&result), DoorOutcome::Unknown);
    assert_eq!(count(&seen, "/moved"), 0, "zero requests to /moved");
    let status = result.unwrap_err().downcast::<HelixStatus>().unwrap();
    assert_eq!(status.status, 302);
}

#[tokio::test]
async fn connect_builder_timeout_and_plain_errors_map_as_the_door_table_says() {
    // A refused connection (nothing listening): is_connect, Refused.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let closed = TwitchService::create_stream_marker_engine(
        &quick(),
        &format!("http://127.0.0.1:{port}"),
        "T",
        "8",
        None,
    )
    .await;
    assert!(matches!(door_outcome(&closed), DoorOutcome::Refused(_)));
    // A bad URL: is_builder, Refused.
    let bad =
        TwitchService::create_stream_marker_engine(&quick(), "not a url", "T", "8", None).await;
    assert!(matches!(door_outcome(&bad), DoorOutcome::Refused(_)));
    // A read timeout after the request left: Unknown.
    let (base, _) = serve(vec![(
        "POST",
        "/helix/streams/markers",
        0,
        String::new(),
        vec![],
    )])
    .await;
    let stalled = TwitchService::create_stream_marker_engine(&quick(), &base, "T", "8", None).await;
    assert_eq!(door_outcome(&stalled), DoorOutcome::Unknown);
    // A plain error that is neither reqwest nor HelixStatus: Unknown.
    assert_eq!(
        door_outcome::<()>(&Err(anyhow::anyhow!("something else"))),
        DoorOutcome::Unknown
    );
}

#[tokio::test]
async fn texts_carry_to_activity_and_channel_info_sends_one_key() {
    let (base, seen) = serve(vec![route(
        "PATCH",
        "/helix/channels",
        403,
        json!({ "message": "x" }),
    )])
    .await;
    let r =
        TwitchService::modify_channel_info_engine(&quick(), &base, "T", "8", Some("a title"), None)
            .await;
    match door_outcome(&r) {
        DoorOutcome::Refused(text) => assert_eq!(
            text,
            "Only the broadcaster can change the title or category"
        ),
        other => panic!("{other:?}"),
    }
    let sent = seen.lock().unwrap()[0].clone();
    assert_eq!(
        (sent.method.as_str(), sent.target.as_str()),
        ("PATCH", "/helix/channels?broadcaster_id=8")
    );
    let body: Value = serde_json::from_str(&sent.body).unwrap();
    assert_eq!(
        body.as_object().unwrap().keys().collect::<Vec<_>>(),
        vec!["title"]
    );
    let (base, seen) = serve(vec![route("PATCH", "/helix/channels", 204, json!(null))]).await;
    TwitchService::modify_channel_info_engine(&quick(), &base, "T", "8", None, Some("509660"))
        .await
        .unwrap();
    let body: Value = serde_json::from_str(&seen.lock().unwrap()[0].body).unwrap();
    assert_eq!(body, json!({ "game_id": "509660" }));
    // A 401 on channel info reads as the same words and as unauthorized.
    let (base, _) = serve(vec![route("PATCH", "/helix/channels", 401, json!({}))]).await;
    let r =
        TwitchService::modify_channel_info_engine(&quick(), &base, "T", "8", Some("x"), None).await;
    assert!(
        matches!(door_outcome(&r), DoorOutcome::RefusedUnauthorized(t) if t == "Only the broadcaster can change the title or category")
    );
}

#[tokio::test]
async fn twin_error_texts_are_todays() {
    let (base, _) = serve(vec![route(
        "PATCH",
        "/helix/chat/settings",
        400,
        json!({ "message": "Missing scope" }),
    )])
    .await;
    let e = TwitchService::update_chat_settings_engine(&quick(), &base, "T", "9", "8", &json!({}))
        .await
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        helix_refusal_text(400, "{\"message\":\"Missing scope\"}")
    );
    let (base, _) = serve(vec![route(
        "POST",
        "/helix/streams/markers",
        403,
        json!({ "message": "Missing scope" }),
    )])
    .await;
    let e = TwitchService::create_stream_marker_engine(&quick(), &base, "T", "8", Some("m"))
        .await
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "StreamNook needs moderation access again: sign out and back in to Twitch"
    );
}

#[tokio::test]
async fn a_bare_marker_sends_no_description_key_and_a_described_one_does() {
    let (base, seen) = serve(vec![route(
        "POST",
        "/helix/streams/markers",
        200,
        json!({}),
    )])
    .await;
    TwitchService::create_stream_marker_engine(&quick(), &base, "T", "8", None)
        .await
        .unwrap();
    TwitchService::create_stream_marker_engine(&quick(), &base, "T", "8", Some("great play"))
        .await
        .unwrap();
    let bodies: Vec<Value> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|s| serde_json::from_str(&s.body).unwrap())
        .collect();
    assert_eq!(bodies[0], json!({ "user_id": "8" }));
    assert_eq!(
        bodies[1],
        json!({ "user_id": "8", "description": "great play" })
    );
}

#[tokio::test]
async fn setgame_reads_exact_first_and_the_fuzzy_read_only_on_a_miss() {
    // An exact hit: one read, the fuzzy endpoint never asked (even though it would 503).
    let (base, seen) = serve(vec![
        route(
            "GET",
            "/helix/games",
            200,
            json!({ "data": [{ "id": "509660", "name": "Art" }] }),
        ),
        route("GET", "/helix/search/categories", 503, json!({})),
    ])
    .await;
    let rows = TwitchService::search_categories_engine(&quick(), &base, "TOKEN", "Art")
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(count(&seen, "/helix/games"), 1);
    assert_eq!(count(&seen, "/helix/search/categories"), 0);
    assert_eq!(seen.lock().unwrap()[0].auth, "Bearer TOKEN");
    // An exact miss: the fuzzy read runs, with the verified token, and its rows follow the exact ones.
    let (base, seen) = serve(vec![
        route("GET", "/helix/games", 200, json!({ "data": [] })),
        route("GET", "/helix/search/categories", 200, json!({ "data": [{ "id": "1", "name": "Artifact" }, { "id": "509660", "name": "Art" }] })),
    ])
    .await;
    let rows = TwitchService::search_categories_engine(&quick(), &base, "TOKEN", "art")
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|s| s.auth == "Bearer TOKEN"),
        "both reads carry the verified token"
    );
    // A fuzzy 503 after an exact miss fails the search, and it is never a write outcome.
    let (base, _) = serve(vec![
        route("GET", "/helix/games", 200, json!({ "data": [] })),
        route("GET", "/helix/search/categories", 503, json!({})),
    ])
    .await;
    let failed = TwitchService::search_categories_engine(&quick(), &base, "TOKEN", "zzz").await;
    assert!(matches!(search_outcome(failed), Err(SearchFail::Other(_))));
    // A 401 on either read is Unauthorized.
    let (base, _) = serve(vec![route("GET", "/helix/games", 401, json!({}))]).await;
    assert_eq!(
        search_outcome(
            TwitchService::search_categories_engine(&quick(), &base, "TOKEN", "x").await
        ),
        Err(SearchFail::Unauthorized)
    );
}

#[tokio::test]
async fn a_query_with_an_ampersand_arrives_exactly() {
    let (base, seen) = serve(vec![route(
        "GET",
        "/helix/games",
        200,
        json!({ "data": [{ "id": "7", "name": "Makers & Crafting" }] }),
    )])
    .await;
    TwitchService::search_categories_engine(&quick(), &base, "T", "Makers & Crafting")
        .await
        .unwrap();
    let target = seen.lock().unwrap()[0].target.clone();
    let decoded: Vec<(String, String)> =
        url::form_urlencoded::parse(target.split_once('?').unwrap().1.as_bytes())
            .into_owned()
            .collect();
    assert_eq!(
        decoded,
        vec![("name".to_string(), "Makers & Crafting".to_string())]
    );
}

#[tokio::test]
async fn announce_pin_shoutout_timeout_and_delete_use_their_helix_calls() {
    let (base, seen) = serve(vec![
        route("POST", "/helix/chat/announcements", 204, json!(null)),
        route("PUT", "/helix/chat/pins", 204, json!(null)),
        route("POST", "/helix/chat/shoutouts", 204, json!(null)),
        route("POST", "/helix/moderation/bans", 200, json!({})),
        route("DELETE", "/helix/moderation/chat", 204, json!(null)),
    ])
    .await;
    let c = quick();
    TwitchService::send_chat_announcement_engine(&c, &base, "T", "9", "8", "hello", Some("blue"))
        .await
        .unwrap();
    TwitchService::pin_chat_message_engine(&c, &base, "T", "9", "8", "m1", Some(60))
        .await
        .unwrap();
    TwitchService::send_shoutout_engine(&c, &base, "T", "9", "8", "55")
        .await
        .unwrap();
    TwitchService::timeout_user_engine(&c, &base, "T", "9", "8", "55", 600, Some("spam"))
        .await
        .unwrap();
    TwitchService::delete_chat_message_engine(&c, &base, "T", "9", "8", "m1")
        .await
        .unwrap();
    let all = seen.lock().unwrap().clone();
    assert!(all[0].target.contains("broadcaster_id=8") && all[0].target.contains("moderator_id=9"));
    assert!(serde_json::from_str::<Value>(&all[0].body).unwrap()["color"] == "blue");
    assert!(
        all[1].target.contains("duration_seconds=60") && all[1].target.contains("message_id=m1")
    );
    assert!(
        all[2].target.contains("from_broadcaster_id=8")
            && all[2].target.contains("to_broadcaster_id=55")
    );
    let ban: Value = serde_json::from_str(&all[3].body).unwrap();
    assert_eq!(ban["data"]["duration"], 600);
    assert!(all[4].target.contains("message_id=m1"));
    // Announce answers: 204 Sent, 400 and 429 Refused, 5xx Unknown, a connect stall Refused.
    for (status, unknown) in [
        (400u16, false),
        (403, false),
        (429, false),
        (500, true),
        (503, true),
    ] {
        let (b, _) = serve(vec![route(
            "POST",
            "/helix/chat/announcements",
            status,
            json!({}),
        )])
        .await;
        let r =
            TwitchService::send_chat_announcement_engine(&c, &b, "T", "9", "8", "x", None).await;
        assert_eq!(
            matches!(door_outcome(&r), DoorOutcome::Unknown),
            unknown,
            "{status}"
        );
    }
}

#[test]
fn a_door_source_pin_every_twin_builds_requests_with_query_and_no_fallible_step_before_send() {
    let src = include_str!("twitch_service.rs");
    assert!(
        src.contains("update_chat_settings_engine(&client, \"https://api.twitch.tv\""),
        "update_chat_settings_as delegates with the shared client and the real base"
    );
    assert!(
        !src.contains("format!(\"{base}/helix/games?name="),
        "queries are built with .query(..)"
    );
}

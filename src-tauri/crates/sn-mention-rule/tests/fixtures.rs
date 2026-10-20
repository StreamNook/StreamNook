//! Runs `fixtures.json`: every case normalized, then judged, then compared.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use sn_mention_rule::judge::{judge, Identity, Input};
use sn_mention_rule::name::MatchOptions;
use sn_mention_rule::normalize::normalize;

#[derive(Deserialize)]
struct File {
    users: HashMap<String, User>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct User {
    id: String,
    login: String,
    display: String,
}

#[derive(Deserialize)]
struct Opts {
    bare_login: bool,
    display_name: bool,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    user: String,
    #[serde(default)]
    opts: Option<Opts>,
    #[serde(default)]
    hidden: Vec<String>,
    /// Sender: user id, login, display name.
    from: [String; 3],
    text: String,
    /// Reply parent: user id, login, display name.
    #[serde(default)]
    reply: Option<[String; 3]>,
    expect: Option<String>,
}

#[test]
fn every_fixture_judges_as_expected() {
    let raw = include_str!("fixtures.json");
    let file: File = serde_json::from_str(raw).expect("fixtures.json reads");
    assert!(file.cases.len() >= 40, "the fixture file keeps at least 40 cases");
    let mut failures = Vec::new();
    for case in &file.cases {
        let user = &file.users[&case.user];
        let identity = Identity::new(&user.id, &user.login, &user.display);
        let opts = case
            .opts
            .as_ref()
            .map(|o| MatchOptions {
                bare_login: o.bare_login,
                display_name: o.display_name,
            })
            .unwrap_or_default();
        let hidden: HashSet<String> = case.hidden.iter().map(|h| h.to_lowercase()).collect();
        let (parent_id, parent_login, parent_display) = match &case.reply {
            Some([id, login, display]) => (Some(id.as_str()), Some(login.as_str()), Some(display.as_str())),
            None => (None, None, None),
        };
        let content = normalize(&case.text, parent_display, parent_login);
        let input = Input {
            sender_id: &case.from[0],
            sender_login: &case.from[1],
            sender_display: &case.from[2],
            content: &content,
            reply_parent_user_id: parent_id,
        };
        let got = judge(&input, &identity, opts, &hidden).map(|k| k.as_str().to_string());
        if got != case.expect {
            failures.push(format!("{}: expected {:?}, got {:?} (normalized {:?})", case.name, case.expect, got, content));
        }
    }
    assert!(failures.is_empty(), "fixture failures:\n{}", failures.join("\n"));
}

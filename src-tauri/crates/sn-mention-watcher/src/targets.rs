//! Who is watched where: the Worker's answer to `GET /watcher/targets`.
//!
//! The Worker returns the opted-in users' live followed channels and the
//! offline chats they hold a lease on (muted ones already left out per user),
//! each marked live or offline, live ones first, and the rules for each user
//! found in them. It pages the answer
//! (`?page=`, `next` names the following page); the pages of one read make one
//! set. The watcher keeps the newest set and keeps using it while the Worker is
//! unreachable, so a Worker outage never parts every channel.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use sn_mention_rule::judge::Identity;
use sn_mention_rule::name::MatchOptions;

pub const PATH: &str = "/watcher/targets";

/// How many pages one read follows at most.
pub const MAX_PAGES: u32 = 100;

#[derive(Debug, Deserialize)]
pub struct Answer {
    #[serde(default)]
    pub channels: Vec<ChannelRow>,
    #[serde(default)]
    pub users: Vec<UserRow>,
    /// The page after this one; none on the last (and on a Worker that does not page).
    #[serde(default)]
    pub next: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ChannelRow {
    pub id: String,
    pub login: String,
    /// A Worker that listed live channels only did not say; they were live.
    #[serde(default = "yes")]
    pub live: bool,
    #[serde(default)]
    pub users: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UserRow {
    pub id: String,
    pub login: String,
    #[serde(default)]
    pub display: String,
    #[serde(default)]
    pub opts: OptsRow,
    #[serde(default)]
    pub hidden_user_ids: Vec<String>,
    #[serde(default)]
    pub muted_channel_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct OptsRow {
    #[serde(default = "yes")]
    pub bare: bool,
    #[serde(default = "yes")]
    pub display: bool,
}

fn yes() -> bool {
    true
}

impl Default for OptsRow {
    fn default() -> Self {
        OptsRow { bare: true, display: true }
    }
}

/// One person's rules, ready for `judge`.
#[derive(Debug, Clone)]
pub struct UserRules {
    pub identity: Identity,
    pub opts: MatchOptions,
    /// Hidden senders, by user id or lowercase login.
    pub hidden: HashSet<String>,
    pub muted: HashSet<String>,
}

/// A followed channel, whether it is live, and who watches it.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub login: String,
    pub live: bool,
    pub users: Vec<String>,
}

/// The watched set: channels by room id, users by Twitch user id.
#[derive(Debug, Clone, Default)]
pub struct LiveSet {
    pub channels: HashMap<String, Channel>,
    pub users: HashMap<String, UserRules>,
}

impl LiveSet {
    #[cfg(test)]
    pub fn from_answer(answer: Answer) -> LiveSet {
        LiveSet::from_answers(vec![answer])
    }

    /// One set from the pages of one read.
    pub fn from_answers(answers: Vec<Answer>) -> LiveSet {
        let mut channel_rows = Vec::new();
        let mut user_rows = Vec::new();
        for a in answers {
            channel_rows.extend(a.channels);
            user_rows.extend(a.users);
        }
        let users: HashMap<String, UserRules> = user_rows
            .into_iter()
            .filter(|u| !u.id.is_empty() && !u.login.is_empty())
            .map(|u| {
                let rules = UserRules {
                    identity: Identity::new(&u.id, &u.login, &u.display),
                    opts: MatchOptions {
                        bare_login: u.opts.bare,
                        display_name: u.opts.display,
                    },
                    hidden: u.hidden_user_ids.iter().map(|h| h.to_lowercase()).collect(),
                    muted: u.muted_channel_ids.into_iter().collect(),
                };
                (u.id, rules)
            })
            .collect();
        let channels = channel_rows
            .into_iter()
            .filter(|c| !c.id.is_empty() && !c.login.is_empty())
            .map(|c| {
                // Only users we have rules for, and never one who muted it.
                let watchers: Vec<String> = c
                    .users
                    .into_iter()
                    .filter(|u| users.get(u).is_some_and(|r| !r.muted.contains(&c.id)))
                    .collect();
                (c.id, Channel { login: c.login.to_lowercase(), live: c.live, users: watchers })
            })
            .filter(|(_, c)| !c.users.is_empty())
            .collect();
        LiveSet { channels, users }
    }

    /// The channel logins to be in, live ones first, each once.
    pub fn wanted(&self) -> Vec<(String, bool)> {
        let mut out: Vec<(String, bool)> = self.channels.values().map(|c| (c.login.clone(), c.live)).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out.dedup_by(|a, b| a.0 == b.0);
        out
    }

    pub fn live_count(&self) -> usize {
        self.channels.values().filter(|c| c.live).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_worker_answer_and_drops_muted_and_unknown_watchers() {
        let raw = r##"{
            "channels": [
                {"id": "1", "login": "Forsen", "users": ["10", "20", "99"]},
                {"id": "2", "login": "xqc", "users": ["20"]},
                {"id": "3", "login": "empty", "users": ["99"]}
            ],
            "users": [
                {"id": "10", "login": "brandon", "display": "Brandon", "opts": {"bare": false, "display": true},
                 "hidden_user_ids": ["Carl"], "muted_channel_ids": []},
                {"id": "20", "login": "alice", "display": "Alice", "muted_channel_ids": ["2"]}
            ]
        }"##;
        let set = LiveSet::from_answer(serde_json::from_str(raw).unwrap());
        assert_eq!(set.channels["1"], Channel { login: "forsen".into(), live: true, users: vec!["10".into(), "20".into()] });
        // Alice muted xqc and nobody else watches it; nobody we know watches "empty".
        assert!(!set.channels.contains_key("2"));
        assert!(!set.channels.contains_key("3"));
        let brandon = &set.users["10"];
        assert!(!brandon.opts.bare_login && brandon.opts.display_name);
        assert!(brandon.hidden.contains("carl"));
        // Options default to on when the Worker leaves them out.
        assert!(set.users["20"].opts.bare_login);
        assert_eq!(set.wanted(), vec![("forsen".to_string(), true)]);
    }

    #[test]
    fn pages_make_one_set_and_live_channels_come_first() {
        let page0 = r##"{
            "channels": [
                {"id": "5", "login": "zeta", "live": true, "users": ["10"]},
                {"id": "6", "login": "alpha", "live": false, "users": ["10"]}
            ],
            "users": [{"id": "10", "login": "brandon", "display": "Brandon"}],
            "page": 0, "pages": 2, "total": 3, "live": 1, "next": 1
        }"##;
        let page1 = r##"{
            "channels": [{"id": "7", "login": "beta", "live": false, "users": ["10", "20"]}],
            "users": [{"id": "10", "login": "brandon", "display": "Brandon"},
                      {"id": "20", "login": "alice", "display": "Alice"}],
            "next": null
        }"##;
        let a0: Answer = serde_json::from_str(page0).unwrap();
        assert_eq!(a0.next, Some(1));
        let a1: Answer = serde_json::from_str(page1).unwrap();
        assert_eq!(a1.next, None);
        let set = LiveSet::from_answers(vec![a0, a1]);
        assert_eq!(set.channels.len(), 3);
        assert_eq!(set.users.len(), 2);
        assert_eq!(set.live_count(), 1);
        assert_eq!(
            set.wanted(),
            vec![("zeta".to_string(), true), ("alpha".to_string(), false), ("beta".to_string(), false)]
        );
        assert_eq!(set.channels["7"].users, vec!["10".to_string(), "20".to_string()]);
    }
}

//! Who is watched where: the Worker's answer to `GET /watcher/targets`.
//!
//! The Worker returns the opted-in users' live followed channels (muted ones
//! already left out per user) and the rules for each user found in them. The
//! watcher keeps the newest answer and keeps using it while the Worker is
//! unreachable, so a Worker outage never parts every channel.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use sn_mention_rule::judge::Identity;
use sn_mention_rule::name::MatchOptions;

pub const PATH: &str = "/watcher/targets";

#[derive(Debug, Deserialize)]
pub struct Answer {
    #[serde(default)]
    pub channels: Vec<ChannelRow>,
    #[serde(default)]
    pub users: Vec<UserRow>,
}

#[derive(Debug, Deserialize)]
pub struct ChannelRow {
    pub id: String,
    pub login: String,
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

/// A live channel and who watches it.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub login: String,
    pub users: Vec<String>,
}

/// The live set: channels by room id, users by Twitch user id.
#[derive(Debug, Clone, Default)]
pub struct LiveSet {
    pub channels: HashMap<String, Channel>,
    pub users: HashMap<String, UserRules>,
}

impl LiveSet {
    pub fn from_answer(answer: Answer) -> LiveSet {
        let users: HashMap<String, UserRules> = answer
            .users
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
        let channels = answer
            .channels
            .into_iter()
            .filter(|c| !c.id.is_empty() && !c.login.is_empty())
            .map(|c| {
                // Only users we have rules for, and never one who muted it.
                let watchers: Vec<String> = c
                    .users
                    .into_iter()
                    .filter(|u| users.get(u).is_some_and(|r| !r.muted.contains(&c.id)))
                    .collect();
                (c.id, Channel { login: c.login.to_lowercase(), users: watchers })
            })
            .filter(|(_, c)| !c.users.is_empty())
            .collect();
        LiveSet { channels, users }
    }

    /// The channel logins to be in.
    pub fn logins(&self) -> HashSet<String> {
        self.channels.values().map(|c| c.login.clone()).collect()
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
        assert_eq!(set.channels["1"], Channel { login: "forsen".into(), users: vec!["10".into(), "20".into()] });
        // Alice muted xqc and nobody else watches it; nobody we know watches "empty".
        assert!(!set.channels.contains_key("2"));
        assert!(!set.channels.contains_key("3"));
        let brandon = &set.users["10"];
        assert!(!brandon.opts.bare_login && brandon.opts.display_name);
        assert!(brandon.hidden.contains("carl"));
        // Options default to on when the Worker leaves them out.
        assert!(set.users["20"].opts.bare_login);
        assert_eq!(set.logins(), ["forsen".to_string()].into_iter().collect());
    }
}

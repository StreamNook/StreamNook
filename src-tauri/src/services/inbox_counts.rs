//! The inbox's unread counts and the one `inbox-changed` emitter. Card 7.6
//! writes the emitter; this stub keeps the names other cards call.

use serde::{Deserialize, Serialize};

pub const EVENT: &str = "inbox-changed";

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Counts {
    pub unread_total: u32,
    pub mentions: u32,
    pub whispers: u32,
    pub island: u32,
    pub rev: u64,
}

/// Which app asks for the inbox view. The phones list everything. The desktop
/// island leaves out mentions that arrived in a chat on screen and the
/// conversations its Clear all put away. MultiChat's Mentions mode leaves out
/// only what Clear all put away (the whisper side applies that rule).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    #[default]
    Phone,
    Island,
    Multichat,
}

impl Surface {
    pub fn skips_seen_live(self) -> bool {
        self == Surface::Island
    }
}

/// Any inbox count may have changed. Card 7.6 makes this publish.
pub fn wake() {}

/// The counts last published. Card 7.6 makes this real.
pub fn snapshot() -> Counts {
    Counts::default()
}

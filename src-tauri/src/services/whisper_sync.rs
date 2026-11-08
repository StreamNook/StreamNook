//! The whisper sync engine. Card 7.4 writes it; these stubs let the commands
//! compile.

use crate::services::whisper_store::Scope;
use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SyncOutcome {
    Done { added: u32 },
    Superseded,
    NoSession,
    WrongSession,
    Partial { added: u32 },
    Failed { reason: String },
    StillRunning,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reason {
    Follow,
    Resume,
    SocketGap,
    InboxOpened,
    ConversationOpened(String),
    AfterTraffic(String),
    Manual,
}

pub fn follow_account(_user_id: Option<String>) {}

pub fn current_scope() -> Option<Scope> {
    None
}

pub fn trigger(_reason: Reason) {}

pub async fn sync_now() -> SyncOutcome {
    SyncOutcome::Failed { reason: "not ready".into() }
}

pub async fn load_older(_key: String) -> Result<u32, String> {
    Err("not ready".into())
}

pub fn note_socket_up(_first_of_process: bool) {}

pub fn note_socket_down() {}

pub fn note_resumed() {}

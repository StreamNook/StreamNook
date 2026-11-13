//! The one door from the automation engine to anything outside the app.
//!
//! The engine is handed an [`Outward`] when it starts and reaches Twitch,
//! streamnook.app and the account registry only through it: the room id, the
//! chat send, the claim call, and the token and account reads (PLAN v37
//! § 4.5). No engine module names a sending, token or lookup function
//! itself, so a test hands the engine a fake and can never reach Twitch or
//! streamnook.app. The real implementation panics in a test build.
//!
//! There is no role model here: no account role, no role map, no `role()`.
//! The token methods (T3 below, merged here by the Rust reconcile) run S7.2's
//! `TokenVerifier`; this is the one file that names `get_token_for`,
//! `AccountStore::list()` and `oauth2/validate`.
//!
//! Whispers, moderation, commands, own-channel topics, speech and the file
//! log join this trait with the phases that add those actions (P2's doors,
//! P3's reward list, P4's Kick methods are edits in their sections).
//!
//! Source pins (PLAN § 8): this file calls only `post_json_with_bearer` (a
//! claim goes out with the bearer the caller hands in). S7.2's no-runtime pin covers the pure
//! engine modules (it exempts `verify.rs` for the D238 deadline); `outward.rs`
//! is not one of them.

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::commands::streamnook_api::ApiResponse;
use crate::services::account_roster;
use crate::services::account_store::AccountStore;
use crate::services::chat_send_limiter::Ticket;
use crate::services::chat_service::SendOutcome;
use crate::services::twitch_service::{OwnReward, RewardsError};
use crate::services::twitch_service::{self, TokenError, TwitchService};

pub use super::verify::{TokenRefusal, VerifiedToken};
use super::verify::{BoxFuture, Fetch, Identity, TokenVerifier, Validation, VerifyIo};
use crate::services::twitch_service::{door_outcome, search_outcome, DoorOutcome, SearchFail};
use crate::services::whisper_send::Outcome as WhisperOutcome;

/// The Helix origin the real doors use (tests pass a fake server's).
pub const HELIX_BASE: &str = "https://api.twitch.tv";

/// The streamnook.app path every claim and release goes to.
pub const CLAIM_PATH: &str = "/api/v1/automations/claim";

/// Twitch's token check.
const VALIDATE_URL: &str = "https://id.twitch.tv/oauth2/validate";

/// One chat message as the engine sends it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatSend {
    pub token: String,
    pub text: String,
    pub parent: Option<String>,
    pub login: String,
    pub room_id: String,
    pub send_as: String,
}

#[async_trait]
pub trait Outward: Send + Sync {
    /// The token for `account_id`, only when Twitch's validate says it is
    /// that account's, within `budget` (past it, `Unreachable`). The door's
    /// verifier decides: the fetch, the validate cache and the remembered
    /// failures.
    async fn verified_token(&self, account_id: &str, budget: Duration) -> Result<VerifiedToken, TokenRefusal>;
    /// The verifier `verified_token` runs, for the engine's upkeep: a 401
    /// drops a token's cached owner, a Helix 401 is remembered, an account
    /// change forgets failures, admission reads a remembered failure.
    fn verifier(&self) -> &TokenVerifier;
    /// Every signed-in account as (id, login), from the registry. A local read.
    fn own_accounts(&self) -> Vec<(String, String)>;
    /// The profile's Twitch user id, when known. A local read.
    fn profile_user_id(&self) -> Option<String>;
    /// The registry's primary account id (the main account), when there is
    /// one. A local read; the registry is saved atomically, so never torn.
    fn primary_id(&self) -> Option<String>;
    /// The account generation (moves on every sign-in, switch and sign-out).
    fn account_generation(&self) -> u64;
    fn command_doors(&self) -> &dyn CommandDoors;
    fn mod_doors(&self) -> &dyn ModDoors;
    fn whisper_door(&self) -> &dyn WhisperDoor;
    /// Twitch refused a token (a 401 from a door or a read): its cached owner
    /// goes (hashed through the verifier's own `token_hash`).
    fn drop_token(&self, token: &VerifiedToken);
    /// The account's failure is remembered for a minute.
    fn record_send_401(&self, account_id: &str);
    /// The Twitch user id of the channel `login`.
    async fn room_id(&self, login: &str) -> Result<String, String>;
    /// Send one chat message on its booked ticket. The ticket is settled
    /// (recorded or cancelled) when this returns.
    async fn chat_send(&self, ticket: &mut Ticket, send: &ChatSend) -> SendOutcome;
    /// POST one claim or release body to streamnook.app with `bearer`, the
    /// owner's verified token (D231: never a token this call fetches
    /// itself). `Err` is a request that never completed; any HTTP answer is
    /// `Ok`.
    async fn claim_post(&self, bearer: &str, owner_id: &str, body: &Value) -> Result<ApiResponse, String>;
    /// P3: the channel's channel point rewards for the editor's picker (Helix
    /// `GET /channel_points/custom_rewards` with `token`, the verified
    /// owner's). Helix, not GQL, so no watchdog manifest entry. The default
    /// answers an error, so a fake that lists no rewards never reaches Twitch.
    async fn custom_rewards(&self, _token: &str, _broadcaster_id: &str) -> Result<Vec<OwnReward>, RewardsError> {
        Err(RewardsError::Transport("not available".into()))
    }
}

/// The Command action's typed Helix doors (D282, D290, D291). Exactly four
/// methods: a fake implements these and nothing else, so a new door is a
/// compile error in the test. The real implementation panics under
/// `cfg(test)`; each of its methods is one line, `door_outcome(&twin(..).await)`
/// or `search_outcome(twin(..).await)`.
#[async_trait]
pub trait CommandDoors: Send + Sync {
    /// `PATCH /chat/settings` as `moderator_id`.
    async fn chat_settings(&self, token: &str, moderator_id: &str, room: &str, settings: &Value) -> DoorOutcome;
    /// `POST /streams/markers` (`broadcaster_id` is the room).
    async fn marker(&self, token: &str, broadcaster_id: &str, description: Option<&str>) -> DoorOutcome;
    /// `PATCH /channels` with one of `title` or `game_id`.
    async fn channel_info(&self, token: &str, broadcaster_id: &str, title: Option<&str>, game_id: Option<&str>) -> DoorOutcome;
    /// The `SetGame` read (exact name first, fuzzy only when needed).
    async fn search_categories(&self, token: &str, query: &str) -> Result<Vec<Value>, SearchFail>;
}

/// The moderation, Pin, Announce and Shoutout doors, with the verified token
/// and `moderator_id = VerifiedToken.user_id`. Real impl panics under `cfg(test)`.
#[async_trait]
pub trait ModDoors: Send + Sync {
    async fn timeout(&self, token: &str, moderator_id: &str, room: &str, target: &str, seconds: u32, reason: Option<&str>) -> DoorOutcome;
    async fn delete(&self, token: &str, moderator_id: &str, room: &str, message_id: &str) -> DoorOutcome;
    async fn pin(&self, token: &str, moderator_id: &str, room: &str, message_id: &str, seconds: Option<u32>) -> DoorOutcome;
    /// On the engine client; Announce keeps its ticketed `post:` booking.
    async fn announce(&self, token: &str, moderator_id: &str, room: &str, text: &str, color: &str) -> DoorOutcome;
    async fn shoutout(&self, token: &str, moderator_id: &str, room: &str, target: &str) -> DoorOutcome;
}

/// The whisper door: `whisper_send::send_in_classified` for the owner's
/// scope. `Err(text)` is a refusal before any Helix call.
#[async_trait]
pub trait WhisperDoor: Send + Sync {
    /// Whether this account's whisper store knows `user_id` (`whisper_store::knows`).
    async fn knows(&self, owner_id: &str, user_id: &str) -> bool;
    /// Send as `send_as` to `to` (`expect_owner: Some(send_as)`, `by_automation: true`).
    async fn send(&self, send_as: &str, to: &WhisperTo, text: &str) -> (Result<(), String>, WhisperOutcome);
    /// The account's whole whisper list is read (`whisper_store::history_complete`): a "no"
    /// from `knows` is then final; before it the rule reads "Waiting for your whisper history".
    async fn history_complete(&self, owner_id: &str) -> bool;
}

/// A whisper's addressee.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhisperTo {
    pub user_id: String,
    pub login: String,
    pub name: String,
}

/// The door the running app uses. It holds the engine's token verifier;
/// every other read goes to the service that owns it.
pub struct RealOutward {
    verifier: TokenVerifier,
    commands: RealCommandDoors,
    mods: RealModDoors,
    whispers: RealWhisperDoor,
}

impl RealOutward {
    /// `runtime` is where each validate runs as its own task.
    pub fn new(runtime: tokio::runtime::Handle) -> RealOutward {
        RealOutward {
            verifier: TokenVerifier::new(real_verify_io(runtime)),
            commands: RealCommandDoors,
            mods: RealModDoors,
            whispers: RealWhisperDoor,
        }
    }
}

/// The real doors: one line each, behind `real_only()` (they panic under `cfg(test)`).
pub struct RealCommandDoors;
pub struct RealModDoors;
pub struct RealWhisperDoor;

fn engine_http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| crate::services::chat_service::engine_client(crate::services::chat_service::ENGINE_CONNECT_TIMEOUT, crate::services::chat_send_limiter::ENGINE_SEND_TIMEOUT))
}

#[async_trait]
impl CommandDoors for RealCommandDoors {
    async fn chat_settings(&self, token: &str, moderator_id: &str, room: &str, settings: &Value) -> DoorOutcome {
        real_only();
        door_outcome(&TwitchService::update_chat_settings_engine(engine_http(), HELIX_BASE, token, moderator_id, room, settings).await)
    }
    async fn marker(&self, token: &str, broadcaster_id: &str, description: Option<&str>) -> DoorOutcome {
        real_only();
        door_outcome(&TwitchService::create_stream_marker_engine(engine_http(), HELIX_BASE, token, broadcaster_id, description).await)
    }
    async fn channel_info(&self, token: &str, broadcaster_id: &str, title: Option<&str>, game_id: Option<&str>) -> DoorOutcome {
        real_only();
        door_outcome(&TwitchService::modify_channel_info_engine(engine_http(), HELIX_BASE, token, broadcaster_id, title, game_id).await)
    }
    async fn search_categories(&self, token: &str, query: &str) -> Result<Vec<Value>, SearchFail> {
        real_only();
        search_outcome(TwitchService::search_categories_engine(engine_http(), HELIX_BASE, token, query).await)
    }
}

#[async_trait]
impl ModDoors for RealModDoors {
    async fn timeout(&self, token: &str, moderator_id: &str, room: &str, target: &str, seconds: u32, reason: Option<&str>) -> DoorOutcome {
        real_only();
        let req = crate::services::moderation::ModRequest {
            provider: None,
            broadcaster_id: Some(room.to_string()),
            channel: None,
            target_user_id: Some(target.to_string()),
            target_name: None,
            action: crate::services::moderation::ModAction::Timeout { seconds },
            reason: reason.map(str::to_string),
            use_saved_reason: false,
            typed: None,
            one_press: false,
        };
        door_outcome(&crate::services::moderation::moderate_token_bound(engine_http(), HELIX_BASE, token, moderator_id, &req).await)
    }
    async fn delete(&self, token: &str, moderator_id: &str, room: &str, message_id: &str) -> DoorOutcome {
        real_only();
        let req = crate::services::moderation::ModRequest {
            provider: None,
            broadcaster_id: Some(room.to_string()),
            channel: None,
            target_user_id: None,
            target_name: None,
            action: crate::services::moderation::ModAction::Delete { message_id: message_id.to_string() },
            reason: None,
            use_saved_reason: false,
            typed: None,
            one_press: false,
        };
        door_outcome(&crate::services::moderation::moderate_token_bound(engine_http(), HELIX_BASE, token, moderator_id, &req).await)
    }
    async fn pin(&self, token: &str, moderator_id: &str, room: &str, message_id: &str, seconds: Option<u32>) -> DoorOutcome {
        real_only();
        door_outcome(&TwitchService::pin_chat_message_engine(engine_http(), HELIX_BASE, token, moderator_id, room, message_id, seconds).await)
    }
    async fn announce(&self, token: &str, moderator_id: &str, room: &str, text: &str, color: &str) -> DoorOutcome {
        real_only();
        door_outcome(&TwitchService::send_chat_announcement_engine(engine_http(), HELIX_BASE, token, moderator_id, room, text, Some(color)).await)
    }
    async fn shoutout(&self, token: &str, moderator_id: &str, room: &str, target: &str) -> DoorOutcome {
        real_only();
        door_outcome(&TwitchService::send_shoutout_engine(engine_http(), HELIX_BASE, token, moderator_id, room, target).await)
    }
}

#[async_trait]
impl WhisperDoor for RealWhisperDoor {
    async fn knows(&self, owner_id: &str, user_id: &str) -> bool {
        real_only();
        match crate::services::whisper_sync::current_scope() {
            Some(scope) if scope.owner == owner_id => crate::services::whisper_store::knows(&scope, user_id),
            _ => false,
        }
    }
    async fn send(&self, send_as: &str, to: &WhisperTo, text: &str) -> (Result<(), String>, WhisperOutcome) {
        real_only();
        use crate::services::whisper_send::{send_in_classified, HelixWhispers, SendTarget};
        use crate::services::whisper_sync::{SystemClock, TokioSleeper};
        let Some(scope) = crate::services::whisper_sync::current_scope() else {
            return (Err("You are not signed in to Twitch.".into()), WhisperOutcome::Refused("You are not signed in to Twitch.".into()));
        };
        let target = SendTarget { key: to.user_id.clone(), user_id: to.user_id.clone(), user_login: to.login.clone(), user_name: to.name.clone(), profile_image_url: None };
        let (result, outcome) =
            send_in_classified(&scope, &HelixWhispers, &SystemClock, &TokioSleeper, target, text.to_string(), Some(send_as.to_string()), None, true, true).await;
        (result.map(|_| ()), outcome)
    }
    async fn history_complete(&self, owner_id: &str) -> bool {
        real_only();
        match crate::services::whisper_sync::current_scope() {
            Some(scope) if scope.owner == owner_id => crate::services::whisper_store::history_complete(&scope),
            _ => false,
        }
    }
}

/// The verifier's reach outside itself: the app's own token path and
/// Twitch's validate.
fn real_verify_io(runtime: tokio::runtime::Handle) -> VerifyIo {
    let clock = super::steady_clock();
    VerifyIo {
        fetch: Box::new(|account_id: String| -> BoxFuture<Fetch> { Box::pin(async move { fetch_token(&account_id).await }) }),
        validate: Box::new(|token: String| -> BoxFuture<Validation> { Box::pin(async move { twitch_identity_of(&token).await }) }),
        // Each validate runs as its own task on the app's runtime; its answer
        // travels through the verifier's own channel (S7.2 `VerifyIo::spawn`).
        spawn: Box::new(move |task: BoxFuture<()>| {
            runtime.spawn(task);
        }),
        token_hash: Box::new(|token: &str| twitch_service::token_hash(token)),
        now_ms: Box::new(move || clock()),
        generation: Box::new(TwitchService::account_generation),
    }
}

/// `account_id`'s token through the app's own path (the main's slot, a
/// linked account's own file), its failure sorted by kind.
async fn fetch_token(account_id: &str) -> Fetch {
    real_only();
    match AccountStore::get_token_for(account_id).await {
        Ok(token) => Fetch::Token(token),
        Err(e) => fetch_failure(&e),
    }
}

/// A missing token or a refused refresh is `Refused` (sign in again); a
/// store or Twitch that did not answer, or anything unknown, is
/// `Unreachable` (the next event).
fn fetch_failure(e: &anyhow::Error) -> Fetch {
    let words = e.to_string();
    match TwitchService::fetch_failure(e) {
        TokenError::NoToken | TokenError::Refused => Fetch::Refused(words),
        TokenError::Unreachable | TokenError::VaultUnavailable => Fetch::Unreachable(words),
    }
}

/// Validate's own client: 3 s to connect, 5 s in all.
fn validate_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default()
    })
}

#[derive(serde::Deserialize)]
struct ValidateBody {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    login: Option<String>,
    #[serde(default)]
    expires_in: u64,
}

/// Which account a token belongs to, from Twitch's validate.
async fn twitch_identity_of(token: &str) -> Validation {
    real_only();
    let sent = validate_client().get(VALIDATE_URL).header("Authorization", format!("OAuth {token}")).send().await;
    let response = match sent {
        Ok(response) => response,
        Err(e) => return Validation::Unreachable(e.to_string()),
    };
    let status = response.status().as_u16();
    if status != 200 {
        return validation_of_status(status);
    }
    match response.json::<ValidateBody>().await {
        Ok(body) => validation_of(body),
        Err(e) => Validation::Unreachable(e.to_string()),
    }
}

/// A non-200 answer: 401 is a rejected token; anything else is Twitch not
/// answering this time (never remembered).
fn validation_of_status(status: u16) -> Validation {
    if status == 401 {
        Validation::Rejected
    } else {
        Validation::Unreachable(format!("Twitch answered {status}"))
    }
}

/// A 200 answer: a user's token names its user; any other names nobody.
fn validation_of(body: ValidateBody) -> Validation {
    match body.user_id.filter(|id| !id.is_empty()) {
        Some(user_id) => Validation::Valid(Identity { user_id, login: body.login.unwrap_or_default(), expires_in_s: body.expires_in }),
        None => Validation::NoUser,
    }
}

fn real_only() {
    if cfg!(test) {
        panic!("the real Outward reaches Twitch and streamnook.app; tests use a fake");
    }
}

#[async_trait]
impl Outward for RealOutward {
    async fn verified_token(&self, account_id: &str, budget: Duration) -> Result<VerifiedToken, TokenRefusal> {
        self.verifier.verify(account_id, budget).await
    }

    fn verifier(&self) -> &TokenVerifier {
        &self.verifier
    }

    fn own_accounts(&self) -> Vec<(String, String)> {
        real_only();
        AccountStore::list().into_iter().map(|a| (a.user_id, a.login)).collect()
    }

    fn profile_user_id(&self) -> Option<String> {
        real_only();
        account_roster::twitch_user_id()
    }

    fn primary_id(&self) -> Option<String> {
        real_only();
        AccountStore::primary().map(|a| a.user_id)
    }

    fn account_generation(&self) -> u64 {
        real_only();
        TwitchService::account_generation()
    }

    fn command_doors(&self) -> &dyn CommandDoors {
        &self.commands
    }

    fn mod_doors(&self) -> &dyn ModDoors {
        &self.mods
    }

    fn whisper_door(&self) -> &dyn WhisperDoor {
        &self.whispers
    }

    fn drop_token(&self, token: &VerifiedToken) {
        self.verifier.drop_token_of(&token.token);
    }

    fn record_send_401(&self, account_id: &str) {
        self.verifier.record_send_401(account_id);
    }

    async fn room_id(&self, login: &str) -> Result<String, String> {
        real_only();
        if let Some(id) = crate::services::chat_send_limiter::room_id_of(login) {
            return Ok(id);
        }
        crate::services::slash_commands::user_by_login(login)
            .await?
            .and_then(|u| u["id"].as_str().map(str::to_string))
            .ok_or_else(|| format!("There is no Twitch channel named {login}"))
    }

    async fn chat_send(&self, ticket: &mut Ticket, send: &ChatSend) -> SendOutcome {
        real_only();
        crate::services::chat_service::ChatService::send_message_helix_only(
            ticket,
            &send.token,
            &send.text,
            send.parent.as_deref(),
            &send.login,
            &send.room_id,
            &send.send_as,
        )
        .await
    }

    async fn claim_post(&self, bearer: &str, owner_id: &str, body: &Value) -> Result<ApiResponse, String> {
        real_only();
        crate::commands::streamnook_api::post_json_with_bearer(CLAIM_PATH, body, bearer, Some(owner_id)).await
    }

    async fn custom_rewards(&self, token: &str, broadcaster_id: &str) -> Result<Vec<OwnReward>, RewardsError> {
        real_only();
        crate::services::twitch_service::custom_rewards_engine(
            crate::services::http::client(),
            crate::services::twitch_service::HELIX_BASE,
            token,
            broadcaster_id,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn fake_io() -> VerifyIo {
        VerifyIo {
            fetch: Box::new(|_: String| -> BoxFuture<Fetch> { Box::pin(async { Fetch::Token("fake".into()) }) }),
            validate: Box::new(|_: String| -> BoxFuture<Validation> {
                Box::pin(async { Validation::Valid(Identity { user_id: "1".into(), login: "me".into(), expires_in_s: 3600 }) })
            }),
            spawn: Box::new(|task: BoxFuture<()>| {
                tokio::spawn(task);
            }),
            token_hash: Box::new(|token: &str| format!("hash-{token}")),
            now_ms: Box::new(|| 0),
            generation: Box::new(|| 1),
        }
    }

    /// The doors the module's own fake hands out: none of them is ever called.
    struct NoDoors;

    #[async_trait]
    impl CommandDoors for NoDoors {
        async fn chat_settings(&self, _token: &str, _moderator_id: &str, _room: &str, _settings: &Value) -> DoorOutcome {
            unreachable!("tests use a fake")
        }
        async fn marker(&self, _token: &str, _broadcaster_id: &str, _description: Option<&str>) -> DoorOutcome {
            unreachable!("tests use a fake")
        }
        async fn channel_info(&self, _token: &str, _broadcaster_id: &str, _title: Option<&str>, _game_id: Option<&str>) -> DoorOutcome {
            unreachable!("tests use a fake")
        }
        async fn search_categories(&self, _token: &str, _query: &str) -> Result<Vec<Value>, SearchFail> {
            unreachable!("tests use a fake")
        }
    }

    #[async_trait]
    impl ModDoors for NoDoors {
        async fn timeout(&self, _token: &str, _moderator_id: &str, _room: &str, _target: &str, _seconds: u32, _reason: Option<&str>) -> DoorOutcome {
            unreachable!("tests use a fake")
        }
        async fn delete(&self, _token: &str, _moderator_id: &str, _room: &str, _message_id: &str) -> DoorOutcome {
            unreachable!("tests use a fake")
        }
        async fn pin(&self, _token: &str, _moderator_id: &str, _room: &str, _message_id: &str, _seconds: Option<u32>) -> DoorOutcome {
            unreachable!("tests use a fake")
        }
        async fn announce(&self, _token: &str, _moderator_id: &str, _room: &str, _text: &str, _color: &str) -> DoorOutcome {
            unreachable!("tests use a fake")
        }
        async fn shoutout(&self, _token: &str, _moderator_id: &str, _room: &str, _target: &str) -> DoorOutcome {
            unreachable!("tests use a fake")
        }
    }

    #[async_trait]
    impl WhisperDoor for NoDoors {
        async fn knows(&self, _owner_id: &str, _user_id: &str) -> bool {
            unreachable!("tests use a fake")
        }
        async fn send(&self, _send_as: &str, _to: &WhisperTo, _text: &str) -> (Result<(), String>, WhisperOutcome) {
            unreachable!("tests use a fake")
        }
        async fn history_complete(&self, _owner_id: &str) -> bool {
            unreachable!("tests use a fake")
        }
    }

    struct Fake {
        verifier: TokenVerifier,
    }

    impl Fake {
        fn new() -> Fake {
            Fake { verifier: TokenVerifier::new(fake_io()) }
        }
    }

    #[async_trait]
    impl Outward for Fake {
        async fn verified_token(&self, account_id: &str, budget: Duration) -> Result<VerifiedToken, TokenRefusal> {
            self.verifier.verify(account_id, budget).await
        }
        fn verifier(&self) -> &TokenVerifier {
            &self.verifier
        }
        fn own_accounts(&self) -> Vec<(String, String)> {
            vec![("1".into(), "me".into())]
        }
        fn profile_user_id(&self) -> Option<String> {
            Some("1".into())
        }
        fn primary_id(&self) -> Option<String> {
            Some("1".into())
        }
        fn account_generation(&self) -> u64 {
            1
        }
        fn command_doors(&self) -> &dyn CommandDoors {
            &NoDoors
        }
        fn mod_doors(&self) -> &dyn ModDoors {
            &NoDoors
        }
        fn whisper_door(&self) -> &dyn WhisperDoor {
            &NoDoors
        }
        fn drop_token(&self, token: &VerifiedToken) {
            self.verifier.drop_token_of(&token.token);
        }
        fn record_send_401(&self, account_id: &str) {
            self.verifier.record_send_401(account_id);
        }
        async fn room_id(&self, _login: &str) -> Result<String, String> {
            Ok("42".into())
        }
        async fn chat_send(&self, ticket: &mut Ticket, _send: &ChatSend) -> SendOutcome {
            crate::services::chat_send_limiter::cancel_booking(ticket);
            SendOutcome::Refused("fake".into())
        }
        async fn claim_post(&self, _bearer: &str, _owner_id: &str, _body: &Value) -> Result<ApiResponse, String> {
            Ok(ApiResponse { status: 200, ok: true, body: "{}".into() })
        }
    }

    fn real() -> RealOutward {
        RealOutward::new(tokio::runtime::Handle::current())
    }

    #[tokio::test]
    async fn a_fake_serves_as_the_engine_door() {
        let door: Arc<dyn Outward> = Arc::new(Fake::new());
        assert_eq!(door.verified_token("1", Duration::from_secs(1)).await.unwrap().token, "fake");
        assert_eq!(door.room_id("anyone").await.unwrap(), "42");
        assert_eq!(door.claim_post("bearer", "1", &serde_json::json!({})).await.unwrap().status, 200);
    }

    #[tokio::test]
    #[should_panic(expected = "tests use a fake")]
    async fn the_real_room_lookup_panics_in_tests() {
        let _ = real().room_id("anyone").await;
    }

    #[tokio::test]
    #[should_panic(expected = "tests use a fake")]
    async fn the_real_claim_call_panics_in_tests() {
        let _ = real().claim_post("bearer", "1", &serde_json::json!({})).await;
    }

    #[tokio::test]
    #[should_panic(expected = "tests use a fake")]
    async fn the_real_token_read_panics_in_tests() {
        let _ = real().verified_token("1", Duration::from_secs(1)).await;
    }

    #[tokio::test]
    #[should_panic(expected = "tests use a fake")]
    async fn the_real_command_door_panics_in_tests() {
        let _ = RealCommandDoors.search_categories("token", "art").await;
    }

    #[tokio::test]
    #[should_panic(expected = "tests use a fake")]
    async fn the_real_mod_door_panics_in_tests() {
        let _ = RealModDoors.shoutout("token", "1", "2", "3").await;
    }

    #[tokio::test]
    #[should_panic(expected = "tests use a fake")]
    async fn the_real_whisper_door_panics_in_tests() {
        let _ = RealWhisperDoor.knows("1", "2").await;
    }

    /// The body of `fn name` in `impl` text starting at `from`: up to the next `fn ` or `}` at the impl's indent.
    fn body_of<'a>(src: &'a str, from: &str, name: &str) -> &'a str {
        let start = src.find(from).expect("the impl exists");
        let rest = &src[start..];
        let at = rest.find(&format!("fn {name}(")).expect("the method exists");
        let rest = &rest[at..];
        let end = rest.find("\n    }\n").expect("the method ends");
        &rest[..end]
    }

    #[test]
    fn each_real_door_is_one_line() {
        let src = include_str!("outward.rs");
        for (from, names) in [
            ("impl CommandDoors for RealCommandDoors", vec!["chat_settings", "marker", "channel_info", "search_categories"]),
            ("impl ModDoors for RealModDoors", vec!["timeout", "delete", "pin", "announce", "shoutout"]),
        ] {
            for name in names {
                let body = body_of(src, from, name);
                let calls = body.matches("door_outcome(").count() + body.matches("search_outcome(").count();
                assert_eq!(calls, 1, "{name}: one outcome mapping");
                assert_eq!(body.matches(".await").count(), 1, "{name}: one request and no other await");
            }
        }
    }

    #[test]
    fn real_401_upkeep_is_one_line() {
        let src = include_str!("outward.rs");
        let drop = body_of(src, "impl Outward for RealOutward", "drop_token");
        assert_eq!(drop.matches("self.verifier.").count(), 1, "{drop}");
        assert_eq!(drop.lines().count(), 2, "signature and one delegation, nothing else");
        let seen = body_of(src, "impl Outward for RealOutward", "record_send_401");
        assert_eq!(seen.matches("self.verifier.").count(), 1, "{seen}");
        assert_eq!(seen.lines().count(), 2);
    }

    #[test]
    fn fetch_failures_sort_into_refused_and_unreachable() {
        let tagged = |kind: TokenError| anyhow::Error::new(kind).context("Token expired");
        assert!(matches!(fetch_failure(&tagged(TokenError::NoToken)), Fetch::Refused(_)));
        assert!(matches!(fetch_failure(&tagged(TokenError::Refused)), Fetch::Refused(_)));
        assert!(matches!(fetch_failure(&tagged(TokenError::Unreachable)), Fetch::Unreachable(_)));
        assert!(matches!(fetch_failure(&tagged(TokenError::VaultUnavailable)), Fetch::Unreachable(_)));
        let unknown = anyhow::anyhow!("something nobody typed");
        assert!(matches!(fetch_failure(&unknown), Fetch::Unreachable(_)), "an unknown failure never says sign in");
    }

    #[test]
    fn validate_answers_map_as_the_plan_says() {
        let body = |user_id: Option<&str>| ValidateBody {
            user_id: user_id.map(str::to_string),
            login: Some("me".into()),
            expires_in: 120,
        };
        assert_eq!(
            validation_of(body(Some("1"))),
            Validation::Valid(Identity { user_id: "1".into(), login: "me".into(), expires_in_s: 120 })
        );
        assert_eq!(validation_of(body(None)), Validation::NoUser);
        assert_eq!(validation_of(body(Some(""))), Validation::NoUser);
        assert_eq!(validation_of_status(401), Validation::Rejected);
        for status in [429, 500, 503] {
            assert!(matches!(validation_of_status(status), Validation::Unreachable(_)), "{status}");
        }
    }

    #[tokio::test]
    #[should_panic(expected = "tests use a fake")]
    async fn the_real_reward_list_panics_in_tests() {
        let _ = real().custom_rewards("token", "1").await;
    }

    #[tokio::test]
    async fn a_fake_that_lists_no_rewards_never_reaches_twitch() {
        let door: Arc<dyn Outward> = Arc::new(Fake::new());
        assert!(matches!(door.custom_rewards("t", "1").await, Err(RewardsError::Transport(_))));
    }
}

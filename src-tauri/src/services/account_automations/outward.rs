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
use crate::services::twitch_service::{self, TokenError, TwitchService};

pub use super::verify::{TokenRefusal, VerifiedToken};
use super::verify::{BoxFuture, Fetch, Identity, TokenVerifier, Validation, VerifyIo};

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
}

/// The door the running app uses. It holds the engine's token verifier;
/// every other read goes to the service that owns it.
pub struct RealOutward {
    verifier: TokenVerifier,
}

impl RealOutward {
    /// `runtime` is where each validate runs as its own task.
    pub fn new(runtime: tokio::runtime::Handle) -> RealOutward {
        RealOutward { verifier: TokenVerifier::new(real_verify_io(runtime)) }
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
}

//! Measured limits of Twitch's private APIs. Several fail SILENTLY when
//! exceeded, so measure before raising one; don't infer from Twitch's
//! published caps, which describe different things.

/// Topics per PubSub LISTEN *frame*. Over this, Twitch sends no RESPONSE and
/// subscribes to nothing. Cliff measured at 28; 20 leaves byte-size headroom.
pub const PUBSUB_LISTEN_TOPICS_PER_FRAME: usize = 20;

/// Topics per PubSub *connection*, across all frames. Twitch documents this one.
pub const PUBSUB_MAX_TOPICS_PER_CONNECTION: usize = 50;

/// Operations per batched GQL POST. 36 returns HTTP 400 with a valid JSON body,
/// so callers must check the response is an array AND matches the request
/// length before mapping results positionally.
pub const GQL_MAX_BATCHED_OPERATIONS: usize = 35;

/// Aliased fields in one GQL query (`c0: channel(id:) { .. }`). Scales much
/// further than batched operations; degrades loudly via `errors` near 150.
pub const GQL_ALIASED_FIELDS_SAFE: usize = 50;

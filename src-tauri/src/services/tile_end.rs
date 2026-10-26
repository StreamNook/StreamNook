//! When a provider stream ended, judged from its upstream playlist. Shared by
//! the relays every build compiles (TikTok, YouTube) and by the desktop's
//! MultiNook server, which reports a tile's end to the page.

/// Playlist fetches in a row an ended provider stream must refuse (404 or 410).
const PROVIDER_END_REFUSALS: u32 = 6;
/// How long those refusals must span, in milliseconds.
const PROVIDER_END_SPAN_MS: u64 = 12_000;

/// What a provider tile's relay has seen of its upstream playlist. Held per
/// tile instance and cleared when the tile is pointed at a new upstream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProviderEndState {
    /// Refusals in a row since the last playlist that was served.
    refusals: u32,
    /// When the first of those refusals arrived (`mono_ms`; 0 = none yet).
    first_refusal_ms: u64,
    /// The end was already reported for this tile.
    announced: bool,
}

/// One answer from a provider tile's upstream playlist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TileFetch {
    /// A playlist was served; `endlist` when it carries `#EXT-X-ENDLIST`.
    Playlist { endlist: bool },
    /// The upstream answered 404 or 410 and the tile's re-sign failed (or the
    /// tile has none), at `at_ms`.
    Refused { at_ms: u64 },
    /// Any other answer. A fetch that got no answer at all is not reported here.
    /// Neither kind counts or resets the run.
    Failed,
}

/// Whether a provider tile's stream has ended: its playlist carries
/// `#EXT-X-ENDLIST`, or the upstream refused `PROVIDER_END_REFUSALS` playlist
/// fetches in a row spanning `PROVIDER_END_SPAN_MS`. Only a served playlist
/// clears the run; any other answer leaves it as it was. The bool is true only
/// for the fetch that first concludes the end.
pub(crate) fn provider_tile_ended(mut state: ProviderEndState, fetch: TileFetch) -> (ProviderEndState, bool) {
    let ended = match fetch {
        TileFetch::Playlist { endlist } => {
            state.refusals = 0;
            state.first_refusal_ms = 0;
            endlist
        }
        TileFetch::Refused { at_ms } => {
            if state.refusals == 0 {
                state.first_refusal_ms = at_ms;
            }
            state.refusals = state.refusals.saturating_add(1);
            state.refusals >= PROVIDER_END_REFUSALS
                && at_ms.saturating_sub(state.first_refusal_ms) >= PROVIDER_END_SPAN_MS
        }
        TileFetch::Failed => return (state, false),
    };
    let first = ended && !state.announced;
    if ended {
        state.announced = true;
    }
    (state, first)
}

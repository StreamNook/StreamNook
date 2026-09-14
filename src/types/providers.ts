// Provider identity for multi-platform chat + events.
//
// Twitch is the established native path; the other platforms light up per
// release phase via the `enabled` flag below. A "source" the user watches is
// identified by a composite `<provider>:<channel>` key (see utils/providerKey).

export type ProviderId = 'twitch' | 'kick' | 'youtube' | 'rumble' | 'tiktok' | 'x';

export const PROVIDER_IDS: ProviderId[] = ['twitch', 'kick', 'youtube', 'rumble', 'tiktok', 'x'];

export const DEFAULT_PROVIDER: ProviderId = 'twitch';

// How sending a message is achieved on a platform. Drives the chat input's
// read-only vs sendable state and which connection a send routes through.
//   native  - existing first-party path (Twitch IRC/Helix)
//   oauth   - official write API behind an account connection (Kick)
//   webview - typed into the platform's own logged-in page (YouTube/Rumble/TikTok/X)
//   none    - sending not supported
export type SendSupport = 'native' | 'oauth' | 'webview' | 'none';

export interface ProviderMeta {
  id: ProviderId;
  label: string;
  // Whether the CHAT adapter is wired up and offered in the add-source picker.
  // Flips to true as each platform's chat phase ships.
  chatEnabled: boolean;
  send: SendSupport;
  // True when even READING chat requires a hosted webview (X). Most read natively.
  readNeedsWebview: boolean;
  // Brand color for the provider chip in the activity feed and source rows.
  color: string;
}

export const PROVIDERS: Record<ProviderId, ProviderMeta> = {
  twitch: { id: 'twitch', label: 'Twitch', chatEnabled: true, send: 'native', readNeedsWebview: false, color: '#9147ff' },
  kick: { id: 'kick', label: 'Kick', chatEnabled: true, send: 'oauth', readNeedsWebview: false, color: '#53fc18' },
  youtube: { id: 'youtube', label: 'YouTube', chatEnabled: true, send: 'webview', readNeedsWebview: false, color: '#ff0000' },
  rumble: { id: 'rumble', label: 'Rumble', chatEnabled: false, send: 'webview', readNeedsWebview: false, color: '#85c742' },
  // TikTok reads anonymously; sending is a ban risk, so the adapter is read-only.
  tiktok: { id: 'tiktok', label: 'TikTok', chatEnabled: true, send: 'none', readNeedsWebview: false, color: '#00f2ea' },
  x: { id: 'x', label: 'X', chatEnabled: false, send: 'webview', readNeedsWebview: true, color: '#1d9bf0' },
};

/// Providers offered in the MultiChat add-source picker, derived from the chat
/// flags above rather than a hand-maintained duplicate list.
export const CHAT_PROVIDERS: ProviderId[] = PROVIDER_IDS.filter((p) => PROVIDERS[p].chatEnabled);

// --- Watch / browse capability axis -----------------------------------------
//
// A second, independent axis from chat: what a platform supports for BROWSING
// and WATCHING streams. UI affordances (Home provider pills, follow buttons,
// search fan-out, context-menu items) gate on this declaratively instead of
// hard-coding platform names. Mirrors the Rust `SourceCaps` returned by
// `provider_source_caps`; `playback` flips true per shipped phase.

export interface ProviderWatchMeta {
  // Browse + watch is shipped for this provider.
  playback: boolean;
  // Shape of the Browse-tab directory surface, or null when there is none.
  //   categories - a category grid then per-category stream lists (Twitch, Kick)
  //   search     - no categories; trending-live grid with search promoted (YouTube)
  //   feed       - a flat recommended-live feed (TikTok)
  browse: 'categories' | 'search' | 'feed' | null;
  search: boolean;
  // Where the followed-live list comes from: the platform itself, or the
  // app-local follow list. null when following isn't offered.
  followedLive: 'native' | 'inApp' | null;
  liveCheck: boolean;
}

export const PROVIDER_WATCH: Record<ProviderId, ProviderWatchMeta> = {
  twitch: { playback: true, browse: 'categories', search: true, followedLive: 'native', liveCheck: true },
  kick: { playback: true, browse: 'categories', search: true, followedLive: 'inApp', liveCheck: true },
  // browse is 'categories' off YouTube's OWN games directory (/gaming/games), not
  // a synthesised taxonomy: each entry is a real game with box art, live viewers,
  // and a topic channel whose /live page lists that game's streams.
  // followedLive here is a STATIC literal while YouTube's real answer is dynamic:
  // youtube_media.rs:787 reports native only while a cookie session exists and
  // falls back to the in-app list otherwise. Nothing in src/ reads this field
  // today, so the divergence is inert; resolve it against the Rust before wiring
  // any UI to it.
  youtube: { playback: true, browse: 'categories', search: true, followedLive: 'native', liveCheck: true },
  tiktok: { playback: false, browse: 'feed', search: false, followedLive: 'inApp', liveCheck: true },
  rumble: { playback: false, browse: null, search: false, followedLive: null, liveCheck: false },
  x: { playback: false, browse: null, search: false, followedLive: null, liveCheck: false },
};

/// Providers whose browse + watch surface is live in this build.
export const WATCHABLE_PROVIDERS: ProviderId[] = PROVIDER_IDS.filter((p) => PROVIDER_WATCH[p].playback);

// --- Grid (MultiNook) eligibility ----------------------------------------
//
// Kept as a separate axis from `playback` rather than folded into it, because
// the two genuinely differ: a platform can be perfectly playable as the solo
// stream and still be unable to run several at once.
//
// YouTube was exactly that case until services/youtube_dash.rs was keyed by
// stream id. Its session, segment cache and playback nonce were process-wide, so
// a second YouTube stream repointed the first one's relay and the first player
// was served the second broadcast's fragments with no error anywhere. That is
// fixed, and YouTube is now grid-eligible.
//
// The map stays because the NEXT platform in that position should be refused
// here with a reason, not discovered by a user. Anything listed is refused with
// its message; anything absent is allowed if it can play at all.
export const GRID_BLOCKED: Partial<Record<ProviderId, string>> = {};

/** True when this platform can be a tile in the grid. */
export function canGridProvider(p: ProviderId): boolean {
  return PROVIDER_WATCH[p].playback && !GRID_BLOCKED[p];
}

/** null when the provider can be a tile, otherwise the reason to show the user. */
export function gridRefusal(p: ProviderId): string | null {
  if (canGridProvider(p)) return null;
  return GRID_BLOCKED[p] ?? `${providerLabel(p)} streams can't be added to the grid yet.`;
}


/// Platforms the MultiNook picker SEARCHES. Deliberately wider than
/// canGridProvider: a refused platform's rows are still fetched and shown, then
/// disabled with gridRefusal's reason. Filtering them out instead would present
/// as "YouTube is missing from search", which reads as a broken search rather
/// than a deliberate limit.
export const GRID_PICKER_PROVIDERS: ProviderId[] = PROVIDER_IDS.filter(
  (p) => PROVIDER_WATCH[p].playback && PROVIDER_WATCH[p].search,
);
export function isProviderId(v: string): v is ProviderId {
  return (PROVIDER_IDS as string[]).includes(v);
}

export function providerLabel(id: string): string {
  return isProviderId(id) ? PROVIDERS[id].label : id;
}

/** A browsable category on a platform (Twitch's "games" equivalent). Mirrors
 *  the Rust `ProviderCategory`. */
export interface ProviderCategory {
  provider: ProviderId;
  /** Platform category id, as a string. */
  id: string;
  name: string;
  thumbnail: string;
  viewer_count: number;
  channel_count: number;
}

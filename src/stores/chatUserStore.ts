import { create } from 'zustand';
import {
  getCosmeticsFromMemoryCache,
  getCosmeticsWithFallback,
  isUserCosmeticsHardFailed,
  subscribeToCosmetics,
} from '../services/cosmeticsCache';
import { isStreamNookUser, getProfilePrefs, whenAtmospheresReady, subscribeAtmospheresVersion, subscribeStreamNookRegistryVersion, subscribeToProfileThemeChanges, getOwnedCosmeticSlugs, isCosmeticsRegistryLoaded } from '../services/supabaseService';
import { resolveEntitlement } from '../services/cosmetics/ownership';
import { getAtmosphere } from '../services/atmospheres';
import { parseCologneTheme, type CologneCosmetics } from '../services/cologneEvent';
import {
  getResolvedIdentity,
  getResolvedIdentityFromCache,
  getIdentityWithCache,
  subscribeResolvedIdentity,
  type ResolvedBadge,
} from '../services/identityService';
import { getGlobalThirdPartyBadges, type ThirdPartyBadge } from '../services/badgeService';
import {
  BTTV_PRO_LOADOUT_KEY,
  BTTV_PRO_BADGE_ID,
  buildBttvProBadge,
  resolveBttvProUrl,
} from '../services/bttvProBadge';
import { snapshotOverrides } from '../utils/userChatOverrides';

/**
 * Represents a user who has chatted in the current channel.
 * Used for @ mention autocomplete suggestions and as the canonical store for
 * a user's 7TV paint + 7TV badge + third-party chat-client badges.
 * Per-message components subscribe to these instead of each maintaining its
 * own useState + fetch effect.
 *
 * Third-party chat-client badges (FFZ / Chatterino / Homies / Chatsen / Chatty /
 * DankChat / BTTV) are resolved here ONCE per unique chatter. For StreamNook
 * members it is their curated loadout (resolved + ownership-checked by the
 * Identity API), which overrides their raw set. For everyone else it is their
 * REAL provider badges, looked up from the prefetched provider databases in
 * Rust (a cache-only call, no per-user network round-trip), the way Chatterino
 * shows them. Either way the per-message render path stays free of network calls.
 */
export interface ChatUser {
  userId: string;
  username: string;
  displayName: string;
  color: string;
  /** Timestamp of last message - used for sorting by recency */
  lastSeen: number;
  /** 7TV paint data if available (for decorated display) */
  paint?: any;
  /** Currently-selected 7TV badge if available */
  seventvBadge?: any;
  /** Third-party chat-client badges resolved to images: a member's curated
   *  loadout, or a non-member's real provider badges. Undefined until resolved;
   *  empty array = member who opted into nothing. */
  thirdPartyBadges?: ThirdPartyBadge[];
  /** The id of the StreamNook Atmosphere this member themes with (if any), so
   *  chat can render the matching animated wash. Undefined until resolved; null =
   *  resolved, none. */
  atmosphereId?: string | null;
  /** CS2 Major Cologne event cosmetics this member has APPLIED (the background
   *  plus which add-ons). Undefined until resolved; null = resolved, none. */
  cologne?: CologneCosmetics | null;
}

interface ChatUserStore {
  /** Map of userId -> ChatUser for O(1) lookups */
  users: Map<string, ChatUser>;
  /** Map of lowercase username -> userId for fast username lookups */
  usernameToId: Map<string, string>;
  
  /**
   * Add or update a user when they send a message. channelContext is accepted
   * for call-site compatibility but no longer used (third-party badges are
   * resolved by twitch_user_id through the Identity API, not by channel).
   */
  addUser: (
    user: Omit<ChatUser, 'lastSeen' | 'paint' | 'seventvBadge' | 'thirdPartyBadges' | 'atmosphereId' | 'cologne'>,
    channelContext?: { channelId: string; channelName: string },
  ) => void;
  
  /** Get a user by username (case-insensitive) */
  getUserByUsername: (username: string) => ChatUser | undefined;
  
  /** Get users matching a search query (prefix match on username/displayName) */
  getMatchingUsers: (query: string, limit?: number) => ChatUser[];
  
  /** Clear all users (call when switching channels) */
  clearUsers: () => void;
}

// Module-scope batched-update coalescer for cosmetic resolutions.
//
// 7TV's batched GraphQL request (see seventvService.requestUserCosmeticsBatched)
// fans one network round-trip out into N user resolutions in the same
// microtask. One setState each would mean N Map clones and N rounds of selector
// evaluation across every mounted ChatMessage, which stutters visibly.
//
// Coalesced, everything enqueued in the same microtask drains into ONE
// setState: one clone, one notification cycle, one commit. A ChatMessage still
// re-renders when its own user's paint or badge changed; others pay nothing.
type CosmeticUpdate = { paint: any; seventvBadge: any };
const pendingCosmeticUpdates = new Map<string, CosmeticUpdate>();
let pendingFlushScheduled = false;

function scheduleStoreFlush() {
  if (pendingFlushScheduled) return;
  pendingFlushScheduled = true;
  queueMicrotask(() => {
    pendingFlushScheduled = false;
    if (pendingCosmeticUpdates.size === 0) return;
    const cosmeticUpdates = new Map(pendingCosmeticUpdates);
    pendingCosmeticUpdates.clear();
    useChatUserStore.setState((state) => {
      const newUsers = new Map(state.users);
      for (const [uid, { paint, seventvBadge }] of cosmeticUpdates) {
        const current = newUsers.get(uid);
        if (current) {
          newUsers.set(uid, { ...current, paint, seventvBadge });
        }
      }
      return { users: newUsers };
    });
  });
}

function enqueueCosmeticUpdate(userId: string, paint: any, seventvBadge: any) {
  pendingCosmeticUpdates.set(userId, { paint, seventvBadge });
  scheduleStoreFlush();
}

// ── Identity changes (color / rename) ────────────────────────────────────────
// Same coalescing contract as the cosmetics above, but for the rare mid-session
// identity change (a /color, a display-name edit). Its own pending map + flush:
// the cosmetics flush hard-codes {paint, seventvBadge} and would clobber a
// same-microtask cosmetics resolution (and un-flip the cosmeticsResolved
// sentinel) if identity fields were routed through it. Each flush spreads the
// CURRENT record at flush time and writes only its own fields, so the two
// coalescers compose in either order.
type IdentityUpdate = { username: string; displayName: string; color: string };
const pendingIdentityUpdates = new Map<string, IdentityUpdate>();
let pendingIdentityFlushScheduled = false;

function scheduleIdentityFlush() {
  if (pendingIdentityFlushScheduled) return;
  pendingIdentityFlushScheduled = true;
  queueMicrotask(() => {
    pendingIdentityFlushScheduled = false;
    if (pendingIdentityUpdates.size === 0) return;
    const updates = new Map(pendingIdentityUpdates);
    pendingIdentityUpdates.clear();
    useChatUserStore.setState((state) => {
      const newUsers = new Map(state.users);
      const newUsernameToId = new Map(state.usernameToId);
      for (const [uid, identity] of updates) {
        const current = newUsers.get(uid);
        if (!current) continue;
        newUsers.set(uid, { ...current, ...identity, lastSeen: Date.now() });
        newUsernameToId.set(identity.username.toLowerCase(), uid);
      }
      return { users: newUsers, usernameToId: newUsernameToId };
    });
  });
}

// ── StreamNook third-party badge loadout ─────────────────────────────────────
// Same once-per-user, read-synchronously contract as the 7TV cosmetics above,
// but sourced from the Identity API's resolved bundle (the member's curated
// badges already resolved to images, server-side ownership-checked). Its own
// microtask coalescer so a burst of members resolving collapses into one
// setState instead of one clone+commit per user.
const pendingThirdPartyUpdates = new Map<string, ThirdPartyBadge[]>();
let pendingThirdPartyFlushScheduled = false;

// Non-members whose real provider badges have already been resolved this session.
// The Rust lookup reads only the prefetched provider databases, so a non-member's
// result cannot change within a session. Without this guard they re-trigger the
// Rust IPC lookup on EVERY message until their separate 7TV cosmetics resolve
// flips the fast path, flooding a busy channel with redundant IPC.
// Members are intentionally NOT gated here (they go through their own identity
// resolve cache, so a live loadout edit still re-resolves). Pruned alongside the
// user map (eviction + channel switch) so it never outgrows the tracked users.
const thirdPartyNonMemberResolved = new Set<string>();

function scheduleThirdPartyFlush() {
  if (pendingThirdPartyFlushScheduled) return;
  pendingThirdPartyFlushScheduled = true;
  queueMicrotask(() => {
    pendingThirdPartyFlushScheduled = false;
    if (pendingThirdPartyUpdates.size === 0) return;
    const updates = new Map(pendingThirdPartyUpdates);
    pendingThirdPartyUpdates.clear();
    useChatUserStore.setState((state) => {
      const newUsers = new Map(state.users);
      for (const [uid, badges] of updates) {
        const current = newUsers.get(uid);
        if (current) newUsers.set(uid, { ...current, thirdPartyBadges: badges });
      }
      return { users: newUsers };
    });
  });
}

// The resolve endpoint already ownership-checks and orders these; map its
// minimal {key,provider,title,image_url} shape onto the ThirdPartyBadge shape
// the chat badge row renders (it reads only id / imageUrl / title / provider).
function mapResolvedBadges(badges: ResolvedBadge[]): ThirdPartyBadge[] {
  return badges.map((b) => ({
    id: b.key,
    title: b.title,
    imageUrl: b.image_url,
    image1x: b.image_url,
    image2x: b.image_url,
    image4x: b.image_url,
    provider: b.provider,
  }));
}

// Resolve a chatter's third-party badges once and push them into the store.
//
// Members: their CURATED loadout (the badges they opted into on their StreamNook
// profile), which overrides their raw set. Cache hit -> enqueue immediately;
// miss -> kick the fetch and let the resolved-identity listener below enqueue
// when it lands (so we never write the store twice).
//
// Everyone else: their REAL provider badges (BTTV / FFZ / Chatterino / Homies /
// Chatsen / Chatty / DankChat), looked up from the prefetched provider databases
// in Rust. That call is a pure in-memory cache hit (no per-user network round
// trip), so it is safe in this once-per-chatter path. We skip the store write
// when the chatter carries no third-party badges (the common case) to avoid
// needless churn. BTTV Pro is intentionally NOT resolved for non-members: it
// needs a per-user live socket lookup, the one thing that would bring back the
// per-chatter network cost, so it stays an opt-in member identity badge.
function ensureThirdPartyResolved(userId: string | undefined) {
  if (!userId) return;

  if (isStreamNookUser(userId)) {
    const cached = getResolvedIdentityFromCache(userId);
    if (cached) {
      pendingThirdPartyUpdates.set(userId, mapResolvedBadges(cached.badges));
      scheduleThirdPartyFlush();
    } else {
      void getResolvedIdentity(userId).catch(() => {});
    }
    // BTTV Pro is WebSocket-only, so the Identity API can't resolve it like the
    // other providers (its loadout key passes through unresolved). Resolve it
    // client-side and merge it into this member's row if they opted it in.
    ensureBttvProResolved(userId);
    return;
  }

  // Resolve a non-member at most once per session. Mark BEFORE the async call so
  // concurrent messages from the same chatter can't each kick a duplicate lookup.
  if (thirdPartyNonMemberResolved.has(userId)) return;
  thirdPartyNonMemberResolved.add(userId);
  void getGlobalThirdPartyBadges(userId)
    .then((badges) => {
      if (badges.length === 0) return;
      pendingThirdPartyUpdates.set(userId, badges);
      scheduleThirdPartyFlush();
    })
    .catch(() => {});
}

// Resolve + merge a member's BTTV Pro badge when their loadout opted it in.
// Kept ADDITIVE and separate from the resolved-badge flush (which REPLACES the
// array) so the two never clobber each other: the flush lands the server-resolved
// contributor badges first (microtask), then this appends Pro after the socket
// lookup returns (network). We read the RAW loadout — not the resolved bundle,
// which drops the unresolved Pro key — to know whether the member opted in.
function ensureBttvProResolved(userId: string) {
  void getIdentityWithCache(userId)
    .then((loadout) => {
      if (!loadout.badges.includes(BTTV_PRO_LOADOUT_KEY)) {
        removeBttvPro(userId);
        return;
      }
      return resolveBttvProUrl(userId).then((url) => {
        if (url) mergeBttvPro(userId, buildBttvProBadge(url));
        else removeBttvPro(userId);
      });
    })
    .catch(() => {});
}

function mergeBttvPro(userId: string, badge: ThirdPartyBadge) {
  useChatUserStore.setState((state) => {
    const u = state.users.get(userId);
    if (!u) return {};
    const existing = u.thirdPartyBadges ?? [];
    if (existing.some((b) => b.id === badge.id)) return {}; // already present
    const newUsers = new Map(state.users);
    newUsers.set(userId, { ...u, thirdPartyBadges: [...existing, badge] });
    return { users: newUsers };
  });
}

function removeBttvPro(userId: string) {
  useChatUserStore.setState((state) => {
    const u = state.users.get(userId);
    if (!u?.thirdPartyBadges?.some((b) => b.id === BTTV_PRO_BADGE_ID)) return {};
    const newUsers = new Map(state.users);
    newUsers.set(userId, {
      ...u,
      thirdPartyBadges: u.thirdPartyBadges.filter((b) => b.id !== BTTV_PRO_BADGE_ID),
    });
    return { users: newUsers };
  });
}

// ── StreamNook Atmosphere chat wash (subscriber profile theme -> chat) ───────
// A subscriber who themes their profile with an Atmosphere also gets a subtle
// STATIC wash + edge bar behind their chat messages, so the cosmetic follows
// them into chat. Resolved once per MEMBER (no-op for non-members), gated on
// subscriber status. No animation in chat (perf): a static gradient only.
const pendingAtmosphereUpdates = new Map<string, string | null>();
let pendingAtmosphereFlushScheduled = false;
// Resolved atmosphere id per user (null = none). Doubles as the once-per-user
// cache AND lets an explicit change set a KNOWN value with no Supabase read, so a
// read can't race the just-fired write.
const atmosphereCache = new Map<string, string | null>();
const atmosphereInFlight = new Set<string>();
// Last time we resolved each member's live theme. Cross-user theme changes are
// PUSHED live by the realtime bridge at the bottom of this file, so the cache
// stays fresh without re-fetching. This TTL is only a backstop: if a viewer
// missed a live event (e.g. disconnected when the member changed it), a cache hit
// still paints the old value instantly, then re-fetches in the background so it
// self-heals on the member's next message, with no channel switch needed.
const lastResolvedAt = new Map<string, number>();
const RESOLVE_TTL_MS = 120_000;

function scheduleAtmosphereFlush() {
  if (pendingAtmosphereFlushScheduled) return;
  pendingAtmosphereFlushScheduled = true;
  queueMicrotask(() => {
    pendingAtmosphereFlushScheduled = false;
    if (pendingAtmosphereUpdates.size === 0) return;
    const updates = new Map(pendingAtmosphereUpdates);
    pendingAtmosphereUpdates.clear();
    useChatUserStore.setState((state) => {
      const newUsers = new Map(state.users);
      for (const [uid, id] of updates) {
        const current = newUsers.get(uid);
        if (current) newUsers.set(uid, { ...current, atmosphereId: id });
      }
      return { users: newUsers };
    });
  });
}

/**
 * Whether this member may actually WEAR the atmosphere they have selected.
 *
 * `user_profile_prefs` is world-writable under the anon key that ships in the
 * bundle, and nothing on the render path re-checked ownership, so anyone could
 * set their theme to a paid atmosphere and have it paint in every StreamNook
 * user's chat. Rendering is the last place that can refuse, so it refuses here.
 *
 * Deliberately per-CLASS, not per-item, for subscriber atmospheres: a viewer has
 * no way to see whether someone else is currently subscribed, and a strict
 * owned-only check would blank a real subscriber whose per-item ownership row
 * has not been granted yet (that RPC runs on their login, not ours).
 *
 * Accolade-gated atmospheres pass through unchecked ON PURPOSE. A viewer cannot
 * verify another member's accolades (they are not loaded cross-user, and adding
 * a fetch here would put a network round trip on the chat render path). Those
 * close when `user_accolades` stops being anon-writable, which makes the
 * accolade itself trustworthy. Do not "fix" this asymmetry by granting accolade
 * atmospheres into `user_cosmetics`: that converts a forgeable accolade into a
 * permanent ownership row and defeats this gate entirely.
 */
export function mayWearAtmosphere(userId: string, id: string | null): boolean {
  if (!id) return true; // clearing is always allowed
  // Our own accounts paint immediately. The server already gated the write, and
  // gating here would make a member's own pick stop previewing.
  if (ownAtmosphereAccounts.has(userId)) return true;
  // Missing data must never read as "not owned", or a paying member is blanked
  // during the registry load window or a Supabase outage.
  if (!isCosmeticsRegistryLoaded()) return true;

  const atm = getAtmosphere(id);
  if (!atm) return true; // unknown to the catalog; nothing will paint anyway
  if (atm.unlock?.kind === 'accolade') return true;

  // Cologne carries '+coin' / '+border' modifiers on the id; ownership is
  // recorded against the base atmosphere.
  const baseId = id.split('+')[0];
  const owned = getOwnedCosmeticSlugs(userId);
  const { everSubscribed } = resolveEntitlement({ ownedSlugs: owned, activeSubscription: false });
  return owned.has(baseId) || everSubscribed;
}

function pushAtmosphere(userId: string, id: string | null) {
  const effective = mayWearAtmosphere(userId, id) ? id : null;
  atmosphereCache.set(userId, effective);
  pendingAtmosphereUpdates.set(userId, effective);
  scheduleAtmosphereFlush();
  if (ownAtmosphereAccounts.has(userId)) persistOwnAtmosphere(userId);
}

// Disk persistence of OUR OWN accounts' resolved Atmosphere id (primary + every
// linked account), so the wash behind our messages paints on frame one of a cold
// launch instead of after the per-sighting prefs fetch. Keyed by account id.
const OWN_ATMOSPHERE_KEY = 'streamnook_own_atmosphere_v1';
const ownAtmosphereAccounts = new Set<string>();

function readPersistedAtmospheres(): Record<string, string | null> {
  try {
    const raw = localStorage.getItem(OWN_ATMOSPHERE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === 'object' ? (parsed as Record<string, string | null>) : {};
  } catch {
    return {};
  }
}

function persistOwnAtmosphere(userId: string): void {
  try {
    const store = readPersistedAtmospheres();
    store[userId] = atmosphereCache.get(userId) ?? null;
    localStorage.setItem(OWN_ATMOSPHERE_KEY, JSON.stringify(store));
  } catch {
    /* ignore (private mode / unavailable storage) */
  }
}

/**
 * Register all of OUR account ids (primary + linked) and seed each one's
 * Atmosphere wash from the id persisted last session, so a cold launch paints it
 * on frame one for every account. The stored value is the already-resolved id, so
 * no catalog wait is needed. Marks them own so a later theme change persists. An
 * explicit change still overwrites live via refreshAtmosphere; the per-sighting
 * resolve still backfills any account with nothing stored yet.
 */
export function registerOwnAtmospheres(userIds: string[]): void {
  const store = readPersistedAtmospheres();
  for (const userId of userIds) {
    if (!userId || ownAtmosphereAccounts.has(userId)) continue; // already registered/seeded
    ownAtmosphereAccounts.add(userId);
    if (userId in store) pushAtmosphere(userId, store[userId]);
  }
}

// Resolve a profile_theme into the store's atmosphere / Cologne fields and cache
// it. Shared by the per-sighting resolver, the explicit self-change path, and the
// live cross-user theme bridge, so all three stay consistent. Cologne is its own
// custom chrome (its add-ons are read from the theme id), not a gradient
// Atmosphere, so it clears atmosphereId and vice-versa.
function applyResolvedTheme(userId: string, profileTheme: string | null | undefined) {
  const cologne = parseCologneTheme(profileTheme);
  if (cologne) {
    pushAtmosphere(userId, null);
    pushCologne(userId, cologne);
  } else {
    const atm = getAtmosphere(profileTheme);
    pushAtmosphere(userId, atm ? atm.id : null);
    pushCologne(userId, null);
  }
  lastResolvedAt.set(userId, Date.now());
}

export function ensureAtmosphereResolved(userId: string) {
  if (!userId || !isStreamNookUser(userId)) return;
  // Cached/known value (incl. one set by an explicit change before the user was
  // in the store): push it to the store for this (re)sighting. The Cologne add-ons
  // are cached alongside the atmosphere (both come from the same selected theme).
  if (atmosphereCache.has(userId)) {
    // Serve the cached value now for an instant, flicker-free paint.
    pendingAtmosphereUpdates.set(userId, atmosphereCache.get(userId)!);
    scheduleAtmosphereFlush();
    pendingCologneUpdates.set(userId, cologneCache.get(userId) ?? null);
    scheduleCologneFlush();
    // Within the TTL, trust the cache (the live bridge keeps it fresh). Past it,
    // fall through to a background re-fetch as a backstop for a missed live event.
    if (Date.now() - (lastResolvedAt.get(userId) ?? 0) < RESOLVE_TTL_MS) return;
  }
  if (atmosphereInFlight.has(userId)) return;
  atmosphereInFlight.add(userId);
  void (async () => {
    try {
      // Visibility reads the world-readable profile_theme (not gated on subscriber
      // status), so a member's selection shows for every viewer like a badge.
      const prefs = await getProfilePrefs(userId);
      // Wait for the atmosphere catalog so we never resolve a real theme to null
      // just because the catalog had not loaded yet.
      await whenAtmospheresReady();
      applyResolvedTheme(userId, prefs.profileTheme);
    } catch {
      /* leave cached so a later sighting retries */
    } finally {
      atmosphereInFlight.delete(userId);
    }
  })();
}

// Set a member's theme to a KNOWN value now (they just changed it), so their
// messages update in real time with no Supabase read. Used for our own accounts
// on an explicit pick (the value is already resolved by the caller).
export function refreshAtmosphere(userId: string, atmosphereId: string | null) {
  applyResolvedTheme(userId, atmosphereId);
}

// ── CS2 Major Cologne event chrome ───────────────────────────────────────────
// The Cologne chrome is an OPT-IN selectable Atmosphere (theme id
// MAJOR_COLOGNE_THEME_ID), unlocked by the Major accolade. It is NOT auto-applied
// from merely holding the accolade. The tier (base = coin + wash, full = adds the
// frame for subscribers) is resolved from the member's SELECTED theme in
// ensureAtmosphereResolved above; this block just owns the store write-through.
const pendingCologneUpdates = new Map<string, CologneCosmetics | null>();
let pendingCologneFlushScheduled = false;
const cologneCache = new Map<string, CologneCosmetics | null>();

function scheduleCologneFlush() {
  if (pendingCologneFlushScheduled) return;
  pendingCologneFlushScheduled = true;
  queueMicrotask(() => {
    pendingCologneFlushScheduled = false;
    if (pendingCologneUpdates.size === 0) return;
    const updates = new Map(pendingCologneUpdates);
    pendingCologneUpdates.clear();
    useChatUserStore.setState((state) => {
      const newUsers = new Map(state.users);
      for (const [uid, cosmetics] of updates) {
        const current = newUsers.get(uid);
        if (current) newUsers.set(uid, { ...current, cologne: cosmetics });
      }
      return { users: newUsers };
    });
  });
}

function pushCologne(userId: string, cosmetics: CologneCosmetics | null) {
  cologneCache.set(userId, cosmetics);
  pendingCologneUpdates.set(userId, cosmetics);
  scheduleCologneFlush();
}


// The tracked-user map is a session-long singleton shared by every chat surface
// (main app + each MultiChat pane), and clearUsers only fires on channel switch.
// Without a cap it accumulated every chatter ever seen (tens of MB of paint/badge
// data over an evening). The cap sits far above any rendered message buffer (the
// per-channel message cap tops out at ~1150), so the least-recently-seen users
// evicted here have long since scrolled out of view across every surface; if one
// speaks again they are simply re-added (a cheap cosmetics re-resolve).
const MAX_TRACKED_USERS = 8000;
const USER_EVICT_SLACK = 1000;
const MATCH_SCAN_CAP = 200;

// Lowercased name pairs, keyed by user-object identity. Identity changes
// (rename, color) replace the ChatUser object via the identity coalescer, so a
// stale entry can never be served; the in-place lastSeen bump never touches
// names. Kills the 2-per-user toLowerCase() allocations the autocomplete scan
// paid on every keystroke.
const lowerNameCache = new WeakMap<ChatUser, { u: string; d: string }>();
function lowerNames(user: ChatUser): { u: string; d: string } {
  let l = lowerNameCache.get(user);
  if (!l) {
    l = { u: user.username.toLowerCase(), d: user.displayName.toLowerCase() };
    lowerNameCache.set(user, l);
  }
  return l;
}

function evictStaleUsers(
  users: Map<string, ChatUser>,
  usernameToId: Map<string, string>,
): void {
  if (users.size <= MAX_TRACKED_USERS + USER_EVICT_SLACK) return;
  // Sweep in batches (only when over cap + slack) so the O(n log n) sort is
  // amortized across many inserts rather than run on every new chatter.
  const sorted = Array.from(users.values()).sort((a, b) => a.lastSeen - b.lastSeen);
  const removeCount = users.size - MAX_TRACKED_USERS;
  for (let i = 0; i < removeCount; i++) {
    const victim = sorted[i];
    users.delete(victim.userId);
    thirdPartyNonMemberResolved.delete(victim.userId);
    const unameKey = victim.username.toLowerCase();
    if (usernameToId.get(unameKey) === victim.userId) {
      usernameToId.delete(unameKey);
    }
  }
}

export const useChatUserStore = create<ChatUserStore>((set, get) => ({
  users: new Map(),
  usernameToId: new Map(),
  
  addUser: (user, _channelContext) => {
    const existingUser = get().users.get(user.userId);

    // Fast path: cosmetics already resolved for this user. Update color/lastSeen
    // in place and skip the cache lookup entirely. paint OR seventvBadge being
    // non-undefined is the "cosmetics resolved" sentinel.
    const cosmeticsResolved =
      existingUser !== undefined &&
      (existingUser.paint !== undefined || existingUser.seventvBadge !== undefined);
    if (cosmeticsResolved) {
      const identityChanged =
        existingUser!.username !== user.username ||
        existingUser!.displayName !== user.displayName ||
        existingUser!.color !== user.color;
      if (identityChanged) {
        pendingIdentityUpdates.set(user.userId, {
          username: user.username,
          displayName: user.displayName,
          color: user.color,
        });
        scheduleIdentityFlush();
        return;
      }
      // Metadata-only bump. lastSeen has NO reactive readers (only call-time
      // sorts: eviction, @-autocomplete, command palette), so mutate in place —
      // no Map clones, no subscriber fan-out. This bypassing of setState is
      // deliberate; do not "fix" it back into a store update.
      existingUser!.lastSeen = Date.now();
      return;
    }

    // First sight of this user. Insert their base shape, then resolve cosmetics.
    set((state) => {
      const newUsers = new Map(state.users);
      const newUsernameToId = new Map(state.usernameToId);
      newUsers.set(user.userId, {
        ...user,
        lastSeen: Date.now(),
        paint: existingUser?.paint,
        seventvBadge: existingUser?.seventvBadge,
        thirdPartyBadges: existingUser?.thirdPartyBadges,
        atmosphereId: existingUser?.atmosphereId,
        cologne: existingUser?.cologne,
      });
      newUsernameToId.set(user.username.toLowerCase(), user.userId);
      // First-sight is the only path that grows the map, so cap it here.
      evictStaleUsers(newUsers, newUsernameToId);
      return { users: newUsers, usernameToId: newUsernameToId };
    });

    // Apply resolved 7TV cosmetics. A genuine answer (the user has a paint, or
    // genuinely has none) sets paint and seventvBadge (possibly to null) so the
    // cosmeticsResolved sentinel flips true. Routes through the module-level
    // coalescer so a burst of resolutions from the batched GraphQL response
    // collapses into one store update.
    //
    // A HARD FAILURE (network / 5xx / "query too complex" batch rejection) is
    // NOT a real answer. We must leave the user unresolved — paint/badge stay
    // undefined — so their next message re-fetches and the cache's 30s self-heal
    // recovers them. Stamping null here would flip the cosmeticsResolved sentinel,
    // sending every later message down the addUser fast-path that never re-fetches,
    // stranding the user paint-less for the whole session.
    const applyCosmetics = (cosmetics: { paints?: any[]; badges?: any[] } | null) => {
      const selectedPaint = cosmetics?.paints?.find((p: any) => p.selected) ?? null;
      const selectedBadge = cosmetics?.badges?.find((b: any) => b.selected) ?? null;
      enqueueCosmeticUpdate(user.userId, selectedPaint, selectedBadge);
    };

    if (user.userId.startsWith('youtube:') || user.userId.startsWith('tiktok:')) {
      // YouTube + TikTok chatters don't resolve 7TV cosmetics — we decorate them
      // with their own native data (profile picture + platform badges) instead, so
      // there's nothing to fetch. Mark them resolved-with-none so the fast path
      // skips re-attempting on every message.
      enqueueCosmeticUpdate(user.userId, null, null);
    } else {
      const cachedCosmetics = getCosmeticsFromMemoryCache(user.userId);
      if (cachedCosmetics && !isUserCosmeticsHardFailed(user.userId)) {
        applyCosmetics(cachedCosmetics);
      } else {
        // The fetch records (or clears) the hard-fail mark before it resolves, so
        // re-read it here. On a hard failure, skip the apply and leave the user
        // unresolved so the next message retries once the 30s window elapses.
        getCosmeticsWithFallback(user.userId)
          .then(() => {
            if (isUserCosmeticsHardFailed(user.userId)) return;
            const resolved = getCosmeticsFromMemoryCache(user.userId);
            if (resolved) applyCosmetics(resolved);
          })
          .catch(() => {});
      }
    }

    // Resolve this member's curated third-party badges once (no-op for non-members).
    ensureThirdPartyResolved(user.userId);
    // Resolve this member's selected Atmosphere / Cologne chrome once (no-op for non-members).
    ensureAtmosphereResolved(user.userId);
  },
  
  getUserByUsername: (username: string) => {
    const { usernameToId, users } = get();
    const userId = usernameToId.get(username.toLowerCase());
    if (userId) {
      return users.get(userId);
    }
    return undefined;
  },
  
  getMatchingUsers: (query: string, limit = 5) => {
    const { users } = get();
    const queryLower = query.toLowerCase();
    const overrides = snapshotOverrides();

    // Filter users whose username, displayName, or (user-set) nickname starts
    // with the query. Inserting an @mention still uses user.username (the real
    // Twitch login) because Twitch IRC doesn't resolve nicknames.
    const matches: ChatUser[] = [];
    for (const user of users.values()) {
      const nick = overrides[user.userId]?.nickname?.toLowerCase();
      const lower = lowerNames(user);
      if (
        lower.u.startsWith(queryLower) ||
        lower.d.startsWith(queryLower) ||
        (nick && nick.startsWith(queryLower))
      ) {
        matches.push(user);
        // Runs per keystroke over up to ~9k users. Cap the collect: past this
        // many matches (single-letter queries), the top-`limit` by recency is
        // close enough for autocomplete and the full sort isn't worth it.
        if (matches.length >= MATCH_SCAN_CAP) break;
      }
    }

    // Sort by recency (most recent first)
    matches.sort((a, b) => b.lastSeen - a.lastSeen);

    return matches.slice(0, limit);
  },
  
  clearUsers: () => {
    // The third-party badge data lives inside the user records being wiped here,
    // so drop the resolved-guard too: a chatter reappearing in the next channel
    // re-resolves cleanly instead of being skipped with no badges.
    thirdPartyNonMemberResolved.clear();
    // The atmosphere / Cologne theme cache deliberately PERSISTS across channels:
    // a member's wash paints instantly when they reappear instead of re-fetching
    // every switch. It is kept fresh by the live theme bridge below (a push on any
    // change) plus the TTL backstop, so it does not go stale.
    set({ users: new Map(), usernameToId: new Map() });
  },
}));

// When the atmosphere catalog (re)loads, re-resolve any tracked member we had
// previously resolved to "no atmosphere": an atmosphere added live that they
// already equip can now render without waiting for a re-sighting. Only
// null-cached users are touched, so a value set by an explicit theme change is
// left alone.
subscribeAtmospheresVersion(() => {
  const users = useChatUserStore.getState().users;
  for (const uid of users.keys()) {
    if (atmosphereCache.get(uid) === null) {
      atmosphereCache.delete(uid);
      atmosphereInFlight.delete(uid);
      ensureAtmosphereResolved(uid);
    }
  }
});

// Live cross-user theme updates: when any member changes the atmosphere / Cologne
// look they wear, the new value is pushed here so their rows repaint for everyone
// already in chat with them, instead of waiting for a re-sighting or channel
// switch. Only members we track (or have cached this session) are touched; the
// rest of the change stream is ignored. Wait for the catalog so a real theme
// never resolves to null.
subscribeToProfileThemeChanges((userId, profileTheme) => {
  if (!userId || !isStreamNookUser(userId)) return;
  const state = useChatUserStore.getState();
  if (!state.users.has(userId) && !atmosphereCache.has(userId)) return;
  void whenAtmospheresReady().then(() => applyResolvedTheme(userId, profileTheme));
});

// Reactive bridge from the shared cosmetics cache into the per-user chat
// store. Anything that writes to cosmeticsCache (chat's own addUser fetch,
// profile-card refresh, ProfileSettings, future surfaces) lands here, and
// we refresh the matching user's paint/badge in the store so their chat
// row repaints. Without this bridge a transient empty result earlier in
// the session stays stuck on screen even after a later fetch succeeded.
subscribeToCosmetics((userId, cosmetics, hardFail) => {
  const state = useChatUserStore.getState();
  const existing = state.users.get(userId);
  if (!existing) return;

  // A hard failure isn't a real answer. Don't stamp a null paint (which would
  // flip the addUser fast-path's cosmeticsResolved sentinel and stop retries);
  // leave whatever's already on the row so a later success can repaint it.
  if (hardFail) return;

  const nextPaint = cosmetics?.paints?.find((p: any) => p.selected) ?? null;
  const nextBadge = cosmetics?.badges?.find((b: any) => b.selected) ?? null;

  // Identity-compare by id so we skip an enqueue when nothing changed.
  const samePaint = (existing.paint?.id ?? null) === (nextPaint?.id ?? null);
  const sameBadge = (existing.seventvBadge?.id ?? null) === (nextBadge?.id ?? null);
  if (samePaint && sameBadge && existing.paint !== undefined && existing.seventvBadge !== undefined) {
    return;
  }

  enqueueCosmeticUpdate(userId, nextPaint, nextBadge);
});

// Repaint chat rows when a member's resolved identity lands or changes. Fires on
// the initial per-user fetch (above) and on a member editing their own loadout
// (setIdentity clears the resolved cache, which re-resolves here). Only touches
// users already in this channel's store; others propagate on their next fetch.
subscribeResolvedIdentity((userId) => {
  if (!useChatUserStore.getState().users.has(userId)) return;
  ensureThirdPartyResolved(userId);
});

// The StreamNook member registry loads/refreshes asynchronously. A member first
// seen before it finished loading was classified as a non-member and shown their
// raw provider badges; when the registry lands, re-resolve any tracked member so
// their curated loadout (which overrides) takes over. Mirrors the atmosphere
// re-resolve above. Non-members are unaffected by a registry change, so we only
// touch users now known to be members.
subscribeStreamNookRegistryVersion(() => {
  const users = useChatUserStore.getState().users;
  for (const uid of users.keys()) {
    if (isStreamNookUser(uid)) {
      ensureThirdPartyResolved(uid);
      ensureAtmosphereResolved(uid);
    }
  }
});

// Simplified emote service - now a thin wrapper around Rust backend
import { invoke } from '@tauri-apps/api/core';
import { convertFileSrc } from '@tauri-apps/api/core';

import { Logger } from '../utils/logger';
import { features } from '../features';
import { IS_MOBILE } from '../utils/platform';
export interface Emote {
  id: string;
  name: string;
  url: string;
  provider: 'twitch' | 'bttv' | '7tv' | 'ffz' | 'kick' | 'youtube';
  isZeroWidth?: boolean;
  localUrl?: string;
  /** Type of emote: "globals", "subscriptions", "bitstier", "follower", "channelpoints", etc. */
  emote_type?: string;
  /** Owner/broadcaster ID for subscription emotes */
  owner_id?: string;
  /** Owner/author display name for emote attribution */
  owner_name?: string;
  /** Emote width in pixels (for aspect ratio sorting) */
  width?: number;
  /** FFZ modifier bitmask (Hidden=1, FlipX=2, ...); present only on FFZ modifiers */
  modifierFlags?: number;
  /** FFZ effect emote composable only by FFZ subscribers (rendering ungated) */
  ffzSubOnly?: boolean;
  /** Composition is gated for this account (YouTube members-only emoji). Shown
   *  in the grid with the same lock treatment as `ffzSubOnly`. */
  locked?: boolean;
  /** Badge text for the lock, e.g. "Members only". Only read when `locked`. */
  lockedLabel?: string;
  /** What to put in the composer when it differs from the display name. YouTube
   *  unicode emoji insert the literal character; everything else inserts `name`. */
  insertText?: string;
}

export interface EmoteSet {
  twitch: Emote[];
  bttv: Emote[];
  '7tv': Emote[];
  ffz: Emote[];
  /** Kick's own native emotes (channel sub set + Global + Emojis). Empty for Twitch. */
  kick: Emote[];
  /** YouTube custom emoji, learned from chat rather than fetched: YouTube exposes
   *  no channel emote-set endpoint, so this fills in as the channel uses them. */
  youtube: Emote[];
  /** Whether the 7TV rows are this channel's real dictionary. Rust sets it false
   *  when the channel document fetch failed and the rows are a fallback (globals
   *  only, or a disk copy), so a picker should keep retrying. Absent on sets that
   *  predate the flag; treat absent as true. */
  seven_tv_ok?: boolean;
}

// Module-level registry of cached emote files (cacheKey -> localPath).
// For 7TV the key is `${id}@${tier}` (see emoteCacheKey); other providers key
// by bare id since they have a single canonical URL.
const cachedEmoteFiles: Map<string, string> = new Map();

// Lazily-built per-set name indexes, keyed by set identity. Emote sets are only
// ever replaced wholesale (never mutated in place), so a WeakMap entry lives
// exactly as long as its set and a refresh/channel switch drops it with the
// old object. Insertion is first-wins in the same provider order as the
// name-lookup chains this replaces.
const emoteLookupCache = new WeakMap<
  EmoteSet,
  { byName: Map<string, Emote>; lowerNames: Set<string> }
>();
const LOOKUP_ORDER = ['7tv', 'bttv', 'ffz', 'twitch', 'kick', 'youtube'] as const;

export function getEmoteLookup(set: EmoteSet): { byName: Map<string, Emote>; lowerNames: Set<string> } {
  let entry = emoteLookupCache.get(set);
  if (!entry) {
    const byName = new Map<string, Emote>();
    const lowerNames = new Set<string>();
    for (const provider of LOOKUP_ORDER) {
      for (const e of set[provider] ?? []) {
        if (!byName.has(e.name)) byName.set(e.name, e);
        lowerNames.add(e.name.toLowerCase());
      }
    }
    entry = { byName, lowerNames };
    emoteLookupCache.set(set, entry);
  }
  return entry;
}

/**
 * Shape emote rows from Rust for the page: camelCase the flags and attach a
 * local URL ONLY when the file is already cached (a Map lookup, never a fetch);
 * the browser loads from the CDN when localUrl is undefined. Used for whole sets
 * and for the rows a live 7TV delta adds, so both paths produce identical rows.
 */
export function enhanceRustEmotes(emotes: any[]): Emote[] {
  return emotes.map((emote) => {
    // 7TV is looked up at the per-DPI tier so the cached size matches what renders.
    const localPath = cachedEmoteFiles.get(emoteCacheKey(emote.id, emote.provider));
    const zeroWidth = emote.is_zero_width !== undefined ? emote.is_zero_width : emote.isZeroWidth;
    return {
      ...emote,
      isZeroWidth: zeroWidth,
      modifierFlags: emote.modifier_flags ?? emote.modifierFlags,
      ffzSubOnly: emote.ffz_sub_only ?? emote.ffzSubOnly,
      localUrl: localPath ? convertFileSrc(localPath) : undefined,
    } as Emote;
  });
}

// --- Per-DPI emote sizing -------------------------------------------------
// 7TV serves discrete size tiers (1x..4x). We cache AND render the smallest
// tier that still looks crisp at the display's pixel density, so the on-disk
// copy matches what is on screen and disk-first render can never soften an
// emote (the reason 7TV historically bypassed the cache: it stored 1x while
// every surface drew 2x). devicePixelRatio is effectively fixed per display;
// the tier is memoized and reset on resize so moving the window to a
// different-density monitor re-picks the right size and caches it fresh.
// 7TV serves 1x=32px, 2x=64px, 3x=96px, 4x=128px (verified against
// 7tv.io/v3/emote-sets/global). `3x` was missing from this union, which is part
// of why the mobile ladder had nowhere sensible to land.
export type EmoteTier = '1x' | '2x' | '3x' | '4x';

const TIER_PX: Record<EmoteTier, number> = { '1x': 32, '2x': 64, '3x': 96, '4x': 128 };
const TIER_ORDER: EmoteTier[] = ['1x', '2x', '3x', '4x'];

// Nominal CSS size of an inline chat emote before the user's emote_scale. The
// exact figure only has to be close: it selects a tier, and the tiers are an
// octave apart.
const INLINE_EMOTE_CSS_PX = 28;

// The user's emote_scale (0.5x to 3x), pushed in at boot. Module-level rather
// than read from the store, so this file keeps no dependency on AppStore.
let _emoteScale = 1;

/** Called at boot with the persisted chat design. Resets the memoized tier. */
export function setInlineEmoteScale(scale: number): void {
  const next = Number.isFinite(scale) && scale > 0 ? scale : 1;
  if (next === _emoteScale) return;
  _emoteScale = next;
  _inlineTier = null;
}

let _inlineTier: EmoteTier | null = null;

export function inlineEmoteTier(): EmoteTier {
  if (_inlineTier) return _inlineTier;
  let dpr = 1;
  try {
    dpr = Math.max(1, window.devicePixelRatio || 1);
  } catch {
    /* non-DOM context */
  }

  if (IS_MOBILE) {
    // Pick the smallest tier that actually covers the rendered glyph.
    //
    // The old rule was `dpr > 2 -> 4x`, written for retina desktops, and on a
    // phone it was badly wrong: a modern handset reports DPR 3 or 4, so EVERY
    // 7TV emote was fetched as a 128px ANIMATED AVIF and then scaled down to a
    // ~28px glyph. Chromium decodes animated AVIF in software (dav1d), per
    // frame, per visible instance - so a busy channel decoded tens of 128px
    // animations continuously to draw thumbnails. One of the larger
    // contributors to the phone running hot.
    //
    // Deriving from the real pixel need also self-corrects: at DPR 4 (this
    // panel's QHD+ mode) or a large emote_scale it picks 4x again, because at
    // that point the screen genuinely has the pixels to show it.
    const needed = INLINE_EMOTE_CSS_PX * _emoteScale * dpr;
    _inlineTier = TIER_ORDER.find((t) => TIER_PX[t] >= needed) ?? '4x';
    return _inlineTier;
  }

  // Desktop is UNCHANGED, deliberately. Windows display scaling at 225-300
  // percent puts a desktop at DPR 2.25-3.0, so a hi-DPI desktop DOES take the
  // `4x` branch today; changing it here would alter the frozen desktop build
  // and orphan its emote disk cache for no measured reason.
  _inlineTier = dpr <= 1 ? '1x' : dpr <= 2 ? '2x' : '4x';
  return _inlineTier;
}

try {
  window.addEventListener('resize', () => {
    _inlineTier = null;
  });
} catch {
  /* non-DOM context */
}

/** CDN URL for a 7TV emote at a given size tier. */
export function sevenTvTierUrl(id: string, tier: EmoteTier = inlineEmoteTier()): string {
  return `https://cdn.7tv.app/emote/${id}/${tier}.avif`;
}

/**
 * Disk-cache key. 7TV is size-tiered (one file per tier) so it keys by
 * `${id}@${tier}`; other providers have a single canonical URL and key by bare
 * id. Caching and lookup MUST use the same key or disk-first silently misses.
 * `@` survives the Rust filename sanitizer (which only strips path separators).
 */
function emoteCacheKey(id: string, provider?: string, tier: EmoteTier = inlineEmoteTier()): string {
  if (provider === '7tv') return `${id}@${tier}`;
  // Provider-namespaced so a Twitch emote and an FFZ emote that share a numeric
  // id can't collide in the flat cache map. Must match emote_cache_target() in
  // src-tauri/src/services/emote_prefetch_service.rs. Falls back to the bare id
  // only when provider is unknown (callers should pass it).
  return provider ? `${provider}-${id}` : id;
}

// Pending downloads to prevent duplicate requests
const pendingDownloads: Map<string, Promise<string | null>> = new Map();

// Queue system for downloads. Deliberately STREAM-POLITE: caching is a
// background optimization that must NEVER compete with the live video for
// bandwidth or main-thread time. We keep it to a single serial download with a
// real gap between each, so the stream always wins. Reusing one pooled HTTP/2
// client (DOWNLOAD_CLIENT in universal_cache_service.rs) keeps even this serial
// trickle cheap per request, so going from 3 concurrent to 1 costs almost
// nothing in fill time while removing the bandwidth contention that made the
// stream stutter.
// Two fill rates. POLITE is the original stream-safe trickle: one serial
// download with a real gap, idle-scheduled, so background caching never competes
// with the live video. BURST kicks in only while an emote picker is open (the
// one moment the user is actively waiting on emotes). The burst is safe because:
// the chat-ingestion rAF coalescing landed after this trickle was written, so
// chat renders no longer starve the video's main-thread buffer appends; the
// downloads themselves run in Rust/tokio OFF the JS main thread on tiny files;
// and the sibling badge cache (badgeImageCacheService) already runs 5 concurrent
// with no playback impact. When no picker is open we fall back to POLITE so we
// are not needlessly hammering the free provider CDNs for the whole session.
const POLITE_CONCURRENT = 1;
const POLITE_DELAY_MS = 250; // Real gap so background caching yields to the stream
const BURST_CONCURRENT = 5;
const BURST_DELAY_MS = 15;
let burstRefs = 0;
const downloadQueue: Array<{ id: string, url: string }> = [];
// Membership mirror of downloadQueue. The queue can hold a whole channel set
// (thousands) while queueEmoteForCaching is called per rendered emote, so the
// dedupe check must be O(1), not a queue scan.
const queuedIds = new Set<string>();
let activeDownloads = 0;
let processingScheduled = false;
let lastDownloadTime = 0;

function burstActive(): boolean { return burstRefs > 0; }
function currentConcurrency(): number { return burstActive() ? BURST_CONCURRENT : POLITE_CONCURRENT; }
function currentDelayMs(): number { return burstActive() ? BURST_DELAY_MS : POLITE_DELAY_MS; }

/**
 * Raise (true) or lower (false) the emote disk-cache fill rate. Ref-counted so
 * multiple open pickers (split panes, popouts) compose — the burst stays on
 * until the last one closes. Call `true` when a picker opens and `false` from
 * the matching effect cleanup when it closes. Safe to over-call; clamps at zero.
 */
export function setEmoteCacheBurst(active: boolean) {
  burstRefs = Math.max(0, burstRefs + (active ? 1 : -1));
  if (burstActive()) pumpQueue();
}

// Settings cache
let cachedSettings: { enabled: boolean; expiryDays: number } | null = null;

let initializationPromise: Promise<void> | null = null;

async function getEmoteCacheSettings(): Promise<{ enabled: boolean; expiryDays: number }> {
  if (cachedSettings) return cachedSettings;
  try {
    const settings = await invoke('load_settings') as any;
    cachedSettings = {
      enabled: settings.cache?.enabled !== false,
      expiryDays: settings.cache?.expiry_days ?? 7
    };
    return cachedSettings;
  } catch (e) {
    Logger.warn('[EmoteService] Failed to load settings:', e);
    return { enabled: true, expiryDays: 7 };
  }
}

// Drain the cache queue. POLITE mode stays one-at-a-time and waits for the main
// thread to go idle between downloads (the original stream-safe trickle). BURST
// mode (a picker is open) launches up to BURST_CONCURRENT in flight with a tiny
// spacing and does NOT wait for idle, so the set lands on disk fast while the
// user is looking at it. The HTTP fetch + disk write happen in Rust either way,
// so this only governs how many tiny requests are in flight at once.
function pumpQueue() {
  if (processingScheduled || downloadQueue.length === 0) return;
  if (activeDownloads >= currentConcurrency()) return;

  const sinceLast = Date.now() - lastDownloadTime;
  const delay = Math.max(0, currentDelayMs() - sinceLast);

  processingScheduled = true;
  const launch = () => {
    processingScheduled = false;
    // Fill every free slot the current mode allows, then let each completion
    // re-pump. Polite mode has a single slot, so this runs serially.
    while (activeDownloads < currentConcurrency() && downloadQueue.length > 0) {
      const next = downloadQueue.shift();
      if (!next) break;
      queuedIds.delete(next.id);
      activeDownloads++;
      lastDownloadTime = Date.now();
      void downloadEmoteIfNeeded(next.id, next.url)
        .catch((e) => Logger.debug(`[EmoteService] Error processing queue item ${next.id}:`, e))
        .finally(() => {
          activeDownloads--;
          pumpQueue();
        });
    }
  };

  if (burstActive()) {
    // User is waiting on these — don't gate on main-thread idle.
    setTimeout(launch, delay);
  } else if (typeof requestIdleCallback === 'function') {
    // Background: wait the polite gap, then for a true idle moment.
    setTimeout(() => requestIdleCallback(launch, { timeout: 10000 }), delay);
  } else {
    setTimeout(launch, delay + 200);
  }
}

async function downloadEmoteIfNeeded(id: string, url: string): Promise<string | null> {
  if (cachedEmoteFiles.has(id)) {
    return cachedEmoteFiles.get(id)!;
  }

  if (pendingDownloads.has(id)) {
    return pendingDownloads.get(id)!;
  }

  // When the disk cache is off, consumers fall back to the CDN URL they already
  // carry. This used to claim the cache was unavailable on mobile; that stopped
  // being true when the flag was turned on for Android. See features.ts.
  if (!features.assetDiskCache) return null;

  const settings = await getEmoteCacheSettings();
  if (!settings.enabled) return null;

  const downloadPromise = (async () => {
    try {
      const localPath = await invoke('download_and_cache_file', {
        cacheType: 'emote',
        id,
        url,
        expiryDays: settings.expiryDays
      }) as string;

      if (localPath) {
        cachedEmoteFiles.set(id, localPath);
        return localPath;
      }
      return null;
    } catch (e) {
      Logger.debug(`[EmoteService] Failed to cache emote ${id}:`, e);
      return null;
    } finally {
      pendingDownloads.delete(id);
    }
  })();

  pendingDownloads.set(id, downloadPromise);
  return downloadPromise;
}

export function queueEmoteForCaching(id: string, url: string, priority: boolean = false) {
  if (cachedEmoteFiles.has(id) || pendingDownloads.has(id) || queuedIds.has(id)) {
    return;
  }

  // Priority items go to front of queue (for search results)
  if (priority) {
    downloadQueue.unshift({ id, url });
  } else {
    downloadQueue.push({ id, url });
  }
  queuedIds.add(id);
  // Drain the queue: idle-gated trickle normally, fast burst while a picker is open.
  pumpQueue();
}

export function getCachedEmoteUrl(
  id: string,
  provider?: string,
  tier: EmoteTier = inlineEmoteTier(),
): string | undefined {
  const path = cachedEmoteFiles.get(emoteCacheKey(id, provider, tier));
  return path ? convertFileSrc(path) : undefined;
}

/**
 * Queue an emote for disk caching at the size it will actually render. 7TV is
 * cached per-tier (key `${id}@${tier}`, URL at that tier) so the stored file
 * matches the on-screen size and disk-first render stays lossless; other
 * providers use their single canonical URL keyed by bare id. Call this only
 * once the emote has been shown (the bytes are already in the WebView), so the
 * cache write piggybacks on a download that already happened.
 */
export function queueEmoteForDisplayCaching(
  id: string,
  provider: string | undefined,
  url: string,
  tier: EmoteTier = inlineEmoteTier(),
  priority: boolean = false,
) {
  if (provider === '7tv') {
    queueEmoteForCaching(emoteCacheKey(id, '7tv', tier), sevenTvTierUrl(id, tier), priority);
  } else {
    queueEmoteForCaching(emoteCacheKey(id, provider), url, priority);
  }
}

/**
 * Proactively queue an ENTIRE channel emote set for disk caching at the size
 * each emote actually renders (per-DPI tier for 7TV, canonical URL otherwise).
 * This is what lets the emote menu render disk-first: by the time the menu is
 * opened, the polite background trickle has pulled the set to disk, so a fresh
 * mount (a later session, or a remount after the in-memory set is refreshed)
 * serves local files instead of re-hitting the provider CDNs on every open.
 *
 * Deliberately reuses the SAME single-serial, idle-scheduled queue as display
 * caching, so this only adds more items to drain over the watch session. It does
 * NOT change the download rate that keeps caching from competing with the live
 * video (see the queue header comment). Items already cached, pending, or queued
 * are skipped by `queueEmoteForCaching`, so calling this repeatedly is cheap.
 */
export function queueChannelEmotesForCaching(set: EmoteSet) {
  const all = [...set.twitch, ...set.bttv, ...set['7tv'], ...set.ffz, ...set.kick];
  for (const e of all) {
    queueEmoteForDisplayCaching(e.id, e.provider, e.url);
  }
}

async function ensureEmoteFileCache() {
  if (cachedEmoteFiles.size > 0) return;

  if (initializationPromise) {
    return initializationPromise;
  }

  initializationPromise = (async () => {
    try {
      Logger.debug('[EmoteService] Initializing emote file cache...');
      const files = await invoke('get_cached_files', { cacheType: 'emote' }) as Record<string, string>;
      Object.entries(files).forEach(([id, path]) => cachedEmoteFiles.set(id, path));
      Logger.debug(`[EmoteService] Emote file cache initialized with ${cachedEmoteFiles.size} entries`);
    } catch (e) {
      Logger.warn('[EmoteService] Failed to init emote file cache:', e);
    } finally {
      initializationPromise = null;
    }
  })();

  return initializationPromise;
}

/**
 * Re-pull the on-disk emote file map and merge it into memory. Unlike the
 * one-shot `ensureEmoteFileCache` (which no-ops once populated), this always
 * runs. Used after the AFK prefetch writes a batch of files so the picker's
 * live disk-first lookup (`getCachedEmoteUrl`) sees them THIS session; without
 * it the newly-cached files would only be picked up on the next launch.
 */
export async function refreshEmoteFileCache(): Promise<void> {
  try {
    const files = await invoke('get_cached_files', { cacheType: 'emote' }) as Record<string, string>;
    Object.entries(files).forEach(([id, path]) => cachedEmoteFiles.set(id, path));
    Logger.debug(`[EmoteService] Emote file cache refreshed (${cachedEmoteFiles.size} entries)`);
  } catch (e) {
    Logger.warn('[EmoteService] Failed to refresh emote file cache:', e);
  }
}

export function preloadChannelEmotes(emotes: Emote[]) {
  if (emotes.length === 0) return;

  // Warming the browser image cache for EVERY channel emote (5 to 10k) decoded
  // 100+ MB of bitmaps on stream entry, most of which never appear in chat.
  // Display-time caching (onLoad handlers in ChatMessage) already warms emotes
  // as they actually show up, so here we only pre-warm a bounded set: cached
  // (on-disk) emotes first since they cost no network, then a few remote ones,
  // up to the cap. The rest load lazily when first used.
  const PRELOAD_CAP = 200;
  const cached = emotes.filter(e => e.localUrl);
  const remote = emotes.filter(e => !e.localUrl);

  const urls = [
    ...cached.map(e => e.localUrl!),
    ...remote.map(e => e.url),
  ].slice(0, PRELOAD_CAP);

  Logger.debug(
    `[EmoteService] Browser preload: warming ${urls.length} of ${emotes.length} emotes (cap ${PRELOAD_CAP}; ${cached.length} cached, ${remote.length} remote)`,
  );

  urls.forEach(url => {
    const img = new Image();
    img.src = url;
  });
}

/**
 * Fetch all emotes for a channel using the high-performance Rust backend
 * This performs concurrent fetching from BTTV, 7TV, and FFZ with serde JSON parsing
 * Also fetches user-specific Twitch emotes (subscriptions, drops, etc.) if authenticated
 * 
 * IMPORTANT: This is "content-first" - we return emotes with CDN URLs immediately.
 * Local cached URLs are only used if they're already in memory (non-blocking).
 * Background caching happens when emotes are displayed via onLoad handlers.
 */
export async function fetchAllEmotes(channelName?: string, channelId?: string): Promise<EmoteSet> {
  // AWAIT cache initialization to ensure cached files are found
  // This populates cachedEmoteFiles so local URLs can be used
  await ensureEmoteFileCache();

  Logger.debug('[EmoteService] Fetching emotes via Rust backend for channel:', channelName, 'ID:', channelId, 'Cached files:', cachedEmoteFiles.size);

  try {
    // Try to get the auth token for user-specific Twitch emotes
    let accessToken: string | null = null;
    try {
      accessToken = await invoke<string>('get_twitch_token');
      Logger.debug('[EmoteService] Auth token available, will fetch user-specific Twitch emotes');
    } catch {
      Logger.debug('[EmoteService] No auth token available, Twitch emotes will be limited to globals');
    }

    // Call the Rust backend which does concurrent fetching with tokio::join!
    const emoteSet = await invoke<EmoteSet>('fetch_channel_emotes', {
      channelName: channelName || null,
      channelId: channelId || null,
      accessToken,
    });

    // Enhance with local URLs ONLY if they're already cached (non-blocking lookup)
    // The browser will load from CDN if localUrl is undefined
    const enhanceWithLocalUrls = enhanceRustEmotes;

    const enhancedSet: EmoteSet = {
      twitch: enhanceWithLocalUrls(emoteSet.twitch),
      bttv: enhanceWithLocalUrls(emoteSet.bttv),
      '7tv': enhanceWithLocalUrls(emoteSet['7tv']),
      ffz: enhanceWithLocalUrls(emoteSet.ffz),
      kick: enhanceWithLocalUrls(emoteSet.kick ?? []),
      // Learned from chat, not fetched — merged in by the picker at render time.
      youtube: [],
      seven_tv_ok: emoteSet.seven_tv_ok ?? true,
    };

    // Count how many emotes got local URLs
    const countLocalUrls = (emotes: Emote[]) => emotes.filter(e => e.localUrl).length;
    const localUrlCounts = {
      twitch: countLocalUrls(enhancedSet.twitch),
      bttv: countLocalUrls(enhancedSet.bttv),
      '7tv': countLocalUrls(enhancedSet['7tv']),
      ffz: countLocalUrls(enhancedSet.ffz),
    };

    Logger.debug('[EmoteService] Fetched emotes from Rust:', {
      twitch: enhancedSet.twitch.length,
      bttv: enhancedSet.bttv.length,
      '7tv': enhancedSet['7tv'].length,
      ffz: enhancedSet.ffz.length,
      cachedFilesInMemory: cachedEmoteFiles.size,
      localUrlsAssigned: localUrlCounts,
    });

    return enhancedSet;
  } catch (error) {
    Logger.error('[EmoteService] Failed to fetch emotes from Rust backend:', error);
    // Return empty set on error
    return {
      twitch: [],
      bttv: [],
      '7tv': [],
      ffz: [],
      kick: [],
      youtube: [],
    };
  }
}

/**
 * Fetch a KICK channel's 7TV emotes (channel set + globals) for the emote picker.
 * Kick has no BTTV/FFZ/native-third-party path, so this fills the 7tv slot only;
 * the same local-URL enhancement as Twitch applies so cached art renders disk-first.
 */
export async function fetchKickChannelEmotes(slug: string): Promise<EmoteSet> {
  await ensureEmoteFileCache();
  try {
    const emoteSet = await invoke<EmoteSet>('get_kick_channel_emotes', { slug });
    Logger.info(
      `[EmoteService] Kick emotes for "${slug}": ${emoteSet.kick?.length ?? 0} native, ${emoteSet['7tv']?.length ?? 0} 7TV`,
    );
    const enhance = (emotes: any[]) =>
      (emotes ?? []).map((emote) => {
        const localPath = cachedEmoteFiles.get(emoteCacheKey(emote.id, emote.provider));
        const zeroWidth = emote.is_zero_width !== undefined ? emote.is_zero_width : emote.isZeroWidth;
        return {
          ...emote,
          isZeroWidth: zeroWidth,
          modifierFlags: emote.modifier_flags ?? emote.modifierFlags,
          ffzSubOnly: emote.ffz_sub_only ?? emote.ffzSubOnly,
          localUrl: localPath ? convertFileSrc(localPath) : undefined,
        };
      });
    return {
      twitch: enhance(emoteSet.twitch),
      bttv: enhance(emoteSet.bttv),
      '7tv': enhance(emoteSet['7tv']),
      ffz: enhance(emoteSet.ffz),
      kick: enhance(emoteSet.kick),
      youtube: [],
    };
  } catch (error) {
    Logger.warn('[EmoteService] Failed to fetch Kick channel emotes:', error);
    return { twitch: [], bttv: [], '7tv': [], ffz: [], kick: [], youtube: [] };
  }
}

/**
 * A YouTube channel's 7TV emotes for the picker. Separate from the channel's
 * OWN emoji (seeded into providerEmoteStore from the chat page) — a channel can
 * have either, both, or neither, so the picker merges the two.
 */
export async function fetchYouTubeChannelEmotes(channel: string): Promise<EmoteSet> {
  await ensureEmoteFileCache();
  try {
    const emoteSet = await invoke<EmoteSet>('get_youtube_channel_emotes', { channel });
    Logger.info(
      `[EmoteService] YouTube 7TV emotes for "${channel}": ${emoteSet['7tv']?.length ?? 0}`,
    );
    const enhance = (emotes: any[]) =>
      (emotes ?? []).map((emote) => {
        const localPath = cachedEmoteFiles.get(emoteCacheKey(emote.id, emote.provider));
        const zeroWidth = emote.is_zero_width !== undefined ? emote.is_zero_width : emote.isZeroWidth;
        return {
          ...emote,
          isZeroWidth: zeroWidth,
          modifierFlags: emote.modifier_flags ?? emote.modifierFlags,
          ffzSubOnly: emote.ffz_sub_only ?? emote.ffzSubOnly,
          localUrl: localPath ? convertFileSrc(localPath) : undefined,
        };
      });
    return {
      twitch: [],
      bttv: [],
      '7tv': enhance(emoteSet['7tv']),
      ffz: [],
      kick: [],
      youtube: [],
    };
  } catch (error) {
    Logger.warn('[EmoteService] Failed to fetch YouTube channel emotes:', error);
    return { twitch: [], bttv: [], '7tv': [], ffz: [], kick: [], youtube: [] };
  }
}

/**
 * Get a specific emote by name from the Rust cache
 */
export async function getEmoteByName(channelId: string | null, emoteName: string): Promise<Emote | null> {
  try {
    const emote = await invoke<Emote | null>('get_emote_by_name', {
      channelId,
      emoteName,
    });
    
    if (emote) {
      // Enhance with local URL if available (tiered lookup for 7TV)
      const localPath = cachedEmoteFiles.get(emoteCacheKey(emote.id, emote.provider));
      const anyEmote = emote as any;
      const zeroWidth = anyEmote.is_zero_width !== undefined ? anyEmote.is_zero_width : emote.isZeroWidth;
      return {
        ...emote,
        isZeroWidth: zeroWidth,
        modifierFlags: anyEmote.modifier_flags ?? emote.modifierFlags,
        ffzSubOnly: anyEmote.ffz_sub_only ?? emote.ffzSubOnly,
        localUrl: localPath ? convertFileSrc(localPath) : undefined
      };
    }
    
    return null;
  } catch (error) {
    Logger.error('[EmoteService] Failed to get emote by name:', error);
    return null;
  }
}

/**
 * Clear the emote cache (both Rust and local file cache)
 */
export async function clearEmoteCache() {
  cachedEmoteFiles.clear();

  try {
    await invoke('clear_emote_cache');
    await invoke('clear_cache'); // Also clear disk cache
    Logger.debug('[EmoteService] Cleared all emote caches');
  } catch (e) {
    Logger.warn('[EmoteService] Failed to clear emote cache:', e);
  }
}

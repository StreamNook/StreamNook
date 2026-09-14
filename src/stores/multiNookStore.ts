import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { helixGet } from '../services/helix';
import { useAppStore } from './AppStore';
import { MultiNookSlot, MultiNookPresetChannel } from '../types';
import type { ProviderId } from '../types/providers';
import { makeKey, parseKey } from '../utils/providerKey';
import { canGridProvider, gridRefusal } from '../types/providers';
import { buildProviderUrl } from '../utils/streamProvider';
import { Logger } from '../utils/logger';

/** Slot ids whose proxy start is currently in flight. The loader effect re-fires
 *  on every slots change, so this guards against re-invoking start_multi_nook for
 *  a tile that's already starting (which would otherwise re-attempt a slow/offline
 *  stream on each sibling that resolves). */
const inFlightStarts = new Set<string>();

/** Disambiguates slot ids minted within the same millisecond. */
let slotIdSeq = 0;

/** A slot's identity. Provider-qualified, because the same name on two
 *  platforms is two different channels, and because case folding a bare login
 *  would destroy a case-sensitive id (utils/providerKey.ts documents which
 *  providers those are). Absent provider means Twitch, so every grid saved
 *  before providers existed keys exactly as it always did. */
function slotKey(slot: Pick<MultiNookSlot, 'provider' | 'channelLogin'>): string {
  return makeKey(slot.provider ?? 'twitch', slot.channelLogin);
}

/** Same, for a (channel, provider) pair that is not a slot yet. */
function channelKey(channelLogin: string, provider: ProviderId = 'twitch'): string {
  return makeKey(provider, channelLogin);
}

/** Put an incoming chat-selection id into the same key space slotKey emits, so
 *  a caller that passes a bare Twitch login still resolves. parseKey reads a
 *  bare key as Twitch, and a numeric channel id has no provider prefix either,
 *  so it normalizes to a key no slot owns and stays correctly unmatched. */
function asSlotKey(id: string): string {
  const { provider, channel } = parseKey(id);
  return makeKey(provider, channel);
}

/** True if `activeId` still matches one of the current slots. Compared on the
 *  composite slot key, so a Kick tile and a Twitch tile that share a login are
 *  never mistaken for each other. */
function isActiveChatValid(slots: MultiNookSlot[], activeId: string | null): boolean {
  if (!activeId) return false;
  const want = asSlotKey(activeId);
  return slots.some((s) => slotKey(s) === want);
}

/** Pick which chat to select: the focused (non-minimized) slot, else the first
 *  visible slot, else the first slot. Returns the slot's COMPOSITE KEY, not an
 *  id or a bare login: it is the only identifier that stays unambiguous across
 *  platforms, and it is available immediately (channelId resolves later, or not
 *  at all for a channel the metadata lookup cannot find). ChatWidget and the
 *  switcher match on the same key. null only when empty. */
function pickActiveChatChannel(slots: MultiNookSlot[]): string | null {
  if (slots.length === 0) return null;
  const visible = slots.filter((s) => !s.isMinimized);
  const pool = visible.length > 0 ? visible : slots;
  const choice = pool.find((s) => s.isFocused) ?? pool[0];
  return choice ? slotKey(choice) : null;
}

/** The slot the chat pane is currently showing, or null. Exported so App can
 *  route a non-Twitch active tile to the provider chat surface. */
export function activeChatSlot(
  slots: MultiNookSlot[],
  activeId: string | null,
): MultiNookSlot | null {
  if (!activeId) return null;
  const want = asSlotKey(activeId);
  return slots.find((s) => slotKey(s) === want) ?? null;
}

export const broadcastMultiNookPresence = (slots: MultiNookSlot[]) => {
  const allSlots = slots;
  
  if (allSlots.length === 0) return;

  // Determine majority game category
  const gameCounts: Record<string, number> = {};
  let maxGame = '';
  let maxCount = 0;
  
  for (const slot of allSlots) {
    if (slot.gameName && slot.gameName.trim() !== '') {
      gameCounts[slot.gameName] = (gameCounts[slot.gameName] || 0) + 1;
      if (gameCounts[slot.gameName] > maxCount) {
        maxCount = gameCounts[slot.gameName];
        maxGame = slot.gameName;
      }
    }
  }

  // Deterministic randomize phrasing based on channel names length
  const phraseHash = allSlots.reduce((acc, s) => acc + s.channelLogin.length, 0);
  
  const phrases = [
    `Watching ${allSlots.length} Streams`,
    `Multi-POV: ${allSlots.length} Streams`,
    `MultiNook \u2014 ${allSlots.length} Streams`
  ];
  const detailsPhrase = phrases[phraseHash % phrases.length];

  const details = allSlots.length === 1
    ? `Watching ${allSlots[0].channelName || allSlots[0].channelLogin}`
    : detailsPhrase;

  // Deterministic randomize separator
  const separators = [', ', ' \u00B7 ', ' | '];
  const separator = separators[(phraseHash + 1) % separators.length];

  let streamerNames = allSlots
    .map(s => s.channelName || s.channelLogin)
    .join(separator);
  
  if (streamerNames.length > 120) {
    streamerNames = streamerNames.substring(0, 110) + `... +${allSlots.length} more`; // Ensure it fits Discord's 128 char limit
  }

  const activityState = allSlots.length === 1
    ? 'MultiNook'
    : streamerNames;

  const presenceArgs = {
    details,
    activityState,
    largeImage: '', // Will be resolved by rust backend based on gameName
    smallImage: '',
    startTime: Date.now(),
    gameName: maxGame,
    streamUrl: 'https://streamnook.app',
  };

  // Discord (gated by settings toggle)
  const settings = useAppStore.getState().settings;
  if (settings?.discord_rpc_enabled) {
    invoke('update_discord_presence', presenceArgs).catch(() => {});
  }
};


interface MultiNookState {
  isMultiNookActive: boolean;
  isChatHidden: boolean;
  activeChatChannelId: string | null;
  /** Id of the preset the current grid was loaded from, or null. Drives the
   *  toolbar's "equipped preset" icon and the Stop action. Persisted with slots. */
  activePresetId: string | null;
  /** Id of the slot currently filling the whole grid area (solo-like), or null.
   *  Ephemeral view state: never persisted, always cleared on exit/teardown. The
   *  maximized tile is restyled in place (no remount) so its HLS player keeps
   *  running; the other tiles stay mounted but hidden behind it. */
  maximizedSlotId: string | null;
  /** True while the toolbar's mute-all is engaged. Overrides every tile's audio
   *  without touching per-slot muted state, so lifting it restores exactly the
   *  focus/mute mix that was playing before. Ephemeral: never persisted. */
  isAllMuted: boolean;
  slots: MultiNookSlot[];
  flyingAnimation: { x: number; y: number; id: number } | null;
  /** Composite key (makeKey) of the card playing the suck-up animation. */
  suckUpKey: string | null;
  recallAnimation: { sourceX: number; sourceY: number; targetX: number; targetY: number; id: number } | null;
  /** Composite key (makeKey) of the card playing the recall animation. */
  materializingKey: string | null;
  
  // Actions
  toggleMultiNook: () => void;
  triggerAddAnimation: (x: number, y: number, channelLogin: string, provider?: ProviderId) => void;
  triggerRecallAnimation: (channelLogin: string, cardX: number, cardY: number, provider?: ProviderId) => void;
  // `provider` is the platform the caller took this channel from. MultiNook
  // resolves every tile as a twitch.tv URL, so anything else is refused HERE
  // rather than at each call site: a missed guard does not fail, it silently
  // plays the same-named TWITCH channel instead (name collisions across
  // platforms are routine), which looks completely convincing.
  addSlot: (channelLogin: string, provider?: ProviderId) => Promise<void>;
  removeSlot: (id: string) => Promise<void>;
  removeSlotByLogin: (channelLogin: string, provider?: ProviderId) => Promise<void>;
  updateSlot: (id: string, updates: Partial<MultiNookSlot>) => void;
  changeSlotQuality: (id: string, quality: string) => Promise<void>;
  retrySlot: (id: string) => void;
  reorderSlots: (newSlots: MultiNookSlot[]) => void;
  toggleFocusSlot: (id: string) => void;
  /** Toggle a tile filling the whole grid area. Maximizing also focuses the tile
   *  (takes over audio + chat) so it behaves like the solo player. Passing the
   *  already-maximized id, or any id while it is maximized, restores the grid. */
  toggleMaximizeSlot: (id: string) => void;
  /** Directly set (or clear with null) the maximized tile. Used by Esc / teardown. */
  setMaximizedSlot: (id: string | null) => void;
  /** Toggle mute-all on/off. Per-slot muted flags are deliberately left alone. */
  toggleAllMuted: () => void;
  /** Directly set mute-all. Used by the tile-level break-out (user unmutes a tile by hand). */
  setAllMuted: (muted: boolean) => void;
  dockSlot: (id: string) => void;
  undockSlot: (id: string) => void;
  swapDockedSlot: (id: string) => void;
  setActiveChatChannelId: (id: string | null) => void;
  toggleChatHidden: () => void;
  batchLoadMissingStreams: () => Promise<void>;
  /** One batched Helix call covering every tile, refreshing the stream title and
   *  category. Both are ephemeral view data: this never writes to settings. */
  refreshSlotMetadata: () => Promise<void>;
  /** One-shot fill of partner/affiliate status for tiles that don't have it yet.
   *  Stable data helix/streams doesn't return, so it isn't part of the poll. */
  backfillBroadcasterTypes: () => Promise<void>;
  loadPresetChannels: (channels: MultiNookPresetChannel[], mode: 'replace' | 'append', presetId?: string) => Promise<void>;
  /** Tag the current grid with the preset it was loaded from (null = no equipped preset). Persisted. */
  setActivePresetId: (id: string | null) => Promise<void>;
  /** Stop and tear down every tile, leaving an empty grid (used by "Stop preset"). Stays in MultiNook. */
  clearAllSlots: () => Promise<void>;
  
  // Synchronization
  resyncAllSlots: () => void;
  
  // Persistence
  loadStoredSlots: () => void;
  saveSlots: () => Promise<void>;
}

export const usemultiNookStore = create<MultiNookState>((set, get) => ({
  isMultiNookActive: false,
  isChatHidden: false,
  activeChatChannelId: null,
  activePresetId: null,
  maximizedSlotId: null,
  isAllMuted: false,
  slots: [],
  flyingAnimation: null,
  suckUpKey: null,
  recallAnimation: null,
  materializingKey: null,

  batchLoadMissingStreams: async () => {
    const slots = get().slots;
    // Skip tiles already loaded, already flagged offline, or with a start in flight.
    const missing = slots.filter((s) => !s.streamUrl && !s.loadError && !inFlightStarts.has(s.id));
    if (missing.length === 0) return;

    missing.forEach((s) => inFlightStarts.add(s.id));

    // Start each proxy independently and apply its result the moment it lands, so a
    // single offline/unreachable stream can't hold up the rest of the grid. The
    // post-load buffer stall this used to cause was NOT a contention problem (no
    // stagger needed): the MultiNook relay wasn't rewriting Twitch's over-declared
    // playlist targetduration, so hls.js under-polled and the buffer drained. That
    // is fixed in the relay (multi_nook_server retarget_playlist), so tiles can
    // cold-start together again.
    await Promise.all(
      missing.map(async (slot) => {
        try {
          const slotProvider = slot.provider ?? 'twitch';
          const url = await invoke<string>('start_multi_nook', {
            streamId: slot.id,
            // buildProviderUrl knows each platform's watch-URL shape and encodes
            // the channel; the old hardcoded twitch.tv template is what made a
            // Kick tile resolve the same-named TWITCH channel instead.
            url: buildProviderUrl(slotProvider, slot.channelLogin),
            quality: slot.quality || 'best', // Per-tile quality (set via the focused tile's gear menu)
            provider: slotProvider,
          });
          set((state) => ({
            slots: state.slots.map((s) => (s.id === slot.id ? { ...s, streamUrl: url, loadError: false } : s)),
          }));
        } catch (err) {
          Logger.error(`Failed to start multi-nook proxy for ${slot.channelLogin}:`, err);
          // Flag the tile offline so it shows the friendly overlay and the loader
          // stops re-attempting it (retry is user-driven via retrySlot / resync).
          set((state) => ({
            slots: state.slots.map((s) => (s.id === slot.id ? { ...s, loadError: true } : s)),
          }));
        } finally {
          inFlightStarts.delete(slot.id);
        }
      }),
    );
  },

  refreshSlotMetadata: async () => {
    // Helix answers about TWITCH channels only. Handing it a Kick slug returns
    // whatever Twitch account owns that name, or nothing, and the "absent means
    // offline" rule below would then report every provider tile as permanently
    // offline. Provider tiles keep the metadata their own resolve gave them.
    const twitchSlots = get().slots.filter((s) => (s.provider ?? 'twitch') === 'twitch');
    const logins = Array.from(
      new Set(twitchSlots.map((s) => s.channelLogin.toLowerCase())),
    ).filter(Boolean);
    if (logins.length === 0) return;

    // Keyed on channelLogin, not channelId: the id is optional on a slot (a
    // preset carries whatever was cached when it was saved), so a broadcaster_id
    // batch would silently skip those tiles. The login is the required key.
    // No chunking needed — the grid is hard-capped at 25, well under Helix's 100.
    let byLogin: Map<string, { title?: string; game_name?: string }>;
    try {
      const qs = logins.map((l) => `user_login=${encodeURIComponent(l)}`).join('&');
      const data = await helixGet<{ data?: Array<{ user_login?: string; title?: string; game_name?: string }> }>('streams', qs);
      byLogin = new Map(
        (data.data || []).map((s: { user_login?: string; title?: string; game_name?: string }) => [
          (s.user_login || '').toLowerCase(),
          { title: s.title, game_name: s.game_name },
        ]),
      );
    } catch (e) {
      Logger.warn('[multiNookStore] Failed to refresh slot titles', e);
      return;
    }

    // Preserve object identity for every tile that didn't actually change: each
    // cell is memoized on its slot's reference, so spreading unconditionally
    // would re-render the whole grid on every poll. That includes the offline
    // branch — a tile with no title must come back as the *same* object.
    let changed = false;
    const next = get().slots.map((s) => {
      // Only Twitch tiles were asked about, so only they can be judged by the
      // answer.
      if ((s.provider ?? 'twitch') !== 'twitch') return s;
      const live = byLogin.get(s.channelLogin.toLowerCase());
      if (!live) {
        // Absent from the response means offline. Drop the title (a stale live
        // title on an offline tile is wrong) but keep the last known category.
        if (s.title === undefined) return s;
        changed = true;
        return { ...s, title: undefined };
      }
      const title = live.title || undefined;
      const gameName = live.game_name || s.gameName;
      if (s.title === title && s.gameName === gameName) return s;
      changed = true;
      return { ...s, title, gameName };
    });
    if (changed) {
      // Deliberately no saveSlots(): every field touched here is ephemeral.
      set({ slots: next });
    }

    await get().backfillBroadcasterTypes();
  },

  backfillBroadcasterTypes: async () => {
    // Partner/affiliate status is stable, and helix/streams doesn't carry it, so
    // this is a one-shot fill rather than part of the recurring poll: once every
    // tile has a value the guard below makes it a no-op. Covers the paths that
    // never see a helix/users response — chiefly loadPresetChannels, which
    // deliberately opens a preset with no per-channel round-trip at all.
    const missing = Array.from(
      new Set(
        get()
          .slots.filter(
            (s) => s.broadcasterType === undefined && (s.provider ?? 'twitch') === 'twitch',
          )
          .map((s) => s.channelLogin.toLowerCase()),
      ),
    ).filter(Boolean);
    if (missing.length === 0) return;

    let byLogin: Map<string, string>;
    try {
      const qs = missing.map((l) => `login=${encodeURIComponent(l)}`).join('&');
      const data = await helixGet<{ data?: Array<{ login?: string; broadcaster_type?: string }> }>('users', qs);
      byLogin = new Map(
        (data.data || []).map((u: { login?: string; broadcaster_type?: string }) => [
          (u.login || '').toLowerCase(),
          u.broadcaster_type || '',
        ]),
      );
    } catch (e) {
      Logger.warn('[multiNookStore] Failed to backfill broadcaster types', e);
      return;
    }

    // Same identity discipline as above: only the tiles that actually resolved
    // get a new object, so this never re-renders the whole grid.
    let changed = false;
    const next = get().slots.map((s) => {
      if (s.broadcasterType !== undefined) return s;
      if ((s.provider ?? 'twitch') !== 'twitch') return s;
      // A login Helix did not answer for (renamed, banned, or simply not a
      // Twitch channel) must still be marked resolved, or it stays `undefined`
      // and this batch re-requests it on EVERY 120s poll for the life of the
      // session. Empty string is the honest answer: asked, no badge.
      const type = byLogin.get(s.channelLogin.toLowerCase()) ?? '';
      changed = true;
      return { ...s, broadcasterType: type };
    });
    if (!changed) return;
    set({ slots: next });
  },

  loadPresetChannels: async (channels, mode, presetId) => {
    // Drop duplicates inside the preset itself, preserving order. Keyed by
    // provider+channel for the same reason slotKey is.
    const seen = new Set<string>();
    const unique = channels.filter((ch) => {
      if (!ch.channelLogin) return false;
      const key = channelKey(ch.channelLogin, ch.provider ?? 'twitch');
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });

    // The gate again, because replace mode below builds slots inline and never
    // reaches addSlot. A preset saved when a platform was allowed, or hand-edited,
    // must not be a way around the refusal.
    const eligible = unique.filter((ch) => canGridProvider(ch.provider ?? 'twitch'));
    const dropped = unique.length - eligible.length;
    if (dropped > 0) {
      useAppStore.getState().addToast(
        dropped === 1
          ? 'One channel in this preset cannot run in the grid and was skipped'
          : `${dropped} channels in this preset cannot run in the grid and were skipped`,
        'info',
      );
    }

    if (mode === 'append') {
      // Reuse the single-add path so dedup-against-grid, the 25 cap, proxy start,
      // and fresh Twitch metadata enrichment all behave exactly like a manual add.
      for (const ch of eligible) {
        await get().addSlot(ch.channelLogin, ch.provider ?? 'twitch');
      }
      return;
    }

    // --- replace mode ---
    // Tear down every live proxy/HLS for the outgoing grid BEFORE swapping slots so
    // nothing is left running (no orphaned proxies = no leak from loading presets).
    const outgoing = get().slots;
    try {
      await invoke('stop_all_multi_nooks');
    } catch (e) {
      Logger.error('[MultiNook] Failed to stop proxies before loading preset', e);
    }
    for (const slot of outgoing) {
      if (slot.channelId) {
        invoke('unregister_active_channel', { channelId: slot.channelId }).catch(() => {});
      }
    }

    const MAX_SLOTS = 25;
    const capped = eligible.slice(0, MAX_SLOTS);
    if (eligible.length > MAX_SLOTS) {
      useAppStore.getState().addToast(
        `Preset has ${eligible.length} channels; loaded the first ${MAX_SLOTS}`,
        'info',
      );
    }

    // Build fresh slots from the preset's cached metadata, with no per-channel Twitch
    // round-trip, so a preset opens instantly. First tile is focused/unmuted; the
    // rest start muted, matching how a normal grid fills in.
    const base = Date.now();
    const newSlots: MultiNookSlot[] = capped.map((ch, i) => ({
      id: `cell-${base}-${i}`,
      provider: ch.provider === 'twitch' ? undefined : ch.provider,
      channelLogin: ch.channelLogin,
      channelId: ch.channelId || undefined,
      channelName: ch.channelName || ch.channelLogin,
      profileImageUrl: ch.profileImageUrl || undefined,
      quality: ch.quality || undefined,
      volume: 0.5,
      muted: i > 0,
      isFocused: i === 0,
    }));

    // streamUrl is intentionally left undefined: MultiNookView's missing-stream
    // loader picks these up and concurrently starts the proxies on the next frame.
    // Tag the grid with the preset it came from (replace mode = the grid IS this preset).
    set({ slots: newSlots, activeChatChannelId: pickActiveChatChannel(newSlots), activePresetId: presetId ?? null, maximizedSlotId: null });

    for (const slot of newSlots) {
      if (slot.channelId) {
        invoke('register_active_channel', { channelId: slot.channelId }).catch(() => {});
      }
    }

    if (newSlots.length > 0) {
      broadcastMultiNookPresence(newSlots);
    }
    await get().saveSlots();
  },

  setActivePresetId: async (id: string | null) => {
    if (get().activePresetId === id) return;
    set({ activePresetId: id });
    await get().saveSlots();
  },

  clearAllSlots: async () => {
    // Stop every live proxy/HLS and drop the grid, but stay in MultiNook (empty
    // grid). Used by "Stop preset": closes out everything the preset opened
    // without deleting the preset, and clears the equipped-preset tag.
    const current = get().slots;
    try {
      await invoke('stop_all_multi_nooks');
    } catch (e) {
      Logger.error('[MultiNook] Failed to stop proxies on clearAllSlots', e);
    }
    for (const slot of current) {
      if (slot.channelId) {
        invoke('unregister_active_channel', { channelId: slot.channelId }).catch(() => {});
      }
    }
    set({ slots: [], activeChatChannelId: null, activePresetId: null, maximizedSlotId: null, isAllMuted: false });

    // Revert presence to idle since nothing is playing.
    const settings = useAppStore.getState().settings;
    if (settings?.discord_rpc_enabled) {
      invoke('set_idle_discord_presence').catch(() => {});
    }
    await get().saveSlots();
  },

  triggerAddAnimation: (x: number, y: number, channelLogin: string, provider: ProviderId = 'twitch') => {
    const id = Date.now();
    // Start suck-up immediately, delay flying dot until card dissolve finishes (350ms)
    const key = channelKey(channelLogin, provider);
    set({ suckUpKey: key });
    // Spawn flying dot after suck-up animation completes
    setTimeout(() => {
      set({ flyingAnimation: { x, y, id } });
    }, 350);
    // Clear suckUpLogin after suck-up animation finishes so card transitions to ghost
    setTimeout(() => {
      if (get().suckUpKey === key) {
        set({ suckUpKey: null });
      }
    }, 400);
    // Clear flying animation after it completes so it doesn't replay on component remounts
    setTimeout(() => {
      if (get().flyingAnimation?.id === id) {
        set({ flyingAnimation: null });
      }
    }, 1400);
  },

  triggerRecallAnimation: (channelLogin: string, cardX: number, cardY: number, provider: ProviderId = 'twitch') => {
    const id = Date.now();
    const key = channelKey(channelLogin, provider);
    
    // Get the MultiNook badge position as the flying dot source
    const badgeBtn = document.getElementById('multinook-return-button');
    const badgeRect = badgeBtn?.getBoundingClientRect();
    const sourceX = badgeRect ? badgeRect.right - 10 : window.innerWidth / 2;
    const sourceY = badgeRect ? badgeRect.top - 5 : 0;
    
    // Set materializing FIRST — card will render content but CSS animation-delay holds it invisible
    set({ materializingKey: key });
    
    // Then remove the slot — card is no longer "queued" but materializingLogin keeps it in animation mode
    get().removeSlotByLogin(channelLogin, provider);
    
    // Spawn reverse flying dot from badge → card position
    set({ recallAnimation: { sourceX, sourceY, targetX: cardX, targetY: cardY, id } });
    
    // Clean up flying dot after it arrives
    setTimeout(() => {
      if (get().recallAnimation?.id === id) {
        set({ recallAnimation: null });
      }
    }, 550);
    
    // Clear materializing after animation-delay (550ms) + animation duration (350ms) completes
    setTimeout(() => {
      if (get().materializingKey === key) {
        set({ materializingKey: null });
      }
    }, 950);
  },

  resyncAllSlots: () => {
    Logger.info("[MultiNook] Forcing concurrent resynchronization of all streams");
    set(state => ({
      slots: state.slots.map(slot => ({
        ...slot,
        streamUrl: undefined, // Clearing streamUrl forces MultiNookCell to natively remount and concurrently invoke start_multi_nook
        loadError: false,     // Give previously-offline tiles another chance
      }))
    }));
  },

  retrySlot: (id: string) => {
    // Clear the offline flag and URL so the loader effect re-attempts this proxy.
    set((state) => ({
      slots: state.slots.map((s) => (s.id === id ? { ...s, streamUrl: undefined, loadError: false } : s)),
    }));
  },

  removeSlotByLogin: async (channelLogin: string, provider: ProviderId = 'twitch') => {
    const key = channelKey(channelLogin, provider);
    const slot = get().slots.find((s) => slotKey(s) === key);
    if (slot) {
      await get().removeSlot(slot.id);
    }
  },

  toggleMultiNook: async () => {
    const currentState = get().isMultiNookActive;
    const newState = !currentState;
    
    if (newState) {
      // Entering multi-nook mode
      if (get().slots.length === 0) {
        get().loadStoredSlots();
      }

      // Always have a chat selected on entry so messages start loading right
      // away (loadStoredSlots seeds it; this covers slots already in memory).
      const slotsNow = get().slots;
      if (!isActiveChatValid(slotsNow, get().activeChatChannelId)) {
        set({ activeChatChannelId: pickActiveChatChannel(slotsNow) });
      }

      // Ensure Home view is hidden
      if (useAppStore.getState().isHomeActive) {
        useAppStore.getState().toggleHome();
      }
      
      // Broadcast restored slots after a short delay
      setTimeout(() => {
        const currentSlots = get().slots;
        if (currentSlots.length > 0) {
          broadcastMultiNookPresence(currentSlots);
          for (const slot of currentSlots) {
            if (slot.channelId) {
               invoke('register_active_channel', { channelId: slot.channelId }).catch(() => {});
            }
          }
        }
      }, 500);
    } else {
      // Exiting multi-nook mode
      try {
        await invoke('stop_all_multi_nooks');
      } catch (e) {
        Logger.error('Failed to stop multi-nook proxies', e);
      }
      
      const currentSlots = get().slots;
      for (const slot of currentSlots) {
        if (slot.channelId) {
           invoke('unregister_active_channel', { channelId: slot.channelId }).catch(() => {});
        }
      }

      set({ activeChatChannelId: null, slots: [], maximizedSlotId: null, isAllMuted: false }); // Maintain chat hidden state

      // Restore Home view if no single stream is playing
      if (!useAppStore.getState().streamUrl) {
        useAppStore.setState({ isHomeActive: true });
      }
      
      // Revert to idle presence
      const settings = useAppStore.getState().settings;
      if (settings?.discord_rpc_enabled) {
        invoke('set_idle_discord_presence').catch(() => {});
      }
    }

    set({ isMultiNookActive: newState });
  },

  addSlot: async (channelLogin: string, provider: ProviderId = 'twitch') => {
    // Backstop for every entry point (picker, quick-add, right-click, player
    // overlay, presets). See the interface comment: an ungated non-Twitch add
    // resolves to a different person's Twitch stream, wearing this channel's
    // name and avatar.
    const refusal = gridRefusal(provider);
    if (refusal) {
      useAppStore.getState().addToast(refusal, 'info');
      return;
    }

    if (get().slots.length >= 25) {
      useAppStore.getState().addToast('Maximum of 25 streams reached', 'warning');
      return;
    }
    
    const addKey = channelKey(channelLogin, provider);
    if (get().slots.some((s) => slotKey(s) === addKey)) {
      useAppStore.getState().addToast(`${channelLogin} is already in the view`, 'info');
      return;
    }

    let resolvedId = '';
    let resolvedName = '';
    let resolvedImage = '';
    let resolvedGameName = '';
    let resolvedTitle = '';
    let resolvedBroadcasterType = '';
    // Twitch identity for a Twitch channel ONLY. Asking helix/users about a Kick
    // slug returns the TWITCH account of that name, which is how a provider tile
    // ended up wearing a stranger's avatar and display name. A provider tile gets
    // its identity from its own adapter instead (see below).
    if (provider !== 'twitch') {
      // The platform's own adapter answers for its own channels. `channel_meta`
      // is part of the StreamSource trait every provider implements.
      try {
        const meta = await invoke<{
          user_id?: string;
          user_name?: string;
          profile_image_url?: string;
          game_name?: string;
          title?: string;
        }>('provider_channel_meta', { provider, channel: channelLogin });
        resolvedId = meta?.user_id ?? '';
        resolvedName = meta?.user_name ?? '';
        resolvedImage = meta?.profile_image_url ?? '';
        resolvedGameName = meta?.game_name ?? '';
        resolvedTitle = meta?.title ?? '';
      } catch (e) {
        // A tile with no metadata still plays; it just shows its login until the
        // next poll. Falling back to Twitch here is what must never happen.
        Logger.warn(`[multiNookStore] ${provider} channel meta failed for`, channelLogin, e);
      }
    } else {
    try {
      const data = await helixGet<{ data?: Array<{ id: string; display_name: string; profile_image_url: string; broadcaster_type: string }> }>('users', `login=${encodeURIComponent(channelLogin)}`).catch(() => null);
      if (data) {
        if (data.data && data.data.length > 0) {
          resolvedId = data.data[0].id;
          resolvedName = data.data[0].display_name;
          resolvedImage = data.data[0].profile_image_url;
          resolvedBroadcasterType = data.data[0].broadcaster_type;

          // Fetch channel info to get the current category and stream title
          try {
            const channelData = await helixGet<{ data?: Array<{ game_name: string; title: string }> }>('channels', `broadcaster_id=${encodeURIComponent(resolvedId)}`);
            {
              if (channelData.data && channelData.data.length > 0) {
                resolvedGameName = channelData.data[0].game_name;
                resolvedTitle = channelData.data[0].title;
              }
            }
          } catch (e) {
            Logger.warn('[multiNookStore] Failed to fetch channel info for game name', e);
          }
        }
      }
    } catch (e) {
      Logger.warn('[multiNookStore] Failed to resolve channel details for', channelLogin, e);
    }
    }

    // Capture latest state AFTER async operations to prevent race conditions from concurrent adds
    const { slots, saveSlots } = get();
    
    // Double check it wasn't added concurrently while we were fetching
    if (slots.some((s) => slotKey(s) === addKey)) {
      return;
    }

    const newSlot: MultiNookSlot = {
      // Date.now() alone collides when two adds race through their awaited
      // fetches, and this id is the React key AND the start/stop_multi_nook
      // handle. loadPresetChannels already disambiguates the same way.
      id: `cell-${Date.now()}-${slotIdSeq++}`,
      provider: provider === 'twitch' ? undefined : provider,
      channelLogin,
      channelId: resolvedId || undefined,
      channelName: resolvedName || channelLogin,
      profileImageUrl: resolvedImage || undefined,
      gameName: resolvedGameName || undefined,
      title: resolvedTitle || undefined,
      broadcasterType: resolvedBroadcasterType || undefined,
      volume: 0.5,
      muted: slots.length > 0, // Auto-mute if it's not the first one
      isFocused: slots.length === 0, // First slot is focused by default
    };

    const newSlots = [...slots, newSlot];
    set({ slots: newSlots });

    // Keep a chat selected: if nothing valid is selected yet (first slot, or the
    // previous selection is gone), focus the slot we just added.
    if (!isActiveChatValid(newSlots, get().activeChatChannelId)) {
       set({ activeChatChannelId: slotKey(newSlot) });
    }
    
    if (newSlot.channelId) {
       invoke('register_active_channel', { channelId: newSlot.channelId }).catch(() => {});
    }
    // The mod view (EventSub channel.moderate) follows the chat connection now,
    // wired in the Rust IRC service, so opening a tile's chat subscribes it
    // automatically. No per-slot call needed here.

    broadcastMultiNookPresence(newSlots);
    await saveSlots();
  },

  removeSlot: async (id: string) => {
    const { slots, saveSlots } = get();
    const slotToRemove = slots.find(s => s.id === id);
    
    if (slotToRemove?.streamUrl) {
      try {
        await invoke('stop_multi_nook', { streamId: id });
      } catch (e) {
        Logger.error(`Failed to stop proxy for slot ${id}`, e);
      }
    }
    
    if (slotToRemove?.channelId) {
       invoke('unregister_active_channel', { channelId: slotToRemove.channelId }).catch(() => {});
    }

    const newSlots = slots.filter(s => s.id !== id);
    
    // If we removed the focused slot, all visible slots should unmute since there's no longer a focused slot
    if (slotToRemove?.isFocused && newSlots.length > 0) {
      newSlots.forEach(s => {
        if (!s.isMinimized) {
          s.muted = false;
        }
      });
    }
    
    // Removing the maximized tile drops back to the grid.
    if (get().maximizedSlotId === id) {
      set({ maximizedSlotId: null });
    }

    // Removing the last tile also un-equips the preset (the grid is now empty).
    set(newSlots.length === 0 ? { slots: newSlots, activePresetId: null } : { slots: newSlots });

    // Always keep a valid selection: if the active chat is now gone (or was
    // never set), fall back to the focused/first remaining slot so chat keeps
    // loading. Resolves to null only when no slots remain.
    if (!isActiveChatValid(newSlots, get().activeChatChannelId)) {
      set({ activeChatChannelId: pickActiveChatChannel(newSlots) });
    }
    
    if (newSlots.length > 0) {
      broadcastMultiNookPresence(newSlots);
    } else {
      // Revert to idle presence since there are no streams
      const settings = useAppStore.getState().settings;
      if (settings?.discord_rpc_enabled) {
        invoke('set_idle_discord_presence').catch(() => {});
      }
    }

    await saveSlots();
  },

  updateSlot: (id: string, updates: Partial<MultiNookSlot>) => {
    const { slots, saveSlots } = get();
    // Only the targeted slot gets a new object, and only when a value really
    // changed. Every tile is memoized on its slot's identity, so spreading
    // unconditionally re-rendered the whole grid — worst case being the Plyr
    // volumechange handler, which calls this continuously during a slider drag.
    let changed = false;
    const newSlots = slots.map(s => {
      if (s.id !== id) return s;
      const keys = Object.keys(updates) as (keyof MultiNookSlot)[];
      if (keys.every(k => s[k] === updates[k])) return s;
      changed = true;
      return { ...s, ...updates };
    });
    if (!changed) return;
    set({ slots: newSlots });
    
    // Only save to settings if it's a persistent config change (not streamUrl)
    if ('volume' in updates || 'muted' in updates || 'isFocused' in updates || 'channelLogin' in updates || 'isMinimized' in updates || 'profileImageUrl' in updates) {
      saveSlots();
    }
  },

  changeSlotQuality: async (id: string, quality: string) => {
    const slot = get().slots.find(s => s.id === id);
    if (!slot || slot.quality === quality) return;

    // Stop the current proxy first so the restart picks up the new quality
    // cleanly (the proxy is keyed by streamId, so a fresh start replaces it).
    if (slot.streamUrl) {
      try {
        await invoke('stop_multi_nook', { streamId: id });
      } catch (e) {
        Logger.warn(`[MultiNook] Failed to stop proxy before quality change for ${id}`, e);
      }
    }

    // Persist the new quality and clear the URL. MultiNookView's missing-stream
    // loader re-invokes start_multi_nook at slot.quality and the cell remounts
    // on the new URL — same path resyncAllSlots uses.
    set(state => ({
      slots: state.slots.map(s => (s.id === id ? { ...s, quality, streamUrl: undefined, loadError: false } : s)),
    }));
    await get().saveSlots();
  },

  reorderSlots: (newSlots: MultiNookSlot[]) => {
    set({ slots: newSlots });
    get().saveSlots();
  },

  toggleFocusSlot: (id: string) => {
    const { slots, saveSlots } = get();
    const slot = slots.find(s => s.id === id);
    if (!slot) return;
    
    const isCurrentlyFocused = slot.isFocused;
    
    // Slots whose focus/mute state is already correct keep their identity, so
    // the memoized tiles that didn't actually change don't re-render.
    const newSlots = slots.map(s => {
      // Toggling focus off clears focus from all and unmutes all non-docked;
      // focusing THIS slot focuses + unmutes it and mutes everyone else.
      const isFocused = isCurrentlyFocused ? false : s.id === id;
      const muted = isCurrentlyFocused ? (s.isMinimized ? true : false) : s.id !== id;
      if (s.isFocused === isFocused && s.muted === muted) return s;
      return { ...s, isFocused, muted };
    });
    
    set({ slots: newSlots });
    saveSlots();
    
    // Jump chat focus to this slot if we are focusing it
    if (!isCurrentlyFocused) {
       set({ activeChatChannelId: slotKey(slot) });
    }
  },

  toggleMaximizeSlot: (id: string) => {
    const { slots, maximizedSlotId, saveSlots } = get();
    const slot = slots.find(s => s.id === id);
    // Only visible tiles can be maximized (docked tiles aren't on the grid).
    if (!slot || slot.isMinimized) return;

    // Already filling the space (this tile or, defensively, any tile) → restore grid.
    if (maximizedSlotId) {
      set({ maximizedSlotId: null });
      return;
    }

    // Maximize this tile AND focus it: unmute it, mute everyone else, so it acts
    // exactly like the solo player. Mirrors toggleFocusSlot's "focus this" branch.
    const newSlots = slots.map(s => {
      const isFocused = s.id === id;
      const muted = s.id !== id;
      if (s.isFocused === isFocused && s.muted === muted) return s;
      return { ...s, isFocused, muted };
    });
    set({ maximizedSlotId: id, slots: newSlots });
    saveSlots();

    // Move chat to the maximized stream so chat matches what you're watching.
    set({ activeChatChannelId: slotKey(slot) });
  },

  setMaximizedSlot: (id: string | null) => {
    set({ maximizedSlotId: id });
  },

  toggleAllMuted: () => {
    set((state) => ({ isAllMuted: !state.isAllMuted }));
  },

  setAllMuted: (muted: boolean) => {
    set({ isAllMuted: muted });
  },

  dockSlot: (id: string) => {
    const { slots, saveSlots } = get();
    const slot = slots.find(s => s.id === id);
    if (!slot || slot.isMinimized) return;

    // Docking acts similarly to muting & minimizing
    const wasFocused = slot.isFocused;
    
    const newSlots = slots.map(s => {
      if (s.id === id) {
        return { ...s, isMinimized: true, muted: true, isFocused: false };
      }
      // If we are docking the focused stream, it loses focus, so we unmute the rest of visible streams
      if (wasFocused && !s.isMinimized) {
        return { ...s, muted: false };
      }
      return s;
    });

    // Docking the maximized tile takes it off the grid → restore the grid view.
    set(get().maximizedSlotId === id ? { slots: newSlots, maximizedSlotId: null } : { slots: newSlots });
    saveSlots();
  },

  undockSlot: (id: string) => {
    const { slots, saveSlots } = get();
    const slot = slots.find(s => s.id === id);
    if (!slot || !slot.isMinimized) return;
    
    const hasFocusedStream = slots.some(s => s.isFocused);
    
    const newSlots = slots.map(s => {
      if (s.id === id) {
        return { 
          ...s, 
          isMinimized: false,
          // Unmute if there is NO focused stream. If someone HAS focus, stay muted.
          muted: hasFocusedStream ? true : false 
        };
      }
      return s;
    });
    
    set({ slots: newSlots });
    saveSlots();
  },

  swapDockedSlot: (id: string) => {
    const { slots, saveSlots } = get();
    const slotToRestore = slots.find(s => s.id === id);
    if (!slotToRestore || !slotToRestore.isMinimized) return;

    const visibleSlots = slots.filter(s => !s.isMinimized);
    if (visibleSlots.length === 0) {
      get().undockSlot(id);
      return;
    }
    
    // Choose target to dock: prefer the focused slot, otherwise the first visible one
    const slotToDock = visibleSlots.find(s => s.isFocused) || visibleSlots[0];
    
    const newSlots = slots.map(s => {
      if (s.id === id) {
        // Restore and focus
        return { ...s, isMinimized: false, isFocused: true, muted: false };
      }
      if (s.id === slotToDock.id) {
        // Dock the old one. If it was the maximized tile, the grid restore is
        // handled below by clearing maximizedSlotId.
        return { ...s, isMinimized: true, isFocused: false, muted: true };
      }
      // If we are swapping, we assume 1-stream viewing mode, so mute all others
      return { ...s, isFocused: false, muted: true };
    });

    // A swap reshuffles which tile is the active one, so drop any fill-the-space
    // overlay back to the grid (the swapped-in tile is freshly restored/focused).
    set(get().maximizedSlotId ? { slots: newSlots, maximizedSlotId: null } : { slots: newSlots });
    saveSlots();

    set({ activeChatChannelId: slotKey(slotToRestore) });
  },

  setActiveChatChannelId: (id: string | null) => {
    set({ activeChatChannelId: id });
  },

  toggleChatHidden: async () => {
    const currentState = get().isChatHidden;
    const newState = !currentState;
    set({ isChatHidden: newState });
    
    const appStore = useAppStore.getState();
    const currentSettings = appStore.settings;
    if (currentSettings) {
      const newSettings = {
        ...currentSettings,
        multi_nook_chat_hidden: newState,
      };
      try {
        await invoke('save_settings', { settings: newSettings });
        useAppStore.setState({ settings: newSettings });
      } catch (e) {
        Logger.error('Failed to save multi_nook_chat_hidden state', e);
      }
    }
  },

  loadStoredSlots: () => {
    const appSettings = useAppStore.getState().settings;
    if (appSettings) {
      if (appSettings.multi_nook_slots && Array.isArray(appSettings.multi_nook_slots)) {
        // Clean up old minimized state on load - anything that was explicitly minimized
        const cleanedSlots = appSettings.multi_nook_slots
          // The gate applies to what comes BACK off disk too. A grid saved while
          // a platform was allowed, or edited by hand, must not restore a tile
          // the app can no longer run: this path builds slots directly and never
          // reaches addSlot.
          .filter((s) => canGridProvider(s.provider ?? 'twitch'))
          .map(s => {
            const cleaned = { ...s };
            delete cleaned.streamUrl;
            delete cleaned.loadError;
            delete cleaned.title;
            return cleaned as MultiNookSlot;
          });
        // Seed the chat selection so a restored grid opens with chat loading,
        // not blank. Restore the equipped-preset tag so the toolbar icon matches.
        set({
          slots: cleanedSlots,
          activeChatChannelId: pickActiveChatChannel(cleanedSlots),
          activePresetId: appSettings.multi_nook_active_preset_id ?? null,
        });
        // Restored avatars are whatever was persisted: they can be stale (Twitch
        // CDN URLs expire / the streamer changed their pic) or were never captured
        // (a network hiccup on the original add), so the offline overlay shows no
        // picture. Refresh them from Helix in the background (one batched call,
        // up to 100 logins) and persist the fresh values.
        void (async () => {
          const logins = Array.from(
            new Set(
              cleanedSlots
                .filter((s) => (s.provider ?? 'twitch') === 'twitch')
                .map((s) => s.channelLogin.toLowerCase()),
            ),
          ).filter(Boolean);
          if (logins.length === 0) return;
          try {
            const qs = logins.slice(0, 100).map((l) => `login=${encodeURIComponent(l)}`).join('&');
            const data = await helixGet<{ data?: Array<{ login?: string; id: string; display_name: string; profile_image_url: string }> }>('users', qs);
            const byLogin = new Map<string, { id: string; display_name: string; profile_image_url: string }>();
            for (const u of data.data || []) byLogin.set((u.login || '').toLowerCase(), u);
            let changed = false;
            const next = get().slots.map((s) => {
              // Guard the APPLY as well as the request. The lookup above asks
              // Helix about Twitch logins only, but this loop runs over EVERY
              // slot, so a Kick tile whose slug matches a Twitch account would
              // take that stranger's id, display name and avatar, and the
              // saveSlots below would persist it.
              if ((s.provider ?? 'twitch') !== 'twitch') return s;
              const u = byLogin.get(s.channelLogin.toLowerCase());
              if (!u || !u.profile_image_url) return s;
              if (
                u.profile_image_url === s.profileImageUrl &&
                s.channelId &&
                s.channelName
              )
                return s;
              changed = true;
              return {
                ...s,
                profileImageUrl: u.profile_image_url,
                channelId: s.channelId || u.id,
                channelName: s.channelName || u.display_name,
              };
            });
            if (changed) {
              set({ slots: next });
              void get().saveSlots();
            }
          } catch (e) {
            Logger.warn('[multiNookStore] Failed to refresh restored slot avatars', e);
          }
        })();
      }
      if (appSettings.multi_nook_chat_hidden !== undefined) {
        set({ isChatHidden: appSettings.multi_nook_chat_hidden });
      }
    }
  },

  saveSlots: async () => {
    // Save to settings.json via AppStore
    const appStore = useAppStore.getState();
    const currentSettings = appStore.settings;
    
    // Strip ephemeral fields (proxy URL, load state, and the live stream title,
    // which would be stale the moment the streamer edits it)
    const cleanSlots = get().slots.map(s => {
      const cleaned = { ...s };
      delete cleaned.streamUrl;
      delete cleaned.loadError;
      delete cleaned.title;
      delete cleaned.broadcasterType;
      return cleaned as MultiNookSlot;
    });
    
    const newSettings = {
      ...currentSettings,
      multi_nook_slots: cleanSlots,
      multi_nook_active_preset_id: get().activePresetId ?? undefined,
    };

    try {
      await invoke('save_settings', { settings: newSettings });
      useAppStore.setState({ settings: newSettings }); // Update local app store reference
    } catch (e) {
      Logger.error('Failed to save multi-nook slots to settings', e);
    }
  }
}));


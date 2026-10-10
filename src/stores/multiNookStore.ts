import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { patchSettings } from '../utils/settingsBroadcast';
import { useAppStore, type StreamStartResult } from './AppStore';
import { DEFAULT_MULTI_NOOK_LAYOUT, type AudioBoostSettings, MultiNookLayout, MultiNookSlot, MultiNookPresetChannel, MultiNookRaid, MultiNookTileMeta, TwitchStream } from '../types';
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
/** The layout setting, with the default for a settings object that predates it. */
export function currentNookLayout(): MultiNookLayout {
  return useAppStore.getState().settings.multi_nook_layout ?? DEFAULT_MULTI_NOOK_LAYOUT;
}

/**
 * The small-tile cap a tile's stream should run under, or null for its own
 * quality. Rust picks the actual rendition (commands/multi_nook.rs
 * tile_quality, with the cap from settings); this mirrors only WHEN the cap
 * applies, so the page knows a tile must restart because its role changed.
 * The main tile and a spotlighted tile are never small; a docked tile is.
 */
function wantedCap(slot: MultiNookSlot, slots: MultiNookSlot[], maximizedSlotId: string | null): number | null {
  const layout = currentNookLayout();
  if (layout.mode === 'grid' || !layout.small_quality_cap) return null;
  if (slot.id === maximizedSlotId) return null;
  const main = slots.find((s) => !s.isMinimized);
  if (main && main.id === slot.id) return null;
  return layout.small_quality_cap;
}

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
  /** True while the chat is pinned to the active channel: focusing, making
   *  main, maximizing or swapping in a tile then leaves chat where it is.
   *  Only the chat switcher moves a pinned chat. Ephemeral: never persisted,
   *  and dropped whenever the pinned channel leaves the grid. */
  isChatPinned: boolean;
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
  /** This tile's own Audio Boost; undefined hands it back to the player's. */
  setSlotAudioBoost: (id: string, boost: AudioBoostSettings | undefined) => void;
  retrySlot: (id: string) => void;
  /** Cover every Twitch tile of the raiding channel with the raid card. The
   *  tile itself is never replaced: the user put that channel in the grid. */
  markSlotsRaided: (raid: MultiNookRaid) => void;
  /** Take the raid card off a tile, back to whatever the player shows. */
  dismissSlotRaid: (id: string) => void;
  reorderSlots: (newSlots: MultiNookSlot[]) => void;
  toggleFocusSlot: (id: string) => void;
  /** Put this tile in the main spot (swapping places with the tile there) and
   *  give it the sound and the chat, as Spotlight does. */
  makeMainSlot: (id: string) => void;
  /** Change the layout. A change of mode or small-tile cap is saved before it
   *  shows, because Rust reads the cap when a tile starts; the strip size shows
   *  at once and is saved a moment later. */
  setLayout: (patch: Partial<MultiNookLayout>) => Promise<void>;
  /** Bring every tile's stream to the quality its role wants: swapped in
   *  place by Rust where it can, else restarted. */
  reconcileTileCaps: () => Promise<void>;
  /** The strip size while its slider is being dragged; null otherwise. Kept
   *  here, not in settings, so a drag re-lays the grid without touching the
   *  app-wide settings object every frame. */
  draftShare: number | null;
  setDraftShare: (share: number | null) => void;
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
  toggleChatPinned: () => void;
  toggleChatHidden: () => void;
  batchLoadMissingStreams: () => Promise<void>;
  /** Apply what Rust reports about the Twitch tiles' channels (services/
   *  multi_nook_meta: one batched poll for the grid, sent on change). All of
   *  it is ephemeral view data: this never writes to settings. */
  applySlotMetadata: (entries: MultiNookTileMeta[]) => void;
  loadPresetChannels: (channels: MultiNookPresetChannel[], mode: 'replace' | 'append', presetId?: string) => Promise<void>;
  /** Tag the current grid with the preset it was loaded from (null = no equipped preset). Persisted. */
  setActivePresetId: (id: string | null) => Promise<void>;
  /** Stop and tear down every tile, leaving an empty grid (used by "Stop preset"). Stays in MultiNook. */
  clearAllSlots: () => Promise<void>;
  /** Close every other tile and leave the grid watching this one channel in the
   *  ordinary single-stream player.
   *
   *  The point is that the kept stream never stops. Its relay is already
   *  serving an upstream the tile resolved when it opened, and
   *  `promote_multi_nook_tile` points the solo relay at that same upstream, so
   *  there is no resolve, no token mint and no spinner between the grid and the
   *  player. A tile the backend cannot hand over that way (a YouTube tile
   *  serves its own relay and never registers) falls back to an ordinary start,
   *  which still works and merely costs the usual load. */
  promoteSlotToSolo: (id: string) => Promise<void>;
  
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
  isChatPinned: false,
  activePresetId: null,
  maximizedSlotId: null,
  isAllMuted: false,
  slots: [],
  flyingAnimation: null,
  suckUpKey: null,
  recallAnimation: null,
  materializingKey: null,
  draftShare: null,
  setDraftShare: (share) => set({ draftShare: share }),

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
    const { slots: all, maximizedSlotId } = get();
    await Promise.all(
      missing.map(async (slot) => {
        try {
          const slotProvider = slot.provider ?? 'twitch';
          const cap = wantedCap(slot, all, maximizedSlotId);
          const url = await invoke<string>('start_multi_nook', {
            streamId: slot.id,
            // buildProviderUrl knows each platform's watch-URL shape and encodes
            // the channel; the old hardcoded twitch.tv template is what made a
            // Kick tile resolve the same-named TWITCH channel instead.
            url: buildProviderUrl(slotProvider, slot.channelLogin),
            quality: slot.quality || 'best', // Per-tile quality (set via the focused tile's gear menu)
            provider: slotProvider,
            // A small tile of a main layout: Rust holds it to the cap.
            small: cap !== null,
          });
          set((state) => ({
            slots: state.slots.map((s) =>
              s.id === slot.id ? { ...s, streamUrl: url, loadError: false, startedCap: cap } : s,
            ),
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
    // A tile still starting when its role changed (a layout switch during a
    // cold start) was skipped by that reconcile; catch it now it is playing.
    void get().reconcileTileCaps();
  },

  applySlotMetadata: (entries) => {
    const byLogin = new Map(entries.map((m) => [m.login.toLowerCase(), m]));
    // Preserve object identity for every tile that didn't actually change: each
    // cell is memoized on its slot's reference, so spreading unconditionally
    // would re-render the whole grid on every report.
    let changed = false;
    let identityChanged = false;
    const next = get().slots.map((s) => {
      // Rust asks Helix about Twitch channels only; a provider tile keeps the
      // metadata its own resolve gave it.
      if ((s.provider ?? 'twitch') !== 'twitch') return s;
      const m = byLogin.get(s.channelLogin.toLowerCase());
      if (!m) return s;
      // The tile follows its channel. Live again (or live at last) while the
      // tile shows the offline card: start it. Ended while playing (Rust holds
      // one missed poll, so this is not a glitch): stop the relay and show the
      // offline card. Only a live-to-offline edge stops a tile, so a channel
      // that went live moments ago (Helix lists it late) is never cut off.
      const offline = !m.live;
      let { streamUrl, loadError } = s;
      if (m.live && s.loadError) {
        streamUrl = undefined;
        loadError = false;
      } else if (offline && s.offline === false && s.streamUrl) {
        void invoke('stop_multi_nook', { streamId: s.id }).catch(() => {});
        streamUrl = undefined;
        loadError = true;
      }
      // Offline: no title (a stale live title on an offline tile is wrong),
      // the last known category kept.
      const title = m.live ? m.title ?? undefined : undefined;
      const gameName = m.game_name ?? s.gameName;
      const broadcasterType = m.broadcaster_type ?? s.broadcasterType;
      // The saved identity: a fresh avatar (Twitch CDN URLs expire), and an id
      // or name the slot never captured.
      const profileImageUrl = m.profile_image_url ?? s.profileImageUrl;
      const channelId = s.channelId || m.user_id || undefined;
      const channelName = s.channelName || m.display_name || undefined;
      if (
        s.offline === offline &&
        s.streamUrl === streamUrl &&
        s.loadError === loadError &&
        s.title === title &&
        s.gameName === gameName &&
        s.broadcasterType === broadcasterType &&
        s.profileImageUrl === profileImageUrl &&
        s.channelId === channelId &&
        s.channelName === channelName
      )
        return s;
      if (s.profileImageUrl !== profileImageUrl || s.channelId !== channelId || s.channelName !== channelName) {
        identityChanged = true;
      }
      changed = true;
      return { ...s, offline, streamUrl, loadError, title, gameName, broadcasterType, profileImageUrl, channelId, channelName };
    });
    if (!changed) return;
    set({ slots: next });
    // Title, category and partner mark are ephemeral; only a repaired
    // identity is worth saving.
    if (identityChanged) void get().saveSlots();
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
      audioBoost: ch.audioBoost,
      volume: 0.5,
      muted: i > 0,
      isFocused: i === 0,
    }));

    // streamUrl is intentionally left undefined: MultiNookView's missing-stream
    // loader picks these up and concurrently starts the proxies on the next frame.
    // Tag the grid with the preset it came from (replace mode = the grid IS this preset).
    set({ slots: newSlots, activeChatChannelId: pickActiveChatChannel(newSlots), isChatPinned: false, activePresetId: presetId ?? null, maximizedSlotId: null });

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

  promoteSlotToSolo: async (id: string) => {
    const keep = get().slots.find((s) => s.id === id);
    if (!keep) return;

    const provider = keep.provider ?? 'twitch';
    const keepKey = slotKey(keep);

    // Point the chat pane at the kept channel BEFORE anything is torn down.
    // MultiNookView holds a background chat connection per visible tile and
    // releases them all on unmount; ChatWidget holds the ACTIVE one separately.
    // Making this tile the active one first means the solo widget inherits a
    // connection whose refcount never reaches zero, so the room is never PARTed
    // and rejoined and the backlog on screen survives the switch.
    if (get().activeChatChannelId !== keepKey) {
      set({ activeChatChannelId: keepKey });
    }

    // Hand the tile's live upstream to the solo relay. Rust stops the rest of
    // the grid itself, but only once the solo relay is serving, so a failure
    // here leaves the grid exactly as it was.
    let preResolved: StreamStartResult | null = null;
    try {
      preResolved = await invoke<StreamStartResult | null>('promote_multi_nook_tile', {
        streamId: id,
      });
    } catch (e) {
      Logger.warn('[MultiNook] Could not promote tile, falling back to a normal start:', e);
    }

    // Every OTHER tile loses its watch registration. The kept channel keeps
    // its own: startStream re-registers it, and unregistering here would
    // briefly aim the heartbeat at nothing.
    for (const slot of get().slots) {
      if (slot.id !== id && slot.channelId) {
        invoke('unregister_active_channel', { channelId: slot.channelId }).catch(() => {});
      }
    }

    // The relays are already gone when promote succeeded; when it did not, the
    // grid is still running and has to be stopped the ordinary way.
    if (!preResolved) {
      try {
        await invoke('stop_all_multi_nooks');
      } catch (e) {
        Logger.error('[MultiNook] Failed to stop proxies while leaving for a single stream', e);
      }
    }

    // What the player and chat know about this channel, from what the tile
    // already had. startStream enriches it (and backfills the thumbnail), but
    // seeding it here means the identity row is never blank.
    const seed: TwitchStream = {
      id: '',
      user_id: keep.channelId || '',
      user_name: keep.channelName || keep.channelLogin,
      user_login: keep.channelLogin,
      title: keep.title || '',
      viewer_count: 0,
      game_name: keep.gameName || '',
      thumbnail_url: '',
      profile_image_url: keep.profileImageUrl || '',
      started_at: new Date().toISOString(),
      ...(provider === 'twitch' ? {} : { provider }),
    };

    // Swap the view in ONE synchronous commit. `streamUrl` is set here rather
    // than waiting for startStream because startStream resolves its stream info
    // first and only then publishes; on the seamless path the picture is
    // already playing, and leaving the grid up meanwhile (or dropping to Home
    // for a frame) is the flicker this whole feature exists to avoid.
    // startStream then re-sets the identical url, and VideoPlayer is keyed on
    // it, so it does not remount.
    if (preResolved) {
      useAppStore.setState({
        streamUrl: preResolved.url,
        activeQuality: preResolved.quality,
        availableQualities: preResolved.available ?? [],
        playbackKind: (preResolved.kind as 'hls' | 'flv' | 'mp4') ?? 'hls',
        currentStream: seed,
        currentMediaType: 'live',
        isHomeActive: false,
      });
    }
    set({
      isMultiNookActive: false,
      slots: [],
      activeChatChannelId: null,
      isChatPinned: false,
      activePresetId: null,
      maximizedSlotId: null,
      isAllMuted: false,
    });
    await get().saveSlots();

    // Full session setup: EventSub, drops, rewind info, presence, chat. With
    // `preResolved` it skips only the resolve; `skipChatRefresh` because the
    // room this channel is in was joined by the grid and is still joined.
    await useAppStore
      .getState()
      .startStream(keep.channelLogin, seed, !!preResolved, preResolved ?? undefined);
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
    set({ slots: [], activeChatChannelId: null, isChatPinned: false, activePresetId: null, maximizedSlotId: null, isAllMuted: false });

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
    // Playing tiles line up in place (components/multi-nook/tileSync, called
    // beside this); only a tile that failed to load starts over.
    const { slots } = get();
    if (!slots.some((s) => s.loadError)) return;
    set({
      slots: slots.map((s) => (s.loadError ? { ...s, streamUrl: undefined, loadError: false } : s)),
    });
  },

  markSlotsRaided: (raid: MultiNookRaid) => {
    const { slots } = get();
    if (!slots.some((s) => (s.provider ?? 'twitch') === 'twitch' && s.channelId === raid.source_id)) return;
    set({
      slots: slots.map((s) =>
        (s.provider ?? 'twitch') === 'twitch' && s.channelId === raid.source_id ? { ...s, raid } : s,
      ),
    });
  },

  dismissSlotRaid: (id: string) => {
    const { slots } = get();
    if (!slots.some((s) => s.id === id && s.raid)) return;
    set({ slots: slots.map((s) => (s.id === id ? { ...s, raid: undefined } : s)) });
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
        set({ activeChatChannelId: pickActiveChatChannel(slotsNow), isChatPinned: false });
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

      set({ activeChatChannelId: null, isChatPinned: false, slots: [], maximizedSlotId: null, isAllMuted: false }); // Maintain chat hidden state

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
      // One Rust lookup: identity, avatar, partner mark, and the live title and
      // category (or the channel's own when offline).
      const row = await invoke<TwitchStream>('resolve_stream_for_login', { login: channelLogin });
      resolvedId = row.user_id || '';
      resolvedName = row.user_name || '';
      resolvedImage = row.profile_image_url || '';
      resolvedBroadcasterType = row.broadcaster_type || '';
      resolvedGameName = row.game_name || '';
      resolvedTitle = row.title || '';
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
      set({ activeChatChannelId: pickActiveChatChannel(newSlots), isChatPinned: false });
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
    
    // Volume and mute change on every scroll notch and slider step: Rust
    // writes just this tile's audio into the saved grid, with no whole-grid
    // save and nothing broadcast to other windows. Anything else structural
    // saves the grid.
    const keys = Object.keys(updates);
    if (keys.length > 0 && keys.every((k) => k === 'volume' || k === 'muted')) {
      const slot = newSlots.find((s) => s.id === id);
      if (slot) {
        invoke('set_multi_nook_slot_audio', { slotId: id, volume: slot.volume, muted: slot.muted }).catch(
          (e: unknown) => Logger.warn('[MultiNook] Failed to save tile audio', e),
        );
      }
      return;
    }
    if ('volume' in updates || 'muted' in updates || 'isFocused' in updates || 'channelLogin' in updates || 'isMinimized' in updates || 'profileImageUrl' in updates) {
      saveSlots();
    }
  },

  setSlotAudioBoost: (id: string, boost: AudioBoostSettings | undefined) => {
    if (!get().slots.some((s) => s.id === id)) return;
    set((state) => ({ slots: state.slots.map((s) => (s.id === id ? { ...s, audioBoost: boost } : s)) }));
    // Written in place by Rust like the tile's volume: a fader drag would
    // otherwise run a whole-grid save once per step.
    invoke('set_multi_nook_slot_audio_boost', { slotId: id, boost: boost ?? null }).catch((e: unknown) =>
      Logger.warn('[MultiNook] Failed to save tile Audio Boost', e),
    );
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
    // on the new URL, the same path retrySlot uses.
    set(state => ({
      slots: state.slots.map(s => (s.id === id ? { ...s, quality, streamUrl: undefined, loadError: false } : s)),
    }));
    await get().saveSlots();
  },

  reorderSlots: (newSlots: MultiNookSlot[]) => {
    set({ slots: newSlots });
    get().saveSlots();
  },

  makeMainSlot: (id: string) => {
    const { slots, saveSlots } = get();
    const slot = slots.find((s) => s.id === id);
    const main = slots.find((s) => !s.isMinimized);
    if (!slot || !main || slot.isMinimized) return;
    const next = [...slots];
    if (main.id !== id) {
      const a = next.findIndex((s) => s.id === main.id);
      const b = next.findIndex((s) => s.id === id);
      [next[a], next[b]] = [next[b], next[a]];
    }
    // Sound and chat follow, like Spotlight: this tile unmuted, the rest muted.
    // Set, never toggled: making the focused tile main must not unfocus it.
    const focused = next.map((s) => {
      const isFocused = s.id === id;
      const muted = s.id !== id;
      if (s.isFocused === isFocused && s.muted === muted) return s;
      return { ...s, isFocused, muted };
    });
    set(get().isChatPinned ? { slots: focused } : { slots: focused, activeChatChannelId: slotKey(slot) });
    saveSlots();
  },

  setLayout: async (patch) => {
    const next: MultiNookLayout = { ...currentNookLayout(), ...patch };
    // Saved before it shows: Rust reads the mode and the cap when a tile
    // starts, so the page must not act on a layout Rust has not seen yet.
    try {
      await patchSettings({ multi_nook_layout: next });
    } catch (e) {
      Logger.error('[MultiNook] Failed to save the layout', e);
      return;
    }
    useAppStore.setState((st) => ({ settings: { ...st.settings, multi_nook_layout: next } }));
  },

  reconcileTileCaps: async () => {
    const { slots, maximizedSlotId } = get();
    const stale = slots.filter(
      (s) => s.streamUrl && !inFlightStarts.has(s.id) && (s.startedCap ?? null) !== wantedCap(s, slots, maximizedSlotId),
    );
    if (stale.length === 0) return;
    stale.forEach((s) => inFlightStarts.add(s.id));

    // Rust swaps a tile's stream in place on its own relay when the player can
    // follow (same platform path, same codec family, same low-latency mode),
    // so it keeps playing. Anything else gets a full restart.
    const restart: MultiNookSlot[] = [];
    await Promise.all(
      stale.map(async (s) => {
        const cap = wantedCap(s, slots, maximizedSlotId);
        try {
          const provider = s.provider ?? 'twitch';
          const outcome = await invoke<'swapped' | 'restart'>('retier_multi_nook_tile', {
            streamId: s.id,
            url: buildProviderUrl(provider, s.channelLogin),
            quality: s.quality || 'best',
            provider,
            small: cap !== null,
          });
          if (outcome === 'swapped') {
            set((state) => ({ slots: state.slots.map((x) => (x.id === s.id ? { ...x, startedCap: cap } : x)) }));
            return;
          }
        } catch (e) {
          Logger.warn(`[MultiNook] Could not change ${s.id}'s quality in place; restarting it`, e);
        }
        restart.push(s);
      }),
    );

    await Promise.all(
      restart.map((s) =>
        invoke('stop_multi_nook', { streamId: s.id }).catch((e: unknown) =>
          Logger.warn(`[MultiNook] Failed to stop ${s.id} before restarting at its new size`, e),
        ),
      ),
    );
    stale.forEach((s) => inFlightStarts.delete(s.id));
    if (restart.length > 0) {
      // Clearing the URL hands the tile to the missing-stream loader, which
      // starts it again under the cap its role now wants.
      const ids = new Set(restart.map((s) => s.id));
      set((state) => ({
        slots: state.slots.map((s) => (ids.has(s.id) ? { ...s, streamUrl: undefined, loadError: false } : s)),
      }));
    }
    // A role that changed again while this ran is caught by one more pass;
    // with nothing stale it returns at once.
    void get().reconcileTileCaps();
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
    
    // Jump chat focus to this slot if we are focusing it, unless chat is pinned
    if (!isCurrentlyFocused && !get().isChatPinned) {
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

    // Move chat to the maximized stream so chat matches what you're watching,
    // unless chat is pinned.
    if (!get().isChatPinned) set({ activeChatChannelId: slotKey(slot) });
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

    // A docked tile stays in the grid, so a pinned chat stays valid through a swap.
    if (!get().isChatPinned) set({ activeChatChannelId: slotKey(slotToRestore) });
  },

  setActiveChatChannelId: (id: string | null) => {
    set({ activeChatChannelId: id });
  },

  toggleChatPinned: () => {
    set((state) => ({ isChatPinned: !state.isChatPinned && state.activeChatChannelId !== null }));
  },

  toggleChatHidden: async () => {
    const currentState = get().isChatHidden;
    const newState = !currentState;
    set({ isChatHidden: newState });
    
    try {
      await patchSettings({ multi_nook_chat_hidden: newState });
      useAppStore.setState((s) => ({ settings: { ...s.settings, multi_nook_chat_hidden: newState } }));
    } catch (e) {
      Logger.error('Failed to save multi_nook_chat_hidden state', e);
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
            delete cleaned.offline;
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
        // Restored avatars can be stale (Twitch CDN URLs expire) or missing:
        // Rust's first tile report (services/multi_nook_meta) replaces them,
        // with no request from here.
      }
      if (appSettings.multi_nook_chat_hidden !== undefined) {
        set({ isChatHidden: appSettings.multi_nook_chat_hidden });
      }
    }
  },

  saveSlots: async () => {
    // Save to settings.json via AppStore
    // Strip ephemeral fields (proxy URL, load state, and the live stream title,
    // which would be stale the moment the streamer edits it)
    const cleanSlots = get().slots.map(s => {
      const cleaned = { ...s };
      delete cleaned.streamUrl;
      delete cleaned.loadError;
      delete cleaned.offline;
      delete cleaned.title;
      delete cleaned.broadcasterType;
      delete cleaned.raid;
      return cleaned as MultiNookSlot;
    });
    
    const activePresetId = get().activePresetId ?? undefined;

    try {
      await patchSettings({
        multi_nook_slots: cleanSlots,
        multi_nook_active_preset_id: activePresetId ?? null,
      });
      useAppStore.setState((s) => ({
        settings: { ...s.settings, multi_nook_slots: cleanSlots, multi_nook_active_preset_id: activePresetId },
      }));
    } catch (e) {
      Logger.error('Failed to save multi-nook slots to settings', e);
    }
  }
}));


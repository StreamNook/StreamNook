import React, { useEffect, useState, useCallback, useRef } from 'react';
import { useSortable } from '@dnd-kit/sortable';
import { AnimatePresence, motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { MultiNookSlot, type AudioBoostSettings } from '../../types';
import { useMultiNookPlayer } from './useMultiNookPlayer';
import { usemultiNookStore } from '../../stores/multiNookStore';
import { useContextMenuStore } from '../../stores/contextMenuStore';
import { useAppStore } from '../../stores/AppStore';
import { buildProviderUrl } from '../../utils/streamProvider';
import { useMediaGlow } from '../../utils/mediaGlow';
import { useChannelSocial } from '../../hooks/useChannelSocial';
import {
  ignoresPlayerMouse,
  createWheelAccumulator,
  stepVolume,
  toggleVolumeMute,
  scrollVolumeOn,
  WHEEL_VOLUME_STEP,
} from '../../utils/playerMouseControls';
import { playerOverlayButtonOn } from '../../utils/playerOverlayButtons';
import { PlayerVolumeOsd } from '../PlayerVolumeOsd';
import { useVolumeOsd } from '../../hooks/useVolumeOsd';
import StreamTitleWithEmojis from '../StreamTitleWithEmojis';
import { Tooltip } from '../ui/Tooltip';
import { TwitchVerifiedMark } from '../ui/TwitchGlyph';
import { ProviderLogo } from '../ProviderLogo';
import { OfflineCard } from '../OfflineRoomScreen';
import { useOfflineRoom } from '../../hooks/useOfflineRoom';
import { ArrowLeftRight, GripHorizontal, Undo2, Loader2, Maximize2, Minimize2, Plus, Check, Radio } from 'lucide-react';
import type { SizeTier } from './nookLayout';
import { Heart, HeartBreak, X as XIcon } from 'phosphor-react';
import { Logger } from '../../utils/logger';
import {
  AUDIO_BOOST_BUTTON_HTML,
  AUDIO_GRAPH_SUPPORTED,
  applyAudioBoost,
  paintAudioBoostButton,
  releaseAudioGraphOnceGone,
  resolveAudioBoost,
} from '../../utils/audioBoost';
import { injectPlyrControl } from '../../utils/plyrControls';
import { AudioBoostControls } from '../AudioBoostFaders';
import { canGridProvider, PROVIDER_WATCH, type ProviderId } from '../../types/providers';

interface MultiNookCellProps {
  slot: MultiNookSlot;
  cssOrder?: number;
  gridSpanClass?: string;
  customStyle?: React.CSSProperties;
  /** True when this tile is filling the whole grid area (solo-like). */
  isMaximized?: boolean;
  /** How much chrome fits the tile's size (nookLayout's tiers): everything,
   *  a trimmed set, or the least. */
  sizeTier?: SizeTier;
  /** One of a main layout's small tiles: offer to make it the main one. */
  canMakeMain?: boolean;
}

/** A single pending "unfocus" (focus toggle-off) shared across all tiles. Clicking
 *  a focused tile defers the unfocus briefly so a double-click (which fills the
 *  space) can cancel it first — that's what stops the audible mute/unmute flip on
 *  the way to maximizing. Focus-ON stays instant; only this toggle-off is deferred. */
let pendingFocusToggle: ReturnType<typeof setTimeout> | null = null;
const clearPendingFocusToggle = () => {
  if (pendingFocusToggle) {
    clearTimeout(pendingFocusToggle);
    pendingFocusToggle = null;
  }
};

const MultiNookCellInner: React.FC<MultiNookCellProps> = ({ slot, cssOrder, gridSpanClass = '', customStyle = {}, isMaximized = false, sizeTier = 'full', canMakeMain = false }) => {
  const full = sizeTier === 'full';
  const mini = sizeTier === 'mini';
  const { id, provider, channelLogin, channelName, channelId, volume, muted, isFocused, streamUrl, isMinimized = false, loadError, profileImageUrl, title, broadcasterType, raid } = slot;
  // Actions only, so read them without subscribing. A bare `usemultiNookStore()`
  // here subscribed this tile to the WHOLE store, which meant any mutation
  // (including a volume drag on a sibling tile) re-rendered every tile in the
  // grid. Zustand actions keep the same identity for the store's lifetime.
  const { toggleFocusSlot, toggleMaximizeSlot, makeMainSlot, dockSlot, removeSlot, changeSlotQuality, setSlotAudioBoost, retrySlot, addSlot, dismissSlotRaid } =
    usemultiNookStore.getState();

  // The raid card offers the raided channel as a new tile, never in place of
  // this one, so it only needs to know whether that channel is already here.
  const raidTargetInGrid = usemultiNookStore(
    (s) =>
      !!raid &&
      s.slots.some(
        (t) => (t.provider ?? 'twitch') === 'twitch' && t.channelLogin.toLowerCase() === raid.target_login.toLowerCase(),
      ),
  );
  const [addingRaidTarget, setAddingRaidTarget] = useState(false);

  // Offline tiles show the offline overlay instead of an endless loading spinner.
  const isLoading = !streamUrl && !loadError;
  // What the offline card shows for a Twitch channel (cached in Rust).
  const offlineRoom = useOfflineRoom(loadError && (provider ?? 'twitch') === 'twitch' ? channelLogin : null);

  // Whether the decoded picture is taller than it is wide, which decides how the
  // tile fits it. Seeded from the platform's usual shape so a portrait source
  // starts correct and never visibly snaps once metadata lands, then corrected
  // from the real frame: a platform's usual shape is a default, not a promise,
  // and a landscape broadcast on a portrait-first platform still fills the cell.
  const [isPortrait, setIsPortrait] = useState(
    () => PROVIDER_WATCH[provider as ProviderId]?.thumbAspect === 'portrait',
  );

  // Toolbar mute-all overrides this tile's audio without touching slot.muted,
  // so unmuting restores the focus/mute mix that was playing before. Selector
  // subscription: the tile only re-renders when the flag itself flips.
  const isAllMuted = usemultiNookStore((s) => s.isAllMuted);

  const { videoRef, playerRef, isPlaying, isBuffering, error } = useMultiNookPlayer({
    streamUrl,
    streamId: id,
    volume,
    muted: muted || isAllMuted,
    isMinimized,
  });

  // StreamNook watch time: Rust registers this tile as a source when its
  // stream starts and drops it when the tile stops; the media element's
  // play/pause is the one thing only the page knows. A pause reports after a
  // grace window, like the main player, so a quality swap's transient pause
  // never gates the minute off.
  useEffect(() => {
    const el = videoRef.current;
    if (!el || !streamUrl) return;
    let pausedTimer: ReturnType<typeof setTimeout> | null = null;
    const onPlaying = () => {
      if (pausedTimer) {
        clearTimeout(pausedTimer);
        pausedTimer = null;
      }
      invoke('report_player_playing', { playing: true, slot: id }).catch(() => {});
    };
    const onPause = () => {
      if (pausedTimer) clearTimeout(pausedTimer);
      pausedTimer = setTimeout(() => {
        pausedTimer = null;
        invoke('report_player_playing', { playing: false, slot: id }).catch(() => {});
      }, 6000);
    };
    if (!el.paused && el.readyState > 2) onPlaying();
    el.addEventListener('playing', onPlaying);
    el.addEventListener('pause', onPause);
    return () => {
      if (pausedTimer) clearTimeout(pausedTimer);
      el.removeEventListener('playing', onPlaying);
      el.removeEventListener('pause', onPause);
    };
  }, [videoRef, streamUrl, id]);

  // Correct the seeded orientation from the real frame once one is decoded.
  useEffect(() => {
    const el = videoRef.current;
    if (!el) return;
    const measure = () => {
      if (el.videoWidth > 0 && el.videoHeight > 0) {
        setIsPortrait(el.videoHeight > el.videoWidth);
      }
    };
    // `loadedmetadata` does not fire for a stream already decoding by the time
    // this attaches, so take a reading up front as well.
    measure();
    el.addEventListener('loadedmetadata', measure);
    el.addEventListener('resize', measure);
    return () => {
      el.removeEventListener('loadedmetadata', measure);
      el.removeEventListener('resize', measure);
    };
  }, [videoRef, streamUrl]);

  // Audio Boost, per tile. A tile with its own settings (slot.audioBoost, set
  // from its boost panel) uses those; one without uses the player's Audio Boost
  // until it is changed here. Same per-element graph as the solo player
  // (utils/audioBoost). Applied only while this tile can be heard, so a tile
  // that stays muted (focus mode mutes every other tile) is never tapped and
  // never opens an audio context of its own. Once tapped, the element keeps its
  // graph until the tile unmounts. `playing` re-applies so a context that
  // started suspended picks up once the stream actually plays.
  const playerBoostSettings = useAppStore((s) => s.settings.video_player?.audio_boost);
  const tileBoostSettings = slot.audioBoost;
  const ownBoost = tileBoostSettings !== undefined;
  const resolvedBoost = resolveAudioBoost(tileBoostSettings ?? playerBoostSettings);
  const audible = !!streamUrl && !muted && !isAllMuted && !isMinimized;
  useEffect(() => {
    const el = videoRef.current;
    if (!el || !audible) return;
    const apply = () => applyAudioBoost(el, resolveAudioBoost(tileBoostSettings ?? playerBoostSettings));
    apply();
    el.addEventListener('playing', apply);
    return () => el.removeEventListener('playing', apply);
  }, [videoRef, audible, tileBoostSettings, playerBoostSettings, streamUrl]);

  // Edits from the tile's panel always land on the tile's own copy, seeded from
  // whatever it was using, so changing one tile never moves the player or any
  // other tile. Reads fresh state so a fast fader drag never clobbers itself.
  const patchTileBoost = useCallback(
    (patch: Partial<AudioBoostSettings>) => {
      const current = usemultiNookStore.getState().slots.find((s) => s.id === id)?.audioBoost;
      const base = resolveAudioBoost(current ?? useAppStore.getState().settings.video_player?.audio_boost);
      setSlotAudioBoost(id, { ...base, ...patch });
    },
    [id, setSlotAudioBoost],
  );

  // The tile's boost panel, opened from the control-bar button.
  const [boostPanelOpen, setBoostPanelOpen] = useState(false);
  const boostPanelRef = useRef<HTMLDivElement>(null);

  // The tile's <video> lives as long as the tile (Plyr swaps in a clone on
  // destroy), so hand it back once it has left the page. See releaseAudioGraph.
  useEffect(() => {
    const el = videoRef.current;
    return () => {
      if (el) releaseAudioGraphOnceGone(el);
    };
  }, [videoRef]);

  // Volume readout for this tile's wheel/middle-click changes.
  const { osd, showOsd } = useVolumeOsd();

  // Follow + subscribe controls. Only the focused, non-docked tile activates the
  // hook so we make one follow/subscription lookup at a time instead of one per
  // tile across the whole grid. Visibility additionally honors the same
  // Player Overlay Buttons setting as the single-stream player.
  // The tile passes its OWN provider to useChannelSocial below, so the overlay is
  // correct for this channel regardless of what the solo player last had.
  const socialEnabled = isFocused && !isMinimized;
  const playerOverlayButtons = useAppStore((s) => s.settings.player_overlay_buttons);
  const mediaGlowEnabled = useAppStore((s) => s.settings.media_glow !== false);
  const {
    isFollowing,
    followLoading,
    checkingFollowStatus,
    heartDropAnimation,
    handleFollowClick,
    isSubscribed,
    hasSubHistory,
    cumulativeMonths,
    subscriberBadgeUrl,
    handleSubscribeClick,
    offersMembership,
  } = useChannelSocial({
    provider: provider ?? 'twitch',
    userId: channelId,
    userLogin: channelLogin,
    userName: channelName,
    enabled: socialEnabled,
  });
  const showFollowButton = socialEnabled && full && playerOverlayButtonOn(playerOverlayButtons, 'follow');
  const showSubscribeButton =
    socialEnabled && full && offersMembership && playerOverlayButtonOn(playerOverlayButtons, 'subscribe');

  // This tile's own quality menu. Every tile offers one, not just the focused
  // tile: the relay kept the list its resolve discovered, so asking for it
  // costs no network call and is keyed by this tile's stream id. Re-read when
  // the tile restarts on a new url (a quality change, a retry).
  const [availableQualities, setAvailableQualities] = useState<string[]>([]);
  useEffect(() => {
    if (!streamUrl) return;
    // A tile the grid refuses should never exist, so this is belt and braces
    // for any provider added to GRID_BLOCKED later.
    if (!canGridProvider(provider ?? 'twitch')) return;
    let cancelled = false;
    invoke<string[]>('get_multi_nook_tile_qualities', {
      streamId: id,
      url: buildProviderUrl(provider ?? 'twitch', channelLogin),
      provider: provider ?? 'twitch',
    })
      .then((qs) => {
        if (!cancelled && qs?.length) setAvailableQualities(qs);
      })
      .catch((e) => Logger.warn(`[MultiNook] Failed to fetch qualities for ${channelLogin}`, e));
    return () => {
      cancelled = true;
    };
  }, [streamUrl, id, channelLogin, provider]);

  // Inject a Quality submenu into this tile's Plyr settings gear — mirrors the
  // single player. Selecting a quality restarts only this tile's proxy via
  // changeSlotQuality (which briefly reloads the cell at the new quality).
  const updateQualityMenu = useCallback(() => {
    const player = playerRef.current as unknown as { elements?: { container?: HTMLElement } } | null;
    const container = player?.elements?.container;
    if (!container || availableQualities.length === 0) return;

    const settingsMenu = container.querySelector('.plyr__menu');
    if (!settingsMenu) return;

    // Remove any previously injected quality menu/button before re-adding
    settingsMenu.querySelector('[data-quality-menu]')?.remove();
    settingsMenu.querySelector('[data-plyr="quality"]')?.remove();

    const settingsHome = settingsMenu.querySelector('[role="menu"]');
    if (!settingsHome) return;

    const displayedQuality = slot.quality || 'best';
    const cap = (q: string) => q.charAt(0).toUpperCase() + q.slice(1);
    // A small tile under the small-tile cap plays below the quality it keeps
    // for when it is large again; say so rather than show a quality it is not
    // playing.
    const capNote = slot.startedCap ? ` · ${slot.startedCap}p while small` : '';

    const qualityMenuItem = document.createElement('button');
    qualityMenuItem.className = 'plyr__control';
    qualityMenuItem.setAttribute('data-plyr', 'quality');
    qualityMenuItem.setAttribute('type', 'button');
    qualityMenuItem.setAttribute('role', 'menuitem');
    qualityMenuItem.innerHTML = `<span>Quality<span class="plyr__menu__value">${cap(displayedQuality)}${capNote}</span></span>`;
    qualityMenuItem.addEventListener('click', () => {
      const submenu = settingsMenu.querySelector('[data-quality-menu]');
      if (submenu) {
        settingsHome.setAttribute('hidden', '');
        submenu.removeAttribute('hidden');
      }
    });

    const speedOption = settingsHome.querySelector('[data-plyr="speed"]');
    if (speedOption) {
      settingsHome.insertBefore(qualityMenuItem, speedOption);
    } else {
      settingsHome.appendChild(qualityMenuItem);
    }

    const qualitySubmenu = document.createElement('div');
    qualitySubmenu.setAttribute('role', 'menu');
    qualitySubmenu.setAttribute('data-quality-menu', '');
    qualitySubmenu.setAttribute('hidden', '');
    qualitySubmenu.innerHTML = `
      <button class="plyr__control plyr__control--back" type="button" data-plyr="back">
        <span>Quality</span>
      </button>
      ${availableQualities
        .map(
          (quality) => `
        <button
          class="plyr__control"
          type="button"
          data-quality="${quality}"
          role="menuitemradio"
          aria-checked="${quality.toLowerCase() === displayedQuality.toLowerCase() ? 'true' : 'false'}"
        >
          <span>${cap(quality)}</span>
        </button>`
        )
        .join('')}
    `;

    const menuContainer = settingsMenu.querySelector('.plyr__menu__container');
    menuContainer?.appendChild(qualitySubmenu);

    qualitySubmenu.querySelector('[data-plyr="back"]')?.addEventListener('click', () => {
      qualitySubmenu.setAttribute('hidden', '');
      settingsHome.removeAttribute('hidden');
    });

    qualitySubmenu.querySelectorAll('[data-quality]').forEach((btn) => {
      if (btn.getAttribute('data-plyr') === 'back') return;
      btn.addEventListener('click', () => {
        const selected = btn.getAttribute('data-quality');
        if (!selected) return;

        qualitySubmenu.querySelectorAll('[data-quality]').forEach((b) => {
          if (b.getAttribute('data-plyr') !== 'back') b.setAttribute('aria-checked', 'false');
        });
        btn.setAttribute('aria-checked', 'true');

        const valueSpan = settingsHome.querySelector('[data-plyr="quality"] .plyr__menu__value');
        if (valueSpan) valueSpan.textContent = cap(selected);

        qualitySubmenu.setAttribute('hidden', '');
        settingsHome.removeAttribute('hidden');

        changeSlotQuality(id, selected);
      });
    });
  }, [availableQualities, slot.quality, slot.startedCap, id, changeSlotQuality, playerRef]);

  // Add the quality submenu once this tile knows its qualities.
  useEffect(() => {
    if (availableQualities.length === 0) return;
    // Defer so Plyr has finished rendering its menu DOM
    const timer = window.setTimeout(() => updateQualityMenu(), 200);
    return () => window.clearTimeout(timer);
    // isPlaying/streamUrl re-trigger after the player (re)initialises
  }, [availableQualities, updateQualityMenu, isPlaying, streamUrl]);

  // The Audio Boost button in this tile's control bar, right after volume, the
  // same control the solo player has. It opens the tile's own boost panel, and
  // is shown but inert on macOS (paintAudioBoostButton says why). One effect
  // both inserts (idempotent, keyed by the attribute) and paints, so a button
  // the retry inserts late still gets the current state.
  const audioBoostOn = resolvedBoost.enabled;
  useEffect(() => {
    const container = (playerRef.current as unknown as { elements?: { container?: HTMLElement } } | null)?.elements
      ?.container;
    if (!container) return;
    paintAudioBoostButton(container.querySelector('[data-streamnook-audioboost]'), audioBoostOn);
    return injectPlyrControl(container, {
      attr: 'data-streamnook-audioboost',
      className: '',
      html: AUDIO_BOOST_BUTTON_HTML,
      onClick: () => {
        if (AUDIO_GRAPH_SUPPORTED) setBoostPanelOpen((o) => !o);
      },
      place: (controls) => {
        const volume = controls.querySelector(':scope > .plyr__volume');
        if (volume) return { after: volume };
        const menu = controls.querySelector(':scope > .plyr__menu');
        return menu ? { before: menu } : null;
      },
      onInserted: (btn) => paintAudioBoostButton(btn, audioBoostOn),
    });
    // isPlaying/streamUrl re-trigger after the player (re)initialises
  }, [audioBoostOn, isPlaying, streamUrl, playerRef]);

  // Close the panel on Escape or a press outside it (the button itself toggles).
  // Capture phase so it runs before the tile's own pointer handlers.
  useEffect(() => {
    if (!boostPanelOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setBoostPanelOpen(false);
    };
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      const panel = boostPanelRef.current;
      const btn = (playerRef.current as unknown as { elements?: { container?: HTMLElement } } | null)?.elements
        ?.container?.querySelector('[data-streamnook-audioboost]');
      if (panel && !panel.contains(target) && !(btn && btn.contains(target))) setBoostPanelOpen(false);
    };
    document.addEventListener('keydown', onKey);
    document.addEventListener('mousedown', onDown, true);
    return () => {
      document.removeEventListener('keydown', onKey);
      document.removeEventListener('mousedown', onDown, true);
    };
  }, [boostPanelOpen, playerRef]);

  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    isDragging,
  } = useSortable({ id });

  // Merge dnd-kit's node ref with our own so we can attach a native listener.
  const cellRef = useRef<HTMLDivElement | null>(null);
  // Slot id rather than a channel key: two tiles can show the same channel,
  // and each one needs its own colour.
  useMediaGlow(videoRef, cellRef, streamUrl ? id : null, mediaGlowEnabled);

  const setRefs = useCallback((node: HTMLDivElement | null) => {
    setNodeRef(node);
    cellRef.current = node;
  }, [setNodeRef]);

  // Capture-phase double-click handler. Runs on the cell (an ancestor of Plyr's
  // container) BEFORE the event reaches Plyr, so stopPropagation here prevents
  // Plyr's own dblclick→fullscreen (which the bridge turns into true OS
  // fullscreen). Double-click now means ONE thing: fill the space / restore.
  useEffect(() => {
    const el = cellRef.current;
    if (!el) return;
    const onDblCapture = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      // Let real player controls (incl. the fullscreen button) behave normally.
      if (target.closest('button') || target.closest('.plyr__controls') || ignoresPlayerMouse(target)) return;
      e.stopPropagation();
      e.preventDefault();
      clearPendingFocusToggle(); // a double-click cancels any deferred unfocus
      toggleMaximizeSlot(id);
    };
    el.addEventListener('dblclick', onDblCapture, { capture: true });
    return () => el.removeEventListener('dblclick', onDblCapture, { capture: true });
  }, [id, toggleMaximizeSlot]);

  // Mouse volume for this tile: wheel to change it, middle click to mute. Scoped
  // to the hovered cell, so each tile is adjusted independently. Persistence
  // rides the volumechange handler inside useMultiNookPlayer — nothing extra is
  // written here. Settings are read at event time so a change applies without
  // rebinding.
  useEffect(() => {
    const el = cellRef.current;
    if (!el) return;
    const accumulate = createWheelAccumulator();
    const playerSettings = () => useAppStore.getState().settings.video_player;
    const volumeTarget = () => playerRef.current ?? videoRef.current;

    // Shift is not special here: the channel About reveal doesn't exist in
    // MultiNook, so there's nothing for it to disambiguate and Shift + scroll
    // just adjusts volume like a plain scroll.
    const onWheel = (e: WheelEvent) => {
      const s = playerSettings();
      if (!scrollVolumeOn(s)) return;
      if (e.deltaY === 0) return; // horizontal scroll isn't ours
      if (ignoresPlayerMouse(e.target)) return;
      const target = volumeTarget();
      if (!target) return;
      e.preventDefault();
      e.stopPropagation();
      const steps = accumulate(e, performance.now());
      if (steps === 0) return;
      const next = stepVolume(target, steps, s?.wheel_volume_step ?? WHEEL_VOLUME_STEP);
      showOsd(next.volume, next.muted);
    };

    // Middle click also steps aside for tile controls, which sit on top of the
    // video and have their own meaning. Suppressing the default on mousedown
    // kills the autoscroll ring without stopping the auxclick that follows.
    const middleClickBlocked = (e: MouseEvent) => {
      if (e.button !== 1) return true;
      if (!(playerSettings()?.middle_click_mute ?? true)) return true;
      const target = e.target as HTMLElement;
      return ignoresPlayerMouse(target) || !!target.closest?.('button');
    };

    const onMouseDown = (e: MouseEvent) => {
      if (middleClickBlocked(e)) return;
      e.preventDefault();
    };

    const onAuxClick = (e: MouseEvent) => {
      if (middleClickBlocked(e)) return;
      const target = volumeTarget();
      if (!target) return;
      e.preventDefault();
      e.stopPropagation();
      const next = toggleVolumeMute(target);
      showOsd(next.volume, next.muted);
    };

    el.addEventListener('wheel', onWheel, { passive: false });
    el.addEventListener('mousedown', onMouseDown, { capture: true });
    el.addEventListener('auxclick', onAuxClick);
    return () => {
      el.removeEventListener('wheel', onWheel);
      el.removeEventListener('mousedown', onMouseDown, { capture: true });
      el.removeEventListener('auxclick', onAuxClick);
    };
  }, [playerRef, videoRef, showOsd]);

  // Map dnd-kit's drag offset cleanly to Framer Motion's coordinate space
  const x = transform ? Math.round(transform.x) : 0;
  const y = transform ? Math.round(transform.y) : 0;
  const scale = transform ? transform.scaleX : 1;

  const style: React.CSSProperties = {
    zIndex: isDragging ? 10 : 1,
    order: cssOrder,
  };

  const combinedStyle = { ...style, ...customStyle };

  // Right-click anywhere on the tile that is not a control. The menu takes a
  // channel the way every other menu in the app does, so the slot's cached
  // identity is shaped into one here; the tile actions address the SLOT and
  // ride along as `slotId`, because two tiles can be the same channel on two
  // platforms and only the slot id tells them apart.
  const handleContextMenu = (e: React.MouseEvent) => {
    const target = e.target as HTMLElement;
    if (target.closest('button') || target.closest('.plyr__controls') || target.closest('.plyr__menu')) {
      return;
    }
    useContextMenuStore.getState().openTileMenu(
      e,
      {
        id: '',
        user_id: channelId || '',
        user_name: channelName || channelLogin,
        user_login: channelLogin,
        title: title || '',
        viewer_count: 0,
        game_name: slot.gameName || '',
        thumbnail_url: '',
        profile_image_url: profileImageUrl || '',
        started_at: new Date().toISOString(),
        ...((provider ?? 'twitch') === 'twitch' ? {} : { provider }),
      },
      id,
    );
  };

  const glassButton = `flex items-center justify-center ${full ? 'p-1.5' : 'p-1'} glass-button rounded-lg`;

  return (
    <motion.div
      layout
      animate={{ x, y, scale }}
      transition={isDragging ? { duration: 0 } : { type: 'spring', stiffness: 350, damping: 30 }}
      ref={setRefs}
      style={combinedStyle}
      onContextMenu={handleContextMenu}
      onClick={(e) => {
        // Ignore clicks on buttons, tools, or plyr control sliders.
        const target = e.target as HTMLElement;
        if (target.closest('button') || target.closest('.plyr__controls') || ignoresPlayerMouse(target)) return;
        // While maximized, a bare click shouldn't change focus (you're already
        // watching this one). The second click of a double-click (detail === 2)
        // is left for the capture-phase dblclick handler that fills the space.
        if (isMaximized || e.detail >= 2) return;
        clearPendingFocusToggle();
        if (!isFocused) {
          // Focusing a tile (the common audio switch) stays instant.
          toggleFocusSlot(id);
        } else {
          // Un-focusing (unmute-all) is deferred so a double-click can cancel it
          // before it fires — no mute/unmute flip while maximizing this tile.
          pendingFocusToggle = setTimeout(() => {
            pendingFocusToggle = null;
            toggleFocusSlot(id);
          }, 260);
        }
      }}
      className={`${gridSpanClass} relative w-full h-full overflow-hidden ${
        isMaximized ? '' : 'rounded-lg border border-white/5'
      } ${
        isFocused && !isMaximized ? 'media-glow-focus' : ''
      } ${
        isDragging ? 'opacity-50 blur-sm' : 'opacity-100'
      } bg-black/40 transition-opacity duration-300 group flex items-center justify-center video-player-container [&_.plyr]:w-full [&_.plyr]:h-full [&_.plyr]:absolute [&_.plyr]:inset-0 ${
        isMaximized ? 'cursor-default' : 'cursor-pointer'
      }`}
    >
      <video
        ref={videoRef}
        className="w-full h-full"
        // `cover` trims a little off a landscape broadcast so the tile has no
        // dead bars, which is right for every 16:9 source. For a PORTRAIT one
        // it is not a trim: the cell keeps a slice about a third of the picture
        // wide and throws the rest away. Those sources get `contain` instead.
        style={{
          backgroundColor: '#000',
          objectFit: isMaximized || isPortrait ? 'contain' : 'cover',
        }}
        autoPlay
        playsInline
      />

      {/* Loading & Error States */}
      {(isLoading || isBuffering) && !error && !loadError && !raid && (
        <div className="absolute inset-0 flex items-center justify-center bg-black/50 backdrop-blur-sm z-10 pointer-events-none">
          <i className="ri-loader-4-line text-4xl text-white animate-spin"></i>
        </div>
      )}

      {/* Offline: the same card the main view shows, compact, over the
          channel's own offline art. A Twitch tile starts its stream on its own
          when the channel goes live (Rust's grid live check), so there is no
          Retry; the toolbar's reload covers an unreachable tile. Chat keeps
          working like any tile's. */}
      <AnimatePresence>
        {loadError && !raid && (
          <OfflineCard
            compact
            room={offlineRoom}
            fallbackName={channelName || channelLogin}
            fallbackAvatar={profileImageUrl}
            live={offlineRoom?.live ?? null}
            joining={false}
            onWatchLive={() => retrySlot(id)}
          />
        )}
      </AnimatePresence>

      {error && !raid && (
        <div className="absolute inset-0 flex flex-col items-center justify-center bg-black/80 z-10 text-rose-500 pointer-events-none">
          <i className="ri-error-warning-fill text-4xl mb-2"></i>
          <p className="text-sm font-medium">{error}</p>
        </div>
      )}

      {/* Raided: the streamer sent their viewers on, and the stream is ending
          or has ended. The card says where they went instead of the player's
          network error, and offers that channel as a tile of its own. */}
      {raid && (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 bg-black/85 backdrop-blur-sm z-30 px-4 text-center">
          {raid.target_image ? (
            <img
              src={raid.target_image}
              alt=""
              className="w-14 h-14 rounded-full object-cover ring-2 ring-accent/60"
            />
          ) : (
            <div className="w-14 h-14 rounded-full bg-white/[0.06] flex items-center justify-center">
              <Radio className="w-5 h-5 text-textMuted" />
            </div>
          )}
          <div className="min-w-0 max-w-[240px]">
            <p className="text-xs text-textMuted truncate">{channelName || channelLogin} raided</p>
            <p className="text-sm font-semibold text-white/95 truncate">{raid.target_name}</p>
            {raid.target_title && (
              <p className="text-xs text-textSecondary truncate mt-0.5">{raid.target_title}</p>
            )}
          </div>
          <div className="flex items-center gap-2">
            {raidTargetInGrid ? (
              <span className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold text-textMuted">
                <Check className="w-3.5 h-3.5" /> In MultiNook
              </span>
            ) : (
              <button
                onClick={async () => {
                  setAddingRaidTarget(true);
                  try {
                    await addSlot(raid.target_login);
                  } finally {
                    setAddingRaidTarget(false);
                  }
                }}
                disabled={addingRaidTarget}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg glass-button text-accent hover:text-white text-xs font-semibold disabled:opacity-60"
              >
                {addingRaidTarget ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Plus className="w-3.5 h-3.5" />}
                Add to MultiNook
              </button>
            )}
            <Tooltip content="Show this tile's stream again" delay={300} side="bottom">
              <button
                onClick={() => dismissSlotRaid(id)}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg glass-button text-textSecondary hover:text-white text-xs font-semibold"
              >
                Dismiss
              </button>
            </Tooltip>
          </div>
        </div>
      )}

      {/* Stream Title Overlay — Top-left (Matches VideoPlayer) */}
      <div
        className={`stream-title-overlay absolute top-0 left-0 right-0 z-40 transition-all duration-300 opacity-0 group-hover:opacity-100`}
      >
        <div className="absolute inset-0 bg-gradient-to-b from-black/60 via-black/20 to-transparent pointer-events-none" />
        <div className={`relative flex items-start justify-between ${full ? 'px-3 pt-2 pb-6' : 'px-2 pt-1.5 pb-4'}`}>
          {/* Absolute Center Grab Handle — hidden while maximized (nothing to
              reorder) and on the smallest tiles, which move with Make main or
              the right-click menu instead. */}
          {!isMaximized && !mini && (
            <div className="absolute left-1/2 -translate-x-1/2 top-1.5 z-20">
              <Tooltip content="Drag to reposition stream" delay={500} side="top">
                <div
                  className={`cursor-grab active:cursor-grabbing flex items-center justify-center ${full ? 'px-3 py-1' : 'px-2 py-0.5'} glass-button rounded-lg text-emerald-300 hover:text-emerald-200 active:scale-95 [&_*]:cursor-grab`}
                  style={{ backgroundColor: 'color-mix(in srgb, rgb(16 185 129) 20%, var(--glass-under, transparent))', backdropFilter: 'blur(16px)' }}
                  {...attributes}
                  {...listeners}
                >
                  <GripHorizontal className={`${full ? 'w-5 h-5' : 'w-4 h-4'} drop-shadow-md`} />
                </div>
              </Tooltip>
            </div>
          )}

          {/* Left: channel identity, then the stream title beneath it */}
          <div className={`flex-1 min-w-0 z-10 ${mini ? 'pr-2' : 'pr-12'}`}>
            {/* Sized to match the full player's identity row. No live ring on the
                avatar though — a tile can be offline. */}
            <div className={`flex items-center min-w-0 ${full ? 'gap-2 mt-1' : 'gap-1.5 mt-0.5'}`}>
              {mini ? null : profileImageUrl ? (
                <img
                  src={profileImageUrl}
                  alt=""
                  draggable={false}
                  className={`${full ? 'w-7 h-7' : 'w-5 h-5'} rounded-full object-cover shrink-0 bg-black/20`}
                />
              ) : (
                <div className={`${full ? 'w-7 h-7 text-[12px]' : 'w-5 h-5 text-[10px]'} rounded-full bg-white/15 shrink-0 flex items-center justify-center font-bold text-white`}>
                  {(channelName || channelLogin || '?').charAt(0).toUpperCase()}
                </div>
              )}
              <Tooltip content={channelName || channelLogin} delay={200} side="top">
                {/* Plain text, not StreamTitleWithEmojis: that component makes a
                    Tauri round-trip per string, and a Twitch display name is the
                    login re-cased or a CJK localization — never emoji. */}
                {/* min-w-0: a flex item won't shrink below its content width by
                    default, which makes `truncate` a no-op and overflows instead. */}
                <h3 className={`${full ? 'text-[15px]' : mini ? 'text-[11px]' : 'text-[13px]'} font-semibold truncate min-w-0 drop-shadow-lg select-none text-white/90`}>
                  {channelName || channelLogin}
                </h3>
              </Tooltip>
              {/* Twitch's partner mark, on Twitch tiles only: broadcasterType is a
                  Helix field and means nothing on another platform. */}
              {(provider ?? 'twitch') === 'twitch' && broadcasterType === 'partner' && (
                <TwitchVerifiedMark size={14} className="text-[#9146FF] shrink-0" />
              )}
              {/* Which platform this tile is. Only shown when it is NOT the
                  default, so an all-Twitch grid looks exactly as it always has. */}
              {provider && provider !== 'twitch' && (
                <ProviderLogo provider={provider} size={13} className="shrink-0" />
              )}
              {isFocused && (
                <Tooltip content="Focused Stream" delay={200} side="right">
                  <i className="ri-focus-3-line text-white/80 text-[13px] shrink-0" />
                </Tooltip>
              )}
            </div>
            {full && title?.trim() && (
              <Tooltip content={title} delay={200} side="top">
                <p className="text-white/70 text-[13px] mt-1 line-clamp-1 drop-shadow-md select-none">
                  <StreamTitleWithEmojis title={title} />
                </p>
              </Tooltip>
            )}
          </div>

          {/* Controls Overlay - Top Right */}
          <div className="flex items-center gap-1.5 shrink-0">
            {/* Follow + Subscribe — focused tile only, honoring the Player
                Overlay Buttons setting like the single-stream player */}
            {showFollowButton && (
                <Tooltip
                  content={
                    checkingFollowStatus
                      ? 'Checking follow status...'
                      : followLoading
                        ? 'Processing...'
                        : isFollowing
                          ? `Unfollow ${channelName || channelLogin}`
                          : `Follow ${channelName || channelLogin}`
                  }
                  delay={200}
                  side="top"
                >
                  <button
                    onClick={handleFollowClick}
                    disabled={followLoading || checkingFollowStatus}
                    className={`${glassButton} ${followLoading || checkingFollowStatus ? 'opacity-60 cursor-wait' : ''}`}
                    style={{ backdropFilter: 'blur(16px)' }}
                  >
                    {followLoading || checkingFollowStatus ? (
                      <Loader2 className="w-4 h-4 animate-spin text-textSecondary" />
                    ) : heartDropAnimation ? (
                      <HeartBreak weight="fill" className="w-4 h-4 text-red-400 animate-heart-drop" />
                    ) : isFollowing ? (
                      <HeartBreak weight="fill" className="w-4 h-4 text-red-400 drop-shadow-[0_0_5px_color-mix(in_srgb,var(--color-error)_70%,transparent)]" />
                    ) : (
                      <Heart weight="fill" className="w-4 h-4 text-emerald-400 drop-shadow-[0_0_5px_color-mix(in_srgb,var(--color-success)_70%,transparent)]" />
                    )}
                  </button>
                </Tooltip>
            )}
            {showSubscribeButton && (
                <Tooltip
                  content={
                    isSubscribed
                      ? `Gift a sub to ${channelName || channelLogin}'s community`
                      : hasSubHistory
                        ? `Resubscribe to ${channelName || channelLogin} (${cumulativeMonths + 1} months)`
                        : `Subscribe to ${channelName || channelLogin}`
                  }
                  delay={200}
                  side="top"
                >
                  <button
                    onClick={handleSubscribeClick}
                    className={glassButton}
                    style={{ backdropFilter: 'blur(16px)' }}
                  >
                    {subscriberBadgeUrl ? (
                      <img
                        src={subscriberBadgeUrl}
                        alt="Subscriber badge"
                        className="w-4 h-4 object-contain"
                        referrerPolicy="no-referrer"
                      />
                    ) : (
                      <svg className="w-4 h-4 text-white" fill="currentColor" viewBox="0 0 20 20">
                        <path d="M9.049 2.927c.3-.921 1.603-.921 1.902 0l1.07 3.292a1 1 0 00.95.69h3.462c.969 0 1.371 1.24.588 1.81l-2.8 2.034a1 1 0 00-.364 1.118l1.07 3.292c.3.921-.755 1.688-1.54 1.118l-2.8-2.034a1 1 0 00-1.175 0l-2.8 2.034c-.784.57-1.838-.197-1.539-1.118l1.07-3.292a1 1 0 00-.364-1.118L2.98 8.72c-.783-.57-.38-1.81.588-1.81h3.461a1 1 0 00.951-.69l1.07-3.292z" />
                      </svg>
                    )}
                  </button>
                </Tooltip>
            )}

            {/* Make main: swap places with the large tile; sound and chat follow. */}
            {canMakeMain && (
              <Tooltip content="Make main" delay={200} side="top">
                <button
                  onClick={() => makeMainSlot(id)}
                  className={glassButton}
                  style={{ backdropFilter: 'blur(16px)' }}
                  aria-label="Make main"
                >
                  <ArrowLeftRight className={`${full ? 'w-4 h-4' : 'w-3.5 h-3.5'} text-white`} />
                </button>
              </Tooltip>
            )}

            {/* Spotlight this stream (fills the space) / restore the grid */}
            {!mini && (
            <Tooltip content={isMaximized ? 'Back to grid · double-click or Esc' : 'Spotlight · double-click'} delay={200} side="top">
              <button
                onClick={() => toggleMaximizeSlot(id)}
                className={glassButton}
                style={{ backdropFilter: 'blur(16px)' }}
              >
                {isMaximized ? (
                  <Minimize2 className="w-4 h-4 text-white" />
                ) : (
                  <Maximize2 className={`${full ? 'w-4 h-4' : 'w-3.5 h-3.5'} text-white`} />
                )}
              </button>
            </Tooltip>
            )}

            {/* Dock (minimize to the tray strip) — hidden while maximized */}
            {!isMaximized && !mini && (
              <Tooltip content="Dock Stream" delay={200} side="top">
                <button
                  onClick={() => dockSlot(id)}
                  className={glassButton}
                  style={{ backdropFilter: 'blur(16px)' }}
                >
                  <Undo2 className="w-4 h-4 text-white" />
                </button>
              </Tooltip>
            )}

            {/* Close (remove from grid) */}
            <Tooltip content="Close Stream" delay={200} side="top">
              <button
                onClick={() => removeSlot(id)}
                className={glassButton}
                style={{ backgroundColor: 'color-mix(in srgb, rgb(239 68 68) 25%, var(--glass-under, transparent))', backdropFilter: 'blur(16px)' }}
              >
                <XIcon weight="bold" className="w-4 h-4 text-red-400" />
              </button>
            </Tooltip>
          </div>
        </div>
      </div>

      {/* This tile's Audio Boost panel. Sits above the control bar and scrolls
          inside a short tile; globals.css pins it to the viewport while the tile
          is fullscreen. Swallows pointer events so it never focuses, maximizes
          or re-volumes the tile underneath. */}
      <AnimatePresence>
        {boostPanelOpen && (
          <motion.div
            ref={boostPanelRef}
            initial={{ opacity: 0, y: 8, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 8, scale: 0.98 }}
            transition={{ duration: 0.16, ease: 'easeOut' }}
            onClick={(e) => e.stopPropagation()}
            onDoubleClick={(e) => e.stopPropagation()}
            onMouseDown={(e) => e.stopPropagation()}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
            }}
            data-no-wheel-volume
            className="tile-audio-boost-panel liquid-glass-panel absolute bottom-14 right-2 z-[60] overflow-y-auto rounded-xl p-4 cursor-default"
            style={{ width: 'min(460px, calc(100% - 16px))', maxHeight: 'calc(100% - 64px)' }}
          >
            <AudioBoostControls
              boost={resolvedBoost}
              onPatch={patchTileBoost}
              note={ownBoost ? 'Set for this stream only.' : 'Matching the main player. Changes here apply to this stream only.'}
              footer={
                ownBoost ? (
                  <button
                    onClick={() => setSlotAudioBoost(id, undefined)}
                    className="text-xs text-textSecondary underline-offset-2 hover:text-textPrimary hover:underline"
                  >
                    Match main player
                  </button>
                ) : null
              }
            />
          </motion.div>
        )}
      </AnimatePresence>

      <PlayerVolumeOsd osd={osd} boost={resolvedBoost.enabled && AUDIO_GRAPH_SUPPORTED ? resolvedBoost.gain : 1} />
    </motion.div>
  );
};

/** Shallow-compares `customStyle`, which MultiNookView rebuilds as a fresh object
 *  literal on every render (`{ width, height }` from the layout solver). Without
 *  this the memo below could never bail. `slot` is compared by reference, which
 *  works because the store now preserves the identity of slots it did not
 *  actually change. */
const sameStyle = (a?: React.CSSProperties, b?: React.CSSProperties): boolean => {
  if (a === b) return true;
  if (!a || !b) return false;
  const aKeys = Object.keys(a) as (keyof React.CSSProperties)[];
  const bKeys = Object.keys(b) as (keyof React.CSSProperties)[];
  if (aKeys.length !== bKeys.length) return false;
  return aKeys.every((k) => a[k] === b[k]);
};

export const MultiNookCell = React.memo(MultiNookCellInner, (prev, next) => {
  if (prev.slot !== next.slot) return false;
  if (prev.cssOrder !== next.cssOrder) return false;
  if (prev.gridSpanClass !== next.gridSpanClass) return false;
  if (prev.isMaximized !== next.isMaximized) return false;
  if (prev.sizeTier !== next.sizeTier || prev.canMakeMain !== next.canMakeMain) return false;
  return sameStyle(prev.customStyle, next.customStyle);
});

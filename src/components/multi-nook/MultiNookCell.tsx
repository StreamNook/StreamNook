import React, { useEffect, useState, useCallback, useRef } from 'react';
import { useSortable } from '@dnd-kit/sortable';
import { motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { MultiNookSlot } from '../../types';
import { useMultiNookPlayer } from './useMultiNookPlayer';
import { usemultiNookStore } from '../../stores/multiNookStore';
import { useAppStore } from '../../stores/AppStore';
import { buildProviderUrl } from '../../utils/streamProvider';
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
import { GripHorizontal, Undo2, Loader2, RefreshCcw, EyeOff, WifiOff, Maximize2, Minimize2 } from 'lucide-react';
import { Heart, HeartBreak, X as XIcon } from 'phosphor-react';
import { Logger } from '../../utils/logger';
import { canGridProvider } from '../../types/providers';

interface MultiNookCellProps {
  slot: MultiNookSlot;
  cssOrder?: number;
  gridSpanClass?: string;
  customStyle?: React.CSSProperties;
  /** True when this tile is filling the whole grid area (solo-like). */
  isMaximized?: boolean;
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

const MultiNookCellInner: React.FC<MultiNookCellProps> = ({ slot, cssOrder, gridSpanClass = '', customStyle = {}, isMaximized = false }) => {
  const { id, provider, channelLogin, channelName, channelId, volume, muted, isFocused, streamUrl, isMinimized = false, loadError, profileImageUrl, title, broadcasterType } = slot;
  // Actions only, so read them without subscribing. A bare `usemultiNookStore()`
  // here subscribed this tile to the WHOLE store, which meant any mutation
  // (including a volume drag on a sibling tile) re-rendered every tile in the
  // grid. Zustand actions keep the same identity for the store's lifetime.
  const { toggleFocusSlot, toggleMaximizeSlot, dockSlot, removeSlot, changeSlotQuality, retrySlot } =
    usemultiNookStore.getState();

  // Offline tiles show the offline overlay instead of an endless loading spinner.
  const isLoading = !streamUrl && !loadError;

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
  const showFollowButton = socialEnabled && playerOverlayButtonOn(playerOverlayButtons, 'follow');
  const showSubscribeButton =
    socialEnabled && offersMembership && playerOverlayButtonOn(playerOverlayButtons, 'subscribe');

  // Available stream qualities for the focused tile's gear menu
  const [availableQualities, setAvailableQualities] = useState<string[]>([]);
  useEffect(() => {
    if (!socialEnabled) return;
    // A tile the grid refuses should never exist, so this is belt and braces
    // for any provider added to GRID_BLOCKED later: get_stream_qualities
    // resolves playback, and resolving is the expensive, side-effectful half of
    // an adapter. Cheap to keep, and it means a refused provider can never reach
    // the backend from here.
    //
    // Note this is a FRONTEND guard on purpose. The solo player calls the very
    // same command (AppStore.getStreamQualities), where resolving is correct and
    // expected, so refusing inside the Rust command would break the solo quality
    // menu to protect the grid.
    if (!canGridProvider(provider ?? 'twitch')) return;
    let cancelled = false;
    invoke<string[]>('get_stream_qualities', { url: buildProviderUrl(provider ?? 'twitch', channelLogin) })
      .then((qs) => {
        if (!cancelled && qs?.length) setAvailableQualities(qs);
      })
      .catch((e) => Logger.warn(`[MultiNook] Failed to fetch qualities for ${channelLogin}`, e));
    return () => {
      cancelled = true;
    };
  }, [socialEnabled, channelLogin, provider]);

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

    const qualityMenuItem = document.createElement('button');
    qualityMenuItem.className = 'plyr__control';
    qualityMenuItem.setAttribute('data-plyr', 'quality');
    qualityMenuItem.setAttribute('type', 'button');
    qualityMenuItem.setAttribute('role', 'menuitem');
    qualityMenuItem.innerHTML = `<span>Quality<span class="plyr__menu__value">${cap(displayedQuality)}</span></span>`;
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
  }, [availableQualities, slot.quality, id, changeSlotQuality, playerRef]);

  // Add the quality submenu when focused; strip it back out when not (so
  // non-focused tiles keep just the default playback gear).
  useEffect(() => {
    const player = playerRef.current as unknown as { elements?: { container?: HTMLElement } } | null;
    const container = player?.elements?.container;
    if (!container) return;

    let timer: number | undefined;
    if (socialEnabled && availableQualities.length > 0) {
      // Defer so Plyr has finished rendering its menu DOM
      timer = window.setTimeout(() => updateQualityMenu(), 200);
    } else {
      const menu = container.querySelector('.plyr__menu');
      menu?.querySelector('[data-quality-menu]')?.remove();
      menu?.querySelector('[data-plyr="quality"]')?.remove();
    }
    return () => {
      if (timer) window.clearTimeout(timer);
    };
    // isPlaying/streamUrl re-trigger after the player (re)initialises
  }, [socialEnabled, availableQualities, updateQualityMenu, isPlaying, streamUrl, playerRef]);

  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    isDragging,
  } = useSortable({ id });

  // Merge dnd-kit's node ref with our own so we can attach a native listener.
  const cellRef = useRef<HTMLDivElement | null>(null);
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
      if (target.closest('button') || target.closest('.plyr__controls') || target.closest('.plyr__menu')) return;
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

  const glassButton = 'flex items-center justify-center p-1.5 glass-button rounded-lg';

  return (
    <motion.div
      layout
      animate={{ x, y, scale }}
      transition={isDragging ? { duration: 0 } : { type: 'spring', stiffness: 350, damping: 30 }}
      ref={setRefs}
      style={combinedStyle}
      onClick={(e) => {
        // Ignore clicks on buttons, tools, or plyr control sliders.
        const target = e.target as HTMLElement;
        if (target.closest('button') || target.closest('.plyr__controls') || target.closest('.plyr__menu')) return;
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
        isFocused && !isMaximized ? 'shadow-[0_0_25px_var(--color-accent-muted)]' : ''
      } ${
        isDragging ? 'opacity-50 blur-sm' : 'opacity-100'
      } bg-black/40 transition-opacity duration-300 group flex items-center justify-center video-player-container [&_.plyr]:w-full [&_.plyr]:h-full [&_.plyr]:absolute [&_.plyr]:inset-0 ${
        isMaximized ? 'cursor-default' : 'cursor-pointer'
      }`}
    >
      <video
        ref={videoRef}
        className="w-full h-full"
        style={{ backgroundColor: '#000', objectFit: isMaximized ? 'contain' : 'cover' }}
        autoPlay
        playsInline
      />

      {/* Loading & Error States */}
      {(isLoading || isBuffering) && !error && !loadError && (
        <div className="absolute inset-0 flex items-center justify-center bg-black/50 backdrop-blur-sm z-10 pointer-events-none">
          <i className="ri-loader-4-line text-4xl text-white animate-spin"></i>
        </div>
      )}

      {/* Offline / unreachable: the proxy could not start (e.g. the streamer is
          offline). Lets the user retry or hide the tile so it stops eating grid
          space while the others play. */}
      {loadError && (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 bg-black/85 backdrop-blur-sm z-30 px-4 text-center">
          {profileImageUrl ? (
            <img
              src={profileImageUrl}
              alt=""
              className="w-12 h-12 rounded-full object-cover ring-2 ring-white/10 grayscale opacity-80"
            />
          ) : (
            <div className="w-12 h-12 rounded-full bg-white/[0.06] flex items-center justify-center">
              <WifiOff className="w-5 h-5 text-textMuted" />
            </div>
          )}
          <div>
            <p className="text-sm font-semibold text-white/90 truncate max-w-[220px]">
              {channelName || channelLogin}
            </p>
            <p className="text-xs text-textMuted mt-0.5">Offline or unreachable</p>
          </div>
          <div className="flex items-center gap-2">
            <Tooltip content="Try loading this stream again" delay={300} side="bottom">
              <button
                onClick={() => retrySlot(id)}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg glass-button text-textSecondary hover:text-accent text-xs font-semibold"
              >
                <RefreshCcw className="w-3.5 h-3.5" /> Retry
              </button>
            </Tooltip>
            <Tooltip content="Hide this stream (tuck it into the dock tray)" delay={300} side="bottom">
              <button
                onClick={() => dockSlot(id)}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg glass-button text-textSecondary hover:text-white text-xs font-semibold"
              >
                <EyeOff className="w-3.5 h-3.5" /> Hide
              </button>
            </Tooltip>
          </div>
        </div>
      )}

      {error && (
        <div className="absolute inset-0 flex flex-col items-center justify-center bg-black/80 z-10 text-rose-500 pointer-events-none">
          <i className="ri-error-warning-fill text-4xl mb-2"></i>
          <p className="text-sm font-medium">{error}</p>
        </div>
      )}

      {/* Stream Title Overlay — Top-left (Matches VideoPlayer) */}
      <div
        className={`stream-title-overlay absolute top-0 left-0 right-0 z-40 transition-all duration-300 opacity-0 group-hover:opacity-100`}
      >
        <div className="absolute inset-0 bg-gradient-to-b from-black/60 via-black/20 to-transparent pointer-events-none" />
        <div className="relative px-3 pt-2 pb-6 flex items-start justify-between">
          {/* Absolute Center Grab Handle — hidden while maximized (nothing to reorder) */}
          {!isMaximized && (
            <div className="absolute left-1/2 -translate-x-1/2 top-1.5 z-20">
              <Tooltip content="Drag to reposition stream" delay={500} side="top">
                <div
                  className="cursor-grab active:cursor-grabbing flex items-center justify-center px-3 py-1 glass-button rounded-lg text-emerald-300 hover:text-emerald-200 active:scale-95 [&_*]:cursor-grab"
                  style={{ backgroundColor: 'rgba(16, 185, 129, 0.20)', backdropFilter: 'blur(16px)' }}
                  {...attributes}
                  {...listeners}
                >
                  <GripHorizontal className="w-5 h-5 drop-shadow-md" />
                </div>
              </Tooltip>
            </div>
          )}

          {/* Left: channel identity, then the stream title beneath it */}
          <div className="flex-1 min-w-0 pr-12 z-10">
            {/* Sized to match the full player's identity row. No live ring on the
                avatar though — a tile can be offline. */}
            <div className="flex items-center gap-2 min-w-0 mt-1">
              {profileImageUrl ? (
                <img
                  src={profileImageUrl}
                  alt=""
                  draggable={false}
                  className="w-7 h-7 rounded-full object-cover shrink-0 bg-black/20"
                />
              ) : (
                <div className="w-7 h-7 rounded-full bg-white/15 shrink-0 flex items-center justify-center text-[12px] font-bold text-white">
                  {(channelName || channelLogin || '?').charAt(0).toUpperCase()}
                </div>
              )}
              <Tooltip content={channelName || channelLogin} delay={200} side="top">
                {/* Plain text, not StreamTitleWithEmojis: that component makes a
                    Tauri round-trip per string, and a Twitch display name is the
                    login re-cased or a CJK localization — never emoji. */}
                {/* min-w-0: a flex item won't shrink below its content width by
                    default, which makes `truncate` a no-op and overflows instead. */}
                <h3 className="text-[15px] font-semibold truncate min-w-0 drop-shadow-lg select-none text-white/90">
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
            {title?.trim() && (
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

            {/* Spotlight this stream (fills the space) / restore the grid */}
            <Tooltip content={isMaximized ? 'Back to grid · double-click or Esc' : 'Spotlight · double-click'} delay={200} side="top">
              <button
                onClick={() => toggleMaximizeSlot(id)}
                className={glassButton}
                style={{ backdropFilter: 'blur(16px)' }}
              >
                {isMaximized ? (
                  <Minimize2 className="w-4 h-4 text-white" />
                ) : (
                  <Maximize2 className="w-4 h-4 text-white" />
                )}
              </button>
            </Tooltip>

            {/* Dock (minimize to the tray strip) — hidden while maximized */}
            {!isMaximized && (
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
                style={{ backgroundColor: 'rgba(239, 68, 68, 0.25)', backdropFilter: 'blur(16px)' }}
              >
                <XIcon weight="bold" className="w-4 h-4 text-red-400" />
              </button>
            </Tooltip>
          </div>
        </div>
      </div>

      <PlayerVolumeOsd osd={osd} />
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
  return sameStyle(prev.customStyle, next.customStyle);
});

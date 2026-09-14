import { Window } from '@tauri-apps/api/window';
import { Gift, User, Settings, Store, Proportions, MessageCircle, Pickaxe, Clock, Tv, Download, LogIn, Sparkles, Check, Pin, PinOff, Home } from 'lucide-react';
import { Minus, X, CornersOut, CornersIn, ArrowsOut, ArrowsIn, Medal } from 'phosphor-react';
import { useState, useEffect, useLayoutEffect, useRef, useMemo, useCallback } from 'react';
import { createPortal } from 'react-dom';
import { motion, AnimatePresence } from 'framer-motion';
import { useShallow } from 'zustand/react/shallow';
import { useAppStore } from '../stores/AppStore';
import PenroseLogo from './PenroseLogo';
import PlatformSwitcher from './PlatformSwitcher';
import AboutWidget from './AboutWidget';
import UpdateOverlay, { type UpdatePhase } from './UpdateOverlay';
import CompactStreamStats from './CompactStreamStats';
import { captureResumeSnapshot } from '../services/sessionResume';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getSelectedCompactViewPreset } from '../constants/compactViewPresets';
import type { DropProgressStatus, DropsSettings, DropProgress } from '../types';
import { deriveDropProgressDisplay } from '../utils/dropProgressDisplay';


import { Logger } from '../utils/logger';
import { useVisibleInterval } from '../utils/useVisibleInterval';
import { handleTitleBarMouseDown } from '../utils/titleBarDrag';
import { Tooltip } from './ui/Tooltip';
import { isSemiquincentennialShowDay, openSemiquincentennialShow } from '../services/semiquincentennialEvent';
import PluginTitleBarButtons from '../plugins-ui/PluginTitleBarButtons';
import { usePluginUpdates } from '../stores/pluginUpdatesStore';

/** Maps a bundle-update-progress payload to a 0–100 fill. The download stage
 *  carries a real byte percentage ("Downloading 47%"); the quick post-download
 *  stages are fixed points near the end. */
const getUpdateStageProgress = (stage: string | null): number => {
  if (!stage) return 0;
  // Real byte progress from the streamed download (already scaled to 0–90).
  const pct = stage.match(/(\d+)\s*%/);
  if (pct) return Math.min(100, parseInt(pct[1], 10));
  const s = stage.toLowerCase();
  if (s.includes('installed') || s.includes('complete')) return 100;
  if (s.includes('restart')) return 98;
  if (s.includes('install')) return 95;
  if (s.includes('extract')) return 92;
  if (s.includes('download')) return 2;
  return 0;
};

const TitleBar = () => {
  // Actions are stable for the store's lifetime, so take them without
  // subscribing; state goes through a shallow-compared selector. This was a
  // whole-store subscription, so the title bar re-rendered on every unrelated
  // store tick.
  const { openSettings, setShowDropsOverlay, setShowMarketplaceOverlay, setShowBadgesOverlay, setShowWhispersOverlay, toggleTheaterMode, toggleWindowFullscreen, toggleKeepOnTop, toggleHome, setHomeActiveTab, addToast } = useAppStore.getState();
  const { isAuthenticated, currentUser, dropProgressActive, dropProgressComplete, isTheaterMode, isWindowFullscreen, isHomeActive, streamUrl, currentMediaType, settings, whisperImportState, updateInfo } = useAppStore(
    useShallow((s) => ({
      isAuthenticated: s.isAuthenticated,
      currentUser: s.currentUser,
      dropProgressActive: s.dropProgressActive,
      dropProgressComplete: s.dropProgressComplete,
      isTheaterMode: s.isTheaterMode,
      isWindowFullscreen: s.isWindowFullscreen,
      isHomeActive: s.isHomeActive,
      streamUrl: s.streamUrl,
      currentMediaType: s.currentMediaType,
      settings: s.settings,
      whisperImportState: s.whisperImportState,
      updateInfo: s.updateInfo,
    })),
  );
  // The pin only renders inside Compact View, so this is the whole story.
  const keepOnTop = settings?.keep_on_top_in_compact === true;
  // Count of installed plugins with an update available, for the Marketplace badge.
  const pluginUpdateCount = usePluginUpdates((s) => s.ids.length);
  // Update flow: 'idle' → 'installing' (download/extract) → 'installed' (staged;
  // the card shows a brief restart notice, then the app auto-restarts) | 'error'.
  const [updatePhase, setUpdatePhase] = useState<UpdatePhase | 'idle'>('idle');
  const [updateProgress, setUpdateProgress] = useState<string | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [showAbout, setShowAbout] = useState(false);
  const [, setShowSplash] = useState(false);
  const [dropsSettings, setDropsSettings] = useState<DropsSettings | null>(null);
  // Whether the separate drops/points credential (its own Twitch sign-in) is
  // present. null = not yet checked. Drives the drops button's "needs sign-in"
  // cue — a logout (main or drops) clears this credential and swaps the gift for
  // a sign-in button.
  const [dropsAuthed, setDropsAuthed] = useState<boolean | null>(null);
  const [isMaximized, setIsMaximized] = useState(false);
  const prevDropProgressActive = useRef(dropProgressActive);
  
  // Automation status state for progress badge and hover preview
  const [dropProgress, setDropProgress] = useState<DropProgressStatus | null>(null);
  // Live per-drop progress, accumulated from 'drops-progress-update' events. The
  // backend's current_drop carries minutes that only move on its slower poll, so
  // the badge percentage is derived from this fresher stream instead — the same
  // source the overlay cards and detail panel trust, keeping all three aligned.
  const [liveProgress, setLiveProgress] = useState<DropProgress[]>([]);
  const [showDropsPreview, setShowDropsPreview] = useState(false);
  const [previewPos, setPreviewPos] = useState<{ top: number; left: number }>({ top: 0, left: 0 });
  const dropsButtonRef = useRef<HTMLDivElement>(null);
  const previewTimeoutRef = useRef<NodeJS.Timeout | null>(null);

  // Dynamic badge icon state
  const [badgeImages, setBadgeImages] = useState<string[]>([]);
  const [currentBadgeUrl, setCurrentBadgeUrl] = useState<string | null>(null);
  const badgeIndexRef = useRef(0);


  // Track window maximize state
  useEffect(() => {
    const checkMaximized = async () => {
      const window = Window.getCurrent();
      const maximized = await window.isMaximized();
      setIsMaximized(maximized);
    };

    checkMaximized();

    // Listen for window resize events
    const unlisten = Window.getCurrent().onResized(async () => {
      await checkMaximized();
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);



  // Load drops settings. These are user preferences that only change when the
  // user toggles them in the settings dialog — polling at 5s was wildly
  // over-aggressive. Once on mount + once an hour as a stale-protection
  // safety net, gated on window visibility.
  const loadDropsSettings = useCallback(async () => {
    try {
      const settings = await invoke<DropsSettings>('get_drops_settings');
      setDropsSettings(settings);
    } catch (err) {
      Logger.error('Failed to get drops settings:', err);
    }
  }, []);
  useEffect(() => {
    loadDropsSettings();
  }, [loadDropsSettings]);
  useVisibleInterval(loadDropsSettings, 60 * 60 * 1000);

  // Track the drops/points sign-in so the drops button can flag when it's gone.
  // Re-checks on login changes, on a slow interval, and on window focus (which
  // fires right after the drops-login window closes, clearing the cue promptly).
  const checkDropsAuth = useCallback(async () => {
    try {
      setDropsAuthed(await invoke<boolean>('is_drops_authenticated'));
    } catch {
      setDropsAuthed(null);
    }
  }, []);
  useEffect(() => {
    checkDropsAuth();
  }, [checkDropsAuth, isAuthenticated]);
  useVisibleInterval(checkDropsAuth, 60 * 1000);
  useEffect(() => {
    const onFocus = () => checkDropsAuth();
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, [checkDropsAuth]);

  // Start the drops/points (separate Twitch) sign-in directly from the title bar
  // login button: open Twitch's device-code page, poll for the token, then clear
  // the prompt. Mirrors the flow the Drops panel and setup wizard use.
  const [isDropsLoggingIn, setIsDropsLoggingIn] = useState(false);
  const handleDropsLogin = useCallback(async () => {
    if (isDropsLoggingIn) return;
    setIsDropsLoggingIn(true);
    try {
      const info = await invoke<{ user_code: string; verification_uri: string; device_code: string; interval: number; expires_in: number }>('start_drops_device_flow');
      addToast(`Enter code ${info.user_code} to enable drops & channel points`, 'info');
      // Opened from Rust bound to the active account's web profile, so it reuses
      // the main login's twitch.tv session — just authorize, no re-login.
      await invoke('open_drops_login_window', { url: info.verification_uri });
      await invoke('poll_drops_token', {
        deviceCode: info.device_code,
        interval: info.interval,
        expiresIn: info.expires_in,
      });
      try {
        await invoke('close_login_overlay', { label: 'drops-login' });
      } catch { /* already dismissed by the backend */ }
      addToast('Signed in — drops & channel points enabled', 'success');
      await checkDropsAuth();
    } catch (e) {
      Logger.error('[TitleBar] Drops login failed:', e);
      addToast('Drops sign-in failed. Please try again.', 'error');
    } finally {
      setIsDropsLoggingIn(false);
    }
  }, [isDropsLoggingIn, addToast, checkDropsAuth]);

  // Seed the progress badge from the bridge-cached automation status (a plugin
  // powering automation reports through it). Live updates arrive on the
  // 'drop-progress' event; this is just a stale-protection backup, so
  // don't let it wipe an active session back to the no-progress (gift) state.
  const loadAutomationStatus = useCallback(() => {
    const status = useAppStore.getState().liveDropProgress;
    if (!status) return;
    if (!status.active && useAppStore.getState().dropProgressActive) return;
    setDropProgress(status);
  }, []);

  useEffect(() => {
    let unlistenStatus: (() => void) | undefined;
    let unlistenProgress: (() => void) | undefined;
    let isMounted = true;

    const setupListeners = async () => {
      // Listen for automation status updates
      const uStatus = await listen<DropProgressStatus>('drop-progress', (event) => {
        setDropProgress(event.payload);
        // Drop the accumulated per-drop progress once automation stops so a finished
        // session's numbers can't leak into the next one's fallback derivation.
        if (!event.payload.active) setLiveProgress([]);
      });
      if (isMounted) unlistenStatus = uStatus;
      else uStatus();

      // Listen for progress updates (more frequent)
      const uProgress = await listen<{ drop_id: string; current_minutes: number; required_minutes: number; campaign_id?: string; drop_name?: string; timestamp?: number | string }>('drops-progress-update', (event) => {
        const { drop_id: dropId, current_minutes: currentMinutes, required_minutes: requiredMinutes } = event.payload;

        // Keep a live, per-drop progress map. The badge percentage is derived
        // from this (via deriveDropProgressDisplay) so it tracks the freshest minutes
        // and can still show a value when current_drop hasn't been set yet.
        setLiveProgress((prev) => {
          const idx = prev.findIndex((p) => p.drop_id === dropId);
          const entry: DropProgress = {
            campaign_id: event.payload.campaign_id || (idx >= 0 ? prev[idx].campaign_id : ''),
            drop_id: dropId,
            current_minutes_watched: currentMinutes,
            required_minutes_watched: requiredMinutes,
            is_claimed: false,
            last_updated: String(event.payload.timestamp ?? ''),
            drop_name: event.payload.drop_name || (idx >= 0 ? prev[idx].drop_name : undefined),
          };
          if (idx >= 0) {
            const next = [...prev];
            next[idx] = { ...next[idx], ...entry };
            return next;
          }
          return [...prev, entry];
        });

        setDropProgress((prev) => {
          if (!prev || !prev.active) return prev;

          // Only update the displayed drop in place when this event is for it.
          // WHICH drop is shown (the one closest to completion) is decided by
          // the backend and delivered via 'drop-progress'. Ignoring
          // other drops' progress events here is what stops the percentage from
          // flipping between rewards (e.g. the 60-min vs the 180-min reward).
          if (prev.current_drop && prev.current_drop.drop_id === dropId) {
            return {
              ...prev,
              current_drop: {
                ...prev.current_drop,
                current_minutes: currentMinutes,
                required_minutes: requiredMinutes
              }
            };
          }

          return prev;
        });
      });
      if (isMounted) unlistenProgress = uProgress;
      else uProgress();
    };

    loadAutomationStatus();
    setupListeners();

    return () => {
      isMounted = false;
      if (unlistenStatus) unlistenStatus();
      if (unlistenProgress) unlistenProgress();
    };
  }, [loadAutomationStatus]);

  // Backup poll: real-time updates come from the event listeners above. This
  // is just a stale-protection net in case an event was missed. 60-min cadence
  // aligned with the drops-settings poll above. Visibility-gated so it doesn't
  // fire when StreamNook is tucked in the tray.
  useVisibleInterval(loadAutomationStatus, 60 * 60 * 1000);

  // Clean up preview timeout on unmount
  useEffect(() => {
    return () => {
      if (previewTimeoutRef.current) {
        clearTimeout(previewTimeoutRef.current);
      }
    };
  }, []);

  // Load global badge images for the dynamic badge icon
  useEffect(() => {
    const loadBadgeImages = async () => {
      try {
        const cachedBadges = await invoke<{ data: Array<{ set_id: string; versions: Array<{ image_url_2x: string }> }> } | null>('get_cached_global_badges');
        if (cachedBadges?.data && cachedBadges.data.length > 0) {
          
          const isExcludedBadge = (setId: string) => {
            const s = setId.toLowerCase();
            return s.includes('sub') || s.includes('found') || 
                   s.includes('predict') || s.includes('mod') || 
                   s.includes('gift') || s.includes('broadcaster') || 
                   s.includes('partner') || s.includes('verified') || 
                   s.includes('bit') || s.includes('cheer') ||
                   s.includes('develop') || s.includes('audio') || 
                   s.includes('video') || s.includes('listen');
          };
          
          const filteredSets = cachedBadges.data.filter(set => !isExcludedBadge(set.set_id));
          
          const urls = filteredSets
            .flatMap(set => set.versions.map(v => v.image_url_2x))
            .filter(Boolean);
          if (urls.length > 0) {
            // Shuffle the URLs for variety
            const shuffled = [...urls].sort(() => Math.random() - 0.5);
            setBadgeImages(shuffled);
            setCurrentBadgeUrl(shuffled[0]);
            badgeIndexRef.current = 0;
          }
        }
      } catch {
        // Silently fail — Medal icon fallback is fine
      }
    };
    loadBadgeImages();
  }, []);

  // Cycle to next badge on unhover
  const cycleBadgeIcon = useCallback(() => {
    if (badgeImages.length === 0) return;
    const nextIndex = (badgeIndexRef.current + 1) % badgeImages.length;
    badgeIndexRef.current = nextIndex;
    setCurrentBadgeUrl(badgeImages[nextIndex]);
  }, [badgeImages]);

  // Calculate progress percentage through the shared rule so the badge matches
  // the overlay cards and detail panel. Prefers the freshest live minutes, and
  // falls back to the drop finishing first when current_drop isn't set yet (so
  // the badge shows a number instead of reverting to the plain gift icon).
  const progressPercent = useMemo(
    () => deriveDropProgressDisplay(dropProgress, liveProgress)?.percent ?? 0,
    [dropProgress, liveProgress],
  );

  // Handle hover preview show/hide with delay
  const handleDropsMouseEnter = () => {
    if (previewTimeoutRef.current) {
      clearTimeout(previewTimeoutRef.current);
    }
    previewTimeoutRef.current = setTimeout(() => {
      if (dropProgressActive && dropProgress?.current_drop) {
        setShowDropsPreview(true);
      }
    }, 300); // 300ms delay before showing preview
  };

  const handleDropsMouseLeave = () => {
    if (previewTimeoutRef.current) {
      clearTimeout(previewTimeoutRef.current);
    }
    previewTimeoutRef.current = setTimeout(() => {
      setShowDropsPreview(false);
    }, 150); // Small delay before hiding
  };

  // The hover preview is portalled to <body> so it escapes the title bar's
  // own stacking context (the bar is position:relative z-50). Rendered inline,
  // the card's z-index:9999 only competes inside that context, so the compact
  // expand-on-hover sidebar overlay (also z-50, but later in the DOM) painted
  // over it. As a body-level portal it sits above the sidebar. Position it just
  // under the drops button with fixed viewport coordinates from the button rect.
  useLayoutEffect(() => {
    if (!showDropsPreview) return;
    const reposition = () => {
      const el = dropsButtonRef.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      setPreviewPos({ top: Math.round(r.bottom + 8), left: Math.round(r.left) });
    };
    reposition();
    window.addEventListener('resize', reposition);
    window.addEventListener('scroll', reposition, true);
    return () => {
      window.removeEventListener('resize', reposition);
      window.removeEventListener('scroll', reposition, true);
    };
  }, [showDropsPreview]);

  useEffect(() => {
    // Detect when automation stops
    if (prevDropProgressActive.current && !dropProgressActive) {
      queueMicrotask(() => setShowSplash(true));
      setTimeout(() => setShowSplash(false), 600);
    }
    prevDropProgressActive.current = dropProgressActive;
  }, [dropProgressActive]);

  const handleMinimize = async () => {
    const window = Window.getCurrent();
    await window.minimize();
  };

  const handleMaximize = async () => {
    const window = Window.getCurrent();
    await window.toggleMaximize();
  };

  const handleClose = async () => {
    const window = Window.getCurrent();
    await window.close();
  };

  const handleStartUpdate = useCallback(async () => {
    // A download or pending restart is uninterruptible; ignore further clicks.
    if (updatePhase === 'installing' || updatePhase === 'installed') return;
    if (!updateInfo) return;

    setUpdateError(null);
    setUpdateProgress('Starting update…');
    setUpdatePhase('installing');

    const unlisten = await listen<string>('bundle-update-progress', (event) => {
      setUpdateProgress(event.payload);
    });

    try {
      await invoke('download_and_install_bundle');
      // Staged. The overlay shows a restart notice, then calls onRestart.
      setUpdatePhase('installed');
    } catch (e) {
      Logger.error('Update failed:', e);
      setUpdateError(String(e));
      setUpdatePhase('error');
    } finally {
      unlisten();
    }
  }, [updateInfo, updatePhase]);

  // Restart into the staged update. The backend spawns the swap-and-relaunch
  // launcher and exits, so on success nothing after this runs. Triggered
  // automatically by the overlay's restart notice.
  const handleApplyUpdateRestart = useCallback(async () => {
    // Snapshot the session first so the relaunched app can restore the stream
    // (and resume automation) the user was on before the update.
    await captureResumeSnapshot();
    try {
      await invoke('restart_to_apply_update');
      // Reached only in a dev build: production swaps the exe and exits the
      // process inside the command above, so this never runs there. Flag the
      // changelog so the post-reload mount pops it for the version we "updated"
      // to (prod gets this for free via the version-change check), then reload
      // the webview to simulate the restart — re-running the resume-on-launch
      // path without killing the `tauri dev` server.
      const latest = useAppStore.getState().updateInfo?.latest_version;
      if (latest) sessionStorage.setItem('streamnook-dev-changelog', latest);
      window.location.reload();
    } catch (e) {
      Logger.error('Restart to apply update failed:', e);
      setUpdateError(String(e));
      setUpdatePhase('error');
    }
  }, []);

  const handleDismissUpdate = useCallback(() => {
    setUpdatePhase('idle');
    setUpdateError(null);
  }, []);

  // Replaces data-tauri-drag-region: a borderless window doesn't get Windows'
  // restore-on-drag, so dragging while maximized has to unmaximize first.
  const onTitleBarMouseDown = useCallback(
    (e: React.MouseEvent<HTMLElement>) => handleTitleBarMouseDown(e, isMaximized),
    [isMaximized],
  );

  return (
    <>
      <div
        onMouseDown={onTitleBarMouseDown}
        className="relative flex items-center justify-between h-[40px] px-3 select-none bg-secondary backdrop-blur-md border-b border-borderSubtle z-50"
      >
        {/* Dynamic Island is rendered at the app root (App.tsx), not here, so it
            can lift above the Settings blur overlay. It still pins to the top
            center via fixed positioning, so it visually sits in this title bar. */}

        <div className="flex items-center gap-2.5" style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}>
          {/* Penrose Logo */}
          <PenroseLogo onClick={() => setShowAbout(true)} />

          {/* Which platform the app is in. Lives here because the title bar is
              the only chrome mounted in every view: a quiet mark-and-name
              readout beside the logo that opens the switcher on hover. */}
          <PlatformSwitcher />

          {/* Grouped action icons */}
          <div className="titlebar-icon-group">
          {/* Home / Return — one dynamic toggle instead of a "Keep Browsing"
              button in the player overlay and a "Return" button on the far side
              of the Home header. Only show when a stream is playing, same rule
              as the Compact View button: with nothing playing there is nothing
              to browse away from or come back to. */}
          {streamUrl && (
            <Tooltip content={isHomeActive ? 'Return to Stream' : 'Keep Browsing'} delay={200}>
              <button
                onClick={() => {
                  if (!isHomeActive) setHomeActiveTab(isAuthenticated ? 'following' : 'recommended');
                  toggleHome();
                }}
                className="titlebar-icon-btn"
                aria-label={isHomeActive ? 'Return to Stream' : 'Keep Browsing'}
              >
                {isHomeActive ? <Tv size={14} /> : <Home size={14} />}
              </button>
            </Tooltip>
          )}

          {/* Drops Button with Inline Progress Badge */}
          <div
            className="relative"
            ref={dropsButtonRef}
            onMouseEnter={handleDropsMouseEnter}
            onMouseLeave={handleDropsMouseLeave}
          >
            {(() => {
              const channelPointsActive = dropsSettings?.auto_claim_channel_points ?? false;
              const isBothActive = dropProgressActive && channelPointsActive;
              // While drops automation is active the indicator is the drop's
              // percentage (even at 0%), like it has always been, never a
              // placeholder gift. The gift shimmer is only for points-only.
              const showProgressBadge = dropProgressActive;
              // Every watch-time reward for the watched game is earned — show a
              // "done" check instead of the idle gift. Active progress wins over it.
              const showCompleteBadge = dropProgressComplete && !dropProgressActive;

              // Drops and channel points run off a separate Twitch sign-in that
              // logout clears (signing out of the main account signs drops out
              // too). Whenever that credential is missing, swap the gift for a
              // sign-in button (clicking it starts the drops sign-in) — including
              // after a full sign-out, not only while the main account is in.
              const needsDropsAuth = dropsAuthed === false;

              // Determine gift box color/shimmer class
              // Silver = channel points only, Gold = drops only, Iridescent = both
              let giftClass = '';
              let title = 'Drops & Points';

              if (needsDropsAuth) {
                title = 'Sign in to enable drops & channel points';
              } else if (isBothActive) {
                giftClass = 'gift-shimmer-iridescent';
                title = 'Drops & Points (Both Active)';
              } else if (dropProgressActive) {
                giftClass = 'gift-shimmer-gold';
                title = `Drops progress: ${progressPercent}%`;
              } else if (showCompleteBadge) {
                title = 'Drops complete — all rewards earned for this game';
              } else if (channelPointsActive) {
                giftClass = 'gift-shimmer-silver';
                title = 'Drops & Points (Channel Points Active)';
              }

              const isAnyAutomationActive = dropProgressActive || channelPointsActive;

              // Credential missing: the drops slot becomes a direct "Login"
              // button rather than the gift, so the action is unmistakable.
              if (needsDropsAuth) {
                return (
                  <Tooltip content={title} delay={200}>
                    <button
                      onClick={handleDropsLogin}
                      disabled={isDropsLoggingIn}
                      className="flex items-center gap-1 px-1.5 py-1 text-[11px] font-medium text-textSecondary hover:text-accent transition-colors disabled:opacity-60 disabled:cursor-not-allowed"
                    >
                      <LogIn size={12} />
                      {isDropsLoggingIn ? 'Signing in…' : 'Login'}
                    </button>
                  </Tooltip>
                );
              }

              return (
                <Tooltip content={title} delay={200}>
                  <button
                    onClick={() => setShowDropsOverlay(true)}
                    className={`titlebar-icon-btn ${showProgressBadge ? 'gap-1' : ''}`}
                  >
                    {showProgressBadge ? (
                      // Replace icon with inline progress percentage badge when automation
                      <span className="drops-progress-inline">
                        {progressPercent}%
                      </span>
                    ) : showCompleteBadge ? (
                      // Done: every watch-time reward for the watched game is earned
                      <Check size={14} className="text-green-400" />
                    ) : (
                      // Normal Gift icon when not automation drops
                      <Gift size={14} className={isAnyAutomationActive ? giftClass : ''} />
                    )}
                  </button>
                </Tooltip>
              );
            })()}

            {/* Hover Preview Card - portalled to body, positioned under the drops button */}
            {showDropsPreview && dropProgressActive && dropProgress?.current_drop && createPortal(
              <div
                className="drops-preview-card-right"
                style={{ position: 'fixed', top: previewPos.top, left: previewPos.left, zIndex: 9999 }}
                onMouseEnter={() => {
                  if (previewTimeoutRef.current) clearTimeout(previewTimeoutRef.current);
                }}
                onMouseLeave={handleDropsMouseLeave}
              >
                {/* Header */}
                <div className="flex items-center gap-2 mb-2">
                  <div className="p-1.5 rounded-md bg-accent/20">
                    <Pickaxe size={14} className="text-accent" />
                  </div>
                  <span className="text-xs font-semibold text-textPrimary">Drop progress</span>
                </div>

                {/* Game & Drop Info */}
                <div className="space-y-1.5">
                  <div className="flex items-center gap-2 text-xs">
                    <span className="text-textMuted">Game:</span>
                    <span className="text-textPrimary font-medium truncate max-w-[140px]">
                      {dropProgress.current_drop.game_name}
                    </span>
                  </div>
                  <div className="flex items-center gap-2 text-xs">
                    <span className="text-textMuted">Drop:</span>
                    <span className="text-textPrimary font-medium truncate max-w-[140px]">
                      {dropProgress.current_drop.drop_name || '…'}
                    </span>
                  </div>
                  {dropProgress.current_channel && (
                    <div className="flex items-center gap-2 text-xs">
                      <Tv size={10} className="text-textMuted" />
                      <span className="text-textSecondary truncate max-w-[160px]">
                        {dropProgress.current_channel.display_name}
                      </span>
                    </div>
                  )}
                </div>

                {/* Progress Bar */}
                <div className="mt-3">
                  <div className="flex items-center justify-between text-xs mb-1">
                    <span className="text-textMuted flex items-center gap-1">
                      <Clock size={10} />
                      Progress
                    </span>
                    <span className="text-accent font-semibold">{progressPercent}%</span>
                  </div>
                  <div className="h-1.5 bg-surface rounded-full overflow-hidden">
                    <div 
                      className="h-full rounded-full automation-progress-bar transition-all duration-500"
                      style={{ width: `${progressPercent}%` }}
                    />
                  </div>
                  <div className="flex items-center justify-between text-[10px] text-textMuted mt-1">
                    <span>{dropProgress.current_drop.current_minutes} min</span>
                    <span>{dropProgress.current_drop.required_minutes} min</span>
                  </div>
                </div>

                {/* Click hint */}
                <div className="mt-2 pt-2 border-t border-borderSubtle">
                  <span className="text-[10px] text-textMuted">Click to view all drops</span>
                </div>
              </div>,
              document.body
            )}
          </div>

          {/* Marketplace Button — opens the plugin store. Badged when installed
              plugins have updates available, so users know without opening it. */}
          <Tooltip
            content={
              pluginUpdateCount > 0
                ? `Marketplace — ${pluginUpdateCount} update${pluginUpdateCount > 1 ? 's' : ''} available`
                : 'Marketplace'
            }
            delay={200}
          >
            <button
              onClick={() => setShowMarketplaceOverlay(true)}
              className="titlebar-icon-btn relative"
            >
              <Store size={14} />
              {pluginUpdateCount > 0 && (
                <span className="absolute -right-0.5 -top-0.5 flex h-3.5 min-w-[14px] items-center justify-center rounded-full bg-amber-500 px-1 text-[9px] font-bold leading-none text-zinc-950">
                  {pluginUpdateCount}
                </span>
              )}
            </button>
          </Tooltip>

          {/* Badges Button — dynamic badge icon that cycles on unhover */}
          <Tooltip content="Global Cosmetics" delay={200}>
            <button
              onClick={() => setShowBadgesOverlay(true)}
              onMouseLeave={cycleBadgeIcon}
              className="titlebar-icon-btn"
            >
              <AnimatePresence mode="popLayout" initial={false}>
                {currentBadgeUrl ? (
                  <motion.img
                    key={currentBadgeUrl}
                    initial={{ opacity: 0, scale: 0.8, y: 5 }}
                    animate={{ opacity: 1, scale: 1, y: 0 }}
                    exit={{ opacity: 0, scale: 0.8, y: -5 }}
                    transition={{ duration: 0.15 }}
                    src={currentBadgeUrl}
                    alt="Badge"
                    className="w-3.5 h-3.5 object-contain"
                    draggable={false}
                  />
                ) : (
                  <motion.div
                    key="medal"
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    exit={{ opacity: 0 }}
                  >
                    <Medal size={14} />
                  </motion.div>
                )}
              </AnimatePresence>
            </button>
          </Tooltip>



          {/* Fireworks show reopen, only on the Fourth itself */}
          {isSemiquincentennialShowDay() && (
            <Tooltip content="Fireworks show" delay={200}>
              <button
                onClick={() => openSemiquincentennialShow()}
                className="titlebar-icon-btn"
              >
                <Sparkles size={14} />
              </button>
            </Tooltip>
          )}

          {/* Settings */}
          <Tooltip content="Settings" delay={200}>
            <button
              onClick={() => openSettings()}
              className="titlebar-icon-btn settings-gear-btn"
            >
              <Settings size={14} />
            </button>
          </Tooltip>
          </div>

          {/* Compact View stats. Lives at the end of the LEFT cluster, not next
              to the Compact View button on the right, because the notification
              island is fixed to the center of the bar and would sit on top of
              them on the smaller presets. Outside the icon-group pill too: that
              pill's bevel means "pressable", and these are readouts. Dropped
              below 700px so a tiny custom preset does not overflow the bar. */}
          {isTheaterMode && streamUrl && streamUrl !== 'offline' && currentMediaType === 'live' && (
            <div className="hidden min-[700px]:flex items-center gap-3 ml-1">
              <CompactStreamStats />
            </div>
          )}

          {/* Update pill — appears only when an update is available. Slides in
              toward the center / notification island and settles to the right of
              the action group with a little spring bounce, instead of popping in
              abruptly. */}
          <AnimatePresence>
            {updateInfo && updatePhase === 'idle' && (
              <motion.div
                key="update-pill"
                initial={{ opacity: 0, scale: 0.7, x: 26 }}
                animate={{ opacity: 1, scale: 1, x: 0 }}
                exit={{ opacity: 0, scale: 0.7, x: 26 }}
                transition={{ type: 'spring', stiffness: 520, damping: 17 }}
              >
                <Tooltip
                  content={`Update v${updateInfo.current_version} → v${updateInfo.latest_version}`}
                  delay={200}
                >
                  <button
                    onClick={handleStartUpdate}
                    className="update-pill flex items-center gap-1.5 h-[26px] pl-2.5 pr-3 rounded-full whitespace-nowrap"
                  >
                    <Download size={13} strokeWidth={2.5} />
                    <span className="text-xs font-semibold tracking-wide">Update</span>
                  </button>
                </Tooltip>
              </motion.div>
            )}
          </AnimatePresence>
        </div>

        <div className="flex items-center gap-2" style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}>
          {/* Buttons contributed by ui plugins (rendered in native style) */}
          <PluginTitleBarButtons />

          {/* Grouped action icons */}
          <div className="titlebar-icon-group">
          {/* Whispers Button */}
          <Tooltip content="Whispers" delay={200}>
            <button
              onClick={() => setShowWhispersOverlay(true)}
              className="titlebar-icon-btn relative"
            >
              <MessageCircle size={14} />
              {/* Import indicator */}
              {whisperImportState.isImporting && (
                <span className="absolute -top-0.5 -right-0.5 w-2 h-2 bg-purple-500 rounded-full animate-pulse" />
              )}
            </button>
          </Tooltip>

          {/* Profile Button */}
          <Tooltip content={isAuthenticated ? 'Profile' : 'Sign in'} delay={200}>
            <button
              onClick={() => openSettings('Profile')}
              className="titlebar-icon-btn"
            >
              {isAuthenticated && currentUser?.profile_image_url ? (
                <img
                  src={currentUser.profile_image_url}
                  alt="Profile"
                  className="w-[18px] h-[18px] rounded-full object-cover"
                />
              ) : (
                <User size={18} />
              )}
            </button>
          </Tooltip>

          {/* Compact View Button - only show when stream is playing */}
          {streamUrl && (
            <Tooltip content={isTheaterMode ? 'Exit Compact View' : `Compact View (${getSelectedCompactViewPreset(settings?.compact_view?.selectedPresetId, settings?.compact_view?.customPresets).name})`} delay={200}>
              <button
                onClick={toggleTheaterMode}
                className={`titlebar-icon-btn ${isTheaterMode ? '!text-accent !bg-accent/15' : ''}`}
              >
                <Proportions size={14} />
              </button>
            </Tooltip>
          )}

          {/* Keep on Top - a Compact View sub-option, so it only appears there */}
          {isTheaterMode && (
            <Tooltip content={keepOnTop ? 'Stop keeping on top' : 'Keep on top'} delay={200}>
              <button
                onClick={() => void toggleKeepOnTop()}
                aria-pressed={keepOnTop}
                className={`titlebar-icon-btn ${keepOnTop ? '!text-accent !bg-accent/15' : ''}`}
              >
                {keepOnTop ? <PinOff size={14} /> : <Pin size={14} />}
              </button>
            </Tooltip>
          )}
          </div>

          {/* Window controls — kept adjacent but not grouped into a pill */}
          <div className="flex items-center gap-1">
          <Tooltip content={isWindowFullscreen ? "Exit full screen (F11)" : "Full screen (F11)"} delay={200}>
            <button
              onClick={() => toggleWindowFullscreen()}
              className="titlebar-window-btn"
              aria-label={isWindowFullscreen ? "Exit full screen" : "Full screen"}
            >
              {isWindowFullscreen ? (
                <ArrowsIn size={14} />
              ) : (
                <ArrowsOut size={14} />
              )}
            </button>
          </Tooltip>
          <Tooltip content="Minimize" delay={200}>
            <button
              onClick={handleMinimize}
              className="titlebar-window-btn"
            >
              <Minus size={14} />
            </button>
          </Tooltip>
          {/* Maximize snaps to the work area (taskbar stays); it is redundant and
              confusing while borderless full screen already covers everything. */}
          {!isWindowFullscreen && (
          <Tooltip content={isMaximized ? "Restore" : "Maximize"} delay={200}>
            <button
              onClick={handleMaximize}
              className="titlebar-window-btn"
            >
              {isMaximized ? (
                <CornersIn size={14} />
              ) : (
                <CornersOut size={14} />
              )}
            </button>
          </Tooltip>
          )}
          <Tooltip content="Close" delay={200}>
            <button
              onClick={handleClose}
              className="titlebar-window-btn titlebar-window-btn-close"
            >
              <X size={14} />
            </button>
          </Tooltip>
          </div>
        </div>
      </div>

      {/* About Widget */}
      {showAbout && <AboutWidget onClose={() => setShowAbout(false)} />}

      {/* Centered update card — staged install progress → restart-to-apply. */}
      <AnimatePresence>
        {updatePhase !== 'idle' && (
          <UpdateOverlay
            phase={updatePhase}
            currentVersion={updateInfo?.current_version}
            latestVersion={updateInfo?.latest_version}
            progressPercent={getUpdateStageProgress(updateProgress)}
            stageLabel={updateProgress?.replace(/\s*\d+\s*%$/, '…') ?? null}
            errorMessage={updateError}
            onRestart={handleApplyUpdateRestart}
            onDismiss={handleDismissUpdate}
          />
        )}
      </AnimatePresence>
    </>
  );
};

export default TitleBar;

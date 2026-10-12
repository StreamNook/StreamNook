import { Window } from '@tauri-apps/api/window';
import { Settings, Proportions, MessageCircle, Pickaxe, Clock, Tv, Download, Check, Pin, PinOff, Lightbulb } from 'lucide-react';
import { Minus, X, CornersOut, CornersIn, ArrowsOut, ArrowsIn, Medal, Package, PuzzlePiece } from 'phosphor-react';
import { IS_MAC, MAC_TRAFFIC_LIGHT_INSET_PX } from '../utils/platform';
import { useState, useEffect, useLayoutEffect, useRef, useMemo, useCallback } from 'react';
import { createPortal } from 'react-dom';
import { motion, AnimatePresence } from 'framer-motion';
import { useShallow } from 'zustand/react/shallow';
import { useAppStore } from '../stores/AppStore';
import PenroseLogo from './PenroseLogo';
import PlatformSwitcher from './PlatformSwitcher';
import NavFlipper from './NavFlipper';
import { MultiNookToggle, MultiChatButton } from './titlebar/WindowActions';
import AccountStack from './titlebar/AccountStack';
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
import { handleTitleBarMouseDown, isTitlebarDragActive } from '../utils/titleBarDrag';
import { START_UPDATE_EVENT } from '../utils/updateEvents';
import { Tooltip } from './ui/Tooltip';
import { AutomationPulse, type AutomationTone } from './ui/AutomationPulse';
import PluginTitleBarButtons from '../plugins-ui/PluginTitleBarButtons';
import { usePluginUpdates } from '../stores/pluginUpdatesStore';
import { useTitleBarAutoHide, useTitleBarLifted } from '../hooks/useTitleBarAutoHide';
import { exitPlayerFullscreen, exitPlayerFullscreenForBarAction } from '../utils/windowFullscreen';
import { opensPageOverPlayer, pointerAtTopEdge, titleBarTuckMode, titleBarTucked } from '../utils/titleBarTuck';

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
  const { openSettings, setShowDropsOverlay, setShowMarketplaceOverlay, setShowBadgesOverlay, setShowWhispersOverlay, toggleTheaterMode, toggleWindowFullscreen, toggleKeepOnTop } = useAppStore.getState();
  const { dropProgressActive, dropProgressComplete, isTheaterMode, streamUrl, currentMediaType, settings, whisperImportState, updateInfo } = useAppStore(
    useShallow((s) => ({
      dropProgressActive: s.dropProgressActive,
      dropProgressComplete: s.dropProgressComplete,
      isTheaterMode: s.isTheaterMode,
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

  // Window focus, published to the DOM for the chrome glaze (see .chrome-glaze
  // in globals.css). CSS cannot ask whether the OS window is focused, so the
  // answer is parked on <body> and the title-bar material reads it from there:
  // the specular streak goes out on a background window, the way native
  // vibrancy does. Purely cosmetic, so a failed subscription is not worth an
  // error path.
  useEffect(() => {
    const mark = (focused: boolean) => {
      document.body.dataset.windowBlurred = focused ? 'false' : 'true';
    };
    mark(document.hasFocus());

    // The subscription resolves asynchronously; a handle arriving after
    // cleanup must be released rather than stored, or the listener outlives
    // the effect (StrictMode double-invokes this).
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    Window.getCurrent()
      .onFocusChanged(({ payload: focused }) => mark(focused))
      .then((u) => {
        if (cancelled) u();
        else unlisten = u;
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      unlisten?.();
      delete document.body.dataset.windowBlurred;
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

  // The drops/points sign-in is a row in the account flyout (AccountStack),
  // fed by the roster Rust pushes, so nothing here polls for it.

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

  // Let other surfaces start the install without duplicating this flow: the
  // changelog popup, where the update notification sends people, has an Install
  // button next to the notes for the new version.
  useEffect(() => {
    const onRequest = () => {
      void handleStartUpdate();
    };
    window.addEventListener(START_UPDATE_EVENT, onRequest);
    return () => window.removeEventListener(START_UPDATE_EVENT, onRequest);
  }, [handleStartUpdate]);

  // How many releases behind, for the pill's label.
  //
  // A pill that reads the same at one patch behind and at twelve is why people
  // sit still: it carries no urgency to escalate. Only shown from two behind, so
  // the common "a patch landed today" case stays as quiet as it should be. Rust
  // counts it with the update check; null when its release cache is cold, in
  // which case the pill says "Update" rather than inventing a number.
  const releasesBehindCount = updateInfo?.releases_behind ?? null;

  // Replaces data-tauri-drag-region: a borderless window doesn't get Windows'
  // restore-on-drag, so dragging while maximized has to unmaximize first.
  // In either full screen a press or double-click on the bar does nothing: no
  // window drag, no maximize.
  const anyFullscreen = useAppStore((s) => s.isWindowFullscreen || s.isPlayerFullscreen);
  const onTitleBarMouseDown = useCallback(
    (e: React.MouseEvent<HTMLElement>) =>
      handleTitleBarMouseDown(e, isMaximized, { fullscreen: anyFullscreen }),
    [isMaximized, anyFullscreen],
  );

  // Stream-only full screen, "Title bar auto-hide" and the player's full
  // screen: the bar tucks up out of sight and the pointer reaching the top edge
  // brings it back. It stays while the cursor is over it and tucks again a
  // moment after the cursor leaves. The leave handler runs in every mode, so a
  // bar revealed when the mode ends never comes back pre-revealed the next time.
  //
  // "Over it" is checked two ways, because the bar holds content from two
  // trees. React's enter/leave follow the component tree, so the menus the bar
  // portals OUT (accounts, platforms, the drops card) count as inside it. The
  // Island's door and Home's strip are portaled IN from elsewhere, so React
  // reports the pointer reaching them as a leave; the tuck timer therefore
  // also asks the DOM's :hover before it fires.
  //
  // The layout never follows the reveal: in this mode nothing reserves the
  // bar's row, so the bar coming back overlays the stream instead of pushing it
  // down (see useTitleBarAutoHide).
  const autoHides = useTitleBarAutoHide();
  // The player's own full screen (double-click, its button, F) pins the player
  // over the whole window in a layer far above the bar, so the bar has to tuck
  // and come back from above that layer too. Layout is not involved: the player
  // covers the window, so nothing underneath moves.
  const playerFullscreen = useAppStore((s) => s.isPlayerFullscreen);
  const tucks = titleBarTuckMode({ layoutAutoHides: autoHides, playerFullscreen });
  // In the tucking modes the bar comes back OVER the player, whose top-right
  // buttons and title overlays share the bar's resting layer and come later in
  // the page, so at its resting layer the bar was drawn and clicked under them.
  // It lifts above the player's whole stack for as long as the mode lasts,
  // unless a page is open over the stream (its own About card and update
  // dialog included), which keeps it under that page as usual.
  const lifted = useTitleBarLifted(showAbout || updatePhase !== 'idle');
  const barRef = useRef<HTMLDivElement>(null);
  const [barRevealed, setBarRevealed] = useState(false);
  const tuckTimerRef = useRef<number | null>(null);
  const cancelTuck = useCallback(() => {
    if (tuckTimerRef.current !== null) {
      window.clearTimeout(tuckTimerRef.current);
      tuckTimerRef.current = null;
    }
  }, []);
  // Only while the bar tucks: anywhere else a pass over the bar would re-render
  // this whole component twice for a state nothing reads.
  const revealBar = useCallback(() => {
    cancelTuck();
    if (tucks) setBarRevealed(true);
  }, [cancelTuck, tucks]);
  const scheduleTuckAfter = useCallback(
    (delayMs: number) => {
      cancelTuck();
      const arm = () => {
        tuckTimerRef.current = window.setTimeout(() => {
          // A window drag runs inside the OS move loop, where the page can see a
          // leave it never made. Wait it out rather than tucking mid-drag. The
          // same for a pointer still on the bar's own pixels: content portaled
          // in from outside its tree (the Island's door) reads as a leave.
          if (isTitlebarDragActive() || barRef.current?.matches(':hover')) {
            arm();
            return;
          }
          tuckTimerRef.current = null;
          setBarRevealed(false);
        }, delayMs);
      };
      arm();
    },
    [cancelTuck],
  );
  const scheduleTuck = useCallback(() => scheduleTuckAfter(400), [scheduleTuckAfter]);
  // The top edge reveals the bar, but the bar slides in UNDER the pointer, and
  // a pointer that then heads straight back down may never register as having
  // entered it, so no leave would ever come to tuck it. Arm the tuck on reveal;
  // entering the bar cancels it. Touch has no hover to hold the bar out, so a
  // tap at the top keeps it long enough to reach a control.
  const revealFromEdge = useCallback(
    (holdMs: number = 1000) => {
      revealBar();
      scheduleTuckAfter(holdMs);
    },
    [revealBar, scheduleTuckAfter],
  );
  useEffect(() => cancelTuck, [cancelTuck]);

  // In use, so it stays out whatever the pointer does: a menu of its own open
  // (any trigger inside the bar reporting aria-expanded, the Island's door
  // included), keyboard focus inside it, or Settings open over the stream.
  // Click focus does not count, or a button pressed once would pin the bar
  // until the next click elsewhere.
  const [barMenuOpen, setBarMenuOpen] = useState(false);
  const [barKeyboardFocus, setBarKeyboardFocus] = useState(false);
  const settingsOpen = useAppStore((s) => s.isSettingsOpen);
  useEffect(() => {
    const el = barRef.current;
    if (!tucks || !el) {
      setBarMenuOpen(false);
      return;
    }
    let open = !!el.querySelector('[aria-expanded="true"]');
    setBarMenuOpen(open);
    // The mode starting under the pointer (back to the stream from the bar's
    // own nav) keeps the bar out until the pointer leaves it, rather than
    // pulling it away mid-hover.
    if (el.matches(':hover')) {
      setBarRevealed(true);
      scheduleTuck();
    }
    const mo = new MutationObserver(() => {
      const next = !!el.querySelector('[aria-expanded="true"]');
      // A menu that closes under the pointer (an item was picked) unmounts
      // without a leave ever reaching the bar, so the bar would stay out
      // until the next pass over it. Start the tuck unless the pointer is on
      // the bar itself.
      if (open && !next && !el.matches(':hover')) scheduleTuck();
      open = next;
      setBarMenuOpen(next);
    });
    mo.observe(el, { subtree: true, childList: true, attributes: true, attributeFilter: ['aria-expanded'] });
    return () => mo.disconnect();
  }, [tucks, scheduleTuck]);
  // Keyboard focus, through React for the menus the bar portals out (they are
  // in its tree, not its DOM)...
  const onBarFocus = useCallback((e: React.FocusEvent<HTMLElement>) => {
    const target = e.target as HTMLElement;
    setBarKeyboardFocus(typeof target.matches === 'function' && target.matches(':focus-visible'));
  }, []);
  const onBarBlur = useCallback(() => setBarKeyboardFocus(false), []);
  // ...and through the DOM for what is portaled in (the Island's door, the
  // bar's first Tab stop), which React never reports to the bar.
  useEffect(() => {
    const el = barRef.current;
    if (!el) return;
    const onIn = (e: FocusEvent) => {
      const target = e.target;
      setBarKeyboardFocus(target instanceof Element && target.matches(':focus-visible'));
    };
    const onOut = (e: FocusEvent) => {
      const next = e.relatedTarget;
      if (!(next instanceof Node) || !el.contains(next)) setBarKeyboardFocus(false);
    };
    // A key pressed while focus sits in the bar is keyboard use too, even when
    // that focus came from a click: Esc closing the notification list leaves
    // focus on its door, and the bar must not tuck away from under it. It
    // stays out until focus leaves the bar.
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Control' || e.key === 'Shift' || e.key === 'Alt' || e.key === 'Meta') return;
      setBarKeyboardFocus(true);
    };
    el.addEventListener('focusin', onIn);
    el.addEventListener('focusout', onOut);
    el.addEventListener('keydown', onKey);
    return () => {
      el.removeEventListener('focusin', onIn);
      el.removeEventListener('focusout', onOut);
      el.removeEventListener('keydown', onKey);
    };
  }, []);
  const barTucked = titleBarTucked({
    autoHides: tucks,
    revealed: barRevealed,
    inUse: barMenuOpen || barKeyboardFocus || settingsOpen,
  });
  // The top edge brings the bar back by POSITION, not by what is under the
  // pointer. The outermost pixels are window resizing on Windows (a native
  // window above the page, in every state but maximized) and on Linux
  // (LinuxResizeEdges, above everything), so the strip below never sees a
  // pointer there; the zone reaches past both. A pointer that leaves the page
  // through the top edge counts too, which is how full screen reports the
  // pointer pressed against the top of the screen.
  useEffect(() => {
    if (!barTucked) return;
    const onMove = (e: PointerEvent) => {
      if (pointerAtTopEdge(e.clientY)) revealFromEdge(e.pointerType === 'mouse' ? undefined : 4000);
    };
    const onDown = (e: PointerEvent) => {
      if (e.pointerType !== 'mouse' && pointerAtTopEdge(e.clientY)) revealFromEdge(4000);
    };
    const onOut = (e: MouseEvent) => {
      if (e.relatedTarget === null && pointerAtTopEdge(e.clientY)) revealFromEdge();
    };
    window.addEventListener('pointermove', onMove, { passive: true });
    window.addEventListener('pointerdown', onDown, { passive: true });
    document.addEventListener('mouseout', onOut);
    return () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerdown', onDown);
      document.removeEventListener('mouseout', onOut);
    };
  }, [barTucked, revealFromEdge]);
  // Anything that opens a page, overlay or modal while the player's full screen
  // is up (a shortcut, the command palette, a notification, the account
  // picture) would open under the video, so it leaves the player's full screen
  // first. A tick later, because a bar click has usually started that exit
  // already and the store only hears of it once the bridge has run.
  useEffect(
    () =>
      useAppStore.subscribe((next, prev) => {
        if (!next.isPlayerFullscreen || !opensPageOverPlayer(prev, next)) return;
        window.setTimeout(() => {
          if (useAppStore.getState().isPlayerFullscreen) exitPlayerFullscreen();
        }, 0);
      }),
    [],
  );
  // Anything in the bar (or a menu it portals) that would take the viewer
  // somewhere else leaves the player's full screen first, through the player's
  // own exit, so the page it opens is not hidden under the video and the
  // window's full screen follows the player's as it always does.
  const onBarClickCapture = useCallback(
    (e: React.MouseEvent<HTMLElement>) => {
      if (playerFullscreen) exitPlayerFullscreenForBarAction(e.target);
    },
    [playerFullscreen],
  );

  return (
    <>
      {barTucked && (
        <div
          aria-hidden
          className="fixed inset-x-0 top-0 h-1 z-[51]"
          // Over the player's full-screen stack, or the edge is dead there.
          style={playerFullscreen ? { zIndex: 'var(--sn-z-titlebar-edge)' } : undefined}
          onMouseEnter={() => revealFromEdge()}
        />
      )}
      {/* macOS decorates this window with an Overlay title bar, so AppKit
          draws real traffic lights at the leading edge. Reserve their footprint
          rather than drawing anything underneath them. */}
      <div
        ref={barRef}
        onMouseDown={IS_MAC ? undefined : onTitleBarMouseDown}
        onClickCapture={onBarClickCapture}
        onMouseEnter={revealBar}
        onMouseLeave={scheduleTuck}
        onFocus={onBarFocus}
        onBlur={onBarBlur}
        // In full screen there is no window to move, and Tauri's region
        // would turn a double-click into a maximize.
        data-tauri-drag-region={IS_MAC && !anyFullscreen ? true : undefined}
        // Paints nothing: no fill, no blur, no rule under it. Everything in
        // here is already a floating glazed pill, so with the plate gone the
        // chrome reads as objects over the page instead of a bar across it —
        // the same move the Home nav made.
        //
        // Losing the full-width `backdrop-blur-md` is a real saving as well: a
        // window-wide compositing layer sampling a backdrop that hardly varied.
        //
        // Out of flow, deliberately. In flow the bar OWNS the top 40px, and
        // since nothing paints there you get a solid strip of the app background
        // whether or not the bar draws a fill of its own, which is why removing
        // the fill did nothing. Overlaying it lets the grid, the
        // player and the glow run to the top edge of the window and the chrome
        // float on top of them, which is also what the glaze was built for: it
        // blurs and tints whatever is actually behind it.
        //
        // The row keeps its height and its drag region; only its place in the
        // layout changes. Surfaces that must not slide under it (the sidebar,
        // the chat column) clear it themselves. The main content column
        // deliberately does not.
        //
        // A three-column grid, `1fr auto 1fr`: the side columns share whatever
        // the centre leaves, so the centre sits at true centre while both
        // clusters fit their half. A side column never shrinks below its
        // cluster, though, so when the left cluster outgrows its half the centre
        // is pushed right instead of sliding underneath it. Home keeps its strip
        // inside that room by drawing the tabs denser (useTitleBarNavDensity).
        // The centre track's floor is 0 rather than its content, so if even the
        // densest strip cannot fit, the centre gives way and the window controls
        // are never pushed off the edge.
        data-titlebar
        className={`absolute inset-x-0 top-0 grid grid-cols-[1fr_minmax(0,auto)_1fr] gap-x-3 items-center h-[40px] px-3 select-none z-50 transition-[transform,opacity] duration-200 ease-out motion-reduce:transition-none ${
          barTucked ? '-translate-y-full opacity-0 pointer-events-none' : ''
        }`}
        style={{
          ...(IS_MAC ? { paddingLeft: MAC_TRAFFIC_LIGHT_INSET_PX } : null),
          // Lifted, the bar sits above the player's whole stack (full screen
          // or the tucking layout); the player's own layers never change.
          ...(lifted ? { zIndex: 'var(--sn-z-titlebar-over-player)' } : null),
        }}
      >
        {/* The centre of the bar, as two slots other components portal into.
            Slots rather than real children, because the things that belong here
            own state that has no business being lifted: the Island owns every
            notification, and Home owns the tab strip's selection, search and
            counts. Portaling lets them stay where they are and still render
            here.

            `pointer-events-none` on the wrapper with `auto` on the slots, so the
            empty space between and around them stays draggable window chrome.

            The Island's SURFACE is still rendered at the app root (App.tsx) so
            it can lift above the Settings blur overlay; only its door sits
            here, and the surface grows out of that door rightward over the nav
            and collapses back into it. */}
        <div className="pointer-events-none col-start-2 row-start-1 flex items-center gap-2">
          {/* Only one of the two is ever filled. With Home's strip in the bar
              the Island door joins the strip (its inline slot) and this one
              stays empty; with no strip the door stands alone here. So each slot
              drops out of layout while empty, and whichever is filled is what
              gets centred, with no stray gap beside it. */}
          <div id="sn-island-slot" className="pointer-events-auto flex items-center empty:hidden" />
          <div id="sn-nav-slot" className="pointer-events-auto flex items-center empty:hidden" />
        </div>

        <div
          data-titlebar-left
          className="col-start-1 row-start-1 justify-self-start flex items-center gap-2.5"
          style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}
        >
          {/* Penrose Logo. On macOS the leading edge belongs to the traffic
              lights, so About moves to the trailing edge instead — into the slot
              the full-screen button vacates there. */}
          {!IS_MAC && <PenroseLogo onClick={() => setShowAbout(true)} />}

          {/* Which platform the app is in. Lives here because the title bar is
              the only chrome mounted in every view: a quiet mark-and-name
              readout beside the logo that opens the switcher on hover. */}
          <PlatformSwitcher />

          {/* Back / forward between the stream and everything else. Its own
              pill rather than a slot in the cluster beside it: navigating is
              not the same kind of thing as opening drops or settings. */}
          <NavFlipper />

          {/* Grouped action icons.
              MultiNook and MultiChat lead, behind a hairline: they act on the
              WINDOW (tile some players, spawn a popout) where the rest of the
              group opens something inside it. They used to live in Home's tab
              strip, which meant they vanished on a category drill-down and
              while watching — this bar is mounted in every view. */}
          <div className="chrome-glaze titlebar-icon-group">
          <MultiNookToggle />
          <MultiChatButton />
          <span className="mx-0.5 h-4 w-px bg-borderSubtle" aria-hidden />
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

              // The drops button pulses while automation runs.
              // Silver = channel points only, Gold = drops only, Iridescent = both
              let automationTone: AutomationTone | null = null;
              let title = 'Drops & Points';

              if (isBothActive) {
                automationTone = 'iridescent';
                title = 'Drops & Points (Both Active)';
              } else if (dropProgressActive) {
                automationTone = 'gold';
                title = `Drops progress: ${progressPercent}%`;
              } else if (showCompleteBadge) {
                title = 'Drops complete — all rewards earned for this game';
              } else if (channelPointsActive) {
                automationTone = 'silver';
                title = 'Drops & Points (Channel Points Active)';
              }

              const isAnyAutomationActive = dropProgressActive || channelPointsActive;

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
                      <Check size={17} className="text-green-400" />
                    ) : (
                      // A package, not a gift box. What arrives is a reward you
                      // earned by watching rather than one somebody handed you,
                      // and the box reads as the thing itself rather than the
                      // wrapping. Phosphor to match the badge and puzzle glyphs
                      // beside it — a heavier stroke in one slot of a cluster
                      // reads as a mistake once the others agree.
                      isAnyAutomationActive && automationTone ? (
                        <AutomationPulse tone={automationTone}>
                          <Package size={17} />
                        </AutomationPulse>
                      ) : (
                        <Package size={17} />
                      )
                    )}
                  </button>
                </Tooltip>
              );
            })()}

            {/* Hover Preview Card - portalled to body, positioned under the drops button */}
            {showDropsPreview && dropProgressActive && dropProgress?.current_drop && createPortal(
              <div
                className="drops-preview-card-right"
                style={{ position: 'fixed', top: previewPos.top, left: previewPos.left, zIndex: 'var(--sn-z-titlebar-menu)' }}
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
                  {dropProgress.current_drop.detail && (
                    <div className="text-[10px] text-textSecondary mt-1 truncate">
                      {dropProgress.current_drop.detail}
                    </div>
                  )}
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
              {/* A puzzle piece, not a shopfront. The button opens the plugin
                  Marketplace and its badge counts plugin UPDATES, so "shop" was
                  only half of what it does; a puzzle piece is the universal
                  glyph for an extension and says the other half. Phosphor
                  rather than Lucide for the thinner stroke, matching the window
                  controls and the badge glyph already in this bar. */}
              <PuzzlePiece size={17} />
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
                    className="w-[17px] h-[17px] object-contain"
                    draggable={false}
                  />
                ) : (
                  <motion.div
                    key="medal"
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    exit={{ opacity: 0 }}
                  >
                    <Medal size={17} />
                  </motion.div>
                )}
              </AnimatePresence>
            </button>
          </Tooltip>



          {/* Settings */}
          <Tooltip content="Settings" delay={200}>
            <button
              onClick={() => openSettings()}
              className="titlebar-icon-btn settings-gear-btn"
            >
              <Settings size={17} />
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
                  content={
                    releasesBehindCount && releasesBehindCount > 1
                      ? `Update v${updateInfo.current_version} → v${updateInfo.latest_version} · at least ${releasesBehindCount} releases behind`
                      : `Update v${updateInfo.current_version} → v${updateInfo.latest_version}`
                  }
                  delay={200}
                >
                  <button
                    onClick={handleStartUpdate}
                    className="update-pill flex items-center gap-1.5 h-[26px] pl-2.5 pr-3 rounded-full whitespace-nowrap"
                  >
                    <Download size={13} strokeWidth={2.5} />
                    <span className="text-xs font-semibold tracking-wide">
                      {releasesBehindCount && releasesBehindCount > 1
                        ? `${releasesBehindCount} updates`
                        : 'Update'}
                    </span>
                  </button>
                </Tooltip>
              </motion.div>
            )}
          </AnimatePresence>
        </div>

        <div
          data-titlebar-right
          className="col-start-3 row-start-1 justify-self-end flex items-center gap-2"
          style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}
        >
          {/* Buttons contributed by ui plugins (rendered in native style) */}
          <PluginTitleBarButtons />

          {/* Grouped action icons */}
          <div className="chrome-glaze titlebar-icon-group">
          {/* Whispers Button */}
          <Tooltip content="Whispers" delay={200}>
            <button
              onClick={() => setShowWhispersOverlay(true)}
              className="titlebar-icon-btn relative"
            >
              <MessageCircle size={17} />
              {/* Import indicator */}
              {whisperImportState.isImporting && (
                <span className="absolute -top-0.5 -right-0.5 w-2 h-2 bg-purple-500 rounded-full animate-pulse" />
              )}
            </button>
          </Tooltip>

          {/* Who you are signed in as, on every platform; hover lists them. */}
          <AccountStack />

          {/* Compact View Button - only show when stream is playing */}
          {streamUrl && (
            <Tooltip content={isTheaterMode ? 'Exit Compact View' : `Compact View (${getSelectedCompactViewPreset(settings?.compact_view?.selectedPresetId, settings?.compact_view?.customPresets).name})`} delay={200}>
              <button
                onClick={toggleTheaterMode}
                className={`titlebar-icon-btn ${isTheaterMode ? '!text-accent !bg-accent/15' : ''}`}
              >
                <Proportions size={17} />
              </button>
            </Tooltip>
          )}

          {/* Immersive: the picture's colour spills onto the letterbox. Sits
              beside Compact View because both are choices about how the player
              occupies the window rather than actions. Only offered while
              something is playing, and only while sampling is on — otherwise
              there is no colour for it to pour. */}
          {streamUrl && settings?.media_glow !== false && (
            <Tooltip content={settings?.immersive_glow ? 'Turn off immersive' : 'Immersive'} delay={200}>
              <button
                onClick={() => {
                  const s = useAppStore.getState().settings;
                  void useAppStore.getState().updateSettings({ ...s, immersive_glow: !s.immersive_glow });
                }}
                aria-pressed={settings?.immersive_glow === true}
                className={`titlebar-icon-btn ${settings?.immersive_glow ? 'is-immersive' : ''}`}
                aria-label="Immersive"
                // Changes the look of the picture in place, so it keeps the
                // player's full screen.
                data-keeps-player-fullscreen
              >
                {/* On, the bulb lights up instead of taking the accent tint
                    every other toggle uses. The tint says "this control is on";
                    a lit bulb says what it turned on, and this is the one
                    control in the bar whose entire subject is colour.

                    The gradient has to live in the document for `url(#...)` to
                    resolve, so it rides along in a zero-sized SVG rather than
                    being a background the stroke cannot use. Rendered only
                    while the mode is on, so it costs nothing otherwise. */}
                {settings?.immersive_glow && (
                  <svg
                    width="0"
                    height="0"
                    aria-hidden="true"
                    focusable="false"
                    style={{ position: 'absolute' }}
                  >
                    <defs>
                      {/* User space, not the default object bounding box: the
                          icon is three separate paths, and per-object units
                          would give the bulb, the collar and the base a full
                          rainbow each instead of one running across the whole
                          thing. Coordinates are the icon's own 24-unit box. */}
                      <linearGradient
                        id="sn-bulb-rainbow"
                        className="sn-bulb-rainbow"
                        gradientUnits="userSpaceOnUse"
                        x1="4"
                        y1="22"
                        x2="20"
                        y2="2"
                      >
                        <stop offset="0%" />
                        <stop offset="25%" />
                        <stop offset="50%" />
                        <stop offset="75%" />
                        <stop offset="100%" />
                      </linearGradient>
                    </defs>
                  </svg>
                )}
                <Lightbulb
                  size={17}
                  color={settings?.immersive_glow ? 'url(#sn-bulb-rainbow)' : undefined}
                />
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
                {keepOnTop ? <PinOff size={17} /> : <Pin size={17} />}
              </button>
            </Tooltip>
          )}
          </div>

          {/* Window controls — kept adjacent but not grouped into a pill.
              On Windows and Linux they also take the platform's caption
              geometry (see .titlebar-window-controls); on macOS this slot holds
              only the About logo, so it keeps the app's own spacing. */}
          <div
            className={IS_MAC ? 'flex items-center gap-1' : 'flex items-center titlebar-window-controls -mr-3'}
            // Minimize, maximize and close act on the window, not the page: they
            // never leave the player's full screen on their way (the full-screen
            // button leaves it itself). On macOS this slot holds only About,
            // which opens a page and so has to leave it like any other.
            data-keeps-player-fullscreen={IS_MAC ? undefined : true}
          >
          {/* macOS already has full screen on the green traffic light, so a
              second control for it is redundant chrome. The slot goes to About,
              which lost its home on the leading edge to the traffic lights. */}
          {IS_MAC ? (
            <PenroseLogo onClick={() => setShowAbout(true)} />
          ) : (
          // Either full screen reads as full screen here. The player's one is
          // left through the player, so Plyr and the window stay in step.
          <Tooltip content={anyFullscreen ? "Exit full screen (F11)" : "Full screen (F11)"} delay={200}>
            <button
              onClick={() => (playerFullscreen ? exitPlayerFullscreen() : toggleWindowFullscreen())}
              className="titlebar-window-btn"
              aria-label={anyFullscreen ? "Exit full screen" : "Full screen"}
            >
              {anyFullscreen ? (
                <ArrowsIn size={14} />
              ) : (
                <ArrowsOut size={14} />
              )}
            </button>
          </Tooltip>
          )}
          {/* Minimise / zoom / close are AppKit's job on macOS: the window is
              decorated with an Overlay title bar, so the real traffic lights are
              already floating at the leading edge. Drawing our own cluster next
              to them would give the window two of every control. The full-screen
              button above stays on both platforms because it drives StreamNook's
              own theatre mode, not the window zoom. */}
          {!IS_MAC && (
            <>
          <Tooltip content="Minimize" delay={200}>
            <button
              onClick={handleMinimize}
              className="titlebar-window-btn"
            >
              <Minus size={14} />
            </button>
          </Tooltip>
          {/* Maximize snaps to the work area (taskbar stays); it is redundant and
              confusing while borderless full screen already covers everything,
              and on a full-screen window it brings the taskbar back over it. */}
          {!anyFullscreen && (
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
            </>
          )}
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

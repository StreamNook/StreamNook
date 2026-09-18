import { useEffect, useState, useRef, lazy, Suspense } from 'react';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import { AnimatePresence, motion } from 'framer-motion';
import { useShallow } from 'zustand/react/shallow';
import { IS_MOBILE, isPortrait, onOrientationChange } from './utils/platform';
import MobileNav from './components/mobile/MobileNav';
import { useAppStore, type WhisperImportProgress, type SettingsTab } from './stores/AppStore';
import { listenForSettingsUpdates } from './utils/settingsBroadcast';
import { trackPresence, isSupabaseConfigured, incrementStat, incrementChannelWatch, subscribeToStreamNookRegistry, subscribeToCosmeticsRegistry, subscribeToAtmospheresRegistry, refreshEntitlementRegistries } from './services/supabaseService';
import { primeClientConfig } from './services/clientConfig';
import { maybeClaimWatchRewards } from './services/watchRewards';
import TitleBar from './components/TitleBar';
import DynamicIsland from './components/DynamicIsland';
import ChannelAboutReveal from './components/ChannelAboutReveal';
import ChatWidget from './components/ChatWidget';
import MainProviderChat from './components/MainProviderChat';
import { activeChatSlot } from './stores/multiNookStore';
import { streamProvider } from './utils/streamProvider';
import { useFollowsStore, type ProviderStreamRow } from './stores/followsStore';
import { useFavoritesStore, type FavoriteStreamRow } from './stores/favoritesStore';
import type { ProviderId } from './types/providers';
import { ModLogsWidget } from './components/chat/ModLogsWidget';
import Home from './components/Home';
import { useCommandPaletteHotkey } from './hooks/useCommandPaletteHotkey';
import { usePlatformSessionCheck } from './hooks/usePlatformSessionCheck';
import { usePlatformAccountSync } from './hooks/usePlatformAccountSync';
import { useKeybindings } from './keybindings';
import { useCurrentStreamStats } from './utils/useCurrentStreamStats';
import { startSnippetSync } from './stores/snippetStore';
import PluginUiHost from './plugins-ui/PluginUiHost';
import PluginUpdatesChecker from './components/plugins/PluginUpdatesChecker';
import PluginOverlayOutlet from './plugins-ui/PluginOverlayOutlet';
import { usemultiNookStore } from './stores/multiNookStore';
import LoadingWidget from './components/LoadingWidget';
import ToastManager from './components/ToastManager';
import DeviceLoginOverlay from './components/DeviceLoginOverlay';
import EntitlementUnlockNote from './components/EntitlementUnlockNote';
import AnnouncementsBanner from './components/AnnouncementsBanner';
import { TooltipManager } from './components/ui/TooltipManager';
import { Tooltip } from './components/ui/Tooltip';
import DropProgressController from './components/plugins/DropProgressController';
import ReminderEngine from './components/ReminderEngine';
import EmoteSpotlight from './components/EmoteSpotlight';
import WhispersWidget from './components/WhispersWidget';
import PluginRuntimeBridge from './components/plugins/PluginRuntimeBridge';
import Sidebar from './components/Sidebar';
import TwitchOverlay from './components/TwitchOverlay';
import ErrorBoundary from './components/ErrorBoundary';
import InputContextMenuHost from './components/InputContextMenuHost';
import ModerationDragLayer from './components/chat/ModerationDragLayer';
import { listen } from '@tauri-apps/api/event';
import { applyModerateEvent } from './utils/applyModerateEvent';
import { handleSeventvEmoteSetUpdate, handleSeventvCosmeticUpdate, type EmoteSetUpdatePayload, type CosmeticUpdatePayload } from './services/seventvEventApi';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
import { useThemeBoot } from './boot/useThemeBoot';
import { getLogicalInnerSize, clampToWorkArea } from './utils/windowSizing';
import { isTitlebarDragActive } from './utils/titleBarDrag';
import { getSelectedCompactViewPreset } from './constants/compactViewPresets';
import { afterBoot, type BootTier } from './utils/startupScheduler';
import { getAppVersion } from './utils/appVersion';

import { Logger } from './utils/logger';

// Heavy overlays load on first open instead of riding the boot bundle. Each
// always-mounted one is gated behind an "ever opened" latch in the component
// body; overlays App already mounts conditionally just gain a Suspense wrapper.
const VideoPlayer = lazy(() => import('./components/VideoPlayer'));
const SettingsDialog = lazy(() => import('./components/SettingsDialog'));
const PublicProfileOverlay = lazy(() => import('./components/PublicProfileOverlay'));
const CommandPalette = lazy(() => import('./components/CommandPalette'));
const MultiNookView = lazy(() =>
  import('./components/multi-nook/MultiNookView').then((m) => ({ default: m.MultiNookView })),
);
const MultiNookChatSwitcher = lazy(() => import('./components/multi-nook/MultiNookChatSwitcher'));
const SearchProfileModal = lazy(() =>
  import('./components/SearchProfileModal').then((m) => ({ default: m.SearchProfileModal })),
);
const DropsOverlay = lazy(() => import('./components/DropsOverlay'));
const MarketplaceOverlay = lazy(() => import('./components/MarketplaceOverlay'));
const BadgesOverlay = lazy(() => import('./components/BadgesOverlay'));
const EmoteSetsOverlay = lazy(() => import('./components/EmoteSetsOverlay'));
const BadgeDetailOverlay = lazy(() => import('./components/BadgeDetailOverlay'));
const ChangelogOverlay = lazy(() => import('./components/ChangelogOverlay'));
const SetupWizard = lazy(() => import('./components/SetupWizard'));
const ClipModal = lazy(() => import('./components/ClipModal'));
const ClipEditor = lazy(() => import('./components/ClipEditor'));
interface BadgeVersion {
  id: string;
  image_url_1x: string;
  image_url_2x: string;
  image_url_4x: string;
  title: string;
  description: string;
  click_action: string | null;
  click_url: string | null;
  set_id?: string;
}

// One-time migration flag for v4.9.1 webview features
// This key is set after the force re-login to ensure it only happens once
const WEBVIEW_RELOGIN_MIGRATION_KEY = 'streamnook-webview-relogin-v4.9.1';

// One-time migration flag for v2.2.0 - force re-login with full webview data clear
const V220_RELOGIN_MIGRATION_KEY = 'streamnook-relogin-v2.2.0';
// Backstop for the first-run wizard. settings.setup_complete is the source of
// truth, but it has been seen reverting to false between launches on Android, so
// completion is also recorded here where nothing else writes it.
const SETUP_COMPLETE_MARKER = 'streamnook-setup-complete';

// Default sizes for different placements (outside component to avoid recreating on each render)
const DEFAULT_CHAT_WIDTH = 402; // For 'right' placement
const DEFAULT_CHAT_HEIGHT = 200; // For 'bottom' placement
// Bumped to -v2 when the saved value switched from physical to logical pixels.
// A stale physical value replayed as a LogicalSize would reopen the app oversized.
const COMPACT_SAVED_SIZE_KEY = 'streamnook-compact-saved-size-v2';

/** Window geometry captured on the way into Compact View. `maximized` is the
 *  state the window was in, not a size: restoring a maximized window by pixel
 *  size gives a floating window with maximized dimensions, not a maximized one. */
type CompactSavedSize = { width: number; height: number; maximized?: boolean };

/** Put the window back the way Compact View found it. Callers set the
 *  self-resize guard first; this only touches geometry. */
async function restoreFromCompact(
  win: ReturnType<typeof getCurrentWindow>,
  saved: CompactSavedSize,
): Promise<void> {
  if (saved.maximized) {
    await win.maximize();
    return;
  }
  const restored = await clampToWorkArea(saved.width, saved.height);
  await win.setSize(new LogicalSize(restored.width, restored.height));
}

function App() {
  useCommandPaletteHotkey();
  useKeybindings();
  // Viewer count is written once when playback starts and never updated after,
  // so without this it shows whatever it was the instant the stream opened, for
  // as long as you watch.
  useCurrentStreamStats();
  // Main window only: popouts learn from the `platform-account-changed` broadcast
  // rather than each running a check of their own.
  usePlatformSessionCheck();
  usePlatformAccountSync();
  // First-paint signal for the hidden-until-ready main window. The Rust
  // command lands with the visibility-gate change; until then the invoke
  // rejects and the catch keeps this a no-op. Runs on every App mount so the
  // runtime-recreated main window flows through the same reveal path.
  useEffect(() => {
    requestAnimationFrame(() => requestAnimationFrame(() => {
      void invoke('reveal_main_window').catch(() => {});
    }));
  }, []);
  useEffect(() => {
    // In-app follows for platforms with no followed-channels API of their own.
    // The backend poller owns liveness and pushes `provider-live-update`; the
    // initial pull just paints whatever it already knows so the sidebar isn't
    // empty for the first poll interval after launch.
    const follows = useFollowsStore.getState();
    void follows.hydrate();
    void follows.refreshLive();
    let unlistenLive: (() => void) | undefined;
    let unlistenFavorites: (() => void) | undefined;
    let cancelled = false;
    // The stream we are WATCHING ended, reported the moment the backend learns
    // it rather than on the liveness poll's schedule. That poll needs two 60s
    // cycles to agree, and never gets there at all if the check itself is
    // erroring — so without this the player just froze on its last frame.
    void listen<{ provider: ProviderId; channel: string }>(
      'provider-stream-offline',
      (event) => {
        const watching = useAppStore.getState().currentStream;
        if (!watching) return;
        const sameChannel =
          (watching.user_login || '').toLowerCase() ===
          (event.payload.channel || '').toLowerCase();
        if (streamProvider(watching) !== event.payload.provider || !sameChannel) return;
        void useAppStore.getState().handleStreamOffline();
      },
    ).then((u) => {
      if (cancelled) u();
    });
    void listen<{ provider: ProviderId; streams: ProviderStreamRow[] }>(
      'provider-live-update',
      (event) => {
        useFollowsStore
          .getState()
          .setProviderLive(event.payload.provider, event.payload.streams ?? []);
      },
    ).then((u) => {
      if (cancelled) u();
      else unlistenLive = u;
    });

    // The same arrangement for FAVORITES, which are a separate watchlist: a
    // favorite need not be followed anywhere, so neither the Twitch follow
    // poller nor the provider one above can see it. The event carries the whole
    // snapshot across every platform, so the store replaces its map wholesale.
    void useFavoritesStore.getState().hydrate();
    void listen<{ source: string; streams: FavoriteStreamRow[] }>(
      'favorites-live-update',
      (event) => {
        useFavoritesStore.getState().setLive(event.payload.streams ?? []);
      },
    ).then((u) => {
      if (cancelled) u();
      else unlistenFavorites = u;
    });

    return () => {
      cancelled = true;
      unlistenLive?.();
      unlistenFavorites?.();
    };
  }, []);
  useEffect(() => {
    // Subscribe to snippet-store updates from MultiChat popouts so changes
    // made over there propagate here without reload.
    let unlistenSnippets: (() => void) | undefined;
    let cancelled = false;
    void startSnippetSync().then((u) => {
      if (cancelled) {
        u?.();
        return;
      }
      unlistenSnippets = u;
    });
    return () => {
      cancelled = true;
      unlistenSnippets?.();
    };
  }, []);
  useEffect(() => {
    // One-shot drops-token validation: the device-code token cannot be
    // refreshed, and is_drops_authenticated only checks that a token file
    // exists — so a revoked/expired token showed "connected" while earning
    // nothing. validate_drops_token clears stored tokens ONLY on a real 401
    // (a transport error rejects and changes nothing), so being offline at
    // boot is safe. Fire and forget, staggered off the boot path.
    return afterBoot(2000, () => {
      invoke('validate_drops_token').catch(() => {});
    });
  }, []);
  // Actions are stable for the store's lifetime, so read them without
  // subscribing. State goes through a shallow-compared selector. Previously this
  // was a bare `useAppStore()`, i.e. a subscription to the ENTIRE store — so
  // every toast, every mod-log entry (which fires on the IRC hot path) and every
  // 30s drops poll re-rendered the root and the whole tree under it.
  const { loadSettings, checkAuthStatus, addToast, setShowBadgesOverlay, setShowWhispersOverlay, updateSettings, loadActiveDropsCache, setProfileModalUser, openSettings } = useAppStore.getState();
  const { chatPlacement: storedChatPlacement, isLoading, isBooting, streamUrl, currentMediaType, showBadgesOverlay, badgesOverlayInitialPaintId, badgesOverlayInitialBadgeId, badgesOverlayInitialStreamNook, badgesOverlayInitialTarget, showWhispersOverlay, settings, isTheaterMode, isHomeActive, profileModalUser } = useAppStore(
    useShallow((s) => ({
      chatPlacement: s.chatPlacement,
      isLoading: s.isLoading,
      isBooting: s.isBooting,
      streamUrl: s.streamUrl,
      currentMediaType: s.currentMediaType,
      showBadgesOverlay: s.showBadgesOverlay,
      badgesOverlayInitialPaintId: s.badgesOverlayInitialPaintId,
      badgesOverlayInitialBadgeId: s.badgesOverlayInitialBadgeId,
      badgesOverlayInitialStreamNook: s.badgesOverlayInitialStreamNook,
      badgesOverlayInitialTarget: s.badgesOverlayInitialTarget,
      showWhispersOverlay: s.showWhispersOverlay,
      settings: s.settings,
      isTheaterMode: s.isTheaterMode,
      isHomeActive: s.isHomeActive,
      profileModalUser: s.profileModalUser,
    })),
  );
  // Channels owned by StreamNook MultiChat popouts. When the currently-watched
  // channel is in here, the in-app chat panel collapses so the popout becomes
  // the sole chat surface — no duplicate chat across windows.
  const channelsInPopouts = useAppStore((s) => s.channelsInPopouts);
  const currentStream = useAppStore((s) => s.currentStream);
  // Watch-reward inputs: the watched channel + its (live-updating) category, so
  // event-reward claims can fire the moment a streamer switches categories.
  const watchRewardChannel = currentStream?.user_login ?? null;
  const watchRewardGame = currentStream?.game_name ?? null;
  const activeChatChannelInPopout = !!(
    currentStream?.user_login &&
    channelsInPopouts.has(currentStream.user_login.toLowerCase())
  );

  // Mobile portrait cannot use a side-docked chat: the stream view is a flex row,
  // so a docked panel squeezes the video to nothing. Rather than fork the layout,
  // force the placement the column path already handles ('bottom') and let
  // mobile.css size the two bands. Landscape is close enough to a narrow desktop
  // window that the stored preference still works there.
  const [isPortraitNow, setIsPortraitNow] = useState(() => IS_MOBILE && isPortrait());
  useEffect(() => {
    if (!IS_MOBILE) return;
    return onOrientationChange(() => setIsPortraitNow(isPortrait()));
  }, []);
  const chatPlacement = IS_MOBILE && isPortraitNow ? 'bottom' : storedChatPlacement;

  const [chatSize, setChatSize] = useState(chatPlacement === 'bottom' ? DEFAULT_CHAT_HEIGHT : DEFAULT_CHAT_WIDTH);
  const [modLogsSize, setModLogsSize] = useState(300); // Default Mod Logs size
  // Chat "reveal on hover" (auto-hide). Only the side docks (left/right) support
  // it: the chat tucks to a thin edge handle and slides out on hover, with the
  // player flexing to make room (no window resize, the reveal lives inside the
  // existing video area).
  const chatAutoHide = settings.chat_auto_hide ?? false;
  const isSideChat = chatPlacement === 'right' || chatPlacement === 'left';
  const autoHideActive = chatAutoHide && isSideChat;
  // Fullscreen chat overlay: while Plyr is in CSS fullscreen the docked chat
  // panel is lifted above the fullscreen layer as a translucent fixed column
  // (globals.css .chat-fullscreen-overlay). Same DOM node, no remount, no
  // second WebView. Hidden with the player controls unless hovered/focused.
  const isPlayerFullscreen = useAppStore((s) => s.isPlayerFullscreen);
  const playerOverlayVisible = useAppStore((s) => s.playerOverlayVisible);
  const fsChat = settings.fullscreen_chat;
  const fsOverlayActive = isPlayerFullscreen && (fsChat?.mode ?? 'overlay') === 'overlay';
  const [fsChatHeld, setFsChatHeld] = useState(false);
  const fsChatSide: 'left' | 'right' =
    fsChat?.side && fsChat.side !== 'auto' ? fsChat.side : chatPlacement === 'left' ? 'left' : 'right';
  const fsChatHidden = fsOverlayActive && (fsChat?.auto_hide ?? true) && !playerOverlayVisible && !fsChatHeld;
  useEffect(() => {
    if (!isPlayerFullscreen) setFsChatHeld(false);
  }, [isPlayerFullscreen]);
  const [chatRevealed, setChatRevealed] = useState(false);
  const chatRevealTimer = useRef<number | null>(null);
  // A streamnook:// deep link that landed while the app was still booting. Played
  // once boot finishes (see the deep-link effects below).
  const pendingWatchChannelRef = useRef<string | null>(null);
  const revealChat = () => {
    if (chatRevealTimer.current) { window.clearTimeout(chatRevealTimer.current); chatRevealTimer.current = null; }
    setChatRevealed(true);
  };
  const hideChatSoon = () => {
    if (chatRevealTimer.current) window.clearTimeout(chatRevealTimer.current);
    chatRevealTimer.current = window.setTimeout(() => setChatRevealed(false), 150);
  };
  // Collapse + drop any pending timer whenever auto-hide stops applying.
  useEffect(() => {
    if (!autoHideActive) setChatRevealed(false);
    return () => { if (chatRevealTimer.current) window.clearTimeout(chatRevealTimer.current); };
  }, [autoHideActive]);
  const isMultiNookActive = usemultiNookStore((s) => s.isMultiNookActive);
  // Which tile the grid's chat pane is showing. Only read to decide WHICH chat
  // surface renders; ChatWidget still synthesizes the Twitch case itself.
  const multiNookSlots = usemultiNookStore((s) => s.slots);
  const activeChatChannelId = usemultiNookStore((s) => s.activeChatChannelId);
  const isChatHidden = usemultiNookStore((s) => s.isChatHidden);
  const slots = usemultiNookStore((s) => s.slots);
  const visibleSlotsLength = slots.filter((s) => !s.isMinimized).length;
  const [isResizing, setIsResizing] = useState(false);
  const [isResizingModLogs, setIsResizingModLogs] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const [selectedBadge, setSelectedBadge] = useState<{ badge: BadgeVersion; setId: string } | null>(null);
  
  // Persist savedWindowSize to localStorage so it survives app restarts. The
  // optional `maximized` flag is additive, so a stored value written before it
  // existed still parses (missing = it wasn't maximized) and needs no key bump.
  const [savedWindowSize, setSavedWindowSize] = useState<CompactSavedSize | null>(() => {
    try {
      const stored = localStorage.getItem(COMPACT_SAVED_SIZE_KEY);
      return stored ? JSON.parse(stored) : null;
    } catch {
      return null;
    }
  });
  
  const [showChangelog, setShowChangelog] = useState(false);
  const [changelogVersion, setChangelogVersion] = useState<string | null>(null);
  // Dev-only: a changelog opened by the simulated-update reload, so its close
  // doesn't persist last_seen_version (the version isn't really installed).
  const devForcedChangelogRef = useRef(false);
  const [showSetupWizard, setShowSetupWizard] = useState(false);

  // Narrow gate subscriptions for the lazy overlays below — never the whole store.
  const isSettingsOpen = useAppStore((s) => s.isSettingsOpen);
  const isProfileViewerOpen = useAppStore((s) => !!s.profileViewerUserId);
  const showDropsOverlay = useAppStore((s) => s.showDropsOverlay);
  const showMarketplaceOverlay = useAppStore((s) => s.showMarketplaceOverlay);
  const showEmoteSetsOverlay = useAppStore((s) => s.showEmoteSetsOverlay);
  const isCommandPaletteOpen = useAppStore((s) => s.isCommandPaletteOpen);
  const isClipModalOpen = useAppStore((s) => s.clipModal !== null);
  const isClipEditorOpen = useAppStore((s) => s.clipEditor !== null);

  // "Ever opened" latches: each always-mounted overlay owns its open/close
  // animation, so once its gate first opens it stays mounted. Setting state
  // during render is the StrictMode-safe way to latch without an effect.
  const [settingsEverOpened, setSettingsEverOpened] = useState(false);
  if (isSettingsOpen && !settingsEverOpened) setSettingsEverOpened(true);
  const [profileViewerEverOpened, setProfileViewerEverOpened] = useState(false);
  if (isProfileViewerOpen && !profileViewerEverOpened) setProfileViewerEverOpened(true);
  const [dropsEverOpened, setDropsEverOpened] = useState(false);
  if (showDropsOverlay && !dropsEverOpened) setDropsEverOpened(true);
  const [marketplaceEverOpened, setMarketplaceEverOpened] = useState(false);
  if (showMarketplaceOverlay && !marketplaceEverOpened) setMarketplaceEverOpened(true);
  const [emoteSetsEverOpened, setEmoteSetsEverOpened] = useState(false);
  if (showEmoteSetsOverlay && !emoteSetsEverOpened) setEmoteSetsEverOpened(true);
  const [commandPaletteEverOpened, setCommandPaletteEverOpened] = useState(false);
  if (isCommandPaletteOpen && !commandPaletteEverOpened) setCommandPaletteEverOpened(true);
  const [clipModalEverOpened, setClipModalEverOpened] = useState(false);
  if (isClipModalOpen && !clipModalEverOpened) setClipModalEverOpened(true);
  const [clipEditorEverOpened, setClipEditorEverOpened] = useState(false);
  if (isClipEditorOpen && !clipEditorEverOpened) setClipEditorEverOpened(true);
  const [setupWizardEverOpened, setSetupWizardEverOpened] = useState(false);
  if (showSetupWizard && !setupWizardEverOpened) setSetupWizardEverOpened(true);

  // Track previous placement and chat size to detect changes
  const prevChatPlacementRef = useRef(chatPlacement);
  const prevChatSizeRef = useRef(chatPlacement === 'bottom' ? DEFAULT_CHAT_HEIGHT : DEFAULT_CHAT_WIDTH);

  // Refs for aspect ratio lock to avoid stale closures
  const aspectRatioLockEnabledRef = useRef(false);
  const chatSizeRef = useRef(chatPlacement === 'bottom' ? DEFAULT_CHAT_HEIGHT : DEFAULT_CHAT_WIDTH);
  const chatPlacementRef = useRef(chatPlacement);
  const isTheaterModeRef = useRef(false);
  const streamUrlRef = useRef<string | null>(null);
  const isMultiNookActiveRef = useRef(false);
  const multiNookSlotsLengthRef = useRef(0);
  const isAdjustingRef = useRef(false);
  // True only while a chat-placement change is actively resizing the window.
  // The aspect-ratio settle effect and the resize listener both bail when this
  // is set, so the placement handler's video-preserving resize is the sole
  // authority during the transition instead of racing two other resizers that
  // use a non-preserving formula and collapse the window toward its minimum.
  const placementResizeInProgressRef = useRef(false);
  // Deadline after our own setSize during which the resize listener stands down.
  // A resize we performed must never be read back as user input: the aspect formula's
  // constant offsets (title bar, gaps) don't scale, so a clamped or rounded write can
  // disagree with the formula by a pixel or two and ping-pong. This also closes the
  // gap in placementResizeInProgressRef, which is cleared in a finally block before
  // the resize event it caused has been delivered.
  const selfResizeUntilRef = useRef(0);
  // When the current channel's chat is owned by a MultiChat popout, main's
  // chat panel JSX is gone — the video player container expands to fill the
  // freed width, but stays 16:9 so the user sees side black bars. The
  // aspect-ratio resize handler needs to know about this and treat chat as
  // hidden for that calculation so the window shrinks to remove the bars.
  const activeChatChannelInPopoutRef = useRef(false);

  // Handle placement changes - preserve video dimensions when moving chat around
  useEffect(() => {
    const handlePlacementChange = async () => {
      if (prevChatPlacementRef.current === chatPlacement) return;

      const oldPlacement = prevChatPlacementRef.current;
      const oldChatSize = prevChatSizeRef.current;

      Logger.debug('[ChatSize] Placement changed from', oldPlacement, 'to', chatPlacement);

      // Set appropriate default based on new placement
      const newSize = chatPlacement === 'bottom' ? DEFAULT_CHAT_HEIGHT : DEFAULT_CHAT_WIDTH;
      Logger.debug('[ChatSize] Setting chat size to', newSize);
      setChatSize(newSize);

      // Auto-hide reveals into the existing video area, so a placement change must
      // NOT resize the window (the chat takes no docked space until you hover it).
      if (autoHideActive) {
        prevChatPlacementRef.current = chatPlacement;
        prevChatSizeRef.current = newSize;
        return;
      }
      chatSizeRef.current = newSize;

      // Only resize window if aspect ratio lock is enabled and stream is playing
      // IMPORTANT: Use the reactive isTheaterMode value, not the ref, to avoid stale state
      // when entering/exiting theater mode (where chat placement changes simultaneously)
      const lockEnabled = aspectRatioLockEnabledRef.current;
      const currentStreamUrl = streamUrlRef.current;
      const currentIsMultiNookActive = isMultiNookActiveRef.current;

      // Skip while compact is active AND on the transition out of it. Leaving
      // Compact View flips isTheaterMode and chatPlacement in one store update,
      // so this handler and the compact effect would both resize the window and
      // race; whichever landed last won, which is how a maximized window came
      // back un-maximized. The compact effect owns geometry in both directions.
      // isTheaterModeRef still holds the pre-toggle value here (a later effect
      // syncs it), so a truthy ref with isTheaterMode false means "just exited".
      if (isTheaterMode || isTheaterModeRef.current) {
        Logger.debug('[ChatSize] Skipping resize - compact view owns this transition');
        prevChatPlacementRef.current = chatPlacement;
        prevChatSizeRef.current = newSize;
        return;
      }

      if (lockEnabled && (currentStreamUrl || currentIsMultiNookActive)) {
        // Claim the resize lock for this whole transition so the aspect-ratio
        // settle effect and the window resize listener stand down instead of
        // firing their own non-preserving resize and fighting this one.
        placementResizeInProgressRef.current = true;
        try {
          const window = getCurrentWindow();

          // Don't adjust if window is maximized
          const isMaximized = await window.isMaximized();
          if (isMaximized) {
            Logger.debug('[ChatSize] Window is maximized, skipping resize');
            prevChatPlacementRef.current = chatPlacement;
            prevChatSizeRef.current = newSize;
            return;
          }

          // Same for OS fullscreen — resizing there fights the fullscreen rect.
          const isFullscreen = await window.isFullscreen();
          if (isFullscreen) {
            Logger.debug('[ChatSize] Window is fullscreen, skipping resize');
            prevChatPlacementRef.current = chatPlacement;
            prevChatSizeRef.current = newSize;
            return;
          }

          // Logical pixels: the offsets below are all CSS pixels, so the window size
          // has to be converted out of physical device pixels before mixing them.
          const size = await getLogicalInnerSize(window);
          const titleBarHeight = 40;

          Logger.debug('[ChatSize] Calculating window size to preserve video dimensions');
          Logger.debug('[ChatSize] Old layout:', oldPlacement, 'with chat size', oldChatSize);
          Logger.debug('[ChatSize] New layout:', chatPlacement, 'with chat size', newSize);

          let targetAspectRatio = 16.0 / 9.0;
          
          // Dynamically measure sidebar instead of hardcoding
          let uiWidthOffset = 64;
          const sidebarEl = document.querySelector('.border-r.border-borderSubtle.flex-shrink-0');
          if (sidebarEl) {
            uiWidthOffset = sidebarEl.getBoundingClientRect().width;
          }
          let uiHeightOffset = 0;

          // Account for the chat resize separator
          if (chatPlacement === 'right' || chatPlacement === 'left') uiWidthOffset += 4;
          if (chatPlacement === 'bottom') uiHeightOffset += 4;

          if (currentIsMultiNookActive) {
            const len = multiNookSlotsLengthRef.current;
            uiWidthOffset += 16; // 8px padding on L/R
            uiHeightOffset += 16; // 8px padding on T/B

            // Add inner gaps (8px each) based on grid matrix
            if (len === 2) { targetAspectRatio = 16.0 / 18.0; uiHeightOffset += 8; }
            else if (len >= 3 && len <= 4) { targetAspectRatio = 16.0 / 9.0; uiWidthOffset += 8; uiHeightOffset += 8; }
            else if (len >= 5 && len <= 6) { targetAspectRatio = 48.0 / 18.0; uiWidthOffset += 16; uiHeightOffset += 8; }
            else if (len >= 7 && len <= 9) { targetAspectRatio = 16.0 / 9.0; uiWidthOffset += 16; uiHeightOffset += 16; }
            else if (len >= 10 && len <= 12) { targetAspectRatio = 64.0 / 27.0; uiWidthOffset += 24; uiHeightOffset += 16; }
            else if (len >= 13 && len <= 16) { targetAspectRatio = 16.0 / 9.0; uiWidthOffset += 24; uiHeightOffset += 24; }
            else if (len >= 17 && len <= 20) { targetAspectRatio = 80.0 / 36.0; uiWidthOffset += 32; uiHeightOffset += 24; }
            else { uiWidthOffset += 32; uiHeightOffset += 32; }
          }

          const [newWidth, newHeight] = await invoke<[number, number]>('calculate_aspect_ratio_size_preserve_video', {
            currentWidth: size.width,
            currentHeight: size.height,
            oldChatSize: oldChatSize,
            newChatSize: newSize,
            oldChatPlacement: oldPlacement,
            newChatPlacement: chatPlacement,
            titleBarHeight: titleBarHeight,
            targetAspectRatio: targetAspectRatio,
            uiWidthOffset: uiWidthOffset,
            uiHeightOffset: uiHeightOffset,
          });

          Logger.debug('[ChatSize] New window size to preserve video:', newWidth, newHeight);

          const clamped = await clampToWorkArea(newWidth, newHeight);
          if (Math.abs(size.width - clamped.width) > 5 || Math.abs(size.height - clamped.height) > 5) {
            selfResizeUntilRef.current = Date.now() + 300;
            await window.setSize(new LogicalSize(clamped.width, clamped.height));
          }
        } catch (error) {
          Logger.error('[ChatSize] Failed to resize window:', error);
        } finally {
          placementResizeInProgressRef.current = false;
        }
      }

      prevChatPlacementRef.current = chatPlacement;
      prevChatSizeRef.current = newSize;
    };

    handlePlacementChange();
    // autoHideActive: re-read the skip-resize branch when auto-hide toggles (the
    // early return on unchanged placement keeps a bare toggle a no-op here).
  }, [chatPlacement, isTheaterMode, autoHideActive]);

  // Listen for badge detail events from chat
  useEffect(() => {
    const handleBadgeDetail = (event: CustomEvent) => {
      const { badge, setId } = event.detail;
      setSelectedBadge({ badge, setId });
    };

    window.addEventListener('show-badge-detail', handleBadgeDetail as EventListener);

    return () => {
      window.removeEventListener('show-badge-detail', handleBadgeDetail as EventListener);
    };
  }, []);

  // Ad auto-pivot: the backend escapes a leaked ad by re-resolving through a
  // clean proxy region and emitting `ad-pivot` with the fresh player URL.
  useEffect(() => {
    const unlistenPromise = listen<{ url: string; region?: string; channel?: string }>(
      'ad-pivot',
      (event) => {
        const { url, region } = event.payload;
        if (url) useAppStore.getState().applyAdPivot(url, region);
      }
    );
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  // A plugin can deep-link into a settings tab — e.g. its plugins-page card
  // pointing at the Integrations tab where its real panel renders — by emitting
  // this event. The host owns the navigation; the plugin only asks.
  useEffect(() => {
    const unlistenPromise = listen<{ tab?: SettingsTab; section?: string }>(
      'streamnook:open-settings',
      (event) => {
        useAppStore.getState().openSettings(event.payload.tab ?? undefined, event.payload.section);
      }
    );
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  // streamnook:// deep links (the share links resolve here). The Rust handler
  // emits `streamnook://watch` for links opened while we're running; the
  // take_pending_watch_link command drains one the app was cold-started with.
  // Either path plays the channel, deferring until boot finishes so startStream
  // has settings/auth loaded.
  useEffect(() => {
    const openChannel = (channel: string) => {
      const login = channel.trim().toLowerCase();
      if (!login) return;
      if (useAppStore.getState().isBooting) {
        pendingWatchChannelRef.current = login;
      } else {
        void useAppStore.getState().startStream(login);
      }
    };

    const unlistenPromise = listen<string>('streamnook:watch', (event) => {
      if (event.payload) openChannel(event.payload);
    });

    // Cold start: the app was launched by the link before this listener existed.
    invoke<string | null>('take_pending_watch_link')
      .then((channel) => { if (channel) openChannel(channel); })
      .catch(() => { /* older backend without the command; safe to ignore */ });

    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  // Flush a deep link that arrived mid-boot, once the app is ready.
  useEffect(() => {
    if (!isBooting && pendingWatchChannelRef.current) {
      const channel = pendingWatchChannelRef.current;
      pendingWatchChannelRef.current = null;
      void useAppStore.getState().startStream(channel);
    }
  }, [isBooting]);

  // The context-menu blocker lives in InputContextMenuHost (rendered below), so
  // MultiChat popouts get the same menu — they render MultiChatWindow, not App.
  //
  // Global Keydown Blocker for Developer Tools (F12, Ctrl+Shift+I, Cmd+Option+I)
  useEffect(() => {
    // Disabled automatically in development environment
    const handleKeyDown = (e: KeyboardEvent) => {
        if (import.meta.env.DEV) return;

        if (e.key === 'F12') {
            e.preventDefault();
        }
        if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === 'i') {
            e.preventDefault();
        }
    };

    document.addEventListener('keydown', handleKeyDown);
    return () => {
        document.removeEventListener('keydown', handleKeyDown);
    };
  }, []);

  // Track presence in Supabase
  useEffect(() => {
    let cleanupPresence: (() => void) | null = null;

    const initPresence = async () => {
      if (isSupabaseConfigured()) {
        const { currentUser, isAuthenticated } = useAppStore.getState();
        let appVersion;
        try {
          appVersion = await getAppVersion();
        } catch (e) {
          Logger.warn('[App] Failed to get app version for presence:', e);
        }

        if (isAuthenticated && currentUser) {
          cleanupPresence = await trackPresence(currentUser.user_id, currentUser.display_name, appVersion);
        } else {
          // Track anonymous presence
          cleanupPresence = await trackPresence(undefined, undefined, appVersion);
        }
      }
    };

    // Presence is telemetry, not boot-critical: stagger it clear of startup.
    const cancel = afterBoot(5000, () => {
      void initPresence();
    });

    return () => {
      cancel();
      if (cleanupPresence) {
        cleanupPresence();
      }
    };
  }, []);

  useEffect(() => {
    if (!isSupabaseConfigured()) return;
    // Deferred registry init: the subscriptions start in the 5s stagger tier,
    // so the cleanup reads the unsubs from variables the deferred task fills in.
    let cleanupRegistry: (() => void) | null = null;
    let cleanupCosmetics: (() => void) | null = null;
    let cleanupAtmospheres: (() => void) | null = null;
    let onFocus: (() => void) | null = null;

    const cancel = afterBoot(5000, () => {
      // Warm the server-controlled switches before the first privileged write, so
      // a login-time write does not pay the manifest fetch (and does not fall
      // back to the legacy path just because the config had not landed yet).
      primeClientConfig();

      cleanupRegistry = subscribeToStreamNookRegistry();
      cleanupCosmetics = subscribeToCosmeticsRegistry();
      cleanupAtmospheres = subscribeToAtmospheresRegistry();

      // When the user returns to the app (e.g. after finishing a purchase on
      // streamnook.app in their browser), re-pull entitlements so a freshly
      // granted badge/perk shows right away even if the realtime channel happened
      // to miss the event. Throttled so rapid alt-tabbing doesn't spam the network.
      let lastResync = 0;
      onFocus = () => {
        const now = Date.now();
        if (now - lastResync < 10_000) return;
        lastResync = now;
        refreshEntitlementRegistries();
      };
      window.addEventListener('focus', onFocus);
    });

    return () => {
      cancel();
      cleanupRegistry?.(); cleanupCosmetics?.(); cleanupAtmospheres?.();
      if (onFocus) window.removeEventListener('focus', onFocus);
    };
  }, []);

  // NOTE (mobile port): the shell-agnostic steps of this boot effect are
  // replicated in src/mobile/boot/useMobileBoot.ts. When adding a boot step
  // here, decide whether the mobile shell needs it too and mirror it there.
  useEffect(() => {
    let isMounted = true;
    const cleanupFunctions: (() => void)[] = [];

    // Cross-window settings sync: when another window saves settings, refresh
    // ours so user-edited values (highlights, custom commands, nicknames, etc.)
    // show up everywhere without needing to reopen this window.
    let unlistenSettingsSync: (() => void) | undefined;
    void listenForSettingsUpdates(() => {
      void useAppStore.getState().loadSettings();
    }).then((unlisten) => {
      if (!isMounted) {
        unlisten();
        return;
      }
      unlistenSettingsSync = unlisten;
    });
    cleanupFunctions.push(() => unlistenSettingsSync?.());

    // Stagger helper scoped to this effect: schedules a task in a boot tier and
    // wires its cancel into this effect's cleanup (mirroring the isMounted
    // pattern the listeners below already use).
    const deferred = (tier: BootTier, task: () => void) => {
      const cancel = afterBoot(tier, task);
      if (isMounted) cleanupFunctions.push(cancel);
      else cancel();
    };

    const initializeApp = async () => {
      try {
        await Promise.allSettled([loadSettings(), checkAuthStatus()]);
      } finally {
        // Auth is now resolved (logged in or confirmed logged out), or a boot
        // step failed — either way drop the boot overlay so the home screen
        // eases in instead of either flashing its logged-out state mid-check
        // or hanging on the loader forever.
        useAppStore.setState({ isBooting: false });
      }

      // Resume the stream (and automation) the user was on before an update restart.
      // Consume-once and best-effort; runs after auth so startStream has a token.
      void import('./services/sessionResume').then(({ resumePreviousSession }) =>
        resumePreviousSession(),
      );

      // Clean up orphaned localStorage from migrated services (one-time cleanup)
      // Badge polling service moved to Rust - remove old localStorage keys
      localStorage.removeItem('streamnook_known_badges');
      localStorage.removeItem('streamnook_notified_available_badges');

      // Load active drops cache on startup (cached for 1 hour)
      deferred(2000, () => {
        loadActiveDropsCache();
      });

      // Auto-sync universal cache if stale (>24 hours since last sync)
      // This downloads the latest badge manifest from GitHub in the background
      deferred(5000, () => {
        import('./services/universalCacheService').then(({ autoSyncUniversalCacheIfStale }) => {
          autoSyncUniversalCacheIfStale();
        });
      });

      // The badge-drop feed (WebSocket + latest.json fallback) is Rust's
      // (services/badge_feed.rs): it must outlive this window.

      // Pre-fetch cosmetics for current user
      const { currentUser, isAuthenticated } = useAppStore.getState();
      if (isAuthenticated && currentUser?.user_id) {
        Logger.debug('[App] Pre-fetching cosmetics for current user...');
        const { registerOwnCosmeticAccounts, revalidateOwnCosmetics, getFullProfileWithFallback } =
          await import('./services/cosmeticsCache');
        const { seedOwnIdentitiesFromCache, getResolvedIdentity, getIdentityWithCache } =
          await import('./services/identityService');
        const { registerOwnAtmospheres } = await import('./stores/chatUserStore');
        const { listAccounts } = await import('./services/accountService');
        const selfId = currentUser.user_id;
        const selfLogin = currentUser.login || currentUser.username;

        // Every account we've added (primary + linked), so each one paints its OWN
        // cosmetics/badges/atmosphere instantly — not just the active account.
        let accountIds = [selfId];
        try {
          const ids = (await listAccounts()).map((a) => a.user_id).filter(Boolean);
          if (ids.length) accountIds = ids.includes(selfId) ? ids : [...ids, selfId];
        } catch {
          /* account registry not ready yet — fall back to the active account */
        }

        // Seed every account's paint/badge, curated third-party badges + loadout,
        // and Atmosphere from disk so chat + the profile card paint on frame one,
        // and register them so later edits write through, per account.
        registerOwnCosmeticAccounts(accountIds);
        seedOwnIdentitiesFromCache(accountIds);
        registerOwnAtmospheres(accountIds);

        // Revalidate in place (no blank window — unlike a deep clear-then-refetch).
        // The active account also warms the FULL profile cache so opening Profile
        // Settings is instant; others warm on demand when switched to. Network
        // revalidation is staggered: the disk seeds above already painted, so
        // this only refreshes them and must not compete with boot fetches.
        deferred(2000, () => {
          revalidateOwnCosmetics(selfId)
            .then(() => getFullProfileWithFallback(selfId, selfLogin, selfId, selfLogin))
            .catch((err: Error) =>
              Logger.error('[App] Failed to pre-fetch user profile:', err),
            );
          getResolvedIdentity(selfId).catch(() => {});
          getIdentityWithCache(selfId).catch(() => {});
          for (const id of accountIds) {
            if (id === selfId) continue;
            revalidateOwnCosmetics(id).catch(() => {});
            getResolvedIdentity(id).catch(() => {});
            getIdentityWithCache(id).catch(() => {});
          }
        });
      }

      // Set up event listeners for drops and channel points
      const addListener = async <T,>(event: string, handler: (event: { payload: T }) => void) => {
        try {
          const unlistenFn = await listen<T>(event, handler);
          if (isMounted) {
            cleanupFunctions.push(unlistenFn);
          } else {
            unlistenFn();
          }
        } catch (e) {
          Logger.warn(`[App] Failed to set up listener for ${event}:`, e);
        }
      };

      // Live 7TV emote-set updates (add/remove/rename) pushed from the shared
      // EventAPI socket in Rust. Updates this window's emote cache + notices.
      await addListener<EmoteSetUpdatePayload>('7tv://emote-set-update', (event) => {
        void handleSeventvEmoteSetUpdate(event.payload);
      });

      // Live 7TV cosmetics (paints/badges) for present users, delivered over the
      // same EventAPI socket. Re-resolves via GQL into the shared cosmetics cache.
      await addListener<CosmeticUpdatePayload>('7tv://cosmetic-update', (event) => {
        void handleSeventvCosmeticUpdate(event.payload);
      });

      // Moderator view: channel.moderate events from the dedicated, chat-tied
      // moderation socket (Rust). Enriches the mod log with the acting
      // moderator's identity. Mounted here (not per-stream) so it works in
      // offline chat and MultiNook with no stream open.
      await addListener<Record<string, unknown>>('eventsub://channel-moderate', (event) => {
        applyModerateEvent(event.payload);
      });

      await addListener<{ points_earned: number }>('channel-points-claimed', (event) => {
        const claim = event.payload;
        addToast(`Claimed ${claim.points_earned} channel points!`, 'success');

        // Track channel points in Supabase
        if (isSupabaseConfigured()) {
          const { currentUser, isAuthenticated } = useAppStore.getState();
          if (isAuthenticated && currentUser?.user_id) {
            incrementStat(currentUser.user_id, 'channel_points_collected', claim.points_earned);
          }
        }
      });

      // Listen for drops automation errors and report them to Discord via logService
      await addListener<{ category: string; message: string }>('drops-error', (event) => {
        const { category, message } = event.payload;
        // Log as error - this will be picked up by logService and sent to Discord
        Logger.error(`[${category}] ${message}`);
      });

      // Listen for start-whisper events from standalone profile windows
      await addListener<{ id: string; login: string; display_name: string; profile_image_url?: string }>('start-whisper', (event) => {
        Logger.debug('[App] Received start-whisper event:', event.payload);
        useAppStore.getState().openWhisperWithUser(event.payload);
      });

      // Listen for refresh-following-list events (triggered by follow/unfollow automation)
      await addListener('refresh-following-list', () => {
        Logger.debug('[App] Received refresh-following-list event, refreshing...');
        useAppStore.getState().loadFollowedStreams();
      });

      // Listen for automation status updates (for title bar gift box animation)
      await addListener<{ active: boolean }>('drop-progress', (event) => {
        Logger.debug('[App] Automation status update:', event.payload.active);
        useAppStore.getState().setDropProgressActive(event.payload.active);
      });

      // Listen for whisper import events (global listener so import works from any UI)
      await addListener<{ step: number; status: string; detail: string; current: number; total: number }>(
        'whisper-import-progress',
        (event) => {
          const { step, status, detail, current, total } = event.payload;
          const { setWhisperImportState } = useAppStore.getState();
          setWhisperImportState({
            progress: { step, status: status as WhisperImportProgress['status'], detail, current, total }
          });

          // Track export progress for step 3
          if (step === 3 && status === 'running') {
            const match = detail.match(/Exporting: (.+)/);
            setWhisperImportState({
              exportProgress: { current, total, username: match ? match[1] : '' }
            });
          }

          // When step 2 completes, set the estimated end time
          if (step === 2 && status === 'complete') {
            const countMatch = detail.match(/Found (\d+) conversations/);
            if (countMatch) {
              const count = parseInt(countMatch[1], 10);
              const SECONDS_PER_CONVERSATION = 3;
              const estimatedSeconds = count * SECONDS_PER_CONVERSATION;
              const endTime = Date.now() + (estimatedSeconds * 1000);
              setWhisperImportState({
                totalConversations: count,
                estimatedEndTime: endTime
              });
            }
          }
        }
      );

      await addListener<{ success: boolean; message: string; conversations: number; messages: number }>(
        'whisper-import-complete',
        (event) => {
          const { success, message, conversations, messages } = event.payload;
          const { setWhisperImportState, addToast } = useAppStore.getState();
          if (success) {
            Logger.debug('[App] Whisper import completed:', conversations, 'conversations,', messages, 'messages');
            setWhisperImportState({
              isImporting: false,
              result: { conversations, messages },
              error: null
            });
            addToast(`Imported ${messages.toLocaleString()} whisper messages from ${conversations} conversations`, 'success');
          } else {
            Logger.error('[App] Whisper import failed:', message);
            setWhisperImportState({
              isImporting: false,
              error: message
            });
            addToast(`Whisper import failed: ${message}`, 'error');
          }
        }
      );

      // Listen for reserved stream going offline (watch token allocation feature)
      await addListener('reserved-stream-offline', () => {
        Logger.debug('[App] Reserved stream went offline, clearing reservation');
        addToast('Reserved stream went offline - token returned to rotation', 'info');
      });

      // streamnook:// links are owned by Rust: its deep-link handler emits
      // `streamnook:watch` (see the effect near the top of this component),
      // and cold starts drain `take_pending_watch_link`. A second
      // subscription here through the plugin's onOpenUrl started every warm
      // link twice (two relay starts 20 to 40 ms apart, the outgoing hls.js
      // instance erroring against a swapped upstream) until 2026-09-06.
    };

    initializeApp();

    // Set up periodic auth check to detect session expiry while watching
    // Check every 5 minutes
    const authCheckInterval = setInterval(async () => {
      const { isAuthenticated: wasAuthenticated, currentStream } = useAppStore.getState();

      // Only check if we were authenticated and are currently watching a stream
      if (wasAuthenticated && currentStream) {
        Logger.debug('[App] Performing periodic auth check...');
        await checkAuthStatus();
      }
    }, 5 * 60 * 1000); // 5 minutes

    return () => {
      isMounted = false;
      cleanupFunctions.forEach(fn => fn());
      clearInterval(authCheckInterval);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadSettings, checkAuthStatus]);

  // Theme/glass/font application is shared with the mobile shell.
  useThemeBoot();
  // Tell CSS when a stream is actually on screen. Decorative chrome animations
  // (the title-bar gift pulse, the update pill's glow, the shared-chat shimmer)
  // are part of how the app feels while you browse, so they run freely on Home.
  // During playback they are pure cost: they repaint the window every frame on
  // top of the video, which measured 0.31 of a CPU core on a 144 Hz display.
  // globals.css holds them still under `html[data-watching]`. Home overlaying a
  // running stream counts as browsing, not watching, so the flag clears there.
  useEffect(() => {
    // 'offline' is the offline-chat sentinel, not a playing stream, so it does
    // not count: there is no video on screen to spend the frames on.
    const hasVideo = (!!streamUrl && streamUrl !== 'offline') || isMultiNookActive;
    const watching = hasVideo && !isHomeActive;
    if (watching) {
      document.documentElement.dataset.watching = 'true';
    } else {
      delete document.documentElement.dataset.watching;
    }
  }, [streamUrl, isMultiNookActive, isHomeActive]);

  // Check if we need to show the first-time setup wizard. Drive purely off
  // setup_complete: if it's false, show the wizard. (Gate on `quality` only as a
  // "settings have hydrated" signal.)
  useEffect(() => {
    if (settings.quality === undefined) return; // wait until settings hydrate
    if (settings.setup_complete) {
      // Mobile-only backstop: settings.setup_complete has been observed reverting
      // to false on Android between launches even after the wizard writes it
      // (root cause not yet identified; nothing in loadSettings or the
      // updateSettings callers accounts for it). Without this marker the wizard
      // reopens on every launch and the app is unusable, so record completion
      // somewhere that survives independently of the settings file. Desktop
      // deliberately does not use it: a settings reset or restore that clears
      // setup_complete must bring the wizard back.
      if (IS_MOBILE) {
        try { localStorage.setItem(SETUP_COMPLETE_MARKER, 'true'); } catch { /* private mode */ }
      }
      Logger.debug('[App] Setup already complete, skipping wizard');
      return;
    }
    if (IS_MOBILE) {
      try {
        if (localStorage.getItem(SETUP_COMPLETE_MARKER) === 'true') {
          Logger.debug('[App] Setup marked complete locally, skipping wizard');
          return;
        }
      } catch { /* private mode */ }
    }
    Logger.debug('[App] Setup not complete - showing wizard');
    setShowSetupWizard(true);
  }, [settings.quality, settings.setup_complete]);

  // Ctrl+Shift+C → force-open the changelog overlay against the current
  // app version (fetches real release notes from GitHub). Useful for
  // re-reading what's new without juggling last_seen_version in settings.
  useEffect(() => {
    const onKey = async (e: KeyboardEvent) => {
      if (e.ctrlKey && e.shiftKey && (e.key === 'C' || e.key === 'c')) {
        e.preventDefault();
        try {
          const currentVersion = await getAppVersion();
          setChangelogVersion(currentVersion);
          setShowChangelog(true);
        } catch (err) {
          Logger.error('[App] Failed to force-open changelog:', err);
        }
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  // Check if we need to show the changelog after an update (and force relogin if needed)
  useEffect(() => {
    const checkForVersionChange = async () => {
      try {
        // Get the current app version
        const currentVersion = await getAppVersion();
        const { settings, logoutFromTwitch, isAuthenticated } = useAppStore.getState();
        const lastSeenVersion = settings.last_seen_version;

        Logger.debug('[App] Version check - Current:', currentVersion, 'Last seen:', lastSeenVersion);

        // The two migrations below force a re-login for DESKTOP users upgrading
        // from v4.9.1 and v2.2.0. They fire when localStorage lacks a marker key
        // and the user is signed in. A fresh Android install has empty
        // localStorage and has never run either version, so both would fire on
        // first launch, log the user straight back out and re-open the setup
        // wizard — every single launch, which is exactly what was happening.
        // Mark them satisfied on mobile so they never run.
        if (IS_MOBILE) {
          localStorage.setItem(WEBVIEW_RELOGIN_MIGRATION_KEY, 'true');
          localStorage.setItem(V220_RELOGIN_MIGRATION_KEY, 'true');
        }

        // One-time force re-login for v4.9.1 webview features
        // This only triggers once per user, ever, and only if they're currently logged in
        const hasCompletedWebviewMigration = localStorage.getItem(WEBVIEW_RELOGIN_MIGRATION_KEY);
        if (!hasCompletedWebviewMigration && isAuthenticated) {
          Logger.debug('[App] One-time force re-login for webview features (v4.9.1)');

          // Mark migration as complete BEFORE logout so it only happens once
          localStorage.setItem(WEBVIEW_RELOGIN_MIGRATION_KEY, 'true');

          // Log the user out
          await logoutFromTwitch();

          // Show a toast explaining why
          addToast(
            'Please log in again to enable new features (whisper import, follow/unfollow)',
            'info'
          );

          // Update last seen version
          await updateSettings({ ...settings, last_seen_version: currentVersion });

          // Show the setup wizard so they can log back in
          setShowSetupWizard(true);
          return;
        }

        // Mark migration as complete for users who weren't logged in (no action needed)
        if (!hasCompletedWebviewMigration) {
          localStorage.setItem(WEBVIEW_RELOGIN_MIGRATION_KEY, 'true');
        }

        // One-time force re-login for v2.2.0 with full webview data clear
        // This ensures Twitch session cookies are fully cleared so user must re-login
        const hasCompletedV220Migration = localStorage.getItem(V220_RELOGIN_MIGRATION_KEY);
        if (!hasCompletedV220Migration && isAuthenticated) {
          Logger.debug('[App] One-time force re-login for v2.2.0 update');

          // Mark migration as complete BEFORE logout so it only happens once
          localStorage.setItem(V220_RELOGIN_MIGRATION_KEY, 'true');

          // Log the user out (clears app tokens)
          await logoutFromTwitch();

          // Also clear WebView2 browsing data (cookies, cache) so Twitch session is fully cleared
          try {
            await invoke('clear_webview_data');
            Logger.debug('[App] WebView2 data cleared successfully');
          } catch (e) {
            Logger.warn('[App] Failed to clear WebView2 data:', e);
          }

          // Show a toast explaining why
          addToast(
            'Please log in again to continue using StreamNook',
            'info'
          );

          // Update last seen version
          await updateSettings({ ...settings, last_seen_version: currentVersion });

          // Show the setup wizard so they can log back in
          setShowSetupWizard(true);
          return;
        }

        // Mark migration as complete for users who weren't logged in (no action needed)
        if (!hasCompletedV220Migration) {
          localStorage.setItem(V220_RELOGIN_MIGRATION_KEY, 'true');
        }

        // If there's no last seen version (first run) or the version has changed
        if (lastSeenVersion && lastSeenVersion !== currentVersion) {
          Logger.debug('[App] Version changed, showing changelog');
          setChangelogVersion(currentVersion);
          setShowChangelog(true);
        } else if (!lastSeenVersion) {
          // First run - just update the last seen version without showing changelog
          Logger.debug('[App] First run, setting initial version');
          updateSettings({ ...settings, last_seen_version: currentVersion });
        }
      } catch (error) {
        Logger.error('[App] Failed to check version:', error);
      }
    };

    // Only run after settings are loaded
    if (settings.quality !== undefined) {
      checkForVersionChange();
    }
  }, [settings.quality, updateSettings, addToast]);

  // Dev-only: after a simulated-update reload, pop the changelog for the version
  // we "updated" to, mirroring how production shows it after a real update. The
  // flag survives the webview reload via sessionStorage.
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    const v = sessionStorage.getItem('streamnook-dev-changelog');
    if (v) {
      sessionStorage.removeItem('streamnook-dev-changelog');
      devForcedChangelogRef.current = true;
      setChangelogVersion(v);
      setShowChangelog(true);
    }
  }, []);

  // Handle changelog close - update the last seen version
  const handleChangelogClose = async () => {
    setShowChangelog(false);

    // A dev-forced preview never really installed that version, so don't record
    // it as seen (that would suppress the real changelog or mis-trigger it later).
    if (devForcedChangelogRef.current) {
      devForcedChangelogRef.current = false;
      return;
    }

    if (changelogVersion) {
      try {
        const { settings } = useAppStore.getState();
        await updateSettings({ ...settings, last_seen_version: changelogVersion });
        Logger.debug('[App] Updated last_seen_version to:', changelogVersion);
      } catch (error) {
        Logger.error('[App] Failed to update last_seen_version:', error);
      }
    }
  };

  // Handle theater mode - resize window to user's selected compact view preset
  useEffect(() => {
    const handleTheaterMode = async () => {
      if (!streamUrl) return; // Only apply when a stream is playing

      try {
        const window = getCurrentWindow();

        if (isTheaterMode) {
          // Entering theater mode - save current size and resize to selected preset
          if (!savedWindowSize) {
            // Logical pixels, matching the LogicalSize this is restored with below.
            const wasMaximized = await window.isMaximized();
            const currentSize = await getLogicalInnerSize(window);
            const sizeToSave = { width: currentSize.width, height: currentSize.height, maximized: wasMaximized };
            setSavedWindowSize(sizeToSave);
            // Persist to localStorage so it survives app restart
            localStorage.setItem(COMPACT_SAVED_SIZE_KEY, JSON.stringify(sizeToSave));
            // Resizing a still-maximized window leaves Windows thinking it's
            // maximized, so the exit path restored pixel dimensions onto a
            // window that never dropped the state. Leave maximized first, then
            // shrink; exiting calls maximize() to put it back.
            if (wasMaximized) await window.unmaximize();
          }

          // Get the selected compact view preset
          const preset = getSelectedCompactViewPreset(
            settings.compact_view?.selectedPresetId,
            settings.compact_view?.customPresets
          );

          // Title bar height is 40px, window borders are 1px each side
          const titleBarHeight = 40;
          const windowBorderWidth = 2; // 1px border on each side
          // Subtract borders so total window width matches the preset exactly
          const targetWidth = preset.width - windowBorderWidth;
          // Recalculate height to maintain 16:9 aspect ratio based on adjusted width
          const videoHeight = Math.round(targetWidth / 16 * 9);
          const targetHeight = videoHeight + titleBarHeight;

          Logger.debug(`Entering compact view - resizing to: ${targetWidth}x${targetHeight} (${preset.name}, video: ${targetWidth}x${videoHeight})`);
          selfResizeUntilRef.current = Date.now() + 300;
          await window.setSize(new LogicalSize(targetWidth, targetHeight));
        } else if (savedWindowSize) {
          // Exiting theater mode - restore previous size
          Logger.debug('Exiting compact view - restoring to:', savedWindowSize.maximized ? 'maximized' : `${savedWindowSize.width}x${savedWindowSize.height}`);
          selfResizeUntilRef.current = Date.now() + 300;
          await restoreFromCompact(window, savedWindowSize);
          setSavedWindowSize(null);
          // Clear from localStorage
          localStorage.removeItem(COMPACT_SAVED_SIZE_KEY);
        }
      } catch (error) {
        Logger.error('Failed to resize window for compact view:', error);
      }
    };

    handleTheaterMode();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isTheaterMode, streamUrl, settings.compact_view?.selectedPresetId]); // Re-run if preset changes while in compact view

  // Keep-on-top, scoped to Compact View. The only place that calls
  // setAlwaysOnTop for the main window: the store action just writes the
  // setting. Runs on mount and on every Compact View entry/exit, so leaving
  // compact always releases the window. Don't read isAlwaysOnTop() here — that
  // command isn't in the capability allowlist, so the setting is the truth.
  useEffect(() => {
    const pinned = isTheaterMode && settings.keep_on_top_in_compact === true;
    getCurrentWindow()
      .setAlwaysOnTop(pinned)
      .catch((err) => Logger.error('[Window] Failed to set always on top:', err));
  }, [settings.keep_on_top_in_compact, isTheaterMode]);

  // On app startup, if we have a saved window size from a previous session where the app
  // closed while in compact view, restore it now (if not currently in theater mode)
  useEffect(() => {
    const restoreSavedWindowSize = async () => {
      // Only restore if we have a saved size AND we're not in theater mode
      if (savedWindowSize && !isTheaterMode) {
        try {
          const window = getCurrentWindow();
          Logger.debug('Restoring window size from previous session:', savedWindowSize.maximized ? 'maximized' : `${savedWindowSize.width}x${savedWindowSize.height}`);
          selfResizeUntilRef.current = Date.now() + 300;
          await restoreFromCompact(window, savedWindowSize);
          setSavedWindowSize(null);
          localStorage.removeItem(COMPACT_SAVED_SIZE_KEY);
        } catch (error) {
          Logger.error('Failed to restore window size:', error);
        }
      }
    };

    // Only run once on mount, with a small delay to ensure app is ready
    const timeout = setTimeout(restoreSavedWindowSize, 500);
    return () => clearTimeout(timeout);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // Run only on mount

  // Keep refs in sync with current values for use in resize listener
  useEffect(() => {
    aspectRatioLockEnabledRef.current = settings.video_player?.lock_aspect_ratio ?? true;
  }, [settings.video_player?.lock_aspect_ratio]);

  useEffect(() => {
    chatSizeRef.current = chatSize;
    prevChatSizeRef.current = chatSize;
  }, [chatSize]);

  useEffect(() => {
    chatPlacementRef.current = chatPlacement;
  }, [chatPlacement]);

  useEffect(() => {
    isTheaterModeRef.current = isTheaterMode;
  }, [isTheaterMode]);

  useEffect(() => {
    streamUrlRef.current = streamUrl;
  }, [streamUrl]);

  useEffect(() => {
    isMultiNookActiveRef.current = isMultiNookActive;
  }, [isMultiNookActive]);

  useEffect(() => {
    multiNookSlotsLengthRef.current = visibleSlotsLength;
  }, [visibleSlotsLength]);

  useEffect(() => {
    activeChatChannelInPopoutRef.current = activeChatChannelInPopout;
  }, [activeChatChannelInPopout]);

  // Track watch time and streams watched in Supabase.
  //
  // Keyed on the *stream identity*, NOT streamUrl. The native resolver
  // reassigns streamUrl mid-stream on every ad pivot and quality change
  // (applyAdPivot / changeStreamQuality). Keying the tracker on streamUrl made
  // each of those tear down and restart the minute interval (silently dropping
  // watch minutes) and re-fire streams_watched (counting one stream many
  // times). currentStream's channel id stays put across those events, so the
  // timer runs uninterrupted and a stream is counted exactly once per session.
  const streamSessionKey = currentStream
    ? (currentStream.user_id || currentStream.user_login || null)
    : null;
  useEffect(() => {
    if (!streamSessionKey || !isSupabaseConfigured()) return;

    const { currentUser, isAuthenticated } = useAppStore.getState();
    if (!isAuthenticated || !currentUser?.user_id) return;

    // Count this stream once when the session begins (ad pivots and quality
    // changes leave currentStream untouched, so they don't recount).
    Logger.debug('[Stats] Stream session started, incrementing streams_watched');
    incrementStat(currentUser.user_id, 'streams_watched', 1);

    // Claim any active watch-to-earn event reward this stream qualifies for.
    // Check on stream start, then every minute below. TWITCH ONLY: the rewards
    // are keyed to Twitch channels and the category fallback is Helix, so on a
    // provider stream this only produced a failed lookup on every tick.
    const rewardsEligible =
      streamProvider(useAppStore.getState().currentStream) === 'twitch';
    if (rewardsEligible) void maybeClaimWatchRewards(
      currentUser.user_id,
      useAppStore.getState().currentStream?.user_login,
      useAppStore.getState().currentStream?.game_name,
    );

    // Accrue watch time every minute. Reads fresh state each tick so it follows
    // the live user/channel even as other store fields churn.
    const watchTimeInterval = setInterval(() => {
      const { isAuthenticated: stillAuth, currentUser: user, currentStream: cs } = useAppStore.getState();
      if (stillAuth && user?.user_id) {
        // Increment by 1/60 of an hour (1 minute)
        incrementStat(user.user_id, 'hours_watched', 1 / 60);
        // Per-channel watch minute for the favorite-channel stat.
        if (cs?.user_id) {
          incrementChannelWatch(user.user_id, {
            id: cs.user_id,
            login: cs.user_login,
            name: cs.user_name || cs.user_login,
          });
        }
        void maybeClaimWatchRewards(user.user_id, cs?.user_login, cs?.game_name);
      }
    }, 60000); // Every minute

    return () => {
      clearInterval(watchTimeInterval);
    };
  }, [streamSessionKey]);

  // A streamer can switch category mid-stream (EventSub channel.update refreshes
  // currentStream.game_name). The watch-time tracker above only re-checks once a
  // minute, so also attempt watch-reward claims the moment the category changes,
  // instead of making the viewer wait for the next tick.
  useEffect(() => {
    if (!isSupabaseConfigured() || !watchRewardChannel) return;
    const { currentUser, isAuthenticated } = useAppStore.getState();
    if (!isAuthenticated || !currentUser?.user_id) return;
    void maybeClaimWatchRewards(currentUser.user_id, watchRewardChannel, watchRewardGame);
  }, [watchRewardChannel, watchRewardGame]);


  // Handle aspect ratio locking when setting changes or chat is resized
  useEffect(() => {
    const adjustWindowForAspectRatio = async () => {
      // Use refs for values that might be stale in closures
      const lockEnabled = aspectRatioLockEnabledRef.current;
      // When the current channel's chat is owned by a MultiChat popout, the
      // in-app chat panel JSX is gone — treat chat as effectively hidden so
      // the resize calculation shrinks the window and the player fills it
      // cleanly at 16:9 instead of growing wider than 16:9 with side bars.
      const chatHiddenByPopout = activeChatChannelInPopoutRef.current;
      const currentChatSize = chatHiddenByPopout ? 0 : chatSizeRef.current;
      const currentChatPlacement = chatHiddenByPopout ? 'hidden' : chatPlacementRef.current;
      const theaterMode = isTheaterModeRef.current;
      const currentStreamUrl = streamUrlRef.current;
      const currentIsMultiNookActive = isMultiNookActiveRef.current;
      const multiNookCount = multiNookSlotsLengthRef.current;

      // Don't adjust if in theater mode - theater mode handles its own sizing
      if (theaterMode || !lockEnabled || (!currentStreamUrl && !currentIsMultiNookActive)) return;

      // Prevent re-entrant calls, and stand down while a placement change is
      // mid-resize (that handler preserves the video dimensions; running the
      // lock formula here against a half-applied window size shrinks it) or a
      // titlebar drag holds the OS modal move loop (a setSize inside it
      // corrupts the loop's cached rect).
      if (isAdjustingRef.current || placementResizeInProgressRef.current || isTitlebarDragActive()) return;
      isAdjustingRef.current = true;

      try {
        const window = getCurrentWindow();

        // Don't adjust if window is maximized
        const isMaximized = await window.isMaximized();
        if (isMaximized) {
          Logger.debug('Window is maximized, skipping aspect ratio adjustment');
          isAdjustingRef.current = false;
          return;
        }

        // Same for OS fullscreen — resizing there fights the fullscreen rect.
        const isFullscreen = await window.isFullscreen();
        if (isFullscreen) {
          Logger.debug('Window is fullscreen, skipping aspect ratio adjustment');
          isAdjustingRef.current = false;
          return;
        }

        // Logical pixels: innerSize() is physical, and every offset below is a CSS
        // pixel. Mixing the two both skews the formula and, once written back as a
        // LogicalSize, multiplies the window by the scale factor on every pass.
        const { width, height, scale } = await getLogicalInnerSize(window);

        Logger.debug('[AspectRatio] Current window size:', width, height);
        Logger.debug('[AspectRatio] Chat size:', currentChatSize);
        Logger.debug('[AspectRatio] Chat placement:', currentChatPlacement);

        // Title bar height is 40px
        const titleBarHeight = 40;

        let targetAspectRatio = 16.0 / 9.0;
        
        // Dynamically measure sidebar
        let uiWidthOffset = 64;
        const sidebarEl = document.querySelector('.border-r.border-borderSubtle.flex-shrink-0');
        if (sidebarEl) {
          uiWidthOffset = sidebarEl.getBoundingClientRect().width;
        }
        let uiHeightOffset = 0;

        // Account for the chat resize separator
        if (currentChatPlacement === 'right') uiWidthOffset += 4;
        if (currentChatPlacement === 'bottom') uiHeightOffset += 4;

        if (currentIsMultiNookActive) {
          const len = multiNookCount;
          uiWidthOffset += 16; // 8px padding on L/R
          uiHeightOffset += 16; // 8px padding on T/B

          if (len === 2) { targetAspectRatio = 16.0 / 18.0; uiHeightOffset += 8; }
          else if (len >= 3 && len <= 4) { targetAspectRatio = 16.0 / 9.0; uiWidthOffset += 8; uiHeightOffset += 8; }
          else if (len >= 5 && len <= 6) { targetAspectRatio = 48.0 / 18.0; uiWidthOffset += 16; uiHeightOffset += 8; }
          else if (len >= 7 && len <= 9) { targetAspectRatio = 16.0 / 9.0; uiWidthOffset += 16; uiHeightOffset += 16; }
          else if (len >= 10 && len <= 12) { targetAspectRatio = 64.0 / 27.0; uiWidthOffset += 24; uiHeightOffset += 16; }
          else if (len >= 13 && len <= 16) { targetAspectRatio = 16.0 / 9.0; uiWidthOffset += 24; uiHeightOffset += 24; }
          else if (len >= 17 && len <= 20) { targetAspectRatio = 80.0 / 36.0; uiWidthOffset += 32; uiHeightOffset += 24; }
          else if (len > 20) { targetAspectRatio = 80.0 / 36.0; uiWidthOffset += 32; uiHeightOffset += 32; }
        }

        const [newWidth, newHeight] = await invoke<[number, number]>('calculate_aspect_ratio_size', {
          currentWidth: width,
          currentHeight: height,
          chatSize: currentChatSize,
          chatPlacement: currentChatPlacement,
          titleBarHeight: titleBarHeight,
          targetAspectRatio: targetAspectRatio,
          uiWidthOffset: uiWidthOffset,
          uiHeightOffset: uiHeightOffset,
        });

        Logger.debug('[AspectRatio] Calculated new size:', newWidth, newHeight);

        // Only resize if dimensions changed significantly (more than 5px difference)
        const clamped = await clampToWorkArea(newWidth, newHeight, scale);
        if (Math.abs(width - clamped.width) > 5 || Math.abs(height - clamped.height) > 5) {
          Logger.debug('[AspectRatio] Resizing window to:', clamped.width, clamped.height);
          selfResizeUntilRef.current = Date.now() + 300;
          await window.setSize(new LogicalSize(clamped.width, clamped.height));
        } else {
          Logger.debug('[AspectRatio] Size difference too small, not resizing');
        }
      } catch (error) {
        Logger.error('Failed to adjust window for aspect ratio:', error);
      } finally {
        isAdjustingRef.current = false;
      }
    };

    // Initial adjustment when settings change
    adjustWindowForAspectRatio();
    // Placement changes are handled by the dedicated placement effect above
    // (which preserves video dimensions). This effect only reacts to chat-drag
    // resizes and lock/stream/layout changes, so chatPlacement is not a dep —
    // adding it here made this effect fire its shrinking formula on every
    // placement toggle and fight the placement handler.
  }, [settings.video_player?.lock_aspect_ratio, chatSize, streamUrl, isTheaterMode, isMultiNookActive, visibleSlotsLength, activeChatChannelInPopout]);

  // Separate effect for the resize listener - only set up once and use refs
  useEffect(() => {
    let debounceTimeout: NodeJS.Timeout | null = null;
    let isMounted = true;
    let unlistenFn: (() => void) | null = null;

    const adjustWindowForAspectRatio = async () => {
      // Use refs for current values
      const lockEnabled = aspectRatioLockEnabledRef.current;
      const chatHiddenByPopout = activeChatChannelInPopoutRef.current;
      const currentChatSize = chatHiddenByPopout ? 0 : chatSizeRef.current;
      const currentChatPlacement = chatHiddenByPopout ? 'hidden' : chatPlacementRef.current;
      const theaterMode = isTheaterModeRef.current;
      const currentStreamUrl = streamUrlRef.current;
      const currentIsMultiNookActive = isMultiNookActiveRef.current;
      const multiNookCount = multiNookSlotsLengthRef.current;

      if (theaterMode || !lockEnabled || (!currentStreamUrl && !currentIsMultiNookActive)) return;
      if (isAdjustingRef.current || placementResizeInProgressRef.current || isTitlebarDragActive()) return;
      isAdjustingRef.current = true;

      try {
        const window = getCurrentWindow();

        const isMaximized = await window.isMaximized();
        if (isMaximized) {
          isAdjustingRef.current = false;
          return;
        }

        const isFullscreen = await window.isFullscreen();
        if (isFullscreen) {
          isAdjustingRef.current = false;
          return;
        }

        // Logical pixels — see the settle effect above for why this conversion is
        // what stops the window from growing on every resize event.
        const { width, height, scale } = await getLogicalInnerSize(window);

        const titleBarHeight = 40;

        let targetAspectRatio = 16.0 / 9.0;
        // Dynamically measure sidebar
        let uiWidthOffset = 64;
        const sidebarEl = document.querySelector('.border-r.border-borderSubtle.flex-shrink-0');
        if (sidebarEl) {
          uiWidthOffset = sidebarEl.getBoundingClientRect().width;
        }
        let uiHeightOffset = 0;

        // Account for the chat resize separator
        if (currentChatPlacement === 'right') uiWidthOffset += 4;
        if (currentChatPlacement === 'bottom') uiHeightOffset += 4;

        if (currentIsMultiNookActive) {
          const len = multiNookCount;
          uiWidthOffset += 16; // 8px padding on L/R
          uiHeightOffset += 16; // 8px padding on T/B

          if (len === 2) { targetAspectRatio = 16.0 / 18.0; uiHeightOffset += 8; }
          else if (len >= 3 && len <= 4) { targetAspectRatio = 16.0 / 9.0; uiWidthOffset += 8; uiHeightOffset += 8; }
          else if (len >= 5 && len <= 6) { targetAspectRatio = 48.0 / 18.0; uiWidthOffset += 16; uiHeightOffset += 8; }
          else if (len >= 7 && len <= 9) { targetAspectRatio = 16.0 / 9.0; uiWidthOffset += 16; uiHeightOffset += 16; }
          else if (len >= 10 && len <= 12) { targetAspectRatio = 64.0 / 27.0; uiWidthOffset += 24; uiHeightOffset += 16; }
          else if (len >= 13 && len <= 16) { targetAspectRatio = 16.0 / 9.0; uiWidthOffset += 24; uiHeightOffset += 24; }
          else if (len >= 17 && len <= 20) { targetAspectRatio = 80.0 / 36.0; uiWidthOffset += 32; uiHeightOffset += 24; }
          else if (len > 20) { targetAspectRatio = 80.0 / 36.0; uiWidthOffset += 32; uiHeightOffset += 32; }
        }

        const [newWidth, newHeight] = await invoke<[number, number]>('calculate_aspect_ratio_size', {
          currentWidth: width,
          currentHeight: height,
          chatSize: currentChatSize,
          chatPlacement: currentChatPlacement,
          titleBarHeight: titleBarHeight,
          targetAspectRatio: targetAspectRatio,
          uiWidthOffset: uiWidthOffset,
          uiHeightOffset: uiHeightOffset,
        });

        const clamped = await clampToWorkArea(newWidth, newHeight, scale);
        if (Math.abs(width - clamped.width) > 5 || Math.abs(height - clamped.height) > 5) {
          Logger.debug('[AspectRatio] Resize event - adjusting to:', clamped.width, clamped.height);
          selfResizeUntilRef.current = Date.now() + 300;
          await window.setSize(new LogicalSize(clamped.width, clamped.height));
        }
      } catch (error) {
        Logger.error('Failed to adjust window for aspect ratio:', error);
      } finally {
        isAdjustingRef.current = false;
      }
    };

    // Use Tauri API directly
    import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
      const window = getCurrentWindow();
      window.onResized(async () => {
        // Debounce resize events
        if (debounceTimeout) {
          clearTimeout(debounceTimeout);
        }
        debounceTimeout = setTimeout(async () => {
          // Never react to a resize we performed ourselves, nor to the
          // restore/move traffic of a titlebar drag still in the OS move loop.
          if (Date.now() < selfResizeUntilRef.current || isTitlebarDragActive()) return;
          // Check refs for current state
          if (aspectRatioLockEnabledRef.current && !isTheaterModeRef.current && (streamUrlRef.current || isMultiNookActiveRef.current)) {
            await adjustWindowForAspectRatio();
          }
        }, 100);
      }).then(unlisten => {
        if (isMounted) unlistenFn = unlisten;
        else unlisten();
      });
    });

    return () => {
      isMounted = false;
      if (unlistenFn) unlistenFn();
      if (debounceTimeout) {
        clearTimeout(debounceTimeout);
      }
    };
  }, []); // Empty deps - set up once and use refs for current values

  // Check for bundle updates on startup
  useEffect(() => {
    const checkUpdates = async () => {
      try {
        const { invoke } = await import('@tauri-apps/api/core');

        interface BundleUpdateStatus {
          update_available: boolean;
          current_version: string;
          latest_version: string;
        }

        const bundleStatus = await invoke('check_for_bundle_update') as BundleUpdateStatus;

        const { setUpdateInfo } = useAppStore.getState();
        setUpdateInfo(
          bundleStatus.update_available
            ? { current_version: bundleStatus.current_version, latest_version: bundleStatus.latest_version }
            : null
        );
      } catch (error) {
        Logger.error('Failed to check for bundle updates:', error);
      }
    };
    // Update checking can wait out the boot stagger; nothing on screen needs it.
    return afterBoot(5000, () => {
      void checkUpdates();
    });
  }, []);

  // Warm the lazy VideoPlayer chunk once boot has settled, so the first stream
  // start doesn't pay the chunk-load latency on top of stream resolution.
  useEffect(() => afterBoot(5000, () => {
    void import('./components/VideoPlayer');
  }), []);

  const handleMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    setIsResizing(true);
  };

  const handleModLogsMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    setIsResizingModLogs(true);
  };

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      // Chat Resizing
      if (isResizing && containerRef.current) {
        const container = containerRef.current;
        const containerRect = container.getBoundingClientRect();

        if (chatPlacement === 'right' || chatPlacement === 'left') {
          // Left dock grows rightward from the left edge; right dock grows
          // leftward from the right edge.
          const newWidth = chatPlacement === 'left'
            ? e.clientX - containerRect.left
            : containerRect.right - e.clientX;
          const maxWidth = containerRect.width - 200;
          const clampedWidth = Math.max(250, Math.min(maxWidth, newWidth));
          setChatSize(clampedWidth);
        } else if (chatPlacement === 'bottom') {
          const newHeight = containerRect.bottom - e.clientY;
          const maxHeight = containerRect.height - 150;
          const clampedHeight = Math.max(150, Math.min(maxHeight, newHeight));
          setChatSize(clampedHeight);
        }
      } 
      // Mod Logs Resizing
      else if (isResizingModLogs) {
        // Mod Logs are always on the opposite side of chat.
        // If chat is Right, Mod Logs is bottom.
        // If chat is Bottom, Mod Logs is right.
        if (chatPlacement === 'right' || chatPlacement === 'left') {
          // Mod logs is at the bottom
          const newHeight = window.innerHeight - e.clientY;
          const clampedHeight = Math.max(150, Math.min(window.innerHeight - 200, newHeight));
          setModLogsSize(clampedHeight);
        } else if (chatPlacement === 'bottom') {
          // Mod logs is at the right
          const newWidth = window.innerWidth - e.clientX;
          const clampedWidth = Math.max(200, Math.min(800, newWidth));
          setModLogsSize(clampedWidth);
        }
      }
    };

    const handleMouseUp = () => {
      setIsResizing(false);
      setIsResizingModLogs(false);
    };

    if (isResizing || isResizingModLogs) {
      window.addEventListener('mousemove', handleMouseMove);
      window.addEventListener('mouseup', handleMouseUp);
      document.body.style.userSelect = 'none';
      
      if (isResizing) {
        document.body.style.cursor = (chatPlacement === 'right' || chatPlacement === 'left') ? 'ew-resize' : 'ns-resize';
      } else {
        document.body.style.cursor = chatPlacement === 'bottom' ? 'ew-resize' : 'ns-resize';
      }
    }

    return () => {
      window.removeEventListener('mousemove', handleMouseMove);
      window.removeEventListener('mouseup', handleMouseUp);
      document.body.style.userSelect = '';
      document.body.style.cursor = '';
    };
  }, [isResizing, isResizingModLogs, chatPlacement]);

  // The chat pane, rendered identically at both docked layouts below. A
  // non-Twitch stream routes through MainProviderChat, which feeds ChatWidget
  // the same `channelOverride` seam MultiChat panes already use.
  //
  // MultiNook is the second caller: its active chat can be ANY tile, so the
  // provider comes from that slot rather than from `currentStream` (which still
  // describes whatever the solo player last had). Routing here, rather than
  // teaching ChatWidget about providers, is deliberate: a channelOverride inside
  // ChatWidget's MultiNook branch would also flip its hype-train source, its
  // VOD-replay gate and its popout header bits, none of which a grid tile wants.
  const activeTile = isMultiNookActive
    ? activeChatSlot(multiNookSlots, activeChatChannelId)
    : null;
  const activeTileProvider = activeTile?.provider ?? 'twitch';
  const chatPane =
    activeTile && activeTileProvider !== 'twitch' ? (
      <MainProviderChat
        provider={activeTileProvider}
        channel={activeTile.channelLogin}
        details={{
          user_id: activeTile.channelId,
          user_name: activeTile.channelName,
          title: activeTile.title,
          profile_image_url: activeTile.profileImageUrl,
          game_name: activeTile.gameName,
        }}
      />
    ) : currentStream && streamProvider(currentStream) !== 'twitch' ? (
      <MainProviderChat
        provider={streamProvider(currentStream)}
        channel={currentStream.user_login}
      />
    ) : (
      <ChatWidget />
    );

  // sn-app-shell: the hook a theme's light treatment paints onto (see
  // styles/theme-prism.css); the room every glass surface sits in.
  return (
    <div className="sn-app-shell flex flex-col h-screen bg-background">
      <ErrorBoundary
        componentName="TitleBar"
        fallback={
          <div className="h-[40px] bg-secondary backdrop-blur-md border-b border-borderSubtle flex items-center justify-center">
            <span className="text-textSecondary text-xs">Title bar error - restart app</span>
          </div>
        }
      >
        {/* The title bar is desktop window chrome: minimise/maximise/close, the
            drag region and the update pill. None of it means anything on a phone,
            and it costs ~40px of a screen that has little to spare. */}
        {!IS_MOBILE && <TitleBar />}
      </ErrorBoundary>
      {/* Dynamic Island lives at the app root (not inside the title bar) so it can
          lift above the Settings blur overlay; it pins itself to the top center. */}
      <ErrorBoundary componentName="DynamicIsland" fallback={null}>
        <DynamicIsland />
      </ErrorBoundary>
      <ErrorBoundary
        componentName="App"
        reportToLogService
        fallbackRender={({ reset }) => (
          <div className="flex flex-1 items-center justify-center bg-background">
            <div className="glass-panel text-center px-6 py-5 rounded-xl max-w-sm">
              <p className="text-textPrimary text-sm font-medium mb-1">Something went wrong</p>
              <p className="text-textSecondary text-xs mb-4">
                The main view ran into an unexpected error. You can recover without losing your session.
              </p>
              <div className="flex items-center justify-center gap-2">
                <button
                  onClick={reset}
                  className="glass-button text-white text-xs font-medium px-3 py-1.5 rounded-lg border border-white/10 hover:bg-white/10 transition-colors"
                >
                  Try again
                </button>
                <button
                  onClick={() => window.location.reload()}
                  className="text-textSecondary hover:text-white text-xs font-medium px-3 py-1.5 rounded-lg transition-colors"
                >
                  Reload app
                </button>
              </div>
            </div>
          </div>
        )}
      >
      <div className="flex flex-1 overflow-hidden">
        {/* Sidebar - only visible when stream is playing. Flips to the right edge
            when chat is docked left with reveal-on-hover, so the left edge belongs
            to the chat hover and the two don't fight over the same zone. */}
        {/* The sidebar is built around edge-hover reveal and drag-to-resize, both
            of which are mouse-only. Mobile navigation is a bottom tab bar instead
            (see MobileNav), driving the same store fields. */}
        {!IS_MOBILE && (
          <ErrorBoundary componentName="Sidebar">
            <Sidebar side={chatPlacement === 'left' && autoHideActive ? 'right' : 'left'} />
          </ErrorBoundary>
        )}

        {/* Main content area with Home/PIP support */}
        <div className="flex-1 relative overflow-hidden">
          {/* Home View - shown when isHomeActive or no stream */}
          <AnimatePresence>
            {(isHomeActive || (!streamUrl && !isLoading && !isMultiNookActive)) && (
              <motion.div
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                transition={{ duration: 0.2, ease: "easeInOut" }}
                className={`absolute inset-0 z-40 ${streamUrl || isMultiNookActive ? 'bg-background/85 sn-glass-veil' : 'bg-background'}`}
              >
                <ErrorBoundary componentName="Home" reportToLogService resetKeys={[isHomeActive]}>
                  <Home />
                </ErrorBoundary>
              </motion.div>
            )}
          </AnimatePresence>

          {/* Loading state when starting stream */}
          <AnimatePresence>
            {isLoading && !streamUrl && !isMultiNookActive && (
              <motion.div
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                transition={{ duration: 0.2 }}
                className="absolute inset-0 z-50 bg-black"
              >
                <LoadingWidget useFunnyMessages={true} />
              </motion.div>
            )}
          </AnimatePresence>

          {/* Stream/Chat View - kept mounted to preserve session */}
          <AnimatePresence>
            {(streamUrl || isMultiNookActive) && (
              <motion.div
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                transition={{ duration: 0.2, ease: "easeInOut" }}
                className={`flex flex-1 h-full overflow-hidden ${
                  settings.show_mod_logs && chatPlacement !== 'hidden'
                    ? (isSideChat ? 'flex-col' : 'flex-row')
                    : (chatPlacement === 'bottom' ? 'flex-col' : 'flex-row')
                } ${isHomeActive ? 'pointer-events-none' : ''}`}
              >
                
                {/* Video & Chat Container */}
                <div 
                  ref={containerRef}
                  // Marker for the mobile layout layer: on a phone in portrait
                  // the first child (the player) is pinned to a 16:9 band and the
                  // chat panel takes the rest. See src/styles/mobile.css.
                  data-sn-stream-container=""
                  className={`flex flex-1 h-full overflow-hidden relative ${chatPlacement === 'bottom' ? 'flex-col' : 'flex-row'}`}
                >
                  <ChannelAboutReveal
                    enabled={!isMultiNookActive && (currentMediaType === 'live' || currentMediaType === 'video' || currentMediaType === 'offline_chat') && !!currentStream?.user_login}
                    channelLogin={currentStream?.user_login}
                  >
                    <AnimatePresence mode="wait">
                      {isMultiNookActive ? (
                        <motion.div 
                          key="multinook"
                          initial={{ opacity: 0 }}
                          animate={{ opacity: 1 }}
                          exit={{ opacity: 0 }}
                          transition={{ duration: 0.15 }}
                          className="w-full h-full absolute inset-0"
                        >
                          <ErrorBoundary componentName="MultiNook" reportToLogService>
                            <Suspense fallback={null}>
                              <MultiNookView />
                            </Suspense>
                          </ErrorBoundary>
                        </motion.div>
                      ) : (
                        <motion.div 
                          key="videoplayer"
                          initial={{ opacity: 0 }}
                          animate={{ opacity: 1 }}
                          exit={{ opacity: 0 }}
                          transition={{ duration: 0.15 }}
                          className="w-full h-full absolute inset-0"
                        >
                          <ErrorBoundary componentName="Video" reportToLogService resetKeys={[streamUrl]}>
                            <Suspense fallback={null}>
                              <VideoPlayer key={streamUrl} />
                            </Suspense>
                          </ErrorBoundary>
                        </motion.div>
                      )}
                    </AnimatePresence>
                    <AnimatePresence>
                      {isLoading && (
                        <motion.div
                          initial={{ opacity: 0 }}
                          animate={{ opacity: 1 }}
                          exit={{ opacity: 0 }}
                          transition={{ duration: 0.2 }}
                          className="absolute inset-0 z-20"
                        >
                          <LoadingWidget useFunnyMessages={true} />
                        </motion.div>
                      )}
                    </AnimatePresence>
                  </ChannelAboutReveal>
                  {/* Chat — gated on whether this channel is currently owned
                      by a StreamNook MultiChat popout. When it is, the popout
                      becomes the sole chat surface and we collapse the in-app
                      chat panel entirely (no duplicate chat across windows). */}
                  {/* 'video' and 'clip' carry chat REPLAY, so they get the panel too. They
                      were missing here while the resize gate directly above already listed
                      'video', which is why a VOD opened from the Watch VOD button (media
                      type 'offline_chat') showed chat but the exact same VOD opened from a
                      profile's Videos tab (media type 'video') did not. Both replay types
                      are guarded on user_login, which playMedia only sets when there is a
                      channel to replay — so a clip with no surviving source VOD still gets
                      no empty panel. */}
                  {chatPlacement !== 'hidden' && (currentMediaType === 'live' || currentMediaType === 'offline_chat' || ((currentMediaType === 'video' || currentMediaType === 'clip') && !!currentStream?.user_login) || isMultiNookActive) && !activeChatChannelInPopout && (
                    autoHideActive ? (
                      // Reveal-on-hover (side docks only): a thin edge handle is
                      // always visible; hovering the wrapper slides the chat out
                      // (width 0 -> chatSize) and the flex-1 video shrinks to make
                      // room in real time. For the LEFT dock, order-first + reverse
                      // put the wrapper on the left with the handle on the video side.
                      <div
                        className={`relative flex h-full flex-shrink-0 ${chatPlacement === 'left' ? 'order-first flex-row-reverse' : 'flex-row'}`}
                        onMouseEnter={revealChat}
                        onMouseLeave={hideChatSoon}
                      >
                        {/* Wider INVISIBLE hover-catch so the chat reveals before
                            you reach the very edge. Absolute (no reserved flex
                            space) and only while collapsed; it overlays the video
                            edge. Tune the width to taste. */}
                        {!chatRevealed && (
                          <div
                            aria-hidden="true"
                            onMouseEnter={revealChat}
                            className={`absolute top-0 bottom-0 z-20 ${chatPlacement === 'left' ? 'left-0' : 'right-0'}`}
                            style={{ width: 24 }}
                          />
                        )}
                        <div
                          aria-hidden="true"
                          className={`h-full w-1.5 flex-shrink-0 cursor-pointer transition-colors ${chatRevealed ? 'bg-borderLight' : 'bg-borderLight/50 hover:bg-accent/70'}`}
                        />
                        {/* "Hover here for chat" cue, pinned to the docked edge and
                            shown only while collapsed (mirrors the About reveal hint).
                            pointer-events-none so the wider catch zone behind it still
                            gets the hover. */}
                        {!chatRevealed && (
                          <div
                            aria-hidden="true"
                            className={`pointer-events-none absolute top-1/2 z-20 flex -translate-y-1/2 items-center justify-center rounded-full border border-white/10 bg-black/40 p-1 text-white/70 backdrop-blur-sm ${chatPlacement === 'left' ? 'left-1.5' : 'right-1.5'}`}
                          >
                            {chatPlacement === 'left' ? <ChevronRight className="h-4 w-4" /> : <ChevronLeft className="h-4 w-4" />}
                          </div>
                        )}
                        <motion.div
                          animate={{ width: chatRevealed ? chatSize : 0 }}
                          transition={{ type: 'spring', stiffness: 350, damping: 30 }}
                          className="h-full flex-shrink-0 overflow-hidden bg-background"
                        >
                          <div className="h-full flex flex-col" style={{ width: `${chatSize}px` }}>
                            {isMultiNookActive && <Suspense fallback={null}><MultiNookChatSwitcher /></Suspense>}
                            <div className="flex-1 overflow-hidden relative">
                              <ErrorBoundary componentName="Chat" reportToLogService resetKeys={[streamUrl, currentMediaType]}>
                                {chatPane}
                              </ErrorBoundary>
                            </div>
                          </div>
                        </motion.div>
                      </div>
                    ) : (
                    <motion.div
                      // The panel opens along ONE axis (width when docked left/right,
                      // height when docked bottom) and must fill the other. Drive
                      // BOTH axes: if the fill axis is left undefined, Framer keeps
                      // the inline size it wrote while the panel was on the other
                      // edge, so after a bottom→right switch the panel stays stuck
                      // at its old height and never refills the column. Pinning the
                      // fill axis to '100%' forces it back to the container size.
                      initial={{ opacity: 0, width: isSideChat ? 0 : '100%', height: chatPlacement === 'bottom' ? 0 : '100%' }}
                      animate={{
                        opacity: (isMultiNookActive && isChatHidden) ? 0 : 1,
                        width: isSideChat ? ((isMultiNookActive && isChatHidden) ? 0 : 'auto') : '100%',
                        height: chatPlacement === 'bottom' ? ((isMultiNookActive && isChatHidden) ? 0 : 'auto') : '100%'
                      }}
                      transition={{ type: "spring", stiffness: 350, damping: 25 }}
                      // Left dock reverses the children so the resize separator sits
                      // on the video-facing (right) side of the chat, and order-first
                      // moves the whole panel ahead of the video.
                      className={`flex flex-shrink-0 ${chatPlacement === 'bottom' ? 'flex-col' : chatPlacement === 'left' ? 'flex-row-reverse order-first' : 'flex-row'}`}
                      style={{ overflow: 'hidden' }}
                    >
                      <Tooltip content={isSideChat ? 'Drag to resize chat width' : 'Drag to resize chat height'} delay={100}>
                        {/* 4px invisible grab area keeps dragging easy; the
                            visible separator is a single 1px hairline. */}
                        <div
                          onMouseDown={handleMouseDown}
                          className={`
                            group flex items-center justify-center flex-shrink-0 z-10
                            ${isSideChat ? 'w-1 cursor-ew-resize' : 'h-1 cursor-ns-resize'}
                            ${fsOverlayActive ? 'hidden' : ''}
                          `}
                        >
                          <div
                            className={`
                              ${isSideChat ? 'w-px h-full' : 'h-px w-full'}
                              bg-borderLight group-hover:bg-accent transition-colors
                              ${isResizing ? 'bg-accent' : ''}
                            `}
                          />
                        </div>
                      </Tooltip>
                      <div
                        data-chat-panel="true"
                        data-fs-hidden={fsChatHidden ? 'true' : undefined}
                        onMouseEnter={fsOverlayActive ? () => setFsChatHeld(true) : undefined}
                        onMouseLeave={fsOverlayActive ? () => setFsChatHeld(false) : undefined}
                        onFocusCapture={fsOverlayActive ? () => setFsChatHeld(true) : undefined}
                        className={`flex-shrink-0 flex flex-col h-full overflow-hidden bg-background ${
                          fsOverlayActive ? `chat-fullscreen-overlay chat-fullscreen-overlay--${fsChatSide}` : ''
                        }`}
                        style={
                          fsOverlayActive
                            ? ({
                                '--fs-chat-width': `${Math.max(240, Math.min(640, fsChat?.width ?? 340))}px`,
                                '--fs-chat-opacity': `${Math.max(0, Math.min(100, fsChat?.opacity ?? 55))}%`,
                              } as React.CSSProperties)
                            : { [isSideChat ? 'width' : 'height']: `${chatSize}px` }
                        }
                      >
                        {isMultiNookActive && <Suspense fallback={null}><MultiNookChatSwitcher /></Suspense>}
                        <div className="flex-1 overflow-hidden relative">
                          <ErrorBoundary componentName="Chat" reportToLogService resetKeys={[streamUrl, currentMediaType]}>
                            {chatPane}
                          </ErrorBoundary>
                        </div>
                      </div>
                    </motion.div>
                    )
                  )}
                </div>

                {/* Mod Logs Panel */}
                <AnimatePresence>
                  {settings.show_mod_logs && (
                    <motion.div
                      initial={{ opacity: 0, [isSideChat ? 'height' : 'width']: 0 }}
                      animate={{ opacity: 1, [isSideChat ? 'height' : 'width']: modLogsSize + 4 }}
                      exit={{ opacity: 0, [isSideChat ? 'height' : 'width']: 0 }}
                      transition={isResizingModLogs ? { duration: 0 } : { type: 'spring', stiffness: 350, damping: 30 }}
                      className={`flex ${isSideChat ? 'flex-col' : 'flex-row'} flex-shrink-0 relative overflow-hidden`}
                    >
                      {/* Resizer */}
                      <Tooltip content={isSideChat ? 'Drag to resize mod logs height' : 'Drag to resize mod logs width'} delay={100}>
                        {/* 4px invisible grab area keeps dragging easy; the
                            visible separator is a single 1px hairline. */}
                        <div
                          onMouseDown={handleModLogsMouseDown}
                          className={`
                            group flex items-center justify-center flex-shrink-0 z-10
                            ${isSideChat ? 'h-1 cursor-ns-resize w-full' : 'w-1 cursor-ew-resize h-full'}
                          `}
                        >
                          <div
                            className={`
                              ${isSideChat ? 'h-px w-full' : 'w-px h-full'}
                              bg-borderLight group-hover:bg-accent transition-colors
                              ${isResizingModLogs ? 'bg-accent' : ''}
                            `}
                          />
                        </div>
                      </Tooltip>
                      <div
                        className="bg-background overflow-hidden relative"
                        style={{
                          [isSideChat ? 'height' : 'width']: `${modLogsSize}px`
                        }}
                      >
                         <ErrorBoundary componentName="Mod Logs" reportToLogService>
                           <ModLogsWidget onOpenSettings={() => openSettings('Moderation')} />
                         </ErrorBoundary>
                      </div>
                    </motion.div>
                  )}
                </AnimatePresence>

              </motion.div>
            )}
          </AnimatePresence>
        </div>
      </div>
      </ErrorBoundary>
      {/* Bottom tab bar replaces the desktop Sidebar on phones. Rendered as a
          sibling of the main content inside the h-screen column so it always
          holds the bottom edge, and hidden while booting so it does not appear
          over the splash. */}
      {IS_MOBILE && !isBooting && (
        <ErrorBoundary componentName="MobileNav">
          <MobileNav />
        </ErrorBoundary>
      )}
      {/* Boot overlay — sits above the home screen from launch until the initial
          auth check resolves, then fades out so home eases in. Without it the
          logged-out nav and empty state flash for a beat before stored
          credentials are verified. Starts below the 40px title bar so window
          controls stay live while booting. */}
      <AnimatePresence>
        {isBooting && (
          <motion.div
            initial={{ opacity: 1 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.45, ease: 'easeInOut' }}
            className="fixed inset-x-0 bottom-0 top-[40px] z-[55] flex items-center justify-center bg-background/90 sn-glass-veil"
          >
            <LoadingWidget fullScreen={false} message="Loading StreamNook" />
          </motion.div>
        )}
      </AnimatePresence>
      {settingsEverOpened && <Suspense fallback={null}><SettingsDialog /></Suspense>}
      {profileViewerEverOpened && <Suspense fallback={null}><PublicProfileOverlay /></Suspense>}
      {dropsEverOpened && <Suspense fallback={null}><DropsOverlay /></Suspense>}
      {marketplaceEverOpened && <Suspense fallback={null}><MarketplaceOverlay /></Suspense>}
      <DropProgressController />
      <ReminderEngine />
      {emoteSetsEverOpened && <Suspense fallback={null}><EmoteSetsOverlay /></Suspense>}
      <EmoteSpotlight />

      {profileModalUser && (
        <Suspense fallback={null}>
          <SearchProfileModal
            user={profileModalUser}
            onClose={() => setProfileModalUser(null)}
          />
        </Suspense>
      )}
      <AnimatePresence>
        {showBadgesOverlay && !selectedBadge && (
          <Suspense fallback={null}>
            <BadgesOverlay
              onClose={() => setShowBadgesOverlay(false)}
              onBadgeClick={(badge, setId) => setSelectedBadge({ badge, setId })}
              initialPaintId={badgesOverlayInitialPaintId}
              initialBadgeId={badgesOverlayInitialBadgeId}
              initialStreamNook={badgesOverlayInitialStreamNook}
              initialTarget={badgesOverlayInitialTarget}
            />
          </Suspense>
        )}
      </AnimatePresence>
      <AnimatePresence>
        {selectedBadge && (
        <Suspense fallback={null}>
          <BadgeDetailOverlay
            badge={selectedBadge.badge}
            setId={selectedBadge.setId}
            onClose={() => {
              setSelectedBadge(null);
              setShowBadgesOverlay(false);
            }}
            onBack={() => setSelectedBadge(null)}
          />
        </Suspense>
        )}
      </AnimatePresence>
      <AnimatePresence>
        {showChangelog && changelogVersion && (
          <Suspense fallback={null}>
            <ChangelogOverlay
              version={changelogVersion}
              onClose={handleChangelogClose}
            />
          </Suspense>
        )}
      </AnimatePresence>
      <WhispersWidget
        isOpen={showWhispersOverlay}
        onClose={() => setShowWhispersOverlay(false)}
      />
      <PluginRuntimeBridge />
      <PluginUiHost />
      <PluginUpdatesChecker />
      <PluginOverlayOutlet />
      {setupWizardEverOpened && (
        <Suspense fallback={null}>
          <SetupWizard
            isOpen={showSetupWizard}
            onClose={() => setShowSetupWizard(false)}
          />
        </Suspense>
      )}
      {settings.setup_complete && !showSetupWizard && <AnnouncementsBanner />}
      <ToastManager />
      <DeviceLoginOverlay />
      <EntitlementUnlockNote />
      <TooltipManager />
      {commandPaletteEverOpened && <Suspense fallback={null}><CommandPalette /></Suspense>}
      <InputContextMenuHost />
      <ModerationDragLayer />
      {clipModalEverOpened && <Suspense fallback={null}><ClipModal /></Suspense>}
      {clipEditorEverOpened && <Suspense fallback={null}><ClipEditor /></Suspense>}
      <TwitchOverlay />
    </div>
  );
}

export default App;


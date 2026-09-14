// MultiChatWindow — top-level shell for a StreamNook MultiChat popout.
//
// Owns the channel list for this window: tabs along the top, "+" to add a
// channel by login, "×" on each tab to drop it. All channels stay
// reference-counted-acquired on the chatConnectionStore so switching tabs is
// lossless and instant; only the active tab is visible, but every tab's
// MultiChatPane stays mounted in the background to preserve drafts, scroll
// position, reply targets, and per-channel emote/badge caches.
//
// URL contract (set by openMultiChatWindow):
//   #/multichat?id=<windowId>&channel=<login>&channelId=<roomId>
//
// `id` keys the localStorage entry so the window restores its tab set across
// re-launches. `channel`+`channelId` seed the window when first opened from a
// "Pop out chat" action; once persisted state exists it takes precedence.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { helixGet } from '../../services/helix';
import { Minus, X, CornersOut, CornersIn, ArrowLineLeft } from 'phosphor-react';
import { Activity, Settings, ShieldCheck, House } from 'lucide-react';
import MultiChatPane from './MultiChatPane';
import { ModLogsWidget } from '../chat/ModLogsWidget';
import ModerationDragLayer from '../chat/ModerationDragLayer';
import { ActivityFeedWidget } from '../activity/ActivityFeedWidget';
import { startActivityNormalizer, stopActivityNormalizer } from '../../services/activityNormalizer';
import { useActivityStore } from '../../stores/activityStore';
import { makeKey, parseKey } from '../../utils/providerKey';
import { CHAT_PROVIDERS, PROVIDERS, type ProviderId } from '../../types/providers';
import { ProviderLogo } from '../ProviderLogo';
import { BlendedChatPane } from './BlendedChatPane';
import ModRoomPane from '../modroom/ModRoomPane';
import { useChannelEmotes } from '../../stores/chatConnectionStore';
import { isCachedModerator, loadModeratedChannelIds, subscribeModeratedChannels } from '../../services/modRoomService';
import type { ModRoomMember } from '../../services/modRoomService';
import { ensureRoom, releaseRoom } from '../../services/modRoomManager';
import { useModRoomStore } from '../../stores/modRoomStore';
import { getCachedAvatar, resolveAvatar } from '../../utils/avatarCache';
import ErrorBoundary from '../ErrorBoundary';
import ChatOnlySettingsModal from './ChatOnlySettingsModal';
import MultiChatToasts from './MultiChatToasts';
import ViewerCounter from './ViewerCounter';
import CommandPalette from '../CommandPalette';
import InputContextMenuHost from '../InputContextMenuHost';
import ClipModal from '../ClipModal';
import VodModal from './VodModal';
import { useCommandPaletteHotkey } from '../../hooks/useCommandPaletteHotkey';
import { usePlatformAccountSync } from '../../hooks/usePlatformAccountSync';
import { useKeybindings } from '../../keybindings';
import { startSnippetSync } from '../../stores/snippetStore';
import PluginUiHost from '../../plugins-ui/PluginUiHost';
import { TooltipManager } from '../ui/TooltipManager';
import {
  acquireChannel,
  releaseChannel,
  useChannelMentionCount,
} from '../../stores/chatConnectionStore';
import { useAppStore } from '../../stores/AppStore';
import {
  applyTheme,
  applyGlassStrength,
  applyFont,
  getThemeById,
  getThemeByIdWithCustom,
  getOledTheme,
  DEFAULT_THEME_ID,
  DEFAULT_GLASS_TRANSPARENCY,
  DEFAULT_FONT_ID,
  OLED_THEME_ID,
} from '../../themes';
import { listenForSettingsUpdates } from '../../utils/settingsBroadcast';
import { MULTICHAT_BASE_WIDTH, MULTICHAT_GEOMETRY_KEY, openMultiChatWindow } from '../../utils/multichatWindow';
import { Tooltip } from '../ui/Tooltip';
import { Logger } from '../../utils/logger';
import type { TwitchStream } from '../../types';
import streamNookLogoUrl from '../../assets/streamnook-logo-128.webp';
import SplitLayout from './SplitLayout';
import { registerPaneFocusController } from '../../keybindings/paneFocusController';
import {
  columnCount,
  findLeaf,
  fromColumns,
  isSplitNode,
  leafCount,
  leaves,
  removeKey,
  setLeafKey,
  showsKey,
  type SplitNode,
} from '../../utils/splitTree';

interface ChannelEntry {
  channel: string;
  /** Source platform. Absent means twitch (back-compat). MultiChat only. */
  provider?: ProviderId;
  channelId: string | null;
  /** Twitch display name (capitalization preserved as the user chose). Used
   *  for tab labels and the popout title bar. Falls back to `channel` if a
   *  lookup hasn't resolved yet — corrected on next refresh. */
  channelName: string;
  /** Saved message filter (settings.chat_query.filters id) bound to this
   *  tab's pane, persisted with the window state. */
  filterId?: string | null;
}

/** Composite identity for a tab. Provider-namespaced (`twitch:foo` / `kick:foo`)
 *  so the SAME channel name on two providers — e.g. Twitch + Kick both
 *  "TheBurntPeanut" — are two distinct tabs. The bare `channel` is NOT a unique
 *  key once mixed-provider sources are allowed; everything that identifies a tab
 *  (active selection, removal, reorder, visibility, React keys) keys on this. */
function entryKey(e: ChannelEntry): string {
  return makeKey(e.provider ?? 'twitch', e.channel);
}

/** Number of MultiChatPanes rendered simultaneously. `1` = tabs (single pane,
 *  active tab visible). 2/3/4 = side-by-side columns showing the first N
 *  channels in the list. Channels beyond N stay JOINed but aren't rendered. */
type LayoutMode = 1 | 2 | 3 | 4;

// Persisted "Go Live" profile (the streamer's own sources + activity filters +
// layout). HYPHEN/`sn-` prefix so the orphan-storage sweep in openMultiChatWindow
// (which targets `streamnook.multichat.` dot keys) leaves it alone.
const GOLIVE_KEY = 'sn-multichat-golive';

function readGoLiveSources(): ChannelEntry[] {
  try {
    const raw = localStorage.getItem(GOLIVE_KEY);
    if (raw) {
      const p = JSON.parse(raw);
      if (Array.isArray(p?.sources)) return p.sources as ChannelEntry[];
    }
  } catch {
    /* ignore */
  }
  return [];
}

function writeGoLiveSources(sources: ChannelEntry[]): void {
  try {
    const raw = localStorage.getItem(GOLIVE_KEY);
    const p = raw ? JSON.parse(raw) : {};
    p.sources = sources;
    localStorage.setItem(GOLIVE_KEY, JSON.stringify(p));
  } catch {
    /* ignore */
  }
}

interface PersistedWindowState {
  channels: ChannelEntry[];
  layoutMode: LayoutMode;
  /** Whether this popout's mod-logs pane is shown. Window-local (not the global
   *  `settings.show_mod_logs`) so toggling it here never touches the main app. */
  showModLogs?: boolean;
  /** Height in px of the mod-logs pane stacked below the chat. */
  modLogsHeight?: number;
  /** Height in px of the activity-feed pane stacked below the chat. */
  activityHeight?: number;
  /** Whether the activity-feed pane is shown. */
  showActivityFeed?: boolean;
  /** Whether the merged "blended" chat view is active. */
  isBlendedMode?: boolean;
  /** Whether the mentions feed (every open source, mentions/replies/highlights
   *  only, as stamped by the Rust rule engine) is active. */
  isMentionsMode?: boolean;
  /** Custom split layout (utils/splitTree). Null means the column presets. */
  layoutTree?: SplitNode | null;
  /** Which pane owns the keyboard (mod keys, tab clicks land here). */
  focusedLeafId?: string | null;
}

const DEFAULT_MOD_LOGS_HEIGHT = 240;
const MIN_MOD_LOGS_HEIGHT = 140;
const DEFAULT_ACTIVITY_HEIGHT = 220;

/** Keep the mod-logs pane tall enough to read but always leave room for the
 *  title bar, tab strip, and a usable slice of chat above it. */
function clampModLogsHeight(height: number): number {
  const viewportCap =
    typeof window !== 'undefined' ? Math.max(MIN_MOD_LOGS_HEIGHT, window.innerHeight - 200) : 600;
  return Math.min(Math.max(MIN_MOD_LOGS_HEIGHT, height), viewportCap);
}

interface ParsedMultiChatParams {
  id: string | null;
  channel: string | null;
  channelId: string | null;
  channelName: string | null;
  /** Multi-channel seed (e.g. popping out all MultiNook tiles at once). */
  channels: ChannelEntry[] | null;
  /** Start fresh with only the seeded channels, dropping any persisted tabs. */
  replace: boolean;
}

function parseMultiChatParams(): ParsedMultiChatParams {
  const hash = window.location.hash;
  const queryIdx = hash.indexOf('?');
  if (queryIdx === -1)
    return { id: null, channel: null, channelId: null, channelName: null, channels: null, replace: false };
  const params = new URLSearchParams(hash.slice(queryIdx + 1));

  let channels: ChannelEntry[] | null = null;
  const channelsRaw = params.get('channels');
  if (channelsRaw) {
    try {
      const arr = JSON.parse(channelsRaw) as unknown;
      if (Array.isArray(arr)) {
        channels = arr
          .filter(
            (c): c is { channel: string; channelId?: string | null; channelName?: string | null } =>
              !!c && typeof c === 'object' && typeof (c as { channel?: unknown }).channel === 'string',
          )
          .map((c) => ({
            channel: c.channel.toLowerCase(),
            channelId: c.channelId ?? null,
            channelName: c.channelName || c.channel,
          }));
      }
    } catch {
      // ignore malformed channels param
    }
  }

  return {
    id: params.get('id'),
    channel: params.get('channel'),
    channelId: params.get('channelId'),
    channelName: params.get('channelName'),
    channels,
    replace: params.get('replace') === '1',
  };
}

const STORAGE_PREFIX = 'streamnook.multichat.';

function storageKey(windowId: string | null): string | null {
  if (!windowId) return null;
  return `${STORAGE_PREFIX}${windowId}`;
}

function loadPersistedState(windowId: string | null): PersistedWindowState | null {
  const key = storageKey(windowId);
  if (!key) return null;
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as unknown;
    // Old format: just an array of channels. Newer format: { channels, layoutMode }.
    if (Array.isArray(parsed)) {
      return {
        channels: parsed.filter(
          (entry): entry is ChannelEntry =>
            !!entry &&
            typeof entry === 'object' &&
            typeof (entry as ChannelEntry).channel === 'string',
        ),
        layoutMode: 1,
      };
    }
    if (parsed && typeof parsed === 'object' && Array.isArray((parsed as any).channels)) {
      const mode = (parsed as any).layoutMode;
      const layoutMode: LayoutMode =
        mode === 2 || mode === 3 || mode === 4 ? mode : 1;
      const rawShow = (parsed as any).showModLogs;
      const rawHeight = (parsed as any).modLogsHeight;
      const rawActivityHeight = (parsed as any).activityHeight;
      return {
        channels: (parsed as any).channels.filter(
          (entry: unknown): entry is ChannelEntry =>
            !!entry &&
            typeof entry === 'object' &&
            typeof (entry as ChannelEntry).channel === 'string',
        ),
        layoutMode,
        showModLogs: typeof rawShow === 'boolean' ? rawShow : undefined,
        modLogsHeight:
          typeof rawHeight === 'number' && Number.isFinite(rawHeight) ? rawHeight : undefined,
        activityHeight:
          typeof rawActivityHeight === 'number' && Number.isFinite(rawActivityHeight)
            ? rawActivityHeight
            : undefined,
        showActivityFeed:
          typeof (parsed as any).showActivityFeed === 'boolean'
            ? (parsed as any).showActivityFeed
            : undefined,
        isBlendedMode:
          typeof (parsed as any).isBlendedMode === 'boolean'
            ? (parsed as any).isBlendedMode
            : undefined,
        isMentionsMode:
          typeof (parsed as any).isMentionsMode === 'boolean'
            ? (parsed as any).isMentionsMode
            : undefined,
        layoutTree: isSplitNode((parsed as any).layoutTree) ? ((parsed as any).layoutTree as SplitNode) : undefined,
        focusedLeafId:
          typeof (parsed as any).focusedLeafId === 'string' ? (parsed as any).focusedLeafId : undefined,
      };
    }
    return null;
  } catch (err) {
    Logger.warn('[MultiChatWindow] localStorage read failed:', err);
    return null;
  }
}

function persistState(
  windowId: string | null,
  channels: ChannelEntry[],
  layoutMode: LayoutMode,
  showModLogs: boolean,
  modLogsHeight: number,
  activityHeight: number,
  showActivityFeed: boolean,
  isBlendedMode: boolean,
  isMentionsMode: boolean,
  layoutTree: SplitNode | null,
  focusedLeafId: string | null,
) {
  const key = storageKey(windowId);
  if (!key) return;
  try {
    const payload: PersistedWindowState = {
      channels,
      layoutMode,
      showModLogs,
      modLogsHeight,
      activityHeight,
      showActivityFeed,
      isBlendedMode,
      isMentionsMode,
      layoutTree,
      focusedLeafId,
    };
    localStorage.setItem(key, JSON.stringify(payload));
  } catch (err) {
    Logger.warn('[MultiChatWindow] localStorage write failed:', err);
  }
}

// Whether the activity + mod-log panes keep collecting (and keep their data in
// memory) while closed. Off by default: a closed pane frees its RAM and stops its
// collector. On: the prior always-on behavior. Hyphen key dodges the orphan sweep.
const KEEP_COLLECTING_KEY = 'streamnook.multichat-keepcollecting';
function loadKeepPanesCollecting(): boolean {
  try {
    return localStorage.getItem(KEEP_COLLECTING_KEY) === '1';
  } catch {
    return false;
  }
}

async function closeWindow(): Promise<void> {
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    await getCurrentWindow().close();
  } catch (err) {
    Logger.error('[MultiChatWindow] close failed:', err);
  }
}

async function toggleMaximize(): Promise<void> {
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    const win = getCurrentWindow();
    const isMax = await win.isMaximized();
    if (isMax) await win.unmaximize();
    else await win.maximize();
  } catch (err) {
    Logger.error('[MultiChatWindow] maximize toggle failed:', err);
  }
}

async function minimizeWindow(): Promise<void> {
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    await getCurrentWindow().minimize();
  } catch (err) {
    Logger.error('[MultiChatWindow] minimize failed:', err);
  }
}

export default function MultiChatWindow() {
  useCommandPaletteHotkey();
  // Keeps this window's composers in step with an account connected in the main
  // window. Event-driven; the periodic session check stays main-window only.
  usePlatformAccountSync();
  // Keyboard moderation in the popout: the active pane registers the mod
  // controller (see ChatWidget), and this drives the hotkeys against it.
  useKeybindings();
  useEffect(() => {
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
  const [params] = useState<ParsedMultiChatParams>(() => parseMultiChatParams());
  const [settingsOpen, setSettingsOpen] = useState<boolean>(false);
  // Which tab the settings modal opens on: the gear opens Chat, the composer's
  // "Log in" badge opens Connections directly.
  const [settingsTab, setSettingsTab] = useState<'chat' | 'connections' | 'panes'>('chat');
  // The blended composer's per-source "Log in" badge asks to open Account
  // Connections rather than push a connect button into the chat space.
  useEffect(() => {
    const openConnections = () => {
      setSettingsTab('connections');
      setSettingsOpen(true);
    };
    window.addEventListener('open-multichat-connections', openConnections);
    return () => window.removeEventListener('open-multichat-connections', openConnections);
  }, []);
  // Badge picker / paint detail surfaces deliberately do NOT live here. Chat
  // badge clicks route through `utils/openBadgesInMain.ts`, which emits a
  // Tauri event picked up by main's tray bridge — main un-hides itself if
  // tray-hidden, focuses, and opens the overlay there. Keeps the popout slim
  // (chat-only surface) and avoids duplicating a heavy picker UI per window.

  // Initial channel list + layout. Persisted tabs from the last session are
  // restored first; if this window was cold-opened via a "Pop out chat" action
  // (channel carried in the URL), that channel is merged in (deduped) and made
  // the active tab. Two things this guards against:
  //   - Without the merge, restored tabs shadow the channel the user just
  //     clicked and it never opens at all (the original bug).
  //   - Without forcing it active, the clicked channel opens behind whichever
  //     restored tab happened to sort first.
  // This mirrors the warm-window path (`multichat-add-channel` handler below),
  // which already appends + activates — so cold-open and add-to-open behave the
  // same: existing tabs stay, the clicked channel opens focused.
  const initial = useMemo(() => {
    const persisted = loadPersistedState(params.id);
    const restored: ChannelEntry[] = (persisted?.channels ?? []).map((c) => ({
      ...c,
      channelName: c.channelName || c.channel,
    }));
    const layoutMode: LayoutMode = persisted?.layoutMode ?? 1;
    const showModLogs = persisted?.showModLogs ?? false;
    const modLogsHeight = clampModLogsHeight(persisted?.modLogsHeight ?? DEFAULT_MOD_LOGS_HEIGHT);
    // First run (no saved window state): default the activity feed ON at ~a quarter
    // of the window, so a brand-new streamer sees their follows/gifts/subs without
    // hunting for the toggle. Returning users keep whatever they last chose.
    const firstTime = !persisted;
    const activityHeight = clampModLogsHeight(
      persisted?.activityHeight ?? (firstTime ? Math.round(window.innerHeight * 0.25) : DEFAULT_ACTIVITY_HEIGHT),
    );
    const showActivityFeed = persisted?.showActivityFeed ?? firstTime;
    // Restore blended mode only on a pure reopen — a `replace` seed (popping out
    // MultiNook tiles) is asking for a fresh split layout, not the merged feed.
    const isBlendedMode = params.replace ? false : (persisted?.isBlendedMode ?? false);
    const isMentionsMode = params.replace ? false : (persisted?.isMentionsMode ?? false);
    const layoutTree: SplitNode | null = params.replace ? null : (persisted?.layoutTree ?? null);
    const focusedLeafId: string | null = persisted?.focusedLeafId ?? null;

    // Seed channels from the URL. A multi-channel list (popping out all MultiNook
    // tiles) takes precedence over the single-channel params. Restored tabs stay;
    // seeds merge in (deduped) and the first seed opens focused.
    const seed: ChannelEntry[] =
      params.channels && params.channels.length > 0
        ? params.channels
        : params.channel
          ? [
              {
                channel: params.channel.toLowerCase(),
                channelId: params.channelId,
                channelName: params.channelName || params.channel.toLowerCase(),
              },
            ]
          : [];

    if (seed.length > 0) {
      // `replace` (popping out all MultiNook tiles) opens a fresh view: only the
      // seeded channels, dropping any previously-persisted tabs. Otherwise merge
      // the seed into the restored set (deduped).
      let channels: ChannelEntry[];
      let seededLayout = layoutMode;
      if (params.replace) {
        channels = [...seed];
        // Auto-split into a column per popped-out channel (capped at 4 columns)
        // so the MultiNook streams open side by side, not stacked as tabs.
        seededLayout = Math.min(Math.max(seed.length, 1), 4) as LayoutMode;
      } else {
        channels = [...restored];
        for (const entry of seed) {
          // Provider-scoped dedup so the same name on two providers both seed in.
          if (!channels.some((c) => entryKey(c) === entryKey(entry))) channels.push(entry);
        }
      }
      return {
        channels,
        layoutMode: seededLayout,
        activeKey: entryKey(seed[0]),
        showModLogs,
        modLogsHeight,
        activityHeight,
        showActivityFeed,
        isBlendedMode,
        isMentionsMode,
        layoutTree,
        focusedLeafId,
      };
    }

    return {
      channels: restored,
      layoutMode,
      activeKey: restored[0] ? entryKey(restored[0]) : null,
      showModLogs,
      modLogsHeight,
      activityHeight,
      showActivityFeed,
      isBlendedMode,
      isMentionsMode,
      layoutTree,
      focusedLeafId,
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const [channels, setChannels] = useState<ChannelEntry[]>(initial.channels);
  const [layoutMode, setLayoutMode] = useState<LayoutMode>(initial.layoutMode);
  const [isBlendedMode, setIsBlendedMode] = useState(initial.isBlendedMode);
  const [isMentionsMode, setIsMentionsMode] = useState(initial.isMentionsMode);
  // Custom split layout. Null = the column presets (layoutMode). The refs let
  // selectChannel read the live tree without re-creating its callback.
  const [layoutTree, setLayoutTree] = useState<SplitNode | null>(initial.layoutTree);
  const [focusedLeafId, setFocusedLeafId] = useState<string | null>(initial.focusedLeafId);
  const layoutTreeRef = useRef<SplitNode | null>(initial.layoutTree);
  const focusedLeafRef = useRef<string | null>(initial.focusedLeafId);
  // A closed pane can leave the focus id dangling; resolve to the first leaf
  // at read time instead of writing state from an effect.
  const effectiveFocusedLeafId = useMemo(() => {
    if (!layoutTree) return focusedLeafId;
    if (focusedLeafId && findLeaf(layoutTree, focusedLeafId)) return focusedLeafId;
    return leaves(layoutTree)[0]?.id ?? null;
  }, [layoutTree, focusedLeafId]);
  // Keyboard pane cycling (Ctrl+Alt+Left/Right) through the keybinding engine.
  useEffect(() => {
    const step = (dir: 1 | -1) => {
      const tree = layoutTreeRef.current;
      if (!tree) return;
      const ls = leaves(tree);
      if (ls.length < 2) return;
      const idx = Math.max(0, ls.findIndex((l) => l.id === focusedLeafRef.current));
      const next = ls[(idx + dir + ls.length) % ls.length];
      setFocusedLeafId(next.id);
    };
    registerPaneFocusController({
      isAvailable: () => !!layoutTreeRef.current && leaves(layoutTreeRef.current).length > 1,
      focusNext: () => step(1),
      focusPrev: () => step(-1),
    });
    return () => registerPaneFocusController(null);
  }, []);
  useEffect(() => {
    layoutTreeRef.current = layoutTree;
    focusedLeafRef.current = effectiveFocusedLeafId;
  }, [layoutTree, effectiveFocusedLeafId]);

  // The active tab, identified by its composite provider:channel key (NOT the
  // bare channel — see `entryKey`). Two same-named sources on different
  // providers must not both read as active.
  const [activeKey, setActiveKey] = useState<string | null>(
    () => initial.activeKey,
  );

  // Mod-logs pane: window-local toggle + draggable height, both persisted in
  // this popout's window state so they survive close/reopen. Independent of the
  // main app's global `settings.show_mod_logs`.
  const [showModLogs, setShowModLogs] = useState<boolean>(initial.showModLogs);
  const [modLogsHeight, setModLogsHeight] = useState<number>(initial.modLogsHeight);

  // Activity feed pane (subs, raids, gifts, hearts across this window's sources).
  // MultiChat-only; it reads a separate store and never affects normal chat.
  // Local toggle for now, default off.
  const [showActivityFeed, setShowActivityFeed] = useState<boolean>(initial.showActivityFeed);
  // User pref: keep the activity + mod-log panes collecting (and holding their data
  // in memory) while closed, vs freeing them on close.
  const [keepPanesCollecting, setKeepPanesCollecting] = useState<boolean>(loadKeepPanesCollecting);
  const updateKeepPanesCollecting = useCallback((v: boolean) => {
    setKeepPanesCollecting(v);
    try {
      localStorage.setItem(KEEP_COLLECTING_KEY, v ? '1' : '0');
    } catch {
      /* ignore */
    }
  }, []);
  const [activityHeight, setActivityHeight] = useState<number>(initial.activityHeight);

  const startModLogsResize = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      const startY = e.clientY;
      const startHeight = modLogsHeight;
      const onMove = (ev: MouseEvent) => {
        // Drag up = taller pane (panel sits below the chat).
        setModLogsHeight(clampModLogsHeight(startHeight + (startY - ev.clientY)));
      };
      const onUp = () => {
        window.removeEventListener('mousemove', onMove);
        window.removeEventListener('mouseup', onUp);
      };
      window.addEventListener('mousemove', onMove);
      window.addEventListener('mouseup', onUp);
    },
    [modLogsHeight],
  );

  const startActivityResize = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      const startY = e.clientY;
      const startHeight = activityHeight;
      const onMove = (ev: MouseEvent) => {
        // Drag up = taller pane (it sits at the bottom of the stack).
        setActivityHeight(clampModLogsHeight(startHeight + (startY - ev.clientY)));
      };
      const onUp = () => {
        window.removeEventListener('mousemove', onMove);
        window.removeEventListener('mouseup', onUp);
      };
      window.addEventListener('mousemove', onMove);
      window.addEventListener('mouseup', onUp);
    },
    [activityHeight],
  );

  const [addInput, setAddInput] = useState('');
  const [addProvider, setAddProvider] = useState<ProviderId>('twitch');
  const [addError, setAddError] = useState<string | null>(null);
  const [addBusy, setAddBusy] = useState(false);
  const [showAdd, setShowAdd] = useState(false);
  // Mod room in MultiChat: a window-level Chat/Mods switch + channel picker (mod
  // rooms are Twitch-only and per-channel, so they don't fit the blended feed).
  const [modsMode, setModsMode] = useState(false);
  const [modChannel, setModChannel] = useState<string | null>(null);
  const [modStatus, setModStatus] = useState<{
    memberCount: number;
    encrypted: boolean;
    connected: boolean;
    members: ModRoomMember[];
  }>({
    memberCount: 0,
    encrypted: false,
    connected: false,
    members: [],
  });
  const [pickerOpen, setPickerOpen] = useState(false);
  const [moderatedSet, setModeratedSet] = useState<Set<string>>(new Set());
  useEffect(() => {
    let active = true;
    loadModeratedChannelIds().then((ids) => {
      if (active) setModeratedSet(new Set(ids));
    });
    // Refresh when the list re-resolves (e.g. right after the one-time consent),
    // so the Chat/Mods toggle and picker appear without an app restart.
    const unsub = subscribeModeratedChannels((ids) => setModeratedSet(new Set(ids)));
    return () => {
      active = false;
      unsub();
    };
  }, []);
  const twitchChannels = useMemo(() => channels.filter((c) => (c.provider ?? 'twitch') === 'twitch'), [channels]);
  // Only channels you actually moderate (Helix list, or the per-channel cache).
  const moderatedTwitch = useMemo(
    () => twitchChannels.filter((c) => !!c.channelId && (moderatedSet.has(c.channelId) || isCachedModerator(c.channelId))),
    [twitchChannels, moderatedSet],
  );
  // The channel you're currently focused on, if you moderate it. Used as the
  // picker's default so opening Mods lands on the relevant room, not the first in
  // the list. An explicit pick (modChannel) always takes priority over this.
  const activeModEntry = useMemo(() => {
    const active = channels.find((c) => entryKey(c) === activeKey);
    if (!active) return null;
    return moderatedTwitch.find((c) => c.channel === active.channel) ?? null;
  }, [channels, activeKey, moderatedTwitch]);
  const modChannelEntry = useMemo(
    () =>
      (modChannel ? moderatedTwitch.find((c) => c.channel === modChannel) : null) ??
      activeModEntry ??
      moderatedTwitch[0] ??
      null,
    [moderatedTwitch, modChannel, activeModEntry],
  );
  const modEmotes = useChannelEmotes(modChannelEntry?.channel ?? null, modChannelEntry?.channelId ?? null, 'twitch');
  // Mods mode cannot outlive the last moderated channel; adjusted during
  // render so the segmented control never paints a dead selection.
  if (modsMode && moderatedTwitch.length === 0) setModsMode(false);
  // Keep every open moderated channel's room connected in the background so the
  // Mods toggle and the picker can show per-channel unread without opening them.
  const moderatedIdsKey = useMemo(
    () =>
      moderatedTwitch
        .map((c) => c.channelId)
        .filter((id): id is string => !!id)
        .sort()
        .join(','),
    [moderatedTwitch],
  );
  useEffect(() => {
    const ids = moderatedIdsKey ? moderatedIdsKey.split(',') : [];
    ids.forEach((id) => ensureRoom(id));
    return () => ids.forEach((id) => releaseRoom(id));
  }, [moderatedIdsKey]);
  const modSessions = useModRoomStore((s) => s.sessions);
  const modUnreadTotal = useMemo(() => {
    let unread = 0;
    let mentions = 0;
    for (const c of moderatedTwitch) {
      if (!c.channelId) continue;
      unread += modSessions[c.channelId]?.unread ?? 0;
      mentions += modSessions[c.channelId]?.mentions ?? 0;
    }
    return { unread, mentions };
  }, [moderatedTwitch, modSessions]);
  // Channel avatars for the picker (resolved once, cached).
  const [avatarMap, setAvatarMap] = useState<Record<string, string>>({});
  useEffect(() => {
    let active = true;
    void Promise.all(
      moderatedTwitch.map(async (c) => [c.channel, getCachedAvatar(c.channel) ?? (await resolveAvatar(c.channel))] as const),
    ).then((pairs) => {
      if (!active) return;
      setAvatarMap((prev) => {
        const next = { ...prev };
        for (const [login, url] of pairs) if (url) next[login] = url;
        return next;
      });
    });
    return () => {
      active = false;
    };
  }, [moderatedTwitch]);

  // Each Tauri window has its own JS context, so the popout's AppStore boots
  // empty even when the main app is authenticated. Hydrate it on mount:
  //   - loadSettings pulls all user preferences from disk, including
  //     `chat_design` (font size/weight, spacing, dividers, timestamps,
  //     mention colors, mention animation, etc.). ChatMessage reads these
  //     directly via useAppStore, so loading them here gives the popout
  //     visual parity with the main app's chat without a separate settings
  //     surface.
  //   - checkAuthStatus pulls the cached user from the Rust-side token cache
  //     so the chat send input works without re-signing in.
  //   - loadFollowedStreams populates the live-following list (the add-
  //     channel dialog reads it for the quick-add panel).
  useEffect(() => {
    const store = useAppStore.getState();
    void store.loadSettings().catch((err) => {
      Logger.warn('[MultiChatWindow] loadSettings failed:', err);
    });
    void store.checkAuthStatus().catch((err) => {
      Logger.warn('[MultiChatWindow] checkAuthStatus failed:', err);
    });
    void store.loadFollowedStreams().catch((err) => {
      Logger.warn('[MultiChatWindow] loadFollowedStreams failed:', err);
    });

    // Refresh settings when ANY other window saves — keeps highlights, custom
    // commands, nicknames, etc. in sync across the main app and every open
    // MultiChat without needing a window reopen.
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listenForSettingsUpdates(() => {
      void useAppStore.getState().loadSettings();
    }).then((u) => {
      if (cancelled) {
        u();
        return;
      }
      unlisten = u;
    });
    return () => {
      cancelled = true;
      void Promise.resolve(unlisten?.()).catch(() => {});
    };
  }, []);

  // Apply the user's theme to THIS window's document. CSS variables and
  // `data-theme` live on each window's own documentElement/body, so a popout
  // that only hydrates settings (above) still paints with the default `:root`
  // palette until it applies the theme itself. Mirrors PluginWindowHost — read
  // the reactive settings slice and re-apply on any theme/glass/font change
  // (the settings-broadcast listener above refreshes them when the main app or
  // another window saves, so changing the theme there updates the popout live).
  const settings = useAppStore((s) => s.settings);
  // The Twitch account the app is signed in with — used to seed the streamer's own
  // Twitch source on a first-time Go Live (no need to make them re-add it).
  const currentUser = useAppStore((s) => s.currentUser);
  useEffect(() => {
    const themeId = settings.theme || DEFAULT_THEME_ID;
    const theme =
      themeId === OLED_THEME_ID
        ? getOledTheme(settings.oled_accent)
        : getThemeByIdWithCustom(themeId, settings.custom_themes || []) ||
          getThemeById(DEFAULT_THEME_ID);
    if (theme) applyTheme(theme);
    applyGlassStrength(settings.glass_transparency ?? DEFAULT_GLASS_TRANSPARENCY);
    applyFont(settings.font ?? DEFAULT_FONT_ID, settings.font_custom);
  }, [
    settings.theme,
    settings.custom_themes,
    settings.glass_transparency,
    settings.font,
    settings.font_custom,
    settings.oled_accent,
  ]);

  // Persist whenever the channel list, layout, or mod-logs prefs change.
  useEffect(() => {
    persistState(
      params.id,
      channels,
      layoutMode,
      showModLogs,
      modLogsHeight,
      activityHeight,
      showActivityFeed,
      isBlendedMode,
      isMentionsMode,
      layoutTree,
      focusedLeafId,
    );
  }, [
    params.id,
    channels,
    layoutMode,
    showModLogs,
    modLogsHeight,
    activityHeight,
    showActivityFeed,
    isBlendedMode,
    isMentionsMode,
    layoutTree,
    focusedLeafId,
  ]);

  // Persist the window's position + size (debounced) so a reopen lands on the same
  // monitor in the same spot. Written to a shared key the spawner reads at creation.
  useEffect(() => {
    let unMoved: (() => void) | undefined;
    let unResized: (() => void) | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    (async () => {
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        const win = getCurrentWindow();
        // Only the default window owns the shared geometry slot; extra
        // windows cascade off it at spawn and never overwrite it.
        if (params.id && params.id !== 'default') return;
        const save = () => {
          if (timer) clearTimeout(timer);
          timer = setTimeout(async () => {
            try {
              if (await win.isMinimized()) return;
              const pos = await win.outerPosition();
              const size = await win.outerSize();
              localStorage.setItem(
                MULTICHAT_GEOMETRY_KEY,
                JSON.stringify({ x: pos.x, y: pos.y, width: size.width, height: size.height }),
              );
            } catch {
              /* ignore */
            }
          }, 400);
        };
        unMoved = await win.onMoved(save);
        unResized = await win.onResized(save);
      } catch (err) {
        Logger.debug('[MultiChatWindow] geometry listeners failed:', err);
      }
    })();
    return () => {
      unMoved?.();
      unResized?.();
      if (timer) clearTimeout(timer);
    };
  }, []);

  // Size the window to the number of side-by-side chat columns so a split
  // layout doesn't squeeze N chats into one chat's width. Tabs mode = 1 column;
  // split modes render min(channels, layoutMode) columns. We only ever GROW the
  // window (and clamp to the monitor) — never shrink it out from under a user
  // who manually narrowed it, and never fight a maximized window.
  // Blended mode is a single merged feed, not N side-by-side columns, so it must not
  // grow the window one base-width per channel (that's what pushed the composer off a
  // narrow/vertical monitor).
  const visibleColumns =
    isBlendedMode || isMentionsMode
      ? 1
      : layoutTree
        ? Math.min(4, columnCount(layoutTree))
        : Math.min(channels.length, layoutMode);
  useEffect(() => {
    if (visibleColumns < 1) return;
    let cancelled = false;
    (async () => {
      try {
        const { getCurrentWindow, LogicalSize, LogicalPosition, currentMonitor } = await import(
          '@tauri-apps/api/window'
        );
        const win = getCurrentWindow();
        if (cancelled || (await win.isMaximized())) return;

        const scale = await win.scaleFactor();
        const monitor = await currentMonitor();

        let targetWidth = MULTICHAT_BASE_WIDTH * visibleColumns;
        if (monitor) {
          const monitorWidth = monitor.size.width / monitor.scaleFactor;
          // Leave a margin so the window never butts flush against the screen edge
          targetWidth = Math.min(targetWidth, Math.max(MULTICHAT_BASE_WIDTH, monitorWidth - 40));
        }

        const inner = await win.innerSize();
        const currentWidth = inner.width / scale;
        const currentHeight = inner.height / scale;

        // Grow only — leave manually-narrowed windows and already-wide windows alone.
        if (currentWidth >= targetWidth - 2) return;

        await win.setSize(new LogicalSize(Math.round(targetWidth), Math.round(currentHeight)));

        // The popout spawns at the main window's right edge, so a wider window can
        // spill off-screen. Nudge it left if its right edge passes the monitor.
        if (monitor) {
          const pos = await win.outerPosition();
          const monitorLeft = monitor.position.x / monitor.scaleFactor;
          const monitorWidth = monitor.size.width / monitor.scaleFactor;
          const winLeft = pos.x / scale;
          if (winLeft + targetWidth > monitorLeft + monitorWidth) {
            const newLeft = Math.max(monitorLeft, monitorLeft + monitorWidth - targetWidth - 10);
            await win.setPosition(new LogicalPosition(Math.round(newLeft), Math.round(pos.y / scale)));
          }
        }
      } catch (err) {
        Logger.warn('[MultiChatWindow] column-aware resize failed:', err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [visibleColumns]);

  // Safety net: keep the window inside the monitor it's on. After moving to a smaller
  // monitor, or restoring a size saved on a larger one, the window can be wider/taller
  // than the current screen, pushing the composer + send button off the edge. Shrink
  // to fit and nudge back into view on mount and (debounced) on move. Never touches a
  // maximized window.
  useEffect(() => {
    let unMoved: (() => void) | undefined;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    (async () => {
      try {
        const { getCurrentWindow, currentMonitor, LogicalSize, LogicalPosition } = await import(
          '@tauri-apps/api/window'
        );
        const win = getCurrentWindow();
        const fit = async () => {
          try {
            if (await win.isMaximized()) return;
            const monitor = await currentMonitor();
            if (!monitor) return;
            const scale = await win.scaleFactor();
            const monLeft = monitor.position.x / monitor.scaleFactor;
            const monTop = monitor.position.y / monitor.scaleFactor;
            const monW = monitor.size.width / monitor.scaleFactor;
            const monH = monitor.size.height / monitor.scaleFactor;
            const inner = await win.innerSize();
            let w = inner.width / scale;
            let h = inner.height / scale;
            if (w > monW + 2 || h > monH + 2) {
              w = Math.min(w, monW);
              h = Math.min(h, monH);
              await win.setSize(new LogicalSize(Math.round(w), Math.round(h)));
            }
            const pos = await win.outerPosition();
            const x = pos.x / scale;
            const y = pos.y / scale;
            const cx = Math.min(Math.max(x, monLeft), monLeft + monW - w);
            const cy = Math.min(Math.max(y, monTop), monTop + monH - h);
            if (Math.abs(cx - x) > 2 || Math.abs(cy - y) > 2) {
              await win.setPosition(new LogicalPosition(Math.round(cx), Math.round(cy)));
            }
          } catch {
            /* ignore */
          }
        };
        await fit();
        if (cancelled) return;
        unMoved = await win.onMoved(() => {
          if (timer) clearTimeout(timer);
          timer = setTimeout(() => void fit(), 300);
        });
      } catch (err) {
        Logger.debug('[MultiChatWindow] fit-to-monitor failed:', err);
      }
    })();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
      unMoved?.();
    };
  }, []);

  // Broadcast this popout's current channel set to the main window so it can
  // hide its own ChatWidget for channels we own — avoids duplicate chat
  // surfaces for the same channel. The Rust side handles the popout-closed
  // case by emitting `multichat-popout-closed` from on_window_event.
  const emitPopoutChannels = useCallback(async () => {
    try {
      const { emit } = await import('@tauri-apps/api/event');
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      const windowLabel = getCurrentWindow().label;
      await emit('multichat-popout-channels', {
        windowLabel,
        // Provider-scoped keys (Twitch stays bare to match its IRC mod-log channel
        // form; others are "provider:channel") so the main window distinguishes the
        // same name across providers for both the chat-surface dedup and the mod-log
        // filter — removing one provider's chat now correctly drops only its entries.
        channels: channels.map((c) =>
          (c.provider ?? 'twitch') === 'twitch' ? c.channel.toLowerCase() : entryKey(c),
        ),
      });
    } catch (err) {
      Logger.warn('[MultiChatWindow] emit multichat-popout-channels failed:', err);
    }
  }, [channels]);
  useEffect(() => {
    void emitPopoutChannels();
  }, [emitPopoutChannels]);
  // Re-broadcast once the main window is (re)created: going live DESTROYS main,
  // which drops its popout-channel tracking, so a freshly-recreated main needs our
  // set again to dedup its own chat surfaces (and mod log) correctly.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void (async () => {
      const { listen } = await import('@tauri-apps/api/event');
      const un = await listen('main-ready', () => void emitPopoutChannels());
      if (cancelled) un();
      else unlisten = un;
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [emitPopoutChannels]);

  // Acquire on add (and on initial mount), release on remove (and on unmount).
  // The "currently acquired set" mirrors `channels` exactly — diff that against
  // the previous render to figure out which to acquire and which to release.
  //
  // Naive release-all-then-acquire-all is wrong: useEffect cleanup runs BEFORE
  // the new effect body, so any channel present in BOTH the old and new lists
  // would briefly drop to refcount 0 on the chatConnectionStore AND on the
  // Rust IRC service. That triggers an IRC PART (and a WS teardown when this
  // window's slice map empties) followed by a JOIN — i.e. adding a new tab
  // would flash a disconnect on every existing tab, including the channel the
  // user was watching popped out from main. Per-channel cache loss + visible
  // chat reconnect = the user perceives it as being kicked out of the stream
  // they were watching. Diff against `acquiredKeysRef` so unchanged channels
  // keep their refcount steady.
  const acquiredKeysRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    // Diff by composite "provider:channel" key so Twitch and a same-named
    // non-Twitch source never collide. Twitch acquire/release still receive the
    // bare login + 'twitch', so their store keys stay bare (byte-identical).
    const keyOf = (e: ChannelEntry) => makeKey(e.provider ?? 'twitch', e.channel);
    const current = new Set(channels.map(keyOf));
    for (const entry of channels) {
      const k = keyOf(entry);
      if (!acquiredKeysRef.current.has(k)) {
        acquiredKeysRef.current.add(k);
        void acquireChannel(entry.channel, entry.channelId, entry.provider ?? 'twitch').catch(
          (err) => Logger.error('[MultiChatWindow] acquire failed:', err),
        );
      }
    }
    for (const k of Array.from(acquiredKeysRef.current)) {
      if (!current.has(k)) {
        acquiredKeysRef.current.delete(k);
        const parsed = parseKey(k);
        void releaseChannel(parsed.channel, parsed.provider).catch((err) =>
          Logger.warn('[MultiChatWindow] release failed:', err),
        );
      }
    }
  }, [channels]);

  // Release everything still held on unmount. The diffing effect above has no
  // cleanup of its own (it must not release on every `channels` change), so
  // window teardown needs its own release pass.
  useEffect(() => {
    return () => {
      for (const k of Array.from(acquiredKeysRef.current)) {
        const parsed = parseKey(k);
        void releaseChannel(parsed.channel, parsed.provider).catch((err) =>
          Logger.warn('[MultiChatWindow] release on unmount failed:', err),
        );
      }
      acquiredKeysRef.current.clear();
    };
  }, []);

  const addChannel = useCallback(
    async (rawLogin: string, provider: ProviderId = 'twitch', providedDisplayName?: string) => {
      const trimmed = rawLogin.trim();
      // A Kick source: chosen via the provider dropdown, or auto-detected from a
      // pasted kick.com link / "kick:" / "kick/" prefix. Read anonymously over
      // Kick's Pusher socket; no Twitch resolve.
      const kickMatch =
        trimmed.match(/^(?:https?:\/\/)?(?:www\.)?kick\.com\/(@?[a-z0-9_]+)/i) ||
        trimmed.match(/^kick[:/](@?[a-z0-9_]+)$/i);
      if (provider === 'kick' || kickMatch) {
        // Kick slugs are [a-z0-9_] only (no spaces/punctuation), so normalize to
        // that: it turns a typed display name like "ice poseidon" into the real
        // slug "iceposeidon", and keeps an invalid char from reaching the backend
        // (a space there crashed the resolver — Tauri window labels reject spaces).
        const slug = (kickMatch ? kickMatch[1] : trimmed)
          .toLowerCase()
          .replace(/[^a-z0-9_]/g, '');
        if (!slug) {
          setAddError('Channel name is required');
          return;
        }
        if (channels.some((c) => c.channel === slug && (c.provider ?? 'twitch') === 'kick')) {
          setAddError(`#${slug} is already open`);
          return;
        }
        setChannels((prev) => [
          ...prev,
          { channel: slug, channelId: null, channelName: providedDisplayName ?? slug, provider: 'kick' },
        ]);
        setActiveKey(makeKey('kick', slug));
        setAddInput('');
        setShowAdd(false);
        return;
      }
      // A YouTube source: chosen via the dropdown, or auto-detected from a pasted
      // youtube.com / youtu.be link. Read anonymously over YouTube's InnerTube API
      // (no login); only live streams have chat.
      const ytFromLink = parseYouTubeInput(trimmed);
      if (provider === 'youtube' || ytFromLink) {
        let identifier = ytFromLink ?? '';
        if (!identifier) {
          // Typed directly with the YouTube dropdown: a bare handle / id. Bare
          // handles get an @ so the backend never confuses one with a video id.
          const raw = trimmed.replace(/^youtube[:/]/i, '').replace(/^#/, '');
          if (/^[A-Za-z0-9_-]{11}$/.test(raw) || /^UC[A-Za-z0-9_-]{22}$/.test(raw)) {
            identifier = raw;
          } else if (raw) {
            identifier = raw.startsWith('@') ? raw : `@${raw}`;
          }
        }
        if (!identifier || identifier === '@') {
          setAddError('Enter a YouTube channel (@handle) or a live URL');
          return;
        }
        if (
          channels.some(
            (c) =>
              c.channel.toLowerCase() === identifier.toLowerCase() &&
              (c.provider ?? 'twitch') === 'youtube',
          )
        ) {
          setAddError(`${identifier} is already open`);
          return;
        }
        setChannels((prev) => [
          ...prev,
          {
            channel: identifier,
            channelId: null,
            channelName: providedDisplayName ?? identifier,
            provider: 'youtube',
          },
        ]);
        setActiveKey(makeKey('youtube', identifier));
        setAddInput('');
        setShowAdd(false);
        return;
      }
      // A TikTok source: chosen via the dropdown, or auto-detected from a pasted
      // tiktok.com link. Read anonymously over TikTok's webcast socket (no login);
      // only creators currently LIVE have chat.
      const ttFromLink = parseTikTokInput(trimmed);
      if (provider === 'tiktok' || ttFromLink) {
        const handle = (ttFromLink ?? trimmed.replace(/^tiktok[:/]/i, '').replace(/^[@#]/, '')).trim();
        if (!handle) {
          setAddError('Enter a TikTok @handle or a LIVE link');
          return;
        }
        if (
          channels.some(
            (c) =>
              c.channel.toLowerCase() === handle.toLowerCase() &&
              (c.provider ?? 'twitch') === 'tiktok',
          )
        ) {
          setAddError(`@${handle} is already open`);
          return;
        }
        setChannels((prev) => [
          ...prev,
          { channel: handle, channelId: null, channelName: providedDisplayName ?? handle, provider: 'tiktok' },
        ]);
        setActiveKey(makeKey('tiktok', handle));
        setAddInput('');
        setShowAdd(false);
        return;
      }
      const login = trimmed.toLowerCase().replace(/^#/, '');
      if (!login) {
        setAddError('Channel name is required');
        return;
      }
      if (channels.some((c) => c.channel === login && (c.provider ?? 'twitch') === 'twitch')) {
        setAddError(`#${login} is already open`);
        return;
      }
      setAddBusy(true);
      setAddError(null);
      try {
        // Resolve by login via Helix Get Users, which works whether or not the
        // channel is streaming (offline chat is the same room). This gives us the
        // broadcaster id + properly-cased name AND confirms the channel exists — a
        // failure here is the "that channel isn't real" gate. (An offline-but-valid
        // channel still adds; the pane header shows "OFFLINE CHAT" on its own.)
        let channelId: string | null = null;
        let channelName = providedDisplayName ?? login;
        try {
          const user = await invoke<{ id?: string; display_name?: string }>('get_user_by_login', {
            login,
          });
          channelId = user?.id ?? null;
          if (user?.display_name) channelName = user.display_name;
        } catch (err) {
          const reason = typeof err === 'string' ? err : err instanceof Error ? err.message : '';
          const msg = /not found/i.test(reason)
            ? `No Twitch channel called "${login}". Check the spelling.`
            : `Couldn't add "${login}" right now. ${reason || 'Please try again.'}`;
          setAddError(msg);
          useAppStore.getState().addToast(msg, 'error');
          return; // don't add a tab that will never connect
        }
        setChannels((prev) => [...prev, { channel: login, channelId, channelName, provider: 'twitch' }]);
        setActiveKey(makeKey('twitch', login));
        setAddInput('');
        setShowAdd(false);
      } finally {
        setAddBusy(false);
      }
    },
    [channels],
  );

  // Hand a channel back to the main app: emit the watch-channel-in-main event
  // so main starts watching the stream (and un-hides itself if tray-hidden).
  // Used both by the title-bar restore button and the per-tab "Watch in main
  // app" right-click action.
  const watchInMain = useCallback(async (entry: ChannelEntry) => {
    try {
      // Ensure main exists + is listening first (going live may have closed it),
      // then hand off the channel for it to start watching.
      const { ensureMainAndEmit } = await import('../../utils/ensureMainWindow');
      await ensureMainAndEmit('watch-channel-in-main', {
        channel: entry.channel,
        channelId: entry.channelId ?? undefined,
        channelName: entry.channelName,
      });
    } catch (err) {
      Logger.error('[MultiChatWindow] emit watch-channel-in-main failed:', err);
    }
  }, []);

  const removeChannel = useCallback((key: string) => {
    // Panes showing the removed channel close; the last pane follows the tabs.
    setLayoutTree((t) => (t ? removeKey(t, key) : t));
    setChannels((prev) => {
      const next = prev.filter((c) => entryKey(c) !== key);
      // If we removed the active tab, pick a neighbor.
      if (key === activeKey) {
        const idx = prev.findIndex((c) => entryKey(c) === key);
        const neighbor = next[idx] ?? next[idx - 1] ?? null;
        setActiveKey(neighbor ? entryKey(neighbor) : null);
      }
      return next;
    });
    // Drop the removed channel's moderator-log entries right away. The store's
    // refcount teardown that normally prunes them is async, and the mod-log panel's
    // reload effect can re-pull a just-removed channel from disk before that lands,
    // leaving stale entries. Prune to the remaining lineup in the mod-log channel
    // form (bare login for Twitch, composite "<provider>:<channel>" for others).
    const remaining = channels
      .filter((c) => entryKey(c) !== key)
      .map((c) => ((c.provider ?? 'twitch') === 'twitch' ? c.channel.toLowerCase() : entryKey(c)));
    useAppStore.getState().pruneModLogsToChannels(remaining);
  }, [activeKey, channels]);

  // Click a tab to focus it. In tabs mode that just switches the visible pane;
  // in split mode it also hoists the channel to position 0 so it lands in the
  // leftmost column. Focusing a single channel always exits blended mode —
  // blended ignores tabs, so a tab click is the user asking to swap back to a
  // single (or split) view of that channel rather than the merged feed.
  const selectChannel = useCallback(
    (key: string) => {
      setActiveKey(key);
      setIsBlendedMode(false);
      setIsMentionsMode(false);
      // Custom splits: a tab click lands in the focused pane unless a pane
      // already shows that channel (or follows the active tab). No hoisting.
      const tree = layoutTreeRef.current;
      if (tree) {
        if (!showsKey(tree, key, key)) {
          const focused = focusedLeafRef.current;
          const target = focused && findLeaf(tree, focused) ? focused : leaves(tree)[0].id;
          setLayoutTree(setLeafKey(tree, target, key));
        }
        return;
      }
      if (layoutMode > 1) {
        setChannels((prev) => {
          const idx = prev.findIndex((c) => entryKey(c) === key);
          if (idx <= 0) return prev;
          const next = [...prev];
          const [entry] = next.splice(idx, 1);
          next.unshift(entry);
          return next;
        });
      }
    },
    [layoutMode],
  );

  // Listen for `multichat-add-channel` events from main. Fired by
  // `openMultiChatWindow` when this popout already exists and the user
  // popped chat out from another stream — main focuses us and forwards the
  // channel here instead of spawning a second window. Dedup against the
  // current channel list (functional setter to avoid stale closure) and
  // either append + activate, or just activate if already a tab.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        const u = await listen<{
          channel: string;
          channelId: string | null;
          channelName: string | null;
        }>('multichat-add-channel', (event) => {
          const login = (event.payload.channel || '').toLowerCase();
          if (!login) return;
          setChannels((prev) => {
            // Provider-scoped: this pop-out path adds a Twitch chat, so only an
            // existing TWITCH tab for this login is a duplicate.
            if (prev.some((c) => c.channel === login && (c.provider ?? 'twitch') === 'twitch'))
              return prev;
            return [
              ...prev,
              {
                channel: login,
                channelId: event.payload.channelId ?? null,
                channelName: event.payload.channelName || login,
              },
            ];
          });
          setActiveKey(makeKey('twitch', login));
        });
        if (cancelled) {
          // Unmounted before listen resolved (StrictMode): guard the unlisten —
          // Tauri's unlisten can reject during teardown (registry gone).
          void Promise.resolve(u()).catch(() => {});
          return;
        }
        unlisten = u;
      } catch (err) {
        Logger.warn('[MultiChatWindow] listen multichat-add-channel failed:', err);
      }
    })();
    return () => {
      cancelled = true;
      void Promise.resolve(unlisten?.()).catch(() => {});
    };
  }, []);

  // Listen for `multichat-set-channels` — fired when an already-open popout is
  // asked to REPLACE its whole tab set (e.g. popping out all MultiNook tiles for
  // a fresh view). Swaps the channel list wholesale; the acquire/release diffing
  // effect handles dropping the old channels and acquiring the new ones.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        const u = await listen<{
          channels: Array<{ channel: string; channelId?: string | null; channelName?: string | null }>;
        }>('multichat-set-channels', (event) => {
          const list = (event.payload.channels || [])
            .filter((c) => c && typeof c.channel === 'string')
            .map((c) => ({
              channel: c.channel.toLowerCase(),
              channelId: c.channelId ?? null,
              channelName: c.channelName || c.channel,
            }));
          if (list.length === 0) return;
          setChannels(list);
          setActiveKey(entryKey(list[0]));
          // Auto-split into a column per channel (capped at 4) — popping out all
          // MultiNook tiles should show them side by side, not stacked as tabs.
          setLayoutMode(Math.min(Math.max(list.length, 1), 4) as LayoutMode);
        });
        if (cancelled) {
          // Unmounted before listen resolved (StrictMode): guard the unlisten —
          // Tauri's unlisten can reject during teardown (registry gone).
          void Promise.resolve(u()).catch(() => {});
          return;
        }
        unlisten = u;
      } catch (err) {
        Logger.warn('[MultiChatWindow] listen multichat-set-channels failed:', err);
      }
    })();
    return () => {
      cancelled = true;
      void Promise.resolve(unlisten?.()).catch(() => {});
    };
  }, []);

  // Normalize a source's tab label to its resolved display name (a YouTube/Kick pane
  // dispatches `multichat-source-resolved` once its channel metadata lands, so e.g. a
  // typed "@jynxzi" becomes "Jynxzi"). Same-window DOM event (pane + window share the
  // popout document). Routing keys off `channel`, so only the display label changes.
  useEffect(() => {
    const onResolved = (e: Event) => {
      const d = (e as CustomEvent<{ provider: ProviderId; channel: string; displayName: string }>).detail;
      if (!d?.displayName || !d.channel) return;
      const key = makeKey(d.provider, d.channel);
      setChannels((prev) =>
        prev.map((c) =>
          entryKey(c) === key && c.channelName !== d.displayName
            ? { ...c, channelName: d.displayName }
            : c,
        ),
      );
    };
    window.addEventListener('multichat-source-resolved', onResolved as EventListener);
    return () => window.removeEventListener('multichat-source-resolved', onResolved as EventListener);
  }, []);

  // Moderator view: channel.moderate events from the dedicated moderation socket
  // (Rust) are emitted to every window. This popout enriches its own mod-log
  // store with the acting moderator's identity, same as the main window.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        const { applyModerateEvent } = await import('../../utils/applyModerateEvent');
        const u = await listen<Record<string, unknown>>('eventsub://channel-moderate', (event) => {
          applyModerateEvent(event.payload);
        });
        if (cancelled) {
          // Unmounted before listen resolved (StrictMode): guard the unlisten —
          // Tauri's unlisten can reject during teardown (registry gone).
          void Promise.resolve(u()).catch(() => {});
          return;
        }
        unlisten = u;
      } catch (err) {
        Logger.warn('[MultiChatWindow] listen channel-moderate failed:', err);
      }
    })();
    return () => {
      cancelled = true;
      void Promise.resolve(unlisten?.()).catch(() => {});
    };
  }, []);

  // Live 7TV emote-set updates pushed from the shared EventAPI socket in Rust.
  // This popout has its own per-window emote cache + chat store, so it applies
  // the change independently of the main window.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        const { handleSeventvEmoteSetUpdate } = await import('../../services/seventvEventApi');
        const u = await listen<import('../../services/seventvEventApi').EmoteSetUpdatePayload>(
          '7tv://emote-set-update',
          (event) => {
            void handleSeventvEmoteSetUpdate(event.payload);
          },
        );
        if (cancelled) {
          // Unmounted before listen resolved (StrictMode): guard the unlisten —
          // Tauri's unlisten can reject during teardown (registry gone).
          void Promise.resolve(u()).catch(() => {});
          return;
        }
        unlisten = u;
      } catch (err) {
        Logger.warn('[MultiChatWindow] listen 7tv://emote-set-update failed:', err);
      }
    })();
    return () => {
      cancelled = true;
      void Promise.resolve(unlisten?.()).catch(() => {});
    };
  }, []);

  // Live 7TV cosmetics (paints/badges) for present users, delivered over the
  // same EventAPI socket. Re-resolves via GQL into this window's cosmetics cache.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        const { handleSeventvCosmeticUpdate } = await import('../../services/seventvEventApi');
        const u = await listen<{ twitch_id: string; action: string }>(
          '7tv://cosmetic-update',
          (event) => {
            void handleSeventvCosmeticUpdate(event.payload);
          },
        );
        if (cancelled) {
          // Unmounted before listen resolved (StrictMode): guard the unlisten —
          // Tauri's unlisten can reject during teardown (registry gone).
          void Promise.resolve(u()).catch(() => {});
          return;
        }
        unlisten = u;
      } catch (err) {
        Logger.warn('[MultiChatWindow] listen 7tv://cosmetic-update failed:', err);
      }
    })();
    return () => {
      cancelled = true;
      void Promise.resolve(unlisten?.()).catch(() => {});
    };
  }, []);

  // Keyboard shortcuts scoped to this popout window. Tauri webviews don't
  // implement default browser behavior for these chords, so claiming them
  // doesn't fight the user agent:
  //   Ctrl+T            — open the Add Channel form
  //   Ctrl+W            — close the active tab (or the window if none)
  //   Ctrl+Tab          — cycle to the next tab
  //   Ctrl+Shift+Tab    — cycle to the previous tab
  //   Ctrl+1..9         — jump to tab N by index
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.ctrlKey || e.altKey || e.metaKey) return;
      // Ignore shortcuts while typing in inputs / textareas / contenteditables,
      // except for Ctrl+Tab which is the most useful "I'm in the input and
      // want to switch tabs" case.
      const target = e.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === 'INPUT' ||
          target.tagName === 'TEXTAREA' ||
          target.isContentEditable)
      ) {
        if (e.key !== 'Tab') return;
      }

      if (e.key.toLowerCase() === 't') {
        e.preventDefault();
        setShowAdd(true);
        setAddError(null);
        return;
      }
      if (e.key.toLowerCase() === 'w') {
        e.preventDefault();
        if (activeKey) {
          removeChannel(activeKey);
        } else {
          void closeWindow();
        }
        return;
      }
      if (e.key === 'Tab' && channels.length > 1) {
        e.preventDefault();
        const idx = channels.findIndex((c) => entryKey(c) === activeKey);
        const safeIdx = idx === -1 ? 0 : idx;
        const nextIdx = e.shiftKey
          ? (safeIdx - 1 + channels.length) % channels.length
          : (safeIdx + 1) % channels.length;
        selectChannel(entryKey(channels[nextIdx]));
        return;
      }
      if (/^[1-9]$/.test(e.key)) {
        const targetIdx = parseInt(e.key, 10) - 1;
        const target = channels[targetIdx];
        if (target) {
          e.preventDefault();
          selectChannel(entryKey(target));
        }
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [channels, activeKey, selectChannel, removeChannel]);

  // Channels currently rendered. Tabs mode = just the active one. Split mode =
  // first N from the list (channels beyond N stay JOINed in the background,
  // ready to swap in without re-JOINing).
  const visibleChannels = useMemo<ChannelEntry[]>(() => {
    if (layoutMode === 1) {
      const active = channels.find((c) => entryKey(c) === activeKey);
      // Fall back to the first channel when the active key is stale (e.g. after
      // leaving blended mode, where no single tab was active) so Tabs view never
      // renders blank with the connections still live in the background.
      return active ? [active] : channels.slice(0, 1);
    }
    return channels.slice(0, layoutMode);
  }, [channels, activeKey, layoutMode]);

  // Composite keys of the currently-rendered channels (provider-namespaced), so
  // the tab strip can mark the right tabs visible without bare-name collisions.
  const visibleSet = useMemo(() => {
    if (layoutTree) {
      const set = new Set<string>();
      for (const l of leaves(layoutTree)) {
        const k = l.key ?? activeKey;
        if (k) set.add(k);
      }
      return set;
    }
    return new Set(visibleChannels.map(entryKey));
  }, [visibleChannels, layoutTree, activeKey]);

  // Composite source keys for this window (provider-namespaced). Every source is
  // Twitch today; Phase-1 providers will carry entry.provider instead.
  const activityKeys = useMemo(
    () => channels.map((c) => makeKey(c.provider ?? 'twitch', c.channel)),
    [channels],
  );

  // Only run the read-only event normalizer while the activity pane is open, so a
  // closed pane keeps neither a running collector nor its events in memory. On open
  // we re-hydrate the pane's history from disk (recent events still show); on close
  // we stop the normalizer and release the in-memory events (disk copy kept). It
  // mirrors event streams into the activity store and never touches chat. Tradeoff:
  // events that occur while the pane is closed aren't captured.
  useEffect(() => {
    // Collect while the pane is open, or always when the user opted to keep
    // collecting; only free the in-memory events when neither holds.
    if (!showActivityFeed && !keepPanesCollecting) return;
    useActivityStore.getState().hydrate();
    void startActivityNormalizer();
    return () => {
      stopActivityNormalizer();
      if (!keepPanesCollecting) useActivityStore.getState().release();
    };
  }, [showActivityFeed, keepPanesCollecting]);

  // Free the in-memory mod-log buffer when its pane is closed (disk history is kept;
  // the widget reloads it on reopen). Chat-side deletion state lives on the slice, so
  // this only releases the pane's RAM, not chat rendering. Skipped when the user opted
  // to keep the panes collecting.
  useEffect(() => {
    if (!showModLogs && !keepPanesCollecting) useAppStore.getState().pruneModLogsToChannels([]);
  }, [showModLogs, keepPanesCollecting]);

  // Title-bar heading. The logo handles the StreamNook brand visually, so the
  // text portion just identifies the surface / active channel:
  //   - empty window:   "MultiChat"
  //   - tabs mode:      the active channel's display name
  //   - split mode:     "MultiChat · N channels"
  const heading = useMemo(() => {
    if (channels.length === 0) return 'MultiChat';
    if (layoutMode > 1 || (layoutTree && leafCount(layoutTree) > 1)) {
      return `MultiChat · ${channels.length} channel${channels.length === 1 ? '' : 's'}`;
    }
    const active = channels.find((c) => entryKey(c) === activeKey);
    if (active) return active.channelName || active.channel;
    return 'MultiChat';
  }, [channels, activeKey, layoutMode]);

  // login -> proper display name, so the mod-logs header chips show real
  // capitalization for every open channel rather than the bare login. Map BOTH
  // the bare login AND the composite provider:channel key, so a Kick source
  // resolves to its display name whether the mod-log pane keys by login (the
  // all-Twitch path) or by composite key (otherwise it leaks the raw "kick:slug").
  const modLogChannelLabels = useMemo(
    () =>
      Object.fromEntries(
        channels.flatMap((c) => {
          const label = c.channelName || c.channel;
          return [
            [c.channel, label],
            [makeKey(c.provider ?? 'twitch', c.channel), label],
          ];
        }),
      ),
    [channels],
  );

  // Kick channels are added before their (Cloudflare-gated) resolve completes, so
  // their tab initially shows the lowercase slug. Once the resolve caches the
  // channel's properly-cased username, upgrade the tab name to it.
  const resolvedKickNamesRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    const pending = channels.filter(
      (c) =>
        c.provider === 'kick' &&
        c.channelName === c.channel &&
        !resolvedKickNamesRef.current.has(c.channel),
    );
    if (pending.length === 0) return;
    let cancelled = false;
    const poll = async () => {
      for (const c of pending) {
        try {
          const meta = await invoke<{ username?: string | null } | null>('get_kick_channel_meta', {
            slug: c.channel,
          });
          const name = meta?.username;
          if (cancelled || !name) continue;
          resolvedKickNamesRef.current.add(c.channel); // resolved — stop polling this one
          if (name.toLowerCase() === c.channel && name !== c.channel) {
            setChannels((prev) =>
              prev.map((e) =>
                e.provider === 'kick' && e.channel === c.channel && e.channelName === e.channel
                  ? { ...e, channelName: name }
                  : e,
              ),
            );
          }
        } catch {
          /* meta not cached yet — keep polling */
        }
      }
    };
    void poll();
    const t = setInterval(poll, 3000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, [channels]);

  // --- Go Live profile: one saved snapshot of the streamer's own sources +
  // activity filters + layout, applied with one click. Applying SWAPS to exactly
  // these sources (acquire/release reconciliation drops anything else). ---
  // Bumps whenever the saved Go Live profile changes (save / edit sources) so the
  // derived values below re-read localStorage.
  const [goLiveVersion, setGoLiveVersion] = useState(0);
  const refreshGoLive = useCallback(() => setGoLiveVersion((v) => v + 1), []);
  const savedSources = useMemo(() => readGoLiveSources(), [goLiveVersion]);
  const goLiveExists = savedSources.length > 0;
  // "Live" is a MODE = chat-only (main app gone). It's set by any main DESTROY (Go
  // Live, OR the core app's X button) and cleared only by the explicit "Live Chat"
  // toggle. Crucially it is NOT cleared when main merely OPENS: peeking at main for a
  // badge/VOD handoff or via the "open main app" button keeps you live. Seeded once
  // from reality so reopening this popout mid-session (main already closed) reads live.
  const [live, setLive] = useState(false);
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void (async () => {
      const { getAllWindows } = await import('@tauri-apps/api/window');
      const exists = (await getAllWindows()).some((w) => w.label === 'main');
      if (!cancelled) setLive(!exists);
      // main DESTROY -> live. Deliberately NO `main-ready` listener (opening main for
      // a peek must not exit live).
      const { listen } = await import('@tauri-apps/api/event');
      const un = await listen('main-closed', () => setLive(true));
      if (cancelled) un();
      else unlisten = un;
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);
  // True once the user has engaged Go Live this session (started setup, loaded, or
  // saved). Gates the exit autosave so a pure watching session never becomes a
  // phantom profile (which would also wrongly light the red "Live Chat" indicator).
  const goLiveEngagedRef = useRef(false);
  // Latest profile snapshot for the unload autosave, which can't read React state
  // closures reliably at exit. Refreshed every render.
  const goLiveSnapshotRef = useRef({
    sources: channels,
    blended: isBlendedMode,
    layoutMode,
    showActivityFeed,
    showModLogs,
  });
  useEffect(() => {
    goLiveSnapshotRef.current = {
      sources: channels,
      blended: isBlendedMode,
      layoutMode,
      showActivityFeed,
      showModLogs,
    };
  });
  // Going live makes MultiChat the standalone surface: fully CLOSE the main app
  // window to free its memory (its ~350MB webview shell + player), leaving only
  // this popout. Anything that later needs main (badge overlay, profile viewer,
  // whisper, watch-in-main, clip/VOD, the "open main app" button) recreates it on
  // demand via `ensure_main_window`. Chat keeps flowing because the popout owns
  // its own connections.
  const hideMainApp = useCallback(async () => {
    try {
      // Stop the stream + drops FIRST (global Rust commands, so they run without
      // the about-to-be-destroyed main's JS), then destroy main via a Rust command.
      // Doing the destroy Rust-side bypasses both the JS `allow-destroy` permission
      // AND the close-to-tray CloseRequested interception. Rust releases this
      // window's IRC claims on destroy; the popout's own claims keep chat alive.
      await invoke('stop_stream').catch(() => {});
      await invoke('stop_drops_monitoring').catch(() => {});
      await invoke('close_main_window');
    } catch (err) {
      Logger.warn('[MultiChatWindow] hideMainApp failed:', err);
    }
  }, []);
  const showMainApp = useCallback(async () => {
    try {
      // Get-or-create: shows main if hidden, or recreates it (from JS using this
      // window's origin) if Go Live closed it — the same path the handoffs use.
      const { ensureMainAlive } = await import('../../utils/ensureMainWindow');
      await ensureMainAlive();
    } catch (err) {
      Logger.warn('[MultiChatWindow] showMainApp failed:', err);
    }
  }, []);
  // Exit live MODE: bring the main app back and clear the indicator. This is the
  // deliberate "I'm done" toggle (the primary "Live Chat" button click), as opposed
  // to the home button / badge / VOD handoffs which open main but STAY live.
  const exitLive = useCallback(() => {
    setLive(false);
    void showMainApp();
  }, [showMainApp]);
  const saveGoLive = useCallback(() => {
    goLiveEngagedRef.current = true;
    try {
      const profile = {
        sources: channels,
        activityHidden: JSON.parse(localStorage.getItem('sn-activity-hidden') || '[]'),
        blended: isBlendedMode,
        layoutMode,
        showActivityFeed,
        showModLogs,
      };
      localStorage.setItem(GOLIVE_KEY, JSON.stringify(profile));
      refreshGoLive();
    } catch (err) {
      Logger.warn('[MultiChatWindow] saveGoLive failed:', err);
    }
  }, [channels, isBlendedMode, layoutMode, showActivityFeed, showModLogs, refreshGoLive]);
  const applyGoLive = useCallback(() => {
    goLiveEngagedRef.current = true;
    try {
      const raw = localStorage.getItem(GOLIVE_KEY);
      if (!raw) return;
      const profile = JSON.parse(raw) as {
        sources?: ChannelEntry[];
        activityHidden?: string[];
        blended?: boolean;
        layoutMode?: LayoutMode;
        showActivityFeed?: boolean;
        showModLogs?: boolean;
      };
      const sources = Array.isArray(profile.sources) ? profile.sources : [];
      // Push the saved activity filters, then nudge the open feed to reload them.
      localStorage.setItem('sn-activity-hidden', JSON.stringify(profile.activityHidden ?? []));
      window.dispatchEvent(new Event('sn-activity-hidden-changed'));
      setChannels(sources);
      setActiveKey(sources[0] ? entryKey(sources[0]) : '');
      if (profile.layoutMode) setLayoutMode(profile.layoutMode);
      // Loading the whole setup assumes the merged/blended feed (Brandon): you're
      // monitoring everything at once, not flipping single tabs.
      setIsBlendedMode(true);
      if (typeof profile.showActivityFeed === 'boolean') setShowActivityFeed(profile.showActivityFeed);
      if (typeof profile.showModLogs === 'boolean') setShowModLogs(profile.showModLogs);
      void hideMainApp();
      setLive(true);
    } catch (err) {
      Logger.warn('[MultiChatWindow] applyGoLive failed:', err);
    }
  }, [hideMainApp]);
  // Go Live with no saved profile yet. Two cases:
  //   - You already have channels open: treat THOSE as your live setup — save them
  //     as your profile and go live (blended), no dropdown. (Not everyone wants to
  //     add every platform, so we don't force the picker open.)
  //   - Truly empty window (first-ever run): seed the signed-in Twitch account and
  //     open the picker on the next platform so the streamer adds the rest.
  // Either way, drop to the standalone MultiChat surface.
  const startGoLiveSetup = useCallback(() => {
    goLiveEngagedRef.current = true;
    setLive(true);
    if (channels.length > 0) {
      saveGoLive();
      setIsBlendedMode(true);
      void hideMainApp();
      return;
    }
    const login = currentUser?.login?.toLowerCase();
    if (login) {
      setChannels([
        { channel: login, channelId: null, channelName: currentUser?.display_name || login, provider: 'twitch' },
      ]);
      setActiveKey(makeKey('twitch', login));
    }
    setAddProvider('kick');
    setAddError(null);
    setShowAdd(true);
    void hideMainApp();
  }, [channels, currentUser, saveGoLive, hideMainApp]);

  // Autosave the Go Live profile on exit so a setup the user built but forgot to
  // save isn't lost (their full config: sources, activity filters, layout, panes).
  // Gated on having engaged Go Live this session so a pure watching session never
  // becomes a phantom profile. localStorage writes are synchronous, so this lands
  // even during unload.
  useEffect(() => {
    const save = () => {
      if (!goLiveEngagedRef.current) return;
      const snap = goLiveSnapshotRef.current;
      if (snap.sources.length === 0) return;
      try {
        localStorage.setItem(
          GOLIVE_KEY,
          JSON.stringify({
            sources: snap.sources,
            activityHidden: JSON.parse(localStorage.getItem('sn-activity-hidden') || '[]'),
            blended: snap.blended,
            layoutMode: snap.layoutMode,
            showActivityFeed: snap.showActivityFeed,
            showModLogs: snap.showModLogs,
          }),
        );
      } catch {
        /* best-effort */
      }
    };
    window.addEventListener('pagehide', save);
    window.addEventListener('beforeunload', save);
    return () => {
      window.removeEventListener('pagehide', save);
      window.removeEventListener('beforeunload', save);
    };
  }, []);

  return (
    <div className="flex h-screen flex-col bg-background text-textPrimary">
      {/* Drag-to-moderate overlay (portals to document.body): brings the main
          app's drag mod tooling into the MultiChat command center. */}
      <ModerationDragLayer />
      {/* This window has no full ToastManager (that one also fires live-stream
          notifications); a minimal surface paints add-channel feedback etc. */}
      <MultiChatToasts />
      <TitleBar
        heading={heading}
        layoutMode={layoutMode}
        onLayoutModeChange={(m) => {
          setLayoutMode(m);
          setLayoutTree(null);
          setIsBlendedMode(false);
          setIsMentionsMode(false);
        }}
        isSplitMode={!!layoutTree && !isBlendedMode && !isMentionsMode}
        onToggleSplits={() => {
          setIsBlendedMode(false);
          setIsMentionsMode(false);
          if (layoutTree) {
            setLayoutTree(null);
            return;
          }
          // Seed from what is on screen: tabs mode becomes one pane that
          // follows the active tab; a column preset becomes those columns.
          const seeded = layoutMode === 1 ? fromColumns([null]) : fromColumns(visibleChannels.map(entryKey));
          setLayoutTree(seeded);
          setFocusedLeafId(leaves(seeded)[0].id);
        }}
        isBlendedMode={isBlendedMode}
        onToggleBlended={() => {
          setIsBlendedMode((v) => !v);
          setIsMentionsMode(false);
        }}
        isMentionsMode={isMentionsMode}
        onToggleMentions={() => {
          setIsMentionsMode((v) => !v);
          setIsBlendedMode(false);
        }}
        canSplit={channels.length > 0}
        // Restore is only meaningful when this popout owns exactly one channel
        // — otherwise we'd have to pick which one to hand back, which the user
        // didn't ask. For multi-channel popouts, per-tab right-click → "Watch
        // in main app" handles the more granular case.
        canRestore={channels.length === 1}
        onRestore={async () => {
          if (channels.length !== 1) return;
          await watchInMain(channels[0]);
          await closeWindow();
        }}
        onClose={closeWindow}
        onMinimize={minimizeWindow}
        onMaximize={toggleMaximize}
        onOpenSettings={() => {
          setSettingsTab('chat');
          setSettingsOpen(true);
        }}
        showModLogs={showModLogs}
        onToggleModLogs={() => setShowModLogs((v) => !v)}
        showActivityFeed={showActivityFeed}
        onToggleActivityFeed={() => setShowActivityFeed((v) => !v)}
        onOpenMainApp={showMainApp}
        channels={channels}
      />
      {/* Always mounted so the modal can play its own open/close (fill) animation
          via AnimatePresence; it renders nothing while closed. */}
      <ChatOnlySettingsModal
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        initialTab={settingsTab}
        keepPanesCollecting={keepPanesCollecting}
        onKeepPanesCollectingChange={updateKeepPanesCollecting}
      />
      <TabStrip
        channels={channels}
        active={activeKey}
        visibleSet={visibleSet}
        layoutMode={layoutMode}
        isBlendedMode={isBlendedMode}
        onSelect={selectChannel}
        onRemove={removeChannel}
        onWatchInMain={async (key) => {
          const entry = channels.find((c) => entryKey(c) === key);
          if (!entry) return;
          await watchInMain(entry);
          // Drop the tab — main now owns this channel. Popout's auto-hide-main
          // event won't suppress main's chat anymore for it.
          removeChannel(key);
        }}
        onMoveToNewWindow={async (key) => {
          const entry = channels.find((c) => entryKey(c) === key);
          if (!entry || (entry.provider ?? 'twitch') !== 'twitch') return;
          await openMultiChatWindow({
            newWindow: true,
            channel: entry.channel,
            channelId: entry.channelId ?? undefined,
            channelName: entry.channelName,
          });
          removeChannel(key);
        }}
        onNewWindow={() => void openMultiChatWindow({ newWindow: true })}
        onReorder={(source, target) => {
          setChannels((prev) => {
            const fromIdx = prev.findIndex((c) => entryKey(c) === source);
            const toIdx = prev.findIndex((c) => entryKey(c) === target);
            if (fromIdx === -1 || toIdx === -1 || fromIdx === toIdx) return prev;
            const next = [...prev];
            const [moved] = next.splice(fromIdx, 1);
            // Insert before the target's new index. After splicing the source
            // out, the target's index shifts left by 1 if source was earlier
            // in the list — compensate so the drop lands where the user
            // visually aimed.
            const insertAt = fromIdx < toIdx ? toIdx - 1 : toIdx;
            next.splice(insertAt, 0, moved);
            return next;
          });
        }}
        onAddClick={() => {
          setShowAdd(true);
          setAddError(null);
        }}
        goLiveHasProfile={goLiveExists}
        goLiveCanSave={channels.length > 0}
        goLiveLive={live}
        goLiveOpenChannels={channels}
        onGoLiveLoad={applyGoLive}
        onGoLiveSave={saveGoLive}
        onGoLiveProfileChanged={refreshGoLive}
        onGoLiveStartSetup={startGoLiveSetup}
        onGoLiveExit={exitLive}
      />
      <AnimatePresence initial={false}>
        {showAdd && (
          <motion.div
            key="add-panel"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.18, ease: 'easeOut' }}
            style={{ overflow: 'hidden' }}
          >
            <AddChannelPanel
              value={addInput}
              onChange={setAddInput}
              provider={addProvider}
              onProviderChange={(p) => {
                setAddProvider(p);
                setAddError(null);
              }}
              onCancel={() => {
                setShowAdd(false);
                setAddInput('');
                setAddError(null);
              }}
              onSubmit={() => void addChannel(addInput, addProvider)}
              onSelectStream={(stream) =>
                void addChannel(stream.user_login, 'twitch', stream.user_name)
              }
              alreadyAdded={channels.map((c) => entryKey(c))}
              error={addError}
              busy={addBusy}
            />
          </motion.div>
        )}
      </AnimatePresence>
      <div className="flex min-h-0 flex-1 flex-col">
        {moderatedTwitch.length > 0 && (
          <div className="flex items-center gap-2 border-b border-borderSubtle px-2 py-1.5">
            <div
              className="relative flex items-center rounded-full p-0.5"
              style={{ background: 'rgba(255,255,255,0.06)', boxShadow: 'inset 0 0 0 1px rgba(255,255,255,0.07)' }}
            >
              {(
                [
                  { key: false, label: 'Chat' },
                  { key: true, label: 'Mods' },
                ] as const
              ).map((seg) => (
                <button
                  key={String(seg.key)}
                  type="button"
                  onClick={() => setModsMode(seg.key)}
                  className={`relative rounded-full px-3 py-[3px] text-[11px] font-semibold transition-colors ${modsMode === seg.key ? 'text-textPrimary' : 'text-textSecondary hover:text-textPrimary'}`}
                >
                  {modsMode === seg.key && (
                    <motion.span
                      layoutId="mc-modroom-toggle"
                      className="absolute inset-0 rounded-full bg-white/[0.13]"
                      style={{ boxShadow: 'inset 0 0 0 1px rgba(255,255,255,0.09)' }}
                      transition={{ type: 'spring', stiffness: 480, damping: 28 }}
                    />
                  )}
                  <span className="relative z-10">{seg.label}</span>
                  {/* Aggregate unread across every moderated room; mentions turn it red. */}
                  {seg.key === true && !modsMode && modUnreadTotal.unread > 0 && (
                    <span
                      className={`relative z-10 ml-1 inline-flex min-w-[14px] items-center justify-center rounded-full px-1 text-[8px] font-bold leading-[14px] ${
                        modUnreadTotal.mentions > 0 ? 'bg-red-500/85 text-white' : 'bg-accent/25 text-accent'
                      }`}
                    >
                      {modUnreadTotal.unread > 9 ? '9+' : modUnreadTotal.unread}
                    </span>
                  )}
                </button>
              ))}
            </div>
            {modsMode && moderatedTwitch.length > 1 && (
              <div className="relative">
                <button
                  type="button"
                  onClick={() => setPickerOpen((v) => !v)}
                  className="flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs text-textPrimary transition-colors hover:bg-surface-hover"
                  style={{ background: 'rgba(255,255,255,0.05)', boxShadow: 'inset 0 0 0 1px rgba(255,255,255,0.07)' }}
                >
                  {modChannelEntry && avatarMap[modChannelEntry.channel] && (
                    <img src={avatarMap[modChannelEntry.channel]} alt="" className="h-4 w-4 shrink-0 rounded-full object-cover" />
                  )}
                  <span className="max-w-[140px] truncate">{modChannelEntry?.channelName ?? 'Select channel'}</span>
                  <svg
                    width="10"
                    height="10"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2.5"
                    className={`shrink-0 transition-transform ${pickerOpen ? 'rotate-180' : ''}`}
                  >
                    <path strokeLinecap="round" strokeLinejoin="round" d="M6 9l6 6 6-6" />
                  </svg>
                </button>
                {pickerOpen && (
                  <>
                    <div className="fixed inset-0 z-40" onClick={() => setPickerOpen(false)} />
                    <div
                      className="absolute left-0 top-full z-50 mt-1 min-w-[180px] overflow-hidden rounded-lg border border-borderSubtle py-1 shadow-lg"
                      style={{ background: 'rgba(18,18,20,0.98)' }}
                    >
                      {moderatedTwitch.map((c) => {
                        const active = c.channel === modChannelEntry?.channel;
                        return (
                          <button
                            key={c.channel}
                            type="button"
                            onClick={() => {
                              setModChannel(c.channel);
                              setPickerOpen(false);
                            }}
                            className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs transition-colors hover:bg-surface-hover ${active ? 'text-accent' : 'text-textPrimary'}`}
                          >
                            {avatarMap[c.channel] ? (
                              <img src={avatarMap[c.channel]} alt="" className="h-5 w-5 shrink-0 rounded-full object-cover" />
                            ) : (
                              <span className="h-5 w-5 shrink-0 rounded-full bg-white/10" />
                            )}
                            <span className="flex-1 truncate">{c.channelName}</span>
                            {(() => {
                              const u = c.channelId ? modSessions[c.channelId]?.unread ?? 0 : 0;
                              const mn = c.channelId ? modSessions[c.channelId]?.mentions ?? 0 : 0;
                              return u > 0 ? (
                                <span
                                  className={`inline-flex min-w-[14px] shrink-0 items-center justify-center rounded-full px-1 text-[8px] font-bold leading-[14px] ${
                                    mn > 0 ? 'bg-red-500/85 text-white' : 'bg-accent/25 text-accent'
                                  }`}
                                >
                                  {u > 9 ? '9+' : u}
                                </span>
                              ) : null;
                            })()}
                            {active && (
                              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" className="shrink-0">
                                <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
                              </svg>
                            )}
                          </button>
                        );
                      })}
                    </div>
                  </>
                )}
              </div>
            )}
            {modsMode && (
              <span className="ml-auto flex items-center gap-2">
                {modStatus.connected && (
                  <Tooltip
                    side="bottom"
                    content={
                      modStatus.members.length > 0 ? (
                        <div className="flex flex-col gap-1 py-0.5">
                          {modStatus.members.map((mm) => (
                            <span key={mm.userId} className="flex items-center gap-1.5 text-xs">
                              <span
                                className={`h-1.5 w-1.5 shrink-0 rounded-full ${mm.active !== false ? 'bg-accent' : 'bg-white/25'}`}
                              />
                              <span className={mm.role === 'broadcaster' ? 'text-[#f0c674]' : 'text-textPrimary'}>
                                {mm.login}
                              </span>
                            </span>
                          ))}
                        </div>
                      ) : (
                        'Just you'
                      )
                    }
                  >
                    <span className="cursor-default text-[11px] text-textSecondary">
                      <span className="font-semibold text-textPrimary">{modStatus.memberCount || 1}</span> in the room
                    </span>
                  </Tooltip>
                )}
                {modStatus.encrypted && (
                  <span
                    className="inline-flex select-none items-center gap-1 rounded-full px-2 py-[3px] text-[11px] font-semibold text-emerald-300"
                    style={{
                      background: 'linear-gradient(180deg, rgba(16,185,129,0.16), rgba(16,185,129,0.08))',
                      boxShadow: 'inset 0 0 0 1px rgba(16,185,129,0.28), inset 0 1px 0 rgba(255,255,255,0.06)',
                    }}
                  >
                    <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                      <rect x="3" y="11" width="18" height="11" rx="2" />
                      <path d="M7 11V7a5 5 0 0 1 10 0v4" />
                    </svg>
                    Encrypted
                  </span>
                )}
              </span>
            )}
          </div>
        )}
        <div className="relative flex min-h-0 flex-1">
          {modsMode && modChannelEntry?.channelId ? (
            <ModRoomPane
              channelId={modChannelEntry.channelId}
              channelLogin={modChannelEntry.channel}
              emotes={modEmotes}
              onStatus={setModStatus}
            />
          ) : channels.length === 0 ? (
            <EmptyState
              onAddClick={() => setShowAdd(true)}
              hasGoLive={goLiveExists}
              onApplyGoLive={applyGoLive}
              onStartSetup={startGoLiveSetup}
            />
          ) : isMentionsMode ? (
            // Mentions feed: every open source, only the rows the Rust rule
            // engine stamped as a mention, a reply to us, or a highlight match.
            <ErrorBoundary
              componentName="Mentions feed"
              resetKeys={[channels.map((c) => `${c.provider ?? 'twitch'}:${c.channel}`).join('|')]}
            >
              <BlendedChatPane channels={channels} mode="mentions" />
            </ErrorBoundary>
          ) : isBlendedMode ? (
            // Blended mode: every open source merged into one time-ordered feed
            // with a per-message provider stripe. Ignores tabs/split layout. Wrapped so
            // a render fault in the merged pane degrades to a reloadable panel instead
            // of unwinding to the root and taking every chat (and the whole window) down.
            <ErrorBoundary
              componentName="Combined chat"
              resetKeys={[channels.map((c) => `${c.provider ?? 'twitch'}:${c.channel}`).join('|')]}
            >
              <BlendedChatPane channels={channels} />
            </ErrorBoundary>
          ) : layoutTree ? (
            // Custom splits: a binary tree of panes (utils/splitTree). Leaves
            // may repeat a channel; a null leaf follows the active tab.
            <SplitLayout
              tree={layoutTree}
              onChange={(next) => setLayoutTree(next)}
              options={channels.map((c) => ({ value: entryKey(c), label: c.channelName || c.channel }))}
              activeLabel="Active tab"
              focusedLeafId={effectiveFocusedLeafId}
              onFocusLeaf={setFocusedLeafId}
              renderLeaf={(l) => {
                const key = l.key ?? activeKey;
                const entry = channels.find((c) => entryKey(c) === key) ?? channels[0];
                if (!entry) return null;
                const k = entryKey(entry);
                return (
                  <ErrorBoundary componentName={`${entry.channelName} chat`} resetKeys={[l.id, k]}>
                    <MultiChatPane
                      channel={entry.channel}
                      channelId={entry.channelId}
                      channelName={entry.channelName}
                      provider={entry.provider}
                      isActive={l.id === effectiveFocusedLeafId}
                      filterId={entry.filterId ?? null}
                      onFilterIdChange={(id) => {
                        setChannels((prev) =>
                          prev.map((e) => (entryKey(e) === k ? { ...e, filterId: id } : e)),
                        );
                      }}
                    />
                  </ErrorBoundary>
                );
              }}
            />
          ) : visibleChannels.length === 0 ? null : (
            // Render the visible channels side-by-side. Tabs mode renders one;
            // split modes render 2–4 columns. Inactive/hidden channels stay
            // reference-counted-acquired on the chatConnectionStore (via the
            // window-level effect above) so their messages keep flowing in the
            // background; switching tabs swaps which channel renders without
            // re-JOINing IRC.
            visibleChannels.map((entry, idx) => (
              <div
                key={entryKey(entry)}
                className={`min-w-0 flex-1 ${idx > 0 ? 'border-l border-borderSubtle' : ''}`}
              >
                <ErrorBoundary componentName={`${entry.channelName} chat`} resetKeys={[entryKey(entry)]}>
                  <MultiChatPane
                    channel={entry.channel}
                    channelId={entry.channelId}
                    channelName={entry.channelName}
                    provider={entry.provider}
                    isActive={entryKey(entry) === activeKey}
                    filterId={entry.filterId ?? null}
                    onFilterIdChange={(id) => {
                      const key = entryKey(entry);
                      setChannels((prev) =>
                        prev.map((e) => (entryKey(e) === key ? { ...e, filterId: id } : e)),
                      );
                    }}
                  />
                </ErrorBoundary>
              </div>
            ))
          )}
        </div>

        {/* Mod-logs pane stacked below the chat. Its top edge is a 1px hairline
            that doubles as a drag handle for resizing. The pane already filters
            its entries to channels with an open chat, so it shows this popout's
            channels; the moderation socket + IRC CLEARCHAT/CLEARMSG feed it in
            every window. Gated on the window-local toggle and on having at least
            one channel open. */}
        {showModLogs && channels.length > 0 && (
          <div
            className="flex flex-shrink-0 flex-col overflow-hidden"
            style={{ height: modLogsHeight }}
          >
            <Tooltip content="Drag to resize mod logs">
            <div
              onMouseDown={startModLogsResize}
              className="group flex h-1 w-full flex-shrink-0 cursor-ns-resize items-center justify-center"
            >
              <div className="h-px w-full bg-borderLight transition-colors group-hover:bg-accent" />
            </div>
            </Tooltip>
            <div className="min-h-0 flex-1 overflow-hidden">
              <ModLogsWidget forceShow channelLabels={modLogChannelLabels} />
            </div>
          </div>
        )}
        {/* Activity feed pane, stacked below chat (and mod logs if both are on).
            MultiChat-only; shows non-chat events for this window's sources. */}
        {showActivityFeed && channels.length > 0 && (
          <div
            className="flex flex-shrink-0 flex-col overflow-hidden"
            style={{ height: activityHeight }}
          >
            <Tooltip content="Drag to resize activity">
              <div
                onMouseDown={startActivityResize}
                className="group flex h-1 w-full flex-shrink-0 cursor-ns-resize items-center justify-center"
              >
                <div className="h-px w-full bg-borderLight transition-colors group-hover:bg-accent" />
              </div>
            </Tooltip>
            <div className="min-h-0 flex-1 overflow-hidden">
              <ActivityFeedWidget
                activeKeys={activityKeys}
                currentKey={activeKey}
                blended={isBlendedMode}
                sources={modLogChannelLabels}
              />
            </div>
          </div>
        )}
      </div>

      {/* Tooltips render via portal to document.body, but they need a
          TooltipManager instance mounted in *this* window's React tree to
          observe the (window-local) TooltipStore. Without this mount,
          hovering a chat badge or any other tooltip-bearing element in the
          popout produces no UI. */}
      <TooltipManager />
      {/* Same reason as TooltipManager: the popout is its own React tree, so
          without a mount here right-clicking the composer falls through to the
          OS menu instead of StreamNook's (cut/copy/paste + spelling). */}
      <InputContextMenuHost />
      <CommandPalette />
      <ClipModal />
      <VodModal />
      {/* Load ui plugins in this window too: the mod-logs pane consumes their
          dock-slot contributions (e.g. a docked reference-list column). */}
      <PluginUiHost />
    </div>
  );
}

// The permanent "Go Live" control on the LEFT of the add-sources (tab) bar. A
// split pill: the main button one-click LOADS the saved setup (swap to exactly
// those sources + activity filters, in blended view). The red "Live Chat" state is
// DERIVED (shown when the open sources already match the saved setup), not toggled;
// the caret opens a menu to load / save-or-update / edit sources.
function GoLiveControl({
  hasProfile,
  canSave,
  live,
  openChannels,
  onLoad,
  onSave,
  onProfileChanged,
  onStartSetup,
  onExitLive,
}: {
  hasProfile: boolean;
  canSave: boolean;
  live: boolean;
  openChannels: ChannelEntry[];
  onLoad: () => void;
  onSave: () => void;
  onProfileChanged: () => void;
  onStartSetup: () => void;
  onExitLive: () => void;
}) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const [view, setView] = useState<'menu' | 'edit'>('menu');
  const [savedSources, setSavedSources] = useState<ChannelEntry[]>([]);
  const openMenu = () => {
    const r = wrapRef.current?.getBoundingClientRect();
    if (r) setPos({ top: r.bottom + 6, left: r.left });
    setView('menu');
    setOpen((v) => !v);
  };
  const openEdit = () => {
    setSavedSources(readGoLiveSources());
    setView('edit');
  };
  const removeSource = (key: string) => {
    const next = savedSources.filter((s) => entryKey(s) !== key);
    writeGoLiveSources(next);
    setSavedSources(next);
    onProfileChanged();
  };
  const addSource = (c: ChannelEntry) => {
    if (savedSources.some((s) => entryKey(s) === entryKey(c))) return;
    const next = [...savedSources, c];
    writeGoLiveSources(next);
    setSavedSources(next);
    onProfileChanged();
  };
  const addable = openChannels.filter((c) => !savedSources.some((s) => entryKey(s) === entryKey(c)));
  // One-click primary: load when a setup exists; on a first-ever click (no setup
  // yet) kick off the guided setup; when already live, open the menu.
  const primary = () => {
    // Live (main app closed) -> clicking brings the main app back (exit standalone).
    // Not live -> go live: load the saved setup if there is one, else first-time setup.
    // Both load/setup close the main app. The caret still opens the save/edit menu.
    if (live) {
      onExitLive();
      return;
    }
    if (hasProfile) onLoad();
    else onStartSetup();
  };
  const tone = live
    ? 'text-red-400 hover:bg-red-500/15'
    : 'text-textSecondary hover:bg-white/10 hover:text-accent';
  return (
    <div
      ref={wrapRef}
      data-tauri-drag-region="false"
      // Same glass treatment as the channel tabs + add button it sits beside, so
      // the live control reads as part of the strip instead of a flat one-off.
      // Live tints the whole pill red. No live backdrop-blur (compositing fix).
      className={`glass-button flex flex-shrink-0 items-center overflow-hidden ${live ? '!bg-red-500/15' : ''}`}
      style={{
        borderRadius: '8px',
        backdropFilter: 'none',
        WebkitBackdropFilter: 'none',
        transition: 'color 0.2s ease, box-shadow 0.2s ease',
      }}
    >
      <Tooltip
        content={
          live
            ? "You're live (main app closed). Click to bring it back"
            : hasProfile
              ? 'Go live: load my sources + close the main app'
              : 'Set up your Go Live channels'
        }
      >
        <button
          type="button"
          onClick={primary}
          className={`flex items-center gap-1.5 py-1 pl-2.5 pr-1.5 text-xs font-semibold transition-colors ${tone}`}
          style={{ backdropFilter: 'none', WebkitBackdropFilter: 'none' }}
        >
          {live ? (
            <span className="relative flex h-2 w-2">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-red-500 opacity-60" />
              <span className="relative inline-flex h-2 w-2 rounded-full bg-red-500" />
            </span>
          ) : (
            <svg width={14} height={14} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
              <circle cx="12" cy="12" r="2" />
              <path d="M16.24 7.76a6 6 0 0 1 0 8.49M7.76 16.24a6 6 0 0 1 0-8.49M19.07 4.93a10 10 0 0 1 0 14.14M4.93 19.07a10 10 0 0 1 0-14.14" />
            </svg>
          )}
          {live ? 'Live Chat' : 'Go Live'}
        </button>
      </Tooltip>
      <Tooltip content="Go Live options">
        <button
          type="button"
          onClick={openMenu}
          aria-label="Go Live options"
          className={`flex items-center py-1 pl-1 pr-1.5 transition-colors ${tone}`}
          style={{ backdropFilter: 'none', WebkitBackdropFilter: 'none' }}
        >
          <svg width={12} height={12} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.5} strokeLinecap="round" strokeLinejoin="round">
            <path d="M6 9l6 6 6-6" />
          </svg>
        </button>
      </Tooltip>
      {open && pos && createPortal(
        <>
          <div className="fixed inset-0 z-[1000]" onClick={() => setOpen(false)} />
          <div
            className="glass-panel fixed z-[1001] w-56 rounded-lg border border-borderLight p-1 shadow-xl"
            // Opaque themed surface: a live backdrop-blur flickers over the chat.
            style={{ top: pos.top, left: pos.left, backgroundColor: 'var(--color-background-tertiary)' }}
          >
            {view === 'edit' ? (
              <div>
                <div className="mb-1 flex items-center gap-2 px-1">
                  <button
                    type="button"
                    onClick={() => setView('menu')}
                    aria-label="Back"
                    className="text-textMuted transition-colors hover:text-white"
                  >
                    <svg width={14} height={14} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.5} strokeLinecap="round" strokeLinejoin="round">
                      <path d="M15 18l-6-6 6-6" />
                    </svg>
                  </button>
                  <span className="text-[10px] font-semibold uppercase tracking-wide text-textMuted">
                    Edit sources
                  </span>
                </div>
                {savedSources.length === 0 ? (
                  <div className="px-2 py-2 text-[11px] text-textMuted">No sources saved yet.</div>
                ) : (
                  savedSources.map((s) => (
                    <div key={entryKey(s)} className="flex items-center gap-2 rounded px-2 py-1.5 text-xs">
                      <ProviderLogo provider={s.provider ?? 'twitch'} size={14} />
                      <span className="min-w-0 flex-1 truncate text-textSecondary">{s.channelName || s.channel}</span>
                      <button
                        type="button"
                        onClick={() => removeSource(entryKey(s))}
                        aria-label="Remove"
                        className="flex-shrink-0 text-textMuted transition-colors hover:text-red-400"
                      >
                        <svg width={13} height={13} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
                          <path d="M18 6 6 18M6 6l12 12" />
                        </svg>
                      </button>
                    </div>
                  ))
                )}
                {addable.length > 0 ? (
                  <>
                    <div className="mt-1 px-2 pb-0.5 pt-1.5 text-[10px] font-semibold uppercase tracking-wide text-textMuted">
                      Add from open chats
                    </div>
                    {addable.map((c) => (
                      <button
                        key={entryKey(c)}
                        type="button"
                        onClick={() => addSource(c)}
                        className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs text-textSecondary transition-colors hover:bg-white/5"
                      >
                        <ProviderLogo provider={c.provider ?? 'twitch'} size={14} />
                        <span className="min-w-0 flex-1 truncate">{c.channelName || c.channel}</span>
                        <svg width={13} height={13} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" className="flex-shrink-0">
                          <path d="M12 5v14M5 12h14" />
                        </svg>
                      </button>
                    ))}
                  </>
                ) : (
                  <div className="px-2 py-1.5 text-[10px] leading-relaxed text-textMuted">
                    Open a channel with the + button to add it to your setup.
                  </div>
                )}
              </div>
            ) : (
              <>
                <div className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wide text-textMuted">
                  Go Live setup
                </div>
                <button
                  type="button"
                  disabled={!hasProfile}
                  onClick={() => {
                    onLoad();
                    setOpen(false);
                  }}
                  className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs text-textSecondary transition-colors hover:bg-white/5 disabled:cursor-default disabled:opacity-40 disabled:hover:bg-transparent"
                >
                  <svg width={13} height={13} viewBox="0 0 24 24" fill="currentColor">
                    <path d="M8 5v14l11-7z" />
                  </svg>
                  Load my sources
                </button>
                <button
                  type="button"
                  disabled={!canSave}
                  onClick={() => {
                    onSave();
                    setOpen(false);
                  }}
                  className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs text-textSecondary transition-colors hover:bg-white/5 disabled:cursor-default disabled:opacity-40 disabled:hover:bg-transparent"
                >
                  <svg width={13} height={13} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
                    <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z" />
                    <polyline points="17 21 17 13 7 13 7 21" />
                    <polyline points="7 3 7 8 15 8" />
                  </svg>
                  {hasProfile ? 'Update with current setup' : 'Save current as my setup'}
                </button>
                <button
                  type="button"
                  disabled={!hasProfile && openChannels.length === 0}
                  onClick={openEdit}
                  className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs text-textSecondary transition-colors hover:bg-white/5 disabled:cursor-default disabled:opacity-40 disabled:hover:bg-transparent"
                >
                  <svg width={13} height={13} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
                    <path d="M12 20h9" />
                    <path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z" />
                  </svg>
                  Edit sources
                </button>
              </>
            )}
          </div>
        </>,
        document.body,
      )}
    </div>
  );
}

// ---------- TitleBar ---------------------------------------------------------

interface TitleBarProps {
  heading: string;
  layoutMode: LayoutMode;
  canSplit: boolean;
  canRestore: boolean;
  showModLogs: boolean;
  showActivityFeed: boolean;
  onLayoutModeChange: (mode: LayoutMode) => void;
  isBlendedMode: boolean;
  onToggleBlended: () => void;
  isMentionsMode: boolean;
  onToggleMentions: () => void;
  isSplitMode: boolean;
  onToggleSplits: () => void;
  onRestore: () => void;
  onClose: () => void;
  onMinimize: () => void;
  onMaximize: () => void;
  onOpenSettings: () => void;
  onToggleModLogs: () => void;
  onToggleActivityFeed: () => void;
  onOpenMainApp: () => void;
  channels: ChannelEntry[];
}

function TitleBar({
  heading,
  layoutMode,
  isBlendedMode,
  onToggleBlended,
  isMentionsMode,
  onToggleMentions,
  isSplitMode,
  onToggleSplits,
  canSplit,
  canRestore,
  showModLogs,
  showActivityFeed,
  onLayoutModeChange,
  onRestore,
  onClose,
  onMinimize,
  onMaximize,
  onOpenSettings,
  onToggleModLogs,
  onToggleActivityFeed,
  onOpenMainApp,
  channels,
}: TitleBarProps) {
  // Track maximized state so the corner-control icon swaps between
  // CornersOut (maximize) and CornersIn (restore) — same behavior as
  // the main app's TitleBar.
  const [isMaximized, setIsMaximized] = useState(false);
  useEffect(() => {
    let unlistenResize: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        const win = getCurrentWindow();
        const maximized = await win.isMaximized();
        if (!cancelled) setIsMaximized(maximized);
        const unlisten = await win.onResized(async () => {
          if (cancelled) return;
          setIsMaximized(await win.isMaximized());
        });
        unlistenResize = unlisten;
      } catch (err) {
        Logger.warn('[MultiChatWindow] isMaximized tracking failed:', err);
      }
    })();
    return () => {
      cancelled = true;
      if (unlistenResize) unlistenResize();
    };
  }, []);

  return (
    <div
      data-tauri-drag-region
      // No `backdrop-blur` here: the title bar sits over an opaque window so the
      // blur has nothing useful to sample, but Chromium/WebView2 smears the
      // scrolling chat near its bottom edge into that live blur layer (content
      // "bleeding" onto the title bar). Solid bg-secondary, no compositing layer.
      className="relative z-50 flex h-[40px] select-none items-center justify-between border-b border-borderSubtle bg-secondary px-3"
    >
      <div
        data-tauri-drag-region
        className="pointer-events-none flex flex-shrink-0 items-center gap-2"
      >
        <img
          src={streamNookLogoUrl}
          alt="StreamNook"
          className="h-4 w-4 object-contain"
          draggable={false}
        />
        <span className="truncate text-xs font-semibold tracking-wide text-textSecondary">
          {heading}
        </span>
      </div>

      <div data-tauri-drag-region className="flex flex-1 items-center justify-center">
        <ViewerCounter channels={channels} />
      </div>

      {canSplit && (
        <div
          className="titlebar-icon-group mr-2 gap-0.5"
          data-tauri-drag-region="false"
        >
          <LayoutToggleButton
            label="Mentions: only messages that mention you, reply to you, or match a highlight"
            active={isMentionsMode}
            onClick={onToggleMentions}
          >
            <span className="text-[12px] font-bold leading-none">@</span>
          </LayoutToggleButton>
          <LayoutToggleButton
            label="Blend all sources into one feed"
            active={isBlendedMode}
            onClick={onToggleBlended}
          >
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <circle cx="9" cy="12" r="6" />
              <circle cx="15" cy="12" r="6" />
            </svg>
          </LayoutToggleButton>
          <span className="mx-0.5 h-4 w-px bg-borderSubtle" aria-hidden />
          <LayoutToggleButton
            label="Tabs"
            active={layoutMode === 1 && !isBlendedMode && !isMentionsMode && !isSplitMode}
            onClick={() => onLayoutModeChange(1)}
          >
            <LayoutIcon mode={1} />
          </LayoutToggleButton>
          <LayoutToggleButton
            label="2 columns"
            active={layoutMode === 2 && !isBlendedMode && !isSplitMode}
            onClick={() => onLayoutModeChange(2)}
          >
            <LayoutIcon mode={2} />
          </LayoutToggleButton>
          <LayoutToggleButton
            label="3 columns"
            active={layoutMode === 3 && !isBlendedMode && !isSplitMode}
            onClick={() => onLayoutModeChange(3)}
          >
            <LayoutIcon mode={3} />
          </LayoutToggleButton>
          <LayoutToggleButton
            label="4 columns"
            active={layoutMode === 4 && !isBlendedMode && !isSplitMode}
            onClick={() => onLayoutModeChange(4)}
          >
            <LayoutIcon mode={4} />
          </LayoutToggleButton>
          <LayoutToggleButton
            label="Custom splits: split any pane right or down, drag the dividers, show a channel twice"
            active={isSplitMode}
            onClick={onToggleSplits}
          >
            <svg className="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
              <rect x="3" y="3" width="18" height="18" rx="2" />
              <path d="M11 3v18M11 13h10" />
            </svg>
          </LayoutToggleButton>
        </div>
      )}

      <div className="flex items-center gap-2" style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}>
        {/* Grouped action icons — restore, mod logs, activity, settings — in the
            same glass pill the core app's title bar uses (`titlebar-icon-group`
            + `titlebar-icon-btn`), so the popout reads as first-class chrome
            instead of a looser one-off. Toggles flag their on-state with the
            shared accent treatment (`!text-accent !bg-accent/15`). */}
        <div className="titlebar-icon-group">
          <Tooltip content="Open main app" delay={200}>
            <button
              type="button"
              onClick={onOpenMainApp}
              data-tauri-drag-region="false"
              className="titlebar-icon-btn hover:!text-accent"
            >
              <House size={15} />
            </button>
          </Tooltip>
          {canRestore && (
            <Tooltip content="Restore to main app" delay={200}>
              <button
                type="button"
                onClick={onRestore}
                data-tauri-drag-region="false"
                className="titlebar-icon-btn hover:!text-accent"
              >
                <ArrowLineLeft size={15} />
              </button>
            </Tooltip>
          )}
          <Tooltip content={showModLogs ? 'Hide mod logs' : 'Show mod logs'} delay={200}>
            <button
              type="button"
              onClick={onToggleModLogs}
              data-tauri-drag-region="false"
              aria-pressed={showModLogs}
              className={`titlebar-icon-btn ${showModLogs ? 'is-active' : ''}`}
            >
              <ShieldCheck size={15} />
            </button>
          </Tooltip>
          <Tooltip content={showActivityFeed ? 'Hide activity' : 'Show activity'} delay={200}>
            <button
              type="button"
              onClick={onToggleActivityFeed}
              data-tauri-drag-region="false"
              aria-pressed={showActivityFeed}
              className={`titlebar-icon-btn ${showActivityFeed ? 'is-active' : ''}`}
            >
              <Activity size={15} />
            </button>
          </Tooltip>
          <Tooltip content="Chat settings" delay={200}>
            <button
              type="button"
              onClick={onOpenSettings}
              data-tauri-drag-region="false"
              className="titlebar-icon-btn settings-gear-btn"
            >
              <Settings size={15} />
            </button>
          </Tooltip>
        </div>

        {/* Window controls — kept adjacent but not grouped into a pill, matching
            the core app so they read as window chrome rather than app actions. */}
        <div className="flex items-center gap-1">
          <Tooltip content="Minimize" delay={200}>
            <button
              type="button"
              onClick={onMinimize}
              data-tauri-drag-region="false"
              className="titlebar-window-btn"
            >
              <Minus size={14} />
            </button>
          </Tooltip>
          <Tooltip content={isMaximized ? 'Restore' : 'Maximize'} delay={200}>
            <button
              type="button"
              onClick={onMaximize}
              data-tauri-drag-region="false"
              className="titlebar-window-btn"
            >
              {isMaximized ? <CornersIn size={14} /> : <CornersOut size={14} />}
            </button>
          </Tooltip>
          <Tooltip content="Close" delay={200}>
            <button
              type="button"
              onClick={onClose}
              data-tauri-drag-region="false"
              className="titlebar-window-btn titlebar-window-btn-close"
            >
              <X size={14} />
            </button>
          </Tooltip>
        </div>
      </div>
    </div>
  );
}

// ---------- Layout toggle ---------------------------------------------------

interface LayoutToggleButtonProps {
  label: string;
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}

function LayoutToggleButton({ label, active, onClick, children }: LayoutToggleButtonProps) {
  return (
    <Tooltip content={label}>
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      aria-pressed={active}
      data-tauri-drag-region="false"
      // Brighter resting color (textSecondary, not textMuted) + a real hover fill
      // so the layout/blend controls are clearly visible at rest; the engaged
      // state uses the same pressed-in accent trench as the icon-group toggles.
      className={`grid h-6 w-7 place-items-center rounded-md transition-all ${
        active
          ? 'bg-surface text-accent shadow-[inset_2px_2px_5px_-2px_rgba(0,0,0,0.55),inset_-2px_-2px_5px_-2px_rgba(255,255,255,0.09)]'
          : 'text-textSecondary hover:bg-white/[0.06] hover:text-textPrimary'
      }`}
    >
      {children}
    </button>
    </Tooltip>
  );
}

// Tiny icon glyph showing the column geometry for the toggle. Each variant is
// a 12x10 rectangle subdivided into N equal columns.
function LayoutIcon({ mode }: { mode: LayoutMode }) {
  const cols = mode;
  const width = 12;
  const height = 10;
  const colWidth = width / cols;
  return (
    <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} fill="none">
      <rect x="0.5" y="0.5" width={width - 1} height={height - 1} stroke="currentColor" strokeWidth="1" />
      {Array.from({ length: cols - 1 }, (_, i) => (
        <line
          key={i}
          x1={(i + 1) * colWidth}
          y1="0.5"
          x2={(i + 1) * colWidth}
          y2={height - 0.5}
          stroke="currentColor"
          strokeWidth="1"
        />
      ))}
    </svg>
  );
}

// ---------- TabStrip --------------------------------------------------------

interface TabStripProps {
  channels: ChannelEntry[];
  active: string | null;
  visibleSet: Set<string>;
  layoutMode: LayoutMode;
  /** Blended view is active: the merged feed ignores tabs, so no single tab is
   *  "the focused one" — render them all inactive so the strip doesn't look
   *  like several channels are pinned down at once. */
  isBlendedMode: boolean;
  onSelect: (channel: string) => void;
  onRemove: (channel: string) => void;
  onWatchInMain: (channel: string) => void;
  /** Move a Twitch tab into a brand-new MultiChat window. */
  onMoveToNewWindow: (channel: string) => void;
  /** Open an empty extra MultiChat window. */
  onNewWindow: () => void;
  onReorder: (sourceChannel: string, targetChannel: string) => void;
  onAddClick: () => void;
  goLiveHasProfile: boolean;
  goLiveCanSave: boolean;
  goLiveLive: boolean;
  goLiveOpenChannels: ChannelEntry[];
  onGoLiveLoad: () => void;
  onGoLiveSave: () => void;
  onGoLiveProfileChanged: () => void;
  onGoLiveStartSetup: () => void;
  onGoLiveExit: () => void;
}

interface TabContextMenuState {
  channel: string;
  x: number;
  y: number;
}

// Tab strip mirrors `MultiNookChatSwitcher`'s glass-button / glass-input
// pattern so the popout's channel switcher looks and feels identical to the
// in-app MultiNook switcher. Active tab: `glass-input` + accent text;
// inactive: `glass-button` + secondary text → primary on hover.
function TabStrip({
  channels,
  active,
  visibleSet,
  layoutMode,
  isBlendedMode,
  onSelect,
  onRemove,
  onWatchInMain,
  onMoveToNewWindow,
  onNewWindow,
  onReorder,
  onAddClick,
  goLiveHasProfile,
  goLiveCanSave,
  goLiveLive,
  goLiveOpenChannels,
  onGoLiveLoad,
  onGoLiveSave,
  onGoLiveProfileChanged,
  onGoLiveStartSetup,
  onGoLiveExit,
}: TabStripProps) {
  const [contextMenu, setContextMenu] = useState<TabContextMenuState | null>(null);
  const [dragOver, setDragOver] = useState<string | null>(null);

  // Close the context menu on outside click / Escape — matches the pattern
  // used by StreamContextMenu in the main app.
  useEffect(() => {
    if (!contextMenu) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setContextMenu(null);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [contextMenu]);

  return (
    <div className="flex flex-shrink-0 items-center gap-2 border-b border-borderSubtle bg-glass/30 px-3 py-2.5 portrait:py-3.5 shadow-sm backdrop-blur-sm">
      {/* Permanent, pinned-left Go Live control: one click loads the streamer's
          saved sources + filters (blended) and flips to a red "Live Chat". */}
      <GoLiveControl
        hasProfile={goLiveHasProfile}
        canSave={goLiveCanSave}
        live={goLiveLive}
        openChannels={goLiveOpenChannels}
        onLoad={onGoLiveLoad}
        onSave={onGoLiveSave}
        onProfileChanged={onGoLiveProfileChanged}
        onStartSetup={onGoLiveStartSetup}
        onExitLive={onGoLiveExit}
      />
      <div className="h-5 w-px flex-shrink-0 bg-borderSubtle" aria-hidden />
      <div className="flex min-w-0 flex-1 items-center gap-2 overflow-x-auto scrollbar-thin">
        <div className="flex min-w-max items-center gap-1.5">
        {channels.map((c) => {
          // Provider-namespaced identity — bare `c.channel` collides when the
          // same name is open on two providers (Twitch + Kick "TheBurntPeanut").
          const key = entryKey(c);
          // In tabs mode the single active tab is highlighted; in split mode
          // every channel currently rendered as a column shows as active so
          // the strip mirrors which channels are visible. Blended view focuses
          // none of them — the feed is merged — so all tabs read as inactive.
          const isActive =
            isBlendedMode
              ? false
              : layoutMode === 1
                ? key === active
                : visibleSet.has(key);
          // In split mode all visible tabs are "seen" continuously; in tabs
          // mode only the active tab is seen. Blended view shows every source's
          // messages in the merged feed, so all are "seen" (no unread badges).
          const isVisible =
            isBlendedMode
              ? true
              : layoutMode === 1
                ? key === active
                : visibleSet.has(key);
          return (
            <TabButton
              key={key}
              entry={c}
              isActive={isActive}
              isVisible={isVisible}
              isDragOver={dragOver === key}
              onSelect={() => onSelect(key)}
              onContextMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setContextMenu({ channel: key, x: e.clientX, y: e.clientY });
              }}
              onRemove={() => onRemove(key)}
              onDragStart={(e) => {
                e.dataTransfer.setData('application/multichat-tab', key);
                e.dataTransfer.effectAllowed = 'move';
              }}
              onDragOver={(e) => {
                if (e.dataTransfer.types.includes('application/multichat-tab')) {
                  e.preventDefault();
                  e.dataTransfer.dropEffect = 'move';
                  setDragOver(key);
                }
              }}
              onDragLeave={() => {
                setDragOver((prev) => (prev === key ? null : prev));
              }}
              onDrop={(e) => {
                e.preventDefault();
                const source = e.dataTransfer.getData('application/multichat-tab');
                setDragOver(null);
                if (source && source !== key) onReorder(source, key);
              }}
            />
          );
        })}
        <Tooltip content="New MultiChat window. Each window is its own renderer, about 150 MB; splits inside one window are free." side="bottom">
          <button
            type="button"
            onClick={onNewWindow}
            aria-label="New MultiChat window"
            className="glass-button flex h-8 w-8 portrait:h-9 portrait:w-9 items-center justify-center text-textSecondary hover:text-accent"
            style={{
              borderRadius: '8px',
              backdropFilter: 'none',
              WebkitBackdropFilter: 'none',
              transition: 'color 0.2s ease, box-shadow 0.2s ease, transform 0.2s ease',
            }}
          >
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <rect x="3" y="5" width="13" height="13" rx="2" />
              <path d="M8 5V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-1" />
            </svg>
          </button>
        </Tooltip>
        <button
          type="button"
          onClick={onAddClick}
          aria-label="Add channel"
          className="glass-button flex h-8 w-8 portrait:h-9 portrait:w-9 items-center justify-center text-textSecondary hover:text-accent"
          // Same glass blur-compositing fix as the tabs: no live backdrop-filter,
          // and transition only cheap properties (never `all`, which animates blur).
          style={{
            borderRadius: '8px',
            backdropFilter: 'none',
            WebkitBackdropFilter: 'none',
            transition: 'color 0.2s ease, box-shadow 0.2s ease, transform 0.2s ease',
          }}
        >
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
            <line x1="6" y1="2" x2="6" y2="10" />
            <line x1="2" y1="6" x2="10" y2="6" />
          </svg>
        </button>
        </div>
      </div>

      {contextMenu && (
        <TabContextMenu
          state={contextMenu}
          channelName={
            channels.find((c) => entryKey(c) === contextMenu.channel)?.channelName ||
            parseKey(contextMenu.channel).channel
          }
          onClose={() => setContextMenu(null)}
          onWatchInMain={() => {
            onWatchInMain(contextMenu.channel);
            setContextMenu(null);
          }}
          canMoveToNewWindow={
            (channels.find((c) => entryKey(c) === contextMenu.channel)?.provider ?? 'twitch') === 'twitch'
          }
          onMoveToNewWindow={() => {
            onMoveToNewWindow(contextMenu.channel);
            setContextMenu(null);
          }}
          onRemove={() => {
            onRemove(contextMenu.channel);
            setContextMenu(null);
          }}
        />
      )}
    </div>
  );
}

// ---------- TabButton -------------------------------------------------------

interface TabButtonProps {
  entry: ChannelEntry;
  isActive: boolean;
  isVisible: boolean;
  isDragOver: boolean;
  onSelect: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
  onRemove: () => void;
  onDragStart: (e: React.DragEvent) => void;
  onDragOver: (e: React.DragEvent) => void;
  onDragLeave: () => void;
  onDrop: (e: React.DragEvent) => void;
}

// Per-tab presentation. Owns its own "last seen mention count" so the unread
// badge lights up only when the signed-in user is @-mentioned in a tab that
// isn't currently visible (tabs mode: not the active tab; split mode: not a
// rendered column). Generic "new message" activity is intentionally not
// surfaced — too noisy for chats with high volume.
function TabButton({
  entry,
  isActive,
  isVisible,
  isDragOver,
  onSelect,
  onContextMenu,
  onRemove,
  onDragStart,
  onDragOver,
  onDragLeave,
  onDrop,
}: TabButtonProps) {
  const currentUserLogin = useAppStore((s) => s.currentUser?.login ?? null);
  const mentionCount = useChannelMentionCount(entry.channel, currentUserLogin);
  // Track the mention count as of when this tab was last visible. Seeded at mount
  // so the IVR backfill doesn't light the badge on creation. We ADJUST state during
  // render (React's supported alternative to an effect for syncing state to a prop)
  // so there's no cascading-render lint and no ref access during render: while
  // visible the baseline tracks the live count so unread stays 0; while hidden it
  // freezes so mentions accumulate until the tab is seen again.
  const [lastSeen, setLastSeen] = useState(mentionCount);
  if (isVisible && lastSeen !== mentionCount) setLastSeen(mentionCount);
  const unread = Math.max(0, mentionCount - lastSeen);
  const unreadLabel = unread > 99 ? '99+' : String(unread);

  return (
    <div
      className={`group relative transition-opacity ${
        isDragOver ? 'opacity-60' : ''
      }`}
      // Drop target only — drag-source role is on the inner <button> below.
      // Splitting them avoids the interactive-element quirk where mousedown
      // on a button is treated as click intent instead of starting a drag.
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      <Tooltip content={entry.channelName || entry.channel}>
      <button
        type="button"
        draggable
        onDragStart={onDragStart}
        onClick={onSelect}
        onContextMenu={onContextMenu}
        className={`relative flex items-center justify-center px-7 py-2 portrait:py-2.5 text-xs portrait:text-sm font-bold tracking-wide ${
          isActive
            ? 'glass-input text-accent font-extrabold'
            : 'glass-button text-textSecondary hover:text-textPrimary'
        }`}
        // The glass classes apply a live `backdrop-filter: blur()`. Above the
        // scrolling chat, Chromium/WebView2 re-composites that blur as content
        // repaints behind the tab and produces intermittent seam/flicker artifacts
        // ("lines through the tabs"). Drop the live blur on the pills (keep the
        // glass color + bevel) to kill it, and only transition cheap properties.
        style={{
          borderRadius: '8px',
          userSelect: 'none',
          backdropFilter: 'none',
          WebkitBackdropFilter: 'none',
          transition: 'color 0.2s ease, box-shadow 0.2s ease, transform 0.2s ease',
        }}
      >
        <ProviderLogo
          provider={entry.provider ?? 'twitch'}
          size={14}
          className="absolute left-2 top-1/2 -translate-y-1/2"
        />
        <span className="truncate text-center">{entry.channelName || entry.channel}</span>
        {unread > 0 && !isVisible && (
          <span
            className="absolute right-7 top-1/2 grid h-4 min-w-[1rem] -translate-y-1/2 place-items-center rounded-full bg-accent px-1.5 text-[10px] font-bold leading-none text-background"
            aria-label={`${unread} unread message${unread === 1 ? '' : 's'}`}
          >
            {unreadLabel}
          </span>
        )}
        <span
          role="button"
          aria-label={`Remove ${entry.channel}`}
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
          className="absolute right-2 top-1/2 grid h-3.5 w-3.5 -translate-y-1/2 place-items-center rounded-full text-textMuted opacity-0 transition-all hover:bg-error/20 hover:text-error group-hover:opacity-100"
        >
          <svg width="8" height="8" viewBox="0 0 8 8" fill="none" stroke="currentColor" strokeWidth="1.5">
            <line x1="1.5" y1="1.5" x2="6.5" y2="6.5" />
            <line x1="6.5" y1="1.5" x2="1.5" y2="6.5" />
          </svg>
        </span>
      </button>
      </Tooltip>
      {isDragOver && (
        <span
          className="pointer-events-none absolute inset-y-0 -left-1 w-0.5 rounded-full bg-accent"
          aria-hidden
        />
      )}
    </div>
  );
}

// ---------- TabContextMenu --------------------------------------------------

interface TabContextMenuProps {
  state: TabContextMenuState;
  channelName: string;
  onClose: () => void;
  onWatchInMain: () => void;
  canMoveToNewWindow: boolean;
  onMoveToNewWindow: () => void;
  onRemove: () => void;
}

function TabContextMenu({
  state,
  channelName,
  onClose,
  onWatchInMain,
  canMoveToNewWindow,
  onMoveToNewWindow,
  onRemove,
}: TabContextMenuProps) {
  // Clamp position to viewport so the menu never opens partially off-screen
  // (matches StreamContextMenu's collision-detection pattern).
  const MENU_WIDTH = 200;
  const MENU_HEIGHT = 90;
  let x = state.x;
  let y = state.y;
  if (x + MENU_WIDTH > window.innerWidth) x = window.innerWidth - MENU_WIDTH - 4;
  if (y + MENU_HEIGHT > window.innerHeight) y = window.innerHeight - MENU_HEIGHT - 4;
  x = Math.max(4, x);
  y = Math.max(4, y);

  // Portal to document.body so the menu escapes the tab-strip's stacking
  // context. Without this, `z-[…]` is local to the strip's stacking context
  // and the menu can render UNDER taller-stack elements in sibling regions
  // (chat header, send input, etc.). Portaling lifts it to the document's
  // top stacking context where z-[9999] actually sits above everything.
  return createPortal(
    <div
      className="fixed inset-0 z-[9999] cursor-default"
      onPointerDown={(e) => {
        e.stopPropagation();
        onClose();
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }}
    >
      <div
        className="absolute w-48 glass-panel rounded-xl flex flex-col p-1 shadow-2xl animate-in fade-in zoom-in-95 duration-150"
        style={{ top: y, left: x }}
        onPointerDown={(e) => e.stopPropagation()}
      >
        <div className="px-3 py-2 border-b border-borderSubtle mb-1 text-[10px] uppercase tracking-wider text-textMuted">
          {channelName}
        </div>
        <button
          type="button"
          onClick={onWatchInMain}
          className="flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium text-textSecondary hover:text-accent hover:bg-glass-hover transition-all"
        >
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <polygon points="6 4 12 8 6 12 6 4" fill="currentColor" />
          </svg>
          <span>Watch in main app</span>
        </button>
        {canMoveToNewWindow && (
          <button
            type="button"
            onClick={onMoveToNewWindow}
            className="flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium text-textSecondary hover:text-accent hover:bg-glass-hover transition-all"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
              <rect x="3" y="5" width="13" height="13" rx="2" />
              <path d="M8 5V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-1" />
            </svg>
            <span>Move to new window</span>
          </button>
        )}
        <button
          type="button"
          onClick={onRemove}
          className="flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium text-textSecondary hover:text-error hover:bg-glass-hover transition-all"
        >
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <line x1="4" y1="4" x2="12" y2="12" />
            <line x1="12" y1="4" x2="4" y2="12" />
          </svg>
          <span>Close tab</span>
        </button>
      </div>
    </div>,
    document.body,
  );
}

// ---------- AddChannelPanel -------------------------------------------------
//
// Rich add-channel surface: live followed channels listed first (quick-add by
// click), with a search input that both filters the followed list AND lets
// the user add any arbitrary channel via Enter — even ones they don't follow
// or that aren't live.

interface AddChannelPanelProps {
  value: string;
  onChange: (next: string) => void;
  provider: ProviderId;
  onProviderChange: (p: ProviderId) => void;
  onSubmit: () => void;
  onCancel: () => void;
  onSelectStream: (stream: TwitchStream) => void;
  alreadyAdded: string[];
  error: string | null;
  busy: boolean;
}

// Providers selectable in the add panel today (read-supported). Twitch has rich
// live-following search; Kick + YouTube are add-by-name / by-link (no public
// search API to autocomplete). Derived from the chat flags so this list can't
// drift from what the adapters actually support.
const ADDABLE_PROVIDERS: ProviderId[] = CHAT_PROVIDERS;

// Extract a stable YouTube source identifier from a pasted link or typed value.
// Returns `@handle` for a channel (case-insensitive at YouTube) or a verbatim
// 11-char video id / UC… channel id (case-SENSITIVE — kept as-is; only the
// composite key is lowercased and the backend resolves with the original case).
function parseYouTubeInput(input: string): string | null {
  const s = input.trim();
  let m = s.match(/youtu\.be\/([A-Za-z0-9_-]{11})/);
  if (m) return m[1];
  if (/youtube\.com\/watch/i.test(s)) {
    m = s.match(/[?&]v=([A-Za-z0-9_-]{11})/);
    if (m) return m[1];
  }
  m = s.match(/youtube\.com\/live\/([A-Za-z0-9_-]{11})/i);
  if (m) return m[1];
  m = s.match(/youtube\.com\/channel\/(UC[A-Za-z0-9_-]{22})/i);
  if (m) return m[1];
  m = s.match(/youtube\.com\/@([A-Za-z0-9_.-]+)/i);
  if (m) return `@${m[1]}`;
  return null;
}

// Extract a TikTok handle from a pasted profile / LIVE link (link-only, like
// YouTube — a bare word only becomes TikTok when the dropdown picks TikTok, so it
// can't hijack a typed Twitch login). Returns the bare unique id (no @).
function parseTikTokInput(input: string): string | null {
  const m = input.trim().match(/tiktok\.com\/@([A-Za-z0-9_.]+)/i);
  return m ? m[1] : null;
}

function AddChannelPanel({
  value,
  onChange,
  provider,
  onProviderChange,
  onSubmit,
  onCancel,
  onSelectStream,
  alreadyAdded,
  error,
  busy,
}: AddChannelPanelProps) {
  const followedStreams = useAppStore((s) => s.followedStreams);
  const [providerMenuOpen, setProviderMenuOpen] = useState(false);
  // The dropdown is PORTALED to document.body so it escapes the add-bar's
  // backdrop-blur stacking context (otherwise z-index can't lift it above the chat
  // panes, and the cursor hits the chat first). Positioned fixed at the trigger.
  const providerTriggerRef = useRef<HTMLButtonElement>(null);
  const [providerMenuPos, setProviderMenuPos] = useState<{ top: number; left: number } | null>(null);

  // Profile images from Helix `/users` — `get_followed_streams` doesn't
  // include `profile_image_url`, so we batch-fetch by user id and cache here.
  // Matches the pattern in Sidebar.tsx.
  const [profileImages, setProfileImages] = useState<Map<string, string>>(new Map());
  const inflightRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    const needed = followedStreams
      .map((s) => s.user_id)
      .filter(
        (id) =>
          !!id &&
          !profileImages.has(id) &&
          !inflightRef.current.has(id),
      );
    if (needed.length === 0) return;
    const unique = Array.from(new Set(needed));
    unique.forEach((id) => inflightRef.current.add(id));

    (async () => {
      try {
        for (let i = 0; i < unique.length; i += 100) {
          const batch = unique.slice(i, i + 100);
          const query = batch.map((id) => `id=${encodeURIComponent(id)}`).join('&');
          let data: { data?: Array<{ id: string; profile_image_url: string }> };
          try {
            data = await helixGet('users', query);
          } catch {
            continue;
          }
          if (data.data && Array.isArray(data.data)) {
            setProfileImages((prev) => {
              const next = new Map(prev);
              for (const u of data.data!) {
                if (u.profile_image_url) next.set(u.id, u.profile_image_url);
              }
              return next;
            });
          }
        }
      } catch (err) {
        Logger.warn('[AddChannelPanel] profile image batch fetch failed:', err);
      } finally {
        unique.forEach((id) => inflightRef.current.delete(id));
      }
    })();
    // profileImages intentionally not in deps — the Set guards against
    // re-fetching ids we already have, and we don't want the effect to loop
    // when setProfileImages updates.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [followedStreams]);

  const query = value.trim().toLowerCase();
  const alreadyAddedSet = useMemo(() => new Set(alreadyAdded), [alreadyAdded]);
  const filtered = useMemo(() => {
    if (!query) return followedStreams;
    return followedStreams.filter((s) => {
      const login = s.user_login.toLowerCase();
      const name = (s.user_name || '').toLowerCase();
      return login.includes(query) || name.includes(query);
    });
  }, [followedStreams, query]);

  const exactFollowedMatch = useMemo(() => {
    if (!query) return null;
    return followedStreams.find((s) => s.user_login.toLowerCase() === query) ?? null;
  }, [followedStreams, query]);

  // Twitch matches the query against the live-following list; Kick has no public
  // search to autocomplete, so it's add-by-name (any non-empty query is addable).
  const isTwitch = provider === 'twitch';
  // Show the "add arbitrary channel" affordance for any query that doesn't exactly
  // match a followed channel (Twitch) — or any non-empty query (other providers).
  const showArbitraryAdd = isTwitch ? !!query && !exactFollowedMatch : query.length > 0;

  return (
    // Only Twitch shows the (tall, scrollable) live-following list, so cap + grow
    // there. The other providers are add-by-name: the panel sizes to its input row
    // + hint, instead of leaving a big blank area where the list would be.
    <div className={`flex flex-col border-b border-borderSubtle bg-secondary/40 backdrop-blur-sm ${isTwitch ? 'max-h-[60%]' : ''}`}>
      <div className="flex items-center gap-2 px-3 py-2">
        <div className="relative">
          <button
            ref={providerTriggerRef}
            type="button"
            onClick={() => {
              if (!providerMenuOpen) {
                const r = providerTriggerRef.current?.getBoundingClientRect();
                if (r) setProviderMenuPos({ top: r.bottom + 4, left: r.left });
              }
              setProviderMenuOpen((v) => !v);
            }}
            disabled={busy}
            aria-haspopup="listbox"
            aria-expanded={providerMenuOpen}
            aria-label="Source platform"
            className="flex items-center gap-1.5 rounded-md border border-borderSubtle bg-background py-1.5 pl-2.5 pr-2 text-xs font-semibold text-textPrimary transition-colors hover:border-accent focus:border-accent focus:outline-none disabled:opacity-60"
          >
            <ProviderLogo provider={provider} size={14} />
            <span>{PROVIDERS[provider].label}</span>
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" className="text-textMuted">
              <path d="M2.5 4L5 6.5L7.5 4" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </button>
          {providerMenuOpen &&
            providerMenuPos &&
            createPortal(
              <>
                {/* Backdrop catches the outside click; both are portaled to body and
                    fixed-positioned so they sit ABOVE the chat panes (the add-bar's
                    backdrop-blur would otherwise trap them below). Opaque themed
                    surface (not translucent glass): a live blur flickers over chat. */}
                <div className="fixed inset-0 z-[1000]" onClick={() => setProviderMenuOpen(false)} />
                <div
                  role="listbox"
                  className="fixed z-[1001] min-w-[7.5rem] overflow-hidden rounded-md border border-borderLight shadow-lg"
                  style={{ backgroundColor: 'var(--color-background-tertiary)', top: providerMenuPos.top, left: providerMenuPos.left }}
                >
                  {ADDABLE_PROVIDERS.map((p) => (
                    <button
                      key={p}
                      type="button"
                      role="option"
                      aria-selected={p === provider}
                      onClick={() => {
                        onProviderChange(p);
                        setProviderMenuOpen(false);
                      }}
                      className={`flex w-full items-center gap-2 px-3 py-1.5 text-xs font-semibold transition-colors hover:bg-white/10 ${
                        p === provider ? 'text-accent' : 'text-textPrimary'
                      }`}
                    >
                      <ProviderLogo provider={p} size={14} />
                      <span>{PROVIDERS[p].label}</span>
                    </button>
                  ))}
                </div>
              </>,
              document.body,
            )}
        </div>
        <input
          autoFocus
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') onSubmit();
            else if (e.key === 'Escape') onCancel();
          }}
          disabled={busy}
          placeholder={
            provider === 'twitch'
              ? 'Search your live following, or type any channel name'
              : `Type a ${PROVIDERS[provider].label} channel name`
          }
          className="flex-1 rounded-md border border-borderSubtle bg-background px-3 py-1.5 text-xs text-textPrimary placeholder:text-textMuted focus:border-accent focus:outline-none disabled:opacity-60"
        />
        <button
          type="button"
          onClick={onCancel}
          disabled={busy}
          className="glass-button px-2.5 py-1.5 text-xs text-textSecondary transition-colors hover:text-textPrimary"
          style={{ borderRadius: '8px' }}
        >
          Cancel
        </button>
      </div>

      {error && <div className="px-3 pb-1.5 text-[11px] text-error">{error}</div>}

      <div className={`overflow-y-auto scrollbar-thin px-1 pb-2 ${isTwitch ? 'min-h-0 flex-1' : ''}`}>
        {isTwitch && filtered.length === 0 && !showArbitraryAdd && (
          <div className="px-3 py-4 text-center text-[11px] text-textMuted">
            {followedStreams.length === 0
              ? 'No live channels in your following right now. Type a channel name and press Enter.'
              : `No follows match "${query}". Press Enter to add anyway.`}
          </div>
        )}

        {!isTwitch && !showArbitraryAdd && (
          <div className="px-3 py-4 text-center text-[11px] text-textMuted">
            Type a {PROVIDERS[provider].label} channel name and press Enter.
          </div>
        )}

        {isTwitch && filtered.map((stream) => {
          // Provider-scoped: only the TWITCH copy of this channel being open counts
          // as "added" — the same name on Kick/YouTube is a different chat.
          const isAdded = alreadyAddedSet.has(makeKey('twitch', stream.user_login));
          return (
            <button
              key={stream.user_id || stream.user_login}
              type="button"
              disabled={busy || isAdded}
              onClick={() => onSelectStream(stream)}
              className={`flex w-full items-center gap-3 rounded-md px-3 py-2 text-left transition-colors ${
                isAdded
                  ? 'cursor-not-allowed opacity-50'
                  : 'hover:bg-surface-hover'
              }`}
            >
              <StreamerAvatar
                stream={stream}
                profileImageUrl={
                  stream.profile_image_url || profileImages.get(stream.user_id) || null
                }
              />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="truncate text-sm font-semibold text-textPrimary">
                    {stream.user_name || stream.user_login}
                  </span>
                  {isAdded && (
                    <span className="rounded bg-surface px-1.5 py-0.5 text-[9px] uppercase tracking-wide text-textMuted">
                      Added
                    </span>
                  )}
                </div>
                <div className="truncate text-[11px] text-textSecondary">
                  {stream.game_name || 'Just chatting'}
                  {typeof stream.viewer_count === 'number' && (
                    <span className="ml-2 text-textMuted">
                      · {stream.viewer_count.toLocaleString()} viewers
                    </span>
                  )}
                </div>
              </div>
              <span className="flex items-center gap-1 text-[10px] font-semibold uppercase tracking-wide text-error">
                <span className="h-1.5 w-1.5 rounded-full bg-error animate-pulse" />
                Live
              </span>
            </button>
          );
        })}

        {showArbitraryAdd && (
          <button
            type="button"
            disabled={busy}
            onClick={onSubmit}
            className="mt-1 flex w-full items-center gap-3 rounded-md border border-dashed border-borderSubtle px-3 py-2 text-left transition-colors hover:bg-surface-hover hover:border-accent"
          >
            <div className="grid h-9 w-9 place-items-center rounded-full bg-surface text-textMuted">
              <svg width="14" height="14" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                <line x1="6" y1="2" x2="6" y2="10" />
                <line x1="2" y1="6" x2="10" y2="6" />
              </svg>
            </div>
            <div className="min-w-0 flex-1">
              <div className="truncate text-sm font-semibold text-textPrimary">
                Add {isTwitch ? `#${query}` : `${query}`}
                {!isTwitch && (
                  <span className="ml-1.5 text-[11px] font-semibold" style={{ color: PROVIDERS[provider].color }}>
                    on {PROVIDERS[provider].label}
                  </span>
                )}
              </div>
              <div className="truncate text-[11px] text-textSecondary">
                Open chat for this channel even if they're not in your following or not live.
              </div>
            </div>
          </button>
        )}
      </div>
    </div>
  );
}

// Renders the streamer's actual Twitch profile picture. NEVER falls back to
// `stream.thumbnail_url` (that's the live stream preview image, not a profile
// avatar). When the Helix batch fetch hasn't resolved yet — or the streamer
// has no profile image — we render an initial-letter placeholder over a
// muted surface tone instead.
function StreamerAvatar({
  stream,
  profileImageUrl,
}: {
  stream: TwitchStream;
  profileImageUrl: string | null;
}) {
  if (profileImageUrl) {
    return (
      <img
        src={profileImageUrl}
        alt=""
        className="h-9 w-9 rounded-full bg-surface object-cover"
        onError={(e) => {
          (e.currentTarget as HTMLImageElement).style.visibility = 'hidden';
        }}
      />
    );
  }
  return (
    <div className="grid h-9 w-9 place-items-center rounded-full bg-surface text-xs font-semibold text-textMuted">
      {(stream.user_login || '?').slice(0, 1).toUpperCase()}
    </div>
  );
}

// ---------- EmptyState ------------------------------------------------------

function EmptyState({
  onAddClick,
  hasGoLive,
  onApplyGoLive,
  onStartSetup,
}: {
  onAddClick: () => void;
  hasGoLive: boolean;
  onApplyGoLive: () => void;
  onStartSetup: () => void;
}) {
  const providers: ProviderId[] = ['twitch', 'kick', 'youtube', 'tiktok'];
  return (
    // Outer fills the chat area (w-full) so the card sits dead-center, not at the
    // flex-start left edge the old content-width layout produced.
    <div className="flex h-full w-full items-center justify-center px-6">
      <motion.div
        initial={{ opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ type: 'spring', stiffness: 320, damping: 26 }}
        className="glass-panel flex w-full max-w-sm flex-col items-center gap-5 rounded-2xl border border-borderSubtle px-8 py-10 text-center shadow-lg"
      >
        <img
          src={streamNookLogoUrl}
          alt="StreamNook"
          className="h-16 w-16 object-contain"
          draggable={false}
        />
        <div className="flex flex-col gap-1.5">
          <h2 className="text-xl font-bold tracking-tight text-textPrimary">All your chats, one place</h2>
          <p className="max-w-[300px] text-xs leading-relaxed text-textMuted">
            Read Twitch, Kick, YouTube, and TikTok chat side by side, or blend them into a single
            live feed.
          </p>
        </div>
        {/* The platforms at a glance — conveys the multi-platform point at a look. */}
        <div className="flex items-center gap-3.5 opacity-80">
          {providers.map((p) => (
            <ProviderLogo key={p} provider={p} size={20} />
          ))}
        </div>
        {/* Path 1: watch a stream. */}
        <button
          type="button"
          onClick={onAddClick}
          className="w-full rounded-md bg-accent px-4 py-2.5 text-sm font-semibold text-background transition-colors hover:bg-accent-hover"
        >
          + Add a stream
        </button>
        {/* Path 2: the streamer's own "Go Live" setup. With a saved profile, one click
            loads it; first time, it starts the guided setup (seeds your Twitch). */}
        <div className="w-full border-t border-borderSubtle pt-4">
          <div className="mb-1.5 text-[11px] font-semibold uppercase tracking-wide text-textMuted">
            About to stream?
          </div>
          <button
            type="button"
            onClick={hasGoLive ? onApplyGoLive : onStartSetup}
            className="flex w-full items-center justify-center gap-2 rounded-md bg-accent/15 px-4 py-2 text-sm font-semibold text-accent transition-colors hover:bg-accent/25"
          >
            <svg width={16} height={16} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
              <circle cx="12" cy="12" r="2" />
              <path d="M16.24 7.76a6 6 0 0 1 0 8.49M7.76 16.24a6 6 0 0 1 0-8.49M19.07 4.93a10 10 0 0 1 0 14.14M4.93 19.07a10 10 0 0 1 0-14.14" />
            </svg>
            Go Live
          </button>
          <div className="mt-1.5 text-[11px] leading-relaxed text-textMuted">
            {hasGoLive
              ? 'Loads your saved channels + filters, blended.'
              : "We'll start with your Twitch — add your other platforms, then save the setup."}
          </div>
        </div>
      </motion.div>
    </div>
  );
}

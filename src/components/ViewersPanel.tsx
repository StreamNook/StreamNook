// Viewers list panel: who is in the current channel's chat, grouped by role
// (Broadcaster / Moderators / VIPs / Viewers).
//
// Data comes from the `get_channel_chatters` Rust command: the full roster where
// the user moderates or owns the channel, otherwise Twitch's public list, which
// names every broadcaster, moderator and VIP but at most 100 viewers (with the
// full count), plus everyone seen chatting recently, so `truncated` is common
// on big channels.
//
// Visual language mirrors ChatSearchBar: an uppercase tracking label, a glass
// input with an inline count, and rounded rows.

import { useState, useEffect, useRef, useMemo, useCallback } from 'react';
import type { MouseEvent, ReactNode } from 'react';
import { VariableSizeList as List, type ListChildComponentProps } from 'react-window';
import { invoke } from '@tauri-apps/api/core';
import { ChevronDown, Gem, RefreshCw, Search, Sword, Users, Video, X } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { useChatUserStore } from '../stores/chatUserStore';
import { readableNameColor } from '../hooks/useNameColor';
import { useAppStore } from '../stores/AppStore';
import { Tooltip } from './ui/Tooltip';
import { Logger } from '../utils/logger';

export interface Chatter {
  user_id: string;
  user_login: string;
  user_name: string;
}

export interface ChannelChatters {
  broadcaster: Chatter[];
  moderators: Chatter[];
  vips: Chatter[];
  viewers: Chatter[];
  total: number;
  truncated: boolean;
  /** Twitch's own badge art per group (Rust, from the global badge set);
   *  null keeps the generic icon. */
  role_badges?: Partial<Record<RoleKey, string | null>>;
}

type RoleKey = 'broadcaster' | 'moderators' | 'vips' | 'viewers';

/** How many chatters the list actually names (fewer than `total` when truncated). */
const listedCount = (d: ChannelChatters): number =>
  d.broadcaster.length + d.moderators.length + d.vips.length + d.viewers.length;

type UsernameClick = (
  userId: string,
  username: string,
  displayName: string,
  color: string,
  badges: Array<{ key: string; info: unknown }>,
  event: MouseEvent,
) => void;

interface ViewersPanelProps {
  broadcasterId: string;
  channelLogin: string;
  onUsernameClick: UsernameClick;
  /** Return to the chat view. */
  onClose: () => void;
}

// Short-lived per-channel cache so toggling the panel off and back on doesn't
// refetch every time. Auto-refresh keeps it live while the panel is open.
const CACHE_TTL_MS = 30_000;
const AUTO_REFRESH_MS = 45_000;
const ROW_HEIGHT = 30;
// A group header after the first carries the divider above it, so it is taller.
const DIVIDED_HEADER_HEIGHT = 40;

const SECTIONS: { role: RoleKey; label: string; icon: LucideIcon }[] = [
  { role: 'broadcaster', label: 'Broadcaster', icon: Video },
  { role: 'moderators', label: 'Moderators', icon: Sword },
  { role: 'vips', label: 'VIPs', icon: Gem },
  { role: 'viewers', label: 'Viewers', icon: Users },
];

const chattersCache = new Map<string, { data: ChannelChatters; ts: number }>();

type LoadState =
  | { kind: 'loading' }
  | { kind: 'ready'; data: ChannelChatters }
  | { kind: 'reauth' }
  | { kind: 'error' };

// Pure fetch (no React state) so it can be shared by the mount effect and the
// refresh/retry handlers without tripping the no-setState-in-effect lint.
async function fetchChatters(broadcasterId: string, channelLogin: string): Promise<ChannelChatters> {
  const data = await invoke<ChannelChatters>('get_channel_chatters', { broadcasterId, channelLogin });
  chattersCache.set(channelLogin.toLowerCase(), { data, ts: Date.now() });
  return data;
}

function errorToState(err: unknown): LoadState {
  const msg = String(err);
  // A bare 401 here means the token predates the moderator:read:chatters scope.
  return msg.includes('REAUTH') || msg.includes('401') ? { kind: 'reauth' } : { kind: 'error' };
}

type FlatRow =
  | { kind: 'header'; role: RoleKey; label: string; icon: LucideIcon; badge: string | null; count: number; first: boolean }
  | { kind: 'chatter'; role: RoleKey; chatter: Chatter };

const rowHeight = (row: FlatRow) => (row.kind === 'header' && !row.first ? DIVIDED_HEADER_HEIGHT : ROW_HEIGHT);

interface RowData {
  rows: FlatRow[];
  collapsed: Partial<Record<RoleKey, boolean>>;
  searching: boolean;
  onToggle: (role: RoleKey) => void;
  onRow: (chatter: Chatter, event: MouseEvent) => void;
}

function PanelRow({ index, style, data }: ListChildComponentProps<RowData>) {
  const row = data.rows[index];

  if (row.kind === 'header') {
    const open = data.searching || !data.collapsed[row.role];
    const Icon = row.icon;
    return (
      <div style={style} className="relative flex flex-col justify-end px-1.5">
        {/* Groups part on a hairline across the list, not on empty space. */}
        {!row.first && <span aria-hidden className="absolute left-3 right-3 top-[6px] h-px bg-white/[0.08]" />}
        <button
          type="button"
          onClick={() => data.onToggle(row.role)}
          disabled={data.searching}
          aria-expanded={open}
          className="group flex h-[28px] w-full items-center gap-2 rounded-md px-2 text-left transition-colors hover:bg-white/[0.03] disabled:cursor-default disabled:hover:bg-transparent"
        >
          {row.badge ? (
            <img src={row.badge} alt="" draggable={false} className="h-[14px] w-[14px] flex-shrink-0 object-contain" />
          ) : (
            <Icon size={12} className="flex-shrink-0 text-white/40" />
          )}
          <span className="text-[10px] font-semibold uppercase tracking-wider text-white/50">{row.label}</span>
          <span className="rounded-full bg-white/[0.06] px-1.5 py-px text-[10px] tabular-nums text-white/45">
            {row.count.toLocaleString()}
          </span>
          {!data.searching && (
            <ChevronDown
              size={12}
              className={`ml-auto flex-shrink-0 text-white/30 transition-transform duration-150 group-hover:text-white/60 ${open ? '' : '-rotate-90'}`}
            />
          )}
        </button>
      </div>
    );
  }

  // A chatter row. Color comes from the chat-user store when the person has been
  // seen talking this session; silent lurkers render in the default text color.
  const stored = useChatUserStore.getState().getUserByUsername(row.chatter.user_login, 'twitch');
  const color = stored?.color ? readableNameColor(stored.color) : undefined;
  const name = row.chatter.user_name || row.chatter.user_login;
  // A localized display name hides the login people type in chat.
  const showLogin = name.toLowerCase() !== row.chatter.user_login.toLowerCase();

  return (
    <div style={style} className="px-1.5">
      <button
        type="button"
        onClick={(e) => data.onRow(row.chatter, e)}
        className="flex h-full w-full items-center gap-1.5 rounded-md pl-7 pr-2 text-left transition-colors hover:bg-white/[0.04]"
      >
        <span className="truncate text-[13px] font-semibold text-textPrimary" style={color ? { color } : undefined}>
          {name}
        </span>
        {showLogin && <span className="truncate text-[11px] text-white/35">{row.chatter.user_login}</span>}
      </button>
    </div>
  );
}

function CenteredState({ children }: { children: ReactNode }) {
  return <div className="flex h-full flex-col items-center justify-center gap-2 px-6 text-center">{children}</div>;
}

export default function ViewersPanel({ broadcasterId, channelLogin, onUsernameClick, onClose }: ViewersPanelProps) {
  // Lazy initial state: show fresh cached data instantly, otherwise start loading.
  // Keeping this out of an effect avoids a synchronous setState on mount.
  const [state, setState] = useState<LoadState>(() => {
    if (!broadcasterId) return { kind: 'error' };
    const cached = chattersCache.get(channelLogin.toLowerCase());
    if (cached && Date.now() - cached.ts < CACHE_TTL_MS) return { kind: 'ready', data: cached.data };
    return { kind: 'loading' };
  });
  const [query, setQuery] = useState('');
  const [collapsed, setCollapsed] = useState<Partial<Record<RoleKey, boolean>>>({});
  const [listHeight, setListHeight] = useState(0);
  const [refreshing, setRefreshing] = useState(false);
  const bodyRef = useRef<HTMLDivElement>(null);

  // Manual refresh / retry (event-handler context). On a transient failure while
  // data is already on screen, keep showing it instead of flipping to an error.
  const load = useCallback(async () => {
    if (!broadcasterId) return;
    setRefreshing(true);
    try {
      const data = await fetchChatters(broadcasterId, channelLogin);
      setState({ kind: 'ready', data });
    } catch (err) {
      Logger.error('[ViewersPanel] get_channel_chatters failed:', err);
      const next = errorToState(err);
      setState((prev) => (prev.kind === 'ready' ? prev : next));
    } finally {
      setRefreshing(false);
    }
  }, [broadcasterId, channelLogin]);

  // Initial load + keep it live while the panel is open. The fetch runs in an
  // inline async function so every setState lands after the awaited round-trip
  // (never a synchronous write during the effect). Cached data, if any, already
  // shows via the lazy initial state above.
  useEffect(() => {
    if (!broadcasterId) return;
    let cancelled = false;
    const run = async () => {
      try {
        const data = await fetchChatters(broadcasterId, channelLogin);
        if (!cancelled) setState({ kind: 'ready', data });
      } catch (err) {
        if (cancelled) return;
        Logger.error('[ViewersPanel] get_channel_chatters failed:', err);
        const next = errorToState(err);
        setState((prev) => (prev.kind === 'ready' ? prev : next));
      }
    };
    void run();
    const id = window.setInterval(() => void run(), AUTO_REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [broadcasterId, channelLogin]);

  // Measure the scroll area so react-window has a concrete height to virtualize.
  useEffect(() => {
    const el = bodyRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) setListHeight(entry.contentRect.height);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const onToggle = useCallback((role: RoleKey) => {
    setCollapsed((c) => ({ ...c, [role]: !c[role] }));
  }, []);

  const onRow = useCallback(
    (chatter: Chatter, event: MouseEvent) => {
      const stored = useChatUserStore.getState().getUserByUsername(chatter.user_login, 'twitch');
      const color = readableNameColor(stored?.color || '#9147FF');
      onUsernameClick(chatter.user_id, chatter.user_login, chatter.user_name, color, [], event);
    },
    [onUsernameClick],
  );

  const data = state.kind === 'ready' ? state.data : null;
  const q = query.trim().toLowerCase();

  const { rows, matches } = useMemo(() => {
    const out: FlatRow[] = [];
    let found = 0;
    if (!data) return { rows: out, matches: found };
    for (const section of SECTIONS) {
      const all = data[section.role];
      const items = q
        ? all.filter((c) => c.user_login.toLowerCase().includes(q) || c.user_name.toLowerCase().includes(q))
        : all;
      if (items.length === 0) continue;
      found += items.length;
      out.push({
        kind: 'header',
        role: section.role,
        label: section.label,
        icon: section.icon,
        badge: data.role_badges?.[section.role] ?? null,
        count: items.length,
        first: out.length === 0,
      });
      // A search overrides collapse so matches are never hidden.
      if (q || !collapsed[section.role]) {
        for (const chatter of items) out.push({ kind: 'chatter', role: section.role, chatter });
      }
    }
    return { rows: out, matches: found };
  }, [data, q, collapsed]);

  // Row heights vary (divided headers), so cached offsets go stale whenever
  // the row list changes.
  const listRef = useRef<List<RowData>>(null);
  useEffect(() => {
    listRef.current?.resetAfterIndex(0);
  }, [rows]);
  const itemSize = useCallback((index: number) => rowHeight(rows[index]), [rows]);

  const rowData = useMemo<RowData>(
    () => ({ rows, collapsed, searching: !!q, onToggle, onRow }),
    [rows, collapsed, q, onToggle, onRow],
  );

  const subtitle = data
    ? data.truncated
      ? `Showing ${listedCount(data).toLocaleString()} of ${data.total.toLocaleString()}. Mods see everyone.`
      : `Everyone here, by role`
    : 'Loading who is in chat';

  return (
    <div className="flex h-full flex-col">
      {/* Header: label, count, refresh, back to chat */}
      <div className="px-3 pb-2 pt-2.5">
        <div className="flex items-center gap-2">
          <span className="text-[11px] font-semibold uppercase tracking-wider text-white/50">In chat</span>
          {data && (
            <span className="rounded-full bg-white/[0.06] px-1.5 py-px text-[10px] font-semibold tabular-nums text-white/60">
              {data.total.toLocaleString()}
            </span>
          )}
          <div className="ml-auto flex items-center gap-0.5">
            <Tooltip content="Refresh" side="bottom">
              <button
                type="button"
                onClick={() => void load()}
                disabled={refreshing}
                className="grid h-6 w-6 place-items-center rounded text-white/50 transition-colors hover:bg-white/10 hover:text-white disabled:opacity-60"
                aria-label="Refresh viewers list"
              >
                <RefreshCw size={12} className={refreshing ? 'animate-spin' : ''} />
              </button>
            </Tooltip>
            <Tooltip content="Back to chat" side="bottom">
              <button
                type="button"
                onClick={onClose}
                className="grid h-6 w-6 place-items-center rounded text-white/50 transition-colors hover:bg-white/10 hover:text-white"
                aria-label="Back to chat"
              >
                <X size={13} />
              </button>
            </Tooltip>
          </div>
        </div>
        <p className="mt-0.5 text-[11px] leading-snug text-white/40">{subtitle}</p>

        <label className="glass-input mt-2 flex items-center gap-2 px-3 py-1.5 focus-within:border-white/20">
          <Search size={13} className="flex-shrink-0 text-textSecondary" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Escape' && query) {
                e.stopPropagation();
                setQuery('');
              }
            }}
            placeholder="Find someone"
            className="min-w-0 flex-1 bg-transparent text-[13px] text-textPrimary outline-none placeholder:text-textSecondary/60"
            spellCheck={false}
            aria-label="Find someone in the viewers list"
          />
          {q && data && (
            <span className="flex-shrink-0 text-[10px] tabular-nums text-textSecondary">
              {matches.toLocaleString()} {matches === 1 ? 'match' : 'matches'}
            </span>
          )}
        </label>
      </div>

      <div ref={bodyRef} className="min-h-0 flex-1 border-t border-white/5">
        {state.kind === 'loading' && (
          <CenteredState>
            <RefreshCw size={14} className="animate-spin text-white/40" />
            <p className="text-xs text-textSecondary">Loading viewers</p>
          </CenteredState>
        )}

        {state.kind === 'reauth' && (
          <CenteredState>
            <p className="text-sm text-textPrimary">Sign in again to load the viewers list.</p>
            <p className="text-xs text-textSecondary/80">Your Twitch sign-in is missing permission to read who is in chat.</p>
            <button
              type="button"
              onClick={() => useAppStore.getState().openSettings('Profile')}
              className="glass-button mt-1 px-3 py-1 text-xs text-textPrimary"
            >
              Open Settings
            </button>
          </CenteredState>
        )}

        {state.kind === 'error' && (
          <CenteredState>
            <p className="text-sm text-textSecondary">Couldn't load the viewers list.</p>
            <button type="button" onClick={() => void load()} className="glass-button px-3 py-1 text-xs text-textPrimary">
              Try again
            </button>
          </CenteredState>
        )}

        {state.kind === 'ready' &&
          (rows.length === 0 ? (
            <CenteredState>
              <p className="text-sm text-textSecondary">{q ? 'No one matches that name.' : 'No one is here yet.'}</p>
            </CenteredState>
          ) : (
            listHeight > 0 && (
              <List
                ref={listRef}
                className="custom-scrollbar"
                height={listHeight}
                width="100%"
                itemCount={rows.length}
                itemSize={itemSize}
                estimatedItemSize={ROW_HEIGHT}
                itemData={rowData}
                overscanCount={8}
              >
                {PanelRow}
              </List>
            )
          ))}
      </div>
    </div>
  );
}

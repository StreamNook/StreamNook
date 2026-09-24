// Viewers list panel — the official Twitch chatters roster for the current
// channel, grouped by role (Broadcaster / Moderators / VIPs / Viewers).
//
// Data comes from the `get_channel_chatters` Rust command, which pages through
// Helix Get Chatters (mod/broadcaster only) and buckets the roster by role using
// the GQL Mods/VIPs lookup. The toggle that mounts this panel is only shown when
// the user is a moderator of the channel, so the roster call is authorized.

import { useState, useEffect, useRef, useMemo, useCallback } from 'react';
import type { MouseEvent } from 'react';
import { FixedSizeList as List, type ListChildComponentProps } from 'react-window';
import { invoke } from '@tauri-apps/api/core';
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
}

type RoleKey = 'broadcaster' | 'moderators' | 'vips' | 'viewers';

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
}

// Short-lived per-channel cache so toggling the panel off and back on doesn't
// refetch every time. Auto-refresh keeps it live while the panel is open.
const CACHE_TTL_MS = 30_000;
const AUTO_REFRESH_MS = 45_000;
const ROW_HEIGHT = 28;

const SECTIONS: { role: RoleKey; label: string }[] = [
  { role: 'broadcaster', label: 'Broadcaster' },
  { role: 'moderators', label: 'Moderators' },
  { role: 'vips', label: 'VIPs' },
  { role: 'viewers', label: 'Viewers' },
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
  | { kind: 'header'; role: RoleKey; label: string; count: number }
  | { kind: 'chatter'; role: RoleKey; chatter: Chatter };

interface RowData {
  rows: FlatRow[];
  collapsed: Partial<Record<RoleKey, boolean>>;
  onToggle: (role: RoleKey) => void;
  onRow: (chatter: Chatter, event: MouseEvent) => void;
}

function Chevron({ open }: { open: boolean }) {
  return (
    <svg
      className={`h-3 w-3 shrink-0 text-textSecondary/60 transition-transform duration-150 ${open ? '' : '-rotate-90'}`}
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24"
      strokeWidth={2.5}
    >
      <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7" />
    </svg>
  );
}

function PanelRow({ index, style, data }: ListChildComponentProps<RowData>) {
  const row = data.rows[index];

  if (row.kind === 'header') {
    const open = !data.collapsed[row.role];
    return (
      <div style={style} className="px-1">
        <button
          type="button"
          onClick={() => data.onToggle(row.role)}
          className="flex h-full w-full items-center gap-1.5 rounded px-2 text-left transition-colors hover:bg-surface-hover"
        >
          <Chevron open={open} />
          <span className="text-[11px] font-semibold uppercase tracking-wide text-textSecondary">
            {row.label}
          </span>
          <span className="text-[11px] tabular-nums text-textSecondary/50">{row.count.toLocaleString()}</span>
        </button>
      </div>
    );
  }

  // A chatter row. Color comes from the chat-user store when the person has been
  // seen talking this session; silent lurkers render in the default text color.
  const stored = useChatUserStore.getState().getUserByUsername(row.chatter.user_login, 'twitch');
  const color = stored?.color ? readableNameColor(stored.color) : undefined;

  return (
    <div style={style} className="px-1">
      <button
        type="button"
        onClick={(e) => data.onRow(row.chatter, e)}
        className="flex h-full w-full items-center rounded pl-7 pr-2 text-left transition-colors hover:bg-surface-hover"
      >
        <span
          className="truncate text-sm font-medium text-textPrimary"
          style={color ? { color } : undefined}
        >
          {row.chatter.user_name || row.chatter.user_login}
        </span>
      </button>
    </div>
  );
}

export default function ViewersPanel({ broadcasterId, channelLogin, onUsernameClick }: ViewersPanelProps) {
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
  const bodyRef = useRef<HTMLDivElement>(null);

  // Manual refresh / retry (event-handler context). On a transient failure while
  // data is already on screen, keep showing it instead of flipping to an error.
  const load = useCallback(async () => {
    if (!broadcasterId) return;
    try {
      const data = await fetchChatters(broadcasterId, channelLogin);
      setState({ kind: 'ready', data });
    } catch (err) {
      Logger.error('[ViewersPanel] get_channel_chatters failed:', err);
      const next = errorToState(err);
      setState((prev) => (prev.kind === 'ready' ? prev : next));
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

  const rows = useMemo<FlatRow[]>(() => {
    if (!data) return [];
    const q = query.trim().toLowerCase();
    const out: FlatRow[] = [];
    for (const section of SECTIONS) {
      const all = data[section.role];
      const items = q
        ? all.filter(
            (c) => c.user_login.toLowerCase().includes(q) || c.user_name.toLowerCase().includes(q),
          )
        : all;
      if (items.length === 0) continue;
      out.push({ kind: 'header', role: section.role, label: section.label, count: items.length });
      // A search overrides collapse so matches are never hidden.
      if (q || !collapsed[section.role]) {
        for (const chatter of items) out.push({ kind: 'chatter', role: section.role, chatter });
      }
    }
    return out;
  }, [data, query, collapsed]);

  const rowData = useMemo<RowData>(() => ({ rows, collapsed, onToggle, onRow }), [rows, collapsed, onToggle, onRow]);

  return (
    <div className="flex h-full flex-col">
      {/* Header: count + search + refresh */}
      <div className="flex items-center gap-2 px-3 pb-2">
        <span className="text-xs font-semibold text-textSecondary">
          {data ? `${data.total.toLocaleString()} ${data.total === 1 ? 'viewer' : 'viewers'}` : 'Viewers'}
        </span>
        <div className="ml-auto flex items-center gap-1.5">
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search"
            className="h-6 w-28 rounded border border-borderSubtle bg-surface px-2 text-xs text-textPrimary placeholder:text-textSecondary/50 focus:border-accent/60 focus:outline-none"
          />
          <Tooltip content="Refresh" side="top">
            <button
              type="button"
              onClick={() => void load()}
              className="grid h-6 w-6 place-items-center rounded text-textSecondary transition-colors hover:bg-surface-hover hover:text-textPrimary"
              aria-label="Refresh viewers list"
            >
              <svg className="h-3.5 w-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
              </svg>
            </button>
          </Tooltip>
        </div>
      </div>

      {data?.truncated && (
        <div className="px-3 pb-1 text-[11px] text-textSecondary/70">
          Showing the first {data.total.toLocaleString()} chatters.
        </div>
      )}

      <div ref={bodyRef} className="min-h-0 flex-1">
        {state.kind === 'loading' && (
          <div className="flex h-full items-center justify-center">
            <p className="text-sm text-textSecondary">Loading viewers…</p>
          </div>
        )}

        {state.kind === 'reauth' && (
          <div className="flex h-full flex-col items-center justify-center gap-2 px-6 text-center">
            <p className="text-sm text-textPrimary">Re-login to load the viewers list.</p>
            <p className="text-xs text-textSecondary/80">
              StreamNook needs a one-time sign-in to read who is in chat.
            </p>
            <button
              type="button"
              onClick={() => useAppStore.getState().openSettings('Profile')}
              className="mt-1 rounded border border-borderSubtle px-3 py-1 text-xs text-textPrimary transition-colors hover:bg-surface-hover"
            >
              Open Settings
            </button>
          </div>
        )}

        {state.kind === 'error' && (
          <div className="flex h-full flex-col items-center justify-center gap-2 px-6 text-center">
            <p className="text-sm text-textSecondary">Couldn't load the viewers list.</p>
            <button
              type="button"
              onClick={() => void load()}
              className="rounded border border-borderSubtle px-3 py-1 text-xs text-textPrimary transition-colors hover:bg-surface-hover"
            >
              Retry
            </button>
          </div>
        )}

        {state.kind === 'ready' &&
          (rows.length === 0 ? (
            <div className="flex h-full items-center justify-center px-6 text-center">
              <p className="text-sm text-textSecondary">
                {query ? 'No one matches that search.' : 'No one is here yet.'}
              </p>
            </div>
          ) : (
            listHeight > 0 && (
              <List
                height={listHeight}
                width="100%"
                itemCount={rows.length}
                itemSize={ROW_HEIGHT}
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

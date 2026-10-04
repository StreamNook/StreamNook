// The chat dock's two surfaces in the main chat: the header capsule that names
// the chat on screen, and the slide-out list of every chat kept open beside
// the stream (the watched stream's chat first, then the docked chats).
//
// Everything here presents state that lives elsewhere: the list, its order,
// the chat on screen and each chat's "mark new messages" setting are Rust's
// (commands/chat_dock.rs, mirrored by chatDockStore); live lines come from
// Rust's channel state; what happened in a hidden chat comes from Rust's
// parked tick (useChannelHeldActivity). A chat that is not on screen renders
// nothing but its row.
//
// Marks follow the Chatterino model: new messages since a chat was last shown
// brighten its name and add a quiet dot, a mention adds an accent count, and
// showing the chat clears both. A chat can be set to mark mentions only.
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { Bell, BellSlash, CaretDown, CircleNotch, MagnifyingGlass, Plus, X } from 'phosphor-react';
import { Tooltip } from '../ui/Tooltip';
import { ProviderLogo } from '../ProviderLogo';
import { useAppStore } from '../../stores/AppStore';
import {
  chatKey,
  dockChat,
  reorderChatDock,
  setDockedChatLight,
  setDockPanelOpen,
  showDockedChat,
  undockChat,
  useChatDockStore,
  type DockedChat,
} from '../../stores/chatDockStore';
import {
  useAnyHeldNew,
  useChannelHeldActivity,
  useChannelHeldMentions,
  useHeldMentionTotal,
  type HeldEntry,
} from '../../stores/chatConnectionStore';
import { useChannelState } from '../../stores/channelStateStore';
import { nameFloor, useCapsuleName, useDockView } from '../../hooks/useChatDock';
import { formatViewerCount } from '../../utils/streamStats';
import { isTwitchLogin, parseChannelInput } from '../../utils/parseChannelInput';
import { streamProvider } from '../../utils/streamProvider';
import { useChannelSearch, type ChannelItem } from '../multi-nook/channelSearch';
import { PROVIDERS, type ProviderId } from '../../types/providers';
import { Logger } from '../../utils/logger';

// ---------------------------------------------------------------------------
// Shared bits
// ---------------------------------------------------------------------------

function Avatar({
  src,
  name,
  live,
  size,
  provider,
}: {
  src: string | null | undefined;
  name: string;
  live?: boolean;
  size: number;
  /** The platform mark, given only where the list mixes platforms. */
  provider?: ProviderId;
}) {
  return (
    <span className="relative flex-shrink-0" style={{ width: size, height: size }}>
      <span
        className="grid h-full w-full place-items-center overflow-hidden rounded-full bg-white/15 font-bold text-white"
        style={{ fontSize: Math.round(size * 0.45) }}
      >
        {src ? (
          <img
            src={src}
            alt=""
            draggable={false}
            className="h-full w-full object-cover"
            onError={(e) => {
              (e.currentTarget as HTMLImageElement).style.display = 'none';
            }}
          />
        ) : (
          name.charAt(0).toUpperCase()
        )}
      </span>
      {/* The bare mark floats on the picture, as in the sidebar: no disc
          behind it, a soft shadow keeps it legible on a light picture. */}
      {provider && (
        <span className="absolute -left-1 -top-1 flex drop-shadow-[0_1px_2px_rgba(0,0,0,0.85)]">
          <ProviderLogo provider={provider} size={Math.max(10, Math.round(size * 0.42))} />
        </span>
      )}
      {/* Live: the sidebar's dot, not a ring, so a long list stays quiet. */}
      {live && (
        <span
          aria-hidden
          className="absolute -bottom-px -right-px rounded-full border-2 border-background bg-live"
          style={{ width: Math.max(8, Math.round(size * 0.36)), height: Math.max(8, Math.round(size * 0.36)) }}
        />
      )}
    </span>
  );
}

/** A mention count, or a quiet dot for new messages. */
function Mark({ mentions, fresh, className = '' }: { mentions: number; fresh: boolean; className?: string }) {
  if (mentions > 0) {
    return (
      <span
        className={`grid h-4 min-w-4 place-items-center rounded-full bg-accent px-1 text-[10px] font-bold leading-none tabular-nums text-background ${className}`}
        aria-label={`${mentions} mention${mentions === 1 ? '' : 's'}`}
      >
        {mentions > 99 ? '99+' : mentions}
      </span>
    );
  }
  if (fresh) return <span className={`h-1.5 w-1.5 rounded-full bg-textPrimary/70 ${className}`} aria-label="New messages" />;
  return null;
}

// ---------------------------------------------------------------------------
// Header capsule
// ---------------------------------------------------------------------------

/**
 * Names the chat on screen and opens the list. Shown while anything is
 * docked. Its mark sums every hidden chat, so a mention elsewhere is visible
 * with the list closed.
 */
export function ChatDockSwitcher() {
  const { held, shown, others, live } = useDockView();
  const stream = useAppStore((s) => s.currentStream);
  const open = useChatDockStore((s) => s.panelOpen);

  // Every chat not on screen: the docked ones, and the watched stream's while
  // a docked chat shows.
  const hidden = useMemo<HeldEntry[]>(() => {
    const out: HeldEntry[] = others
      .filter((c) => !shown || chatKey(c) !== chatKey(shown))
      .map((c) => ({ provider: c.provider, channel: c.login, light: c.light_on_new }));
    if (shown && live) out.push({ provider: live.provider, channel: live.login, light: true });
    return out;
  }, [others, shown, live]);
  const mentions = useHeldMentionTotal(hidden);
  const fresh = useAnyHeldNew(hidden);
  const name = useCapsuleName();

  if (!held) return null;
  const avatar = shown ? shown.avatar_url : stream?.profile_image_url;
  const total = others.length + (live ? 1 : 0);

  return (
    <Tooltip content={`${total} chats open`} side="bottom">
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          setDockPanelOpen(!open);
        }}
        aria-expanded={open}
        aria-label={`Chat list: ${name}, ${total} chats`}
        data-dock-capsule
        className="chrome-glaze chrome-glaze--flat chat-header-capsule pointer-events-auto min-w-0 max-w-full"
      >
        <span className="chat-header-pill min-w-0 !pl-0.5 !pr-1.5 text-textPrimary">
          <Avatar src={avatar} name={name} size={18} live={!shown} />
          <span data-fit-label className="chat-dock-name max-w-[6.5rem] truncate" style={{ minWidth: nameFloor(name) }}>
            {name}
          </span>
        </span>
        <span className="flex flex-shrink-0 items-center gap-1 pr-1.5 text-[11px] font-semibold tabular-nums text-textSecondary">
          +{total - 1}
          <Mark mentions={mentions} fresh={fresh} />
          <CaretDown size={11} weight="bold" className={`transition-transform ${open ? 'rotate-180' : ''}`} />
        </span>
      </button>
    </Tooltip>
  );
}

// ---------------------------------------------------------------------------
// Slide-out list
// ---------------------------------------------------------------------------

/** Keeps an empty second line its height, so a row does not jump when its
 *  live line lands. */
const NBSP = String.fromCharCode(0xa0);

interface RowProps {
  chat: DockedChat;
  active: boolean;
  focused: boolean;
  showPlatform: boolean;
  draggable: boolean;
  onShow: () => void;
  onDragStart: () => void;
  onDropOn: () => void;
}

function DockRow({ chat, active, focused, showPlatform, draggable, onShow, onDragStart, onDropOn }: RowProps) {
  const isTwitch = chat.provider === 'twitch';
  // Twitch reads Rust's channel state; other platforms read the dock's own
  // poller (chat_dock_live.rs). Either way the row shows the same live line.
  const state = useChannelState(isTwitch ? chat.login : null);
  const other = useChatDockStore((s) => (isTwitch ? undefined : s.live[chatKey(chat)]));
  const activity = useChannelHeldActivity(chat.provider, chat.login);
  const mentions = useChannelHeldMentions(chat.provider, chat.login);
  const fresh = chat.light_on_new && activity >= 1;
  const line = isTwitch
    ? state?.viewers_at != null
      ? { live: !!state.started_at, category: state.game_name, title: state.title, viewers: state.viewer_count }
      : null
    : other
      ? { live: other.live, category: other.category, title: other.title, viewers: other.viewer_count }
      : null;
  const live = !!line?.live;
  const offline = !!line && !line.live;
  const name = chat.display_name || chat.login;
  // Until the first answer lands, the mark (or, in a list of one platform, the
  // name of it) says where the chat is.
  const sub = live
    ? line?.category || line?.title || 'Live'
    : offline
      ? 'Offline'
      : showPlatform
        ? NBSP
        : PROVIDERS[chat.provider]?.label ?? '';
  const [over, setOver] = useState(false);

  return (
    <div
      role="option"
      aria-selected={active}
      draggable={draggable}
      onDragStart={(e) => {
        e.dataTransfer.effectAllowed = 'move';
        onDragStart();
      }}
      onDragOver={(e) => {
        if (!draggable) return;
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        e.preventDefault();
        setOver(false);
        onDropOn();
      }}
      onClick={onShow}
      onAuxClick={(e) => {
        // Middle click closes, as on a tab.
        if (e.button === 1) void undockChat(chatKey(chat));
      }}
      className={`group relative mx-1.5 flex cursor-pointer items-center gap-2.5 rounded-lg px-2 py-1.5 transition-colors ${
        active ? 'bg-white/[0.08]' : focused ? 'bg-white/[0.05]' : 'hover:bg-white/[0.04]'
      } ${offline ? 'opacity-60' : ''}`}
    >
      {over && <span aria-hidden className="absolute -top-px left-3 right-3 h-0.5 rounded-full bg-accent" />}
      <Avatar src={chat.avatar_url} name={name} size={26} live={live} provider={showPlatform ? chat.provider : undefined} />
      <span className="min-w-0 flex-1">
        <span className={`block truncate text-[13px] font-semibold ${active || fresh || mentions > 0 ? 'text-textPrimary' : 'text-textSecondary'}`}>
          {name}
        </span>
        <span className="block truncate text-[11px] text-textMuted">{sub}</span>
      </span>
      <span className="flex flex-shrink-0 items-center gap-1.5">
        {live && line?.viewers != null && (
          <span className="text-[11px] tabular-nums text-textSecondary group-hover:hidden">
            {formatViewerCount(line.viewers)}
          </span>
        )}
        <Mark mentions={mentions} fresh={fresh} className="group-hover:hidden" />
        <span className="hidden items-center gap-0.5 group-hover:flex">
          <Tooltip content={chat.light_on_new ? 'Mark new messages (on). Click for mentions only' : 'Mentions only. Click to mark new messages too'} side="top">
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                void setDockedChatLight(chatKey(chat), !chat.light_on_new);
              }}
              className="grid h-6 w-6 place-items-center rounded-md text-textSecondary transition-colors hover:bg-white/10 hover:text-textPrimary"
              aria-label={chat.light_on_new ? 'Mark mentions only' : 'Mark new messages'}
            >
              {chat.light_on_new ? <Bell size={13} /> : <BellSlash size={13} />}
            </button>
          </Tooltip>
          <Tooltip content="Close this chat" side="top">
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                void undockChat(chatKey(chat));
              }}
              className="grid h-6 w-6 place-items-center rounded-md text-textSecondary transition-colors hover:bg-white/10 hover:text-error"
              aria-label={`Close ${name}`}
            >
              <X size={13} />
            </button>
          </Tooltip>
        </span>
      </span>
    </div>
  );
}

/** The watched stream's chat, first in the list. */
function WatchingRow({
  active,
  focused,
  showPlatform,
  onShow,
}: {
  active: boolean;
  focused: boolean;
  showPlatform: boolean;
  onShow: () => void;
}) {
  const stream = useAppStore((s) => s.currentStream);
  const provider: ProviderId = stream ? streamProvider(stream) : 'twitch';
  const activity = useChannelHeldActivity(provider, stream?.user_login);
  const mentions = useChannelHeldMentions(provider, stream?.user_login);
  if (!stream) return null;
  const name = stream.user_name || stream.user_login;
  return (
    <div
      role="option"
      aria-selected={active}
      onClick={onShow}
      className={`mx-1.5 flex cursor-pointer items-center gap-2.5 rounded-lg px-2 py-1.5 transition-colors ${
        active ? 'bg-white/[0.08]' : focused ? 'bg-white/[0.05]' : 'hover:bg-white/[0.04]'
      }`}
    >
      <Avatar src={stream.profile_image_url} name={name} size={26} live provider={showPlatform ? provider : undefined} />
      <span className="min-w-0 flex-1">
        <span className={`block truncate text-[13px] font-semibold ${active || activity > 0 ? 'text-textPrimary' : 'text-textSecondary'}`}>
          {name}
        </span>
        <span className="block truncate text-[11px] text-textMuted">{stream.game_name || 'Live'}</span>
      </span>
      {stream.viewer_count != null && (
        <span className="text-[11px] tabular-nums text-textSecondary">{formatViewerCount(stream.viewer_count)}</span>
      )}
      <Mark mentions={mentions} fresh={activity >= 1} />
    </div>
  );
}

const SECTION_LABEL = 'px-3.5 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-wider text-textMuted';
const SECTION_RULE = 'mt-2 border-t border-white/[0.06] pt-3';

/** Platforms whose chat can be docked (what chat_dock.rs accepts). */
const CHAT_PROVIDERS: ProviderId[] = ['twitch', 'kick', 'youtube', 'tiktok'];

/** A search row as a chat to dock. Only Twitch rows carry a channel id: a
 *  provider row's id is its login or a platform id Helix must never see. */
function itemToChat(it: ChannelItem): Omit<DockedChat, 'light_on_new'> {
  const provider = it.provider ?? 'twitch';
  return {
    provider,
    login: it.login,
    channel_id: provider === 'twitch' ? it.id : '',
    display_name: it.displayName || it.login,
    avatar_url: it.avatarUrl ?? null,
  };
}

/** A channel the search found, one click from being docked. */
function ResultRow({
  item,
  focused,
  showPlatform,
  onAdd,
}: {
  item: ChannelItem;
  focused: boolean;
  showPlatform: boolean;
  onAdd: () => void;
}) {
  const provider = item.provider ?? 'twitch';
  const sub = item.isLive ? item.gameName || 'Live' : 'Offline';
  return (
    <div
      role="option"
      aria-selected={false}
      onClick={onAdd}
      className={`mx-1.5 flex cursor-pointer items-center gap-2.5 rounded-lg px-2 py-1.5 transition-colors ${focused ? 'bg-white/[0.05]' : 'hover:bg-white/[0.04]'}`}
    >
      <Avatar
        src={item.avatarUrl}
        name={item.displayName || item.login}
        size={26}
        live={item.isLive}
        provider={showPlatform ? provider : undefined}
      />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13px] font-semibold text-textSecondary">{item.displayName || item.login}</span>
        <span className="block truncate text-[11px] text-textMuted">{sub}</span>
      </span>
      <Plus size={13} className="flex-shrink-0 text-textSecondary" />
    </div>
  );
}

/**
 * Turn what was typed into a chat to dock. A link names its own platform; a
 * bare word is a Twitch login, which Rust resolves to its id and picture.
 */
async function resolveTyped(raw: string): Promise<Omit<DockedChat, 'light_on_new'> | null> {
  const s = raw.trim().replace(/^@/, '');
  if (!s) return null;
  const link = parseChannelInput(s);
  const provider: ProviderId | null = link ? link.provider : isTwitchLogin(s) ? 'twitch' : null;
  const login = link ? link.channel : s;
  if (!provider) return null;
  if (provider !== 'twitch') {
    return { provider, login, channel_id: '', display_name: login, avatar_url: null };
  }
  try {
    const user = await invoke<{ id: string; login: string; display_name: string; profile_image_url?: string | null }>(
      'get_user_by_login',
      { login: login.toLowerCase() },
    );
    return {
      provider: 'twitch',
      login: user.login,
      channel_id: user.id,
      display_name: user.display_name || user.login,
      avatar_url: user.profile_image_url ?? null,
    };
  } catch (e) {
    Logger.warn('[ChatDock] could not find that channel:', e);
    return null;
  }
}

/**
 * The chat list: a dropdown under the header capsule, as tall as its rows (up
 * to 70% of the chat, then it scrolls). Rendered inside the main chat (which
 * is `relative`) and placed under the capsule, or under the header when it
 * opens from the chat menu with nothing docked yet. A click anywhere else or
 * Escape closes it; the chat below stays as it is.
 */
export function ChatDockPanel({ top }: { top: number }) {
  const open = useChatDockStore((s) => s.panelOpen);
  return (
    <AnimatePresence>
      {open && <PanelBody key="dock-panel" top={top} />}
    </AnimatePresence>
  );
}

function PanelBody({ top }: { top: number }) {
  const { shown, others, live } = useDockView();
  const liveName = useAppStore((s) => s.currentStream?.user_name ?? '');
  // The app's channel finder (MultiNook's Add Stream, the preset and reminder
  // pickers): live follows first, then every channel on the chat platforms,
  // searched in Rust. Anything already open is left out.
  const excludeKeys = useMemo(() => {
    const keys = new Set(others.map(chatKey));
    if (live) keys.add(chatKey(live));
    return keys;
  }, [others, live]);
  const search = useChannelSearch({ excludeKeys, providers: CHAT_PROVIDERS });
  const query = search.searchInput;
  const setQuery = search.setSearchInput;
  const [focus, setFocus] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const dragKey = useRef<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState<{ left: number; top: number; maxHeight: number } | null>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Under the capsule, kept inside the chat; re-placed when the chat resizes.
  // The observer's first call does the initial placement.
  useLayoutEffect(() => {
    const el = panelRef.current;
    const root = el?.offsetParent as HTMLElement | null;
    if (!el || !root) return;
    const ro = new ResizeObserver(() => {
      const r = root.getBoundingClientRect();
      const cap = root.querySelector<HTMLElement>('[data-dock-capsule]')?.getBoundingClientRect();
      const t = cap ? cap.bottom - r.top + 6 : top + 6;
      const left = Math.max(8, Math.min(cap ? cap.left - r.left : 8, r.width - el.offsetWidth - 8));
      setPlace({ left, top: t, maxHeight: Math.max(160, Math.min(r.height * 0.7, r.height - t - 8)) });
    });
    ro.observe(root);
    return () => ro.disconnect();
  }, [top]);

  // A click outside the list (the capsule toggles on its own) closes it.
  useEffect(() => {
    const onDown = (e: PointerEvent) => {
      const target = e.target as Element | null;
      if (!target || panelRef.current?.contains(target) || target.closest('[data-dock-capsule]')) return;
      setDockPanelOpen(false);
    };
    document.addEventListener('pointerdown', onDown, true);
    return () => document.removeEventListener('pointerdown', onDown, true);
  }, []);

  const close = () => setDockPanelOpen(false);
  const show = (key: string | null) => {
    void showDockedChat(key);
    close();
  };

  const q = query.trim().toLowerCase().replace(/^@/, '');
  const docked = useMemo(
    () => (q ? others.filter((c) => c.login.toLowerCase().includes(q) || c.display_name.toLowerCase().includes(q)) : others),
    [others, q],
  );
  const following = q ? search.followingItems.slice(0, 5) : [];
  const channels = q ? search.searchItems : [];
  const results = [...following, ...channels];
  // Platform marks only where platforms mix, and then on every row.
  const openMixed = new Set([...(live ? [live.provider] : []), ...others.map((c) => c.provider)]).size > 1;
  const resultsMixed = new Set(results.map((it) => it.provider ?? 'twitch')).size > 1;

  // One keyboard order: the watched stream (always at rest, and when the
  // search names it, since the results leave out what is already open), the
  // docked matches, then channels to add.
  const showWatching = !!live && (!q || live.login.toLowerCase().includes(q) || liveName.toLowerCase().includes(q));
  type Entry = { kind: 'watching' } | { kind: 'docked'; chat: DockedChat } | { kind: 'result'; item: ChannelItem };
  const entries: Entry[] = [
    ...(showWatching ? [{ kind: 'watching' } as const] : []),
    ...docked.map((chat) => ({ kind: 'docked', chat }) as const),
    ...results.map((item) => ({ kind: 'result', item }) as const),
  ];
  const focused = Math.min(focus, Math.max(entries.length - 1, 0));
  const resultBase = (showWatching ? 1 : 0) + docked.length;

  const addResult = async (item: ChannelItem) => {
    await dockChat(itemToChat(item), false);
    search.reset();
  };

  const addTyped = async () => {
    setBusy(true);
    setError(null);
    const chat = await resolveTyped(query);
    setBusy(false);
    if (!chat) {
      setError('No channel by that name. Try a Twitch name or a channel link.');
      return;
    }
    if (live && chatKey(chat) === chatKey(live)) {
      setError('That is the stream you are watching.');
      return;
    }
    await dockChat(chat, false);
    search.reset();
  };

  const activate = (e: Entry | undefined) => {
    if (!e) {
      if (q) void addTyped();
      return;
    }
    if (e.kind === 'watching') show(null);
    else if (e.kind === 'docked') show(chatKey(e.chat));
    else void addResult(e.item);
  };

  const onDrop = (target: DockedChat) => {
    const from = dragKey.current;
    dragKey.current = null;
    const to = chatKey(target);
    if (!from || from === to) return;
    const keys = others.map(chatKey).filter((k) => k !== from);
    keys.splice(keys.indexOf(to), 0, from);
    void reorderChatDock(keys);
  };

  const shownKey = shown ? chatKey(shown) : null;
  // A pasted link or exact name the search did not turn up.
  const canAddTyped = !!q && !search.isSearching && results.length === 0 && docked.length === 0;

  return (
    <motion.div
      ref={panelRef}
      role="listbox"
      aria-label="Open chats"
      initial={{ opacity: 0, y: -4, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: -4, scale: 0.98 }}
      transition={{ duration: 0.14, ease: 'easeOut' }}
      // A flyout's base stays readable at every Glassiness: this opens over a
      // moving chat, which a see-through panel let show through its rows.
      className="glass-flyout absolute z-50 flex w-[min(calc(100%-16px),320px)] origin-top-left flex-col !px-0 !pb-1.5 !pt-1.5"
      style={{
        left: place?.left ?? 8,
        top: place?.top ?? top + 6,
        maxHeight: place?.maxHeight,
        visibility: place ? 'visible' : 'hidden',
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation();
          if (query) setQuery('');
          else close();
        } else if (e.key === 'ArrowDown') {
          e.preventDefault();
          setFocus((f) => Math.min(f + 1, entries.length - 1));
        } else if (e.key === 'ArrowUp') {
          e.preventDefault();
          setFocus((f) => Math.max(f - 1, 0));
        } else if (e.key === 'Enter') {
          e.preventDefault();
          activate(entries[focused]);
        }
      }}
    >
      <label className="glass-input mx-1.5 mb-1 flex flex-shrink-0 items-center gap-2 px-2.5 py-1.5 focus-within:border-white/20">
        <MagnifyingGlass size={13} className="flex-shrink-0 text-textSecondary" />
        <input
          ref={inputRef}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setFocus(0);
            setError(null);
          }}
          placeholder="Find or add a chat"
          spellCheck={false}
          className="min-w-0 flex-1 bg-transparent text-[13px] text-textPrimary outline-none placeholder:text-textSecondary/60"
          aria-label="Find a chat, or type a channel to add"
        />
      </label>

      <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto">
        {showWatching && (
          <>
            <div className={SECTION_LABEL}>Watching</div>
            <WatchingRow active={!shown} focused={focused === 0} showPlatform={openMixed} onShow={() => show(null)} />
          </>
        )}

        {docked.length > 0 && (
          <>
            <div className={`${SECTION_LABEL} ${showWatching ? 'mt-2 border-t border-white/[0.06] pt-3' : ''}`}>
              Docked
            </div>
            {docked.map((chat, i) => {
              const key = chatKey(chat);
              return (
                <DockRow
                  key={key}
                  chat={chat}
                  active={key === shownKey}
                  focused={focused === i + (showWatching ? 1 : 0)}
                  showPlatform={openMixed}
                  draggable={!q}
                  onShow={() => show(key)}
                  onDragStart={() => {
                    dragKey.current = key;
                  }}
                  onDropOn={() => onDrop(chat)}
                />
              );
            })}
          </>
        )}

        {others.length === 0 && !q && (
          <p className="px-3.5 pb-1 pt-2.5 text-[12px] leading-relaxed text-textSecondary">
            Dock a chat to keep it open beside the stream. Type a channel above, or pick Dock this chat in any chat&apos;s menu.
          </p>
        )}

        {following.length > 0 && (
          <>
            <div className={`${SECTION_LABEL} ${docked.length > 0 ? SECTION_RULE : ''}`}>Following · live</div>
            {following.map((item, i) => (
              <ResultRow
                key={`f-${item.provider ?? 'twitch'}:${item.login}`}
                item={item}
                focused={focused === resultBase + i}
                showPlatform={resultsMixed}
                onAdd={() => void addResult(item)}
              />
            ))}
          </>
        )}

        {q && (channels.length > 0 || search.isSearching) && (
          <>
            <div className={`${SECTION_LABEL} ${docked.length + following.length > 0 ? SECTION_RULE : ''} flex items-center gap-1.5`}>
              All channels
              {search.isSearching && <CircleNotch size={11} className="animate-spin text-textSecondary" />}
            </div>
            {channels.map((item, i) => (
              <ResultRow
                key={`s-${item.provider ?? 'twitch'}:${item.login}`}
                item={item}
                focused={focused === resultBase + following.length + i}
                showPlatform={resultsMixed}
                onAdd={() => void addResult(item)}
              />
            ))}
            {channels.length === 0 && <p className="px-3.5 pb-1 text-[11px] text-textMuted">Searching</p>}
          </>
        )}

        {canAddTyped && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void addTyped()}
            className="mx-1.5 mt-2 flex w-[calc(100%-12px)] items-center gap-2.5 rounded-lg border border-dashed border-white/15 px-2 py-2 text-left text-[12px] text-textSecondary transition-colors hover:border-white/25 hover:text-textPrimary disabled:opacity-60"
          >
            <Plus size={14} className="flex-shrink-0" />
            <span className="min-w-0 truncate">
              Dock <span className="font-semibold text-textPrimary">{query.trim()}</span>
            </span>
          </button>
        )}
        {error && <p className="px-3.5 pt-2 text-[11px] text-error">{error}</p>}
      </div>
    </motion.div>
  );
}

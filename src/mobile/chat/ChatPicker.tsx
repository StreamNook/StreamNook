// The "add a chat" picker: a search for any channel, then your follows that
// are live, then your follows that are offline. Tap a row to open its chat; a
// chat already open is marked "Open" where it sits, and tapping it switches to
// it. The lists are Rust's (`chat_picker`, built from the Following snapshot,
// so opening this makes no request); searching is `search_chat_rooms`.
//
// Drawn inline as the Chats tab when no chat is open, and inside AddChatSheet
// from the strip's + (and the chat menu's "Add a chat").
import React, { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Check, MagnifyingGlass, Plus } from 'phosphor-react';
import { ChatsMinimizeButton } from './ChatsMinimizeButton';
import { useAppStore } from '../../stores/AppStore';
import { useChatTabsStore } from './chatTabsStore';
import { Logger } from '../../utils/logger';
import type { ProviderId } from '../../types/providers';
import { ProviderMark } from '../../components/ProviderLogo';
import { SevenTvAvatar } from '../../components/SevenTvAvatar';
import { lastLiveLabel } from '../../utils/lastLive';
import { formatViewerCount } from '../../utils/streamStats';
import { CollabAvatarStack } from '../../components/SharedViewers';
import { groupFor, isTogether, type ChannelGroup } from '../../utils/sharedViewers';

/** A channel as `chat_picker`, `search_chat_rooms` and `chat_room_from_link` answer it. */
interface PickerRow {
  provider: ProviderId;
  key: string;
  user_login: string;
  user_id: string;
  user_name: string;
  profile_image_url: string | null;
  is_live: boolean;
  viewer_count: number | null;
  game_name: string | null;
  last_live?: string | null;
  open?: boolean;
  pasted?: boolean;
}

interface PickerLists {
  live: PickerRow[];
  offline: PickerRow[];
  mixed_platforms: boolean;
}

/** The platforms the phone chats on. */
const PLATFORMS = ['twitch', 'kick'];
const EMPTY: PickerLists = { live: [], offline: [], mixed_platforms: false };

const Row: React.FC<{
  row: PickerRow;
  open: boolean;
  mark: boolean;
  following?: boolean;
  /** A live room streaming together or sharing its chat, as the cards say it. */
  group?: ChannelGroup;
  onPick: (row: PickerRow) => void;
}> = ({ row, open, mark, following, group, onPick }) => {
  const name = row.user_name || row.user_login;
  const detail = row.pasted
    ? 'Open this Kick chat'
    : row.is_live
      ? [row.game_name, row.viewer_count != null ? formatViewerCount(row.viewer_count) : null, following ? 'following' : null]
          .filter(Boolean)
          .join(' · ') || 'Live'
      : row.last_live !== undefined
        ? lastLiveLabel(row.last_live)
        : 'Offline';
  return (
    <button
      onClick={() => onPick(row)}
      className={`sn-touch w-full flex items-center gap-3 px-2 py-1.5 rounded-xl text-left active:opacity-70 ${
        open ? 'bg-white/[0.04]' : ''
      }`}
    >
      <span className="relative shrink-0">
        {row.profile_image_url ? (
          <SevenTvAvatar
            cosmeticsId={row.provider === 'twitch' ? row.user_id || null : null}
            px={36}
            src={row.profile_image_url}
            alt=""
            className={`w-9 h-9 rounded-full object-cover ${row.is_live ? '' : 'opacity-60 saturate-50'}`}
            draggable={false}
          />
        ) : (
          <span className="w-9 h-9 rounded-full bg-surface flex items-center justify-center text-[13px] font-semibold text-textMuted">
            {name.charAt(0).toUpperCase()}
          </span>
        )}
        {row.is_live && (
          <span
            aria-label="Live"
            className="absolute right-0 bottom-0 w-2.5 h-2.5 rounded-full bg-live ring-2 ring-background"
          />
        )}
      </span>
      <span className="flex-1 min-w-0">
        <span className="flex items-center gap-1.5 text-[15px] font-medium text-textPrimary">
          <span className="truncate">{name}</span>
          {mark && row.provider !== 'twitch' && <ProviderMark provider={row.provider} size={12} />}
        </span>
        {/* One line: the group's faces and word lead it when there is one. */}
        <span className="flex items-center gap-1 min-w-0 text-[12.5px] text-textMuted">
          {group && (
            <>
              <span className="shrink-0">
                <CollabAvatarStack collab={group} size={14} />
              </span>
              <span className="shrink-0">{isTogether(group) ? 'Together' : 'Shared chat'} ·</span>
            </>
          )}
          <span className="truncate">{detail}</span>
        </span>
      </span>
      {open ? (
        <span className="shrink-0 h-[22px] px-2.5 rounded-full flex items-center gap-1 text-[11.5px] font-semibold text-textPrimary bg-accent/20">
          <Check size={10} weight="bold" />
          Open
        </span>
      ) : (
        <span className="shrink-0 w-7 h-7 rounded-full chrome-glaze chrome-glaze--flat flex items-center justify-center text-textSecondary">
          <Plus size={12} weight="bold" />
        </span>
      )}
    </button>
  );
};

const Label: React.FC<{ text: string; count?: number }> = ({ text, count }) => (
  <div className="flex items-center justify-between px-2 pt-3 pb-1 text-[12px] font-semibold uppercase tracking-wide text-textMuted">
    <span>{text}</span>
    {count != null && <span className="normal-case tracking-normal font-medium tabular-nums">{count}</span>}
  </div>
);

const LIST_FADE = {
  WebkitMaskImage: 'linear-gradient(to bottom, transparent, #000 12px)',
  maskImage: 'linear-gradient(to bottom, transparent, #000 12px)',
} as const;

export const ChatPicker: React.FC<{
  /** After a chat opens (the sheet closes itself). */
  onPicked?: () => void;
  /** Extra space under the lists (the inline picker clears the tab bar). */
  bottomClearance?: string;
  /** Scroll the lists here, under a search field that stays put and never
   *  fades (the Chats screen). In a sheet, the sheet scrolls everything. */
  scrolls?: boolean;
  /** Where the picker is the whole Chats screen: a Close beside the search
   *  leaves Chats. */
  onClose?: () => void;
}> = ({ onPicked, bottomClearance, scrolls = false, onClose }) => {
  const tabs = useChatTabsStore((s) => s.tabs);
  const revision = useChatTabsStore((s) => s.revision);
  const addTab = useChatTabsStore((s) => s.addTab);
  const setActive = useChatTabsStore((s) => s.setActive);
  // The snapshot's Following lists moved: read the picker again.
  const following = useAppStore((s) => s.following);
  const collabs = useAppStore((s) => s.collaborations);
  const sharedChats = useAppStore((s) => s.sharedChats);
  const groupOf = (row: PickerRow): ChannelGroup | undefined =>
    row.is_live ? groupFor(collabs, sharedChats, { provider: row.provider, user_id: row.user_id }) : undefined;

  const [lists, setLists] = useState<PickerLists>(EMPTY);
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<PickerRow[]>([]);
  const [searching, setSearching] = useState(false);
  const [pasted, setPasted] = useState<PickerRow | null>(null);

  useEffect(() => {
    let live = true;
    void invoke<PickerLists>('chat_picker', { platforms: PLATFORMS })
      .then((l) => live && setLists(l ?? EMPTY))
      .catch((err) => Logger.warn('[ChatPicker] lists failed:', err));
    return () => {
      live = false;
    };
  }, [revision, following]);

  useEffect(() => {
    let cancelled = false;
    void invoke<PickerRow | null>('chat_room_from_link', { query })
      .then((row) => !cancelled && setPasted(row ?? null))
      .catch(() => !cancelled && setPasted(null));
    return () => {
      cancelled = true;
    };
  }, [query]);

  useEffect(() => {
    const q = query.trim();
    if (q.length < 2 || pasted) {
      setResults([]);
      setSearching(false);
      return;
    }
    let cancelled = false;
    setSearching(true);
    const timer = setTimeout(async () => {
      try {
        const rows = await invoke<PickerRow[]>('search_chat_rooms', { query: q });
        if (!cancelled) setResults(rows ?? []);
      } catch (err) {
        Logger.warn('[ChatPicker] search failed:', err);
        if (!cancelled) setResults([]);
      } finally {
        if (!cancelled) setSearching(false);
      }
    }, 320);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [query, pasted]);

  const openKeys = useMemo(() => new Set(tabs.map((t) => t.channel)), [tabs]);
  const followed = useMemo(
    () => new Set([...lists.live, ...lists.offline].map((r) => r.key)),
    [lists],
  );

  const pick = (row: PickerRow) => {
    if (openKeys.has(row.key)) {
      setActive(row.key);
    } else {
      addTab(row.user_login, row.user_id || null, row.user_name || row.user_login, row.profile_image_url, row.provider);
    }
    setQuery('');
    onPicked?.();
  };

  const searchingNow = query.trim().length >= 2 || !!pasted;
  const found = pasted ? [pasted] : results;
  const mixedFound = new Set(found.map((r) => r.provider)).size > 1;

  return (
    <div className={`flex flex-col min-h-0 ${scrolls ? 'flex-1' : ''}`}>
      <div className="flex items-center gap-1.5 mx-1 mb-1 shrink-0">
        {/* Chats' way out, the header's minimize arrow, left of the search on its center line. */}
        {onClose && <ChatsMinimizeButton onMinimize={onClose} className="-ml-0.5" />}
      <div className="flex-1 min-w-0 flex items-center gap-2 glass-input px-3">
        <MagnifyingGlass size={16} className="text-textMuted shrink-0" />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search any channel"
          className="flex-1 bg-transparent py-2.5 text-[15px] text-textPrimary placeholder:text-textMuted outline-none"
          autoCapitalize="off"
          autoCorrect="off"
          enterKeyHint="search"
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.nativeEvent.isComposing) e.currentTarget.blur();
          }}
        />
      </div>
      </div>
      <div
        className={`flex flex-col ${scrolls ? 'flex-1 min-h-0 overflow-y-auto' : ''}`}
        // Rows scrolling up fade out under the field; at rest the first label
        // sits below the fade (its own 12 px of top padding), fully drawn.
        style={{ paddingBottom: bottomClearance, ...(scrolls ? LIST_FADE : null) }}
      >
        {searchingNow ? (
          searching && found.length === 0 ? (
            <div className="py-6 text-center text-sm text-textMuted">Searching…</div>
          ) : found.length === 0 ? (
            <div className="py-6 text-center text-sm text-textMuted">No channels found.</div>
          ) : (
            <>
              <Label text="Results" />
              {found.map((row) => (
                <Row
                  key={row.key}
                  row={row}
                  open={openKeys.has(row.key)}
                  mark={mixedFound}
                  following={followed.has(row.key)}
                  group={groupOf(row)} onPick={pick}
                />
              ))}
            </>
          )
        ) : (
          <>
            <Label text="Live" count={lists.live.length} />
            {lists.live.length === 0 ? (
              <div className="px-2 py-2 text-[13px] text-textMuted">Nobody you follow is live.</div>
            ) : (
              lists.live.map((row) => (
                <Row key={row.key} row={row} open={openKeys.has(row.key)} mark={lists.mixed_platforms} group={groupOf(row)} onPick={pick} />
              ))
            )}
            {lists.offline.length > 0 && (
              <>
                <Label text="Offline" count={lists.offline.length} />
                {lists.offline.map((row) => (
                  <Row
                    key={row.key}
                    row={row}
                    open={openKeys.has(row.key)}
                    mark={lists.mixed_platforms}
                    group={groupOf(row)} onPick={pick}
                  />
                ))}
              </>
            )}
          </>
        )}
      </div>
    </div>
  );
};

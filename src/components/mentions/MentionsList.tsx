// The Mentions screen's list: every mention, reply and highlight Rust filed
// for the signed-in account, newest first. Rust owns the entries and their
// read state; this draws a page of them and sends what the viewer does.
//
// MultiChat's Mentions mode. The main window shows the same inbox as part of
// the Dynamic Island's notification history instead.

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { AtSign, BellOff, Bell, CheckCheck, Trash2, X } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import { ProviderLogo } from '../ProviderLogo';
import { useAppStore } from '../../stores/AppStore';
import { MENTION_INBOX_CHANGED_EVENT, toggleMentionChatMuted } from '../../stores/mentionInboxStore';
import { isProviderId } from '../../types/providers';
import type { MentionEntry, MentionKind, MentionPage } from '../../types';
import { agoLabel } from '../../utils/lastLive';
import { Logger } from '../../utils/logger';

const PAGE = 50;
/** `get_mention_inbox` caps a page here. */
const PAGE_MAX = 200;

const KIND_WORDS: Record<MentionKind, string> = {
  mention: 'mentioned you',
  reply: 'replied to you',
  word: 'matched a highlight',
};

const timeLabel = (atMs: number, now: number): string =>
  (agoLabel(new Date(atMs).toISOString(), now) ?? '').replace(/ ago$/, '');

interface RowProps {
  entry: MentionEntry;
  now: number;
  showLogo: boolean;
  muted: boolean;
  onOpen: (entry: MentionEntry) => void;
  onRemove: (entry: MentionEntry) => void;
  onToggleMute: (entry: MentionEntry) => void;
}

function MentionRow({ entry, now, showLogo, muted, onOpen, onRemove, onToggleMute }: RowProps) {
  // Rust stores the readable color the chat row shows; drawn as is.
  const color = entry.sender_color || undefined;
  const platform = isProviderId(entry.platform) ? entry.platform : null;
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => onOpen(entry)}
      onKeyDown={(e) => {
        if (e.target !== e.currentTarget) return;
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onOpen(entry);
        }
      }}
      data-mention-row={entry.id}
      className="group relative flex cursor-pointer gap-2.5 px-4 py-3 transition-colors hover:bg-surface-hover focus-visible:bg-surface-hover focus-visible:outline-none"
    >
      {/* Unread dot, on the first line's cap height. Read rows keep the
          column so nothing shifts when a row is read. */}
      <span className="flex h-[18px] w-1.5 shrink-0 items-center" aria-hidden>
        {!entry.read && <span className="h-1.5 w-1.5 rounded-full bg-accent" />}
      </span>
      {/* A read row dims its content (name, chat, message); the muted words
          around them are already quiet and stay readable. */}
      <div className="min-w-0 flex-1">
        <div className="flex h-[18px] items-center gap-1.5 text-[12.5px] leading-[18px]">
          <span
            className={`min-w-0 max-w-[45%] shrink-[3] truncate font-semibold ${entry.read ? 'opacity-60' : ''}`}
            style={{ color }}
          >
            {entry.sender_name}
          </span>
          <span className="shrink-0 text-textMuted">{KIND_WORDS[entry.kind]} in</span>
          <span className={`flex min-w-[6ch] flex-initial items-center gap-1 text-textSecondary ${entry.read ? 'opacity-60' : ''}`}>
            {showLogo && platform && <ProviderLogo provider={platform} size={12} />}
            <span className="min-w-0 truncate">{entry.channel_login}</span>
          </span>
          {/* Time, swapped for the row's actions on hover. One fixed slot, so
              the line never reflows between the two. */}
          <span className="relative ml-auto flex h-[18px] w-[52px] shrink-0 items-center justify-end pl-1">
            <span className="text-[11px] tabular-nums text-textMuted transition-opacity group-hover:opacity-0 group-focus-within:opacity-0">
              {timeLabel(entry.at_ms, now)}
            </span>
            <span className="absolute inset-y-0 -right-2 flex items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
              <Tooltip content={muted ? 'Unmute this chat' : 'Mute this chat'} side="top">
                <button
                  type="button"
                  aria-label={muted ? 'Unmute this chat' : 'Mute this chat'}
                  onClick={(e) => { e.stopPropagation(); onToggleMute(entry); }}
                  className="flex h-6 w-6 items-center justify-center rounded-md text-textMuted transition-colors hover:bg-surface-active hover:text-textPrimary"
                >
                  {muted ? <Bell size={13} /> : <BellOff size={13} />}
                </button>
              </Tooltip>
              <Tooltip content="Remove" side="top">
                <button
                  type="button"
                  aria-label="Remove"
                  onClick={(e) => { e.stopPropagation(); onRemove(entry); }}
                  className="flex h-6 w-6 items-center justify-center rounded-md text-textMuted transition-colors hover:bg-surface-active hover:text-textPrimary"
                >
                  <X size={16} />
                </button>
              </Tooltip>
            </span>
          </span>
        </div>
        {entry.removed ? (
          <p className="mt-1 text-[13px] italic leading-snug text-textMuted">Message deleted by a moderator</p>
        ) : (
          <p className={`mt-1 line-clamp-3 break-words text-[13px] leading-snug text-textPrimary ${entry.read ? 'opacity-60' : ''}`}>
            {entry.text}
          </p>
        )}
      </div>
    </div>
  );
}

export interface MentionsListProps {
  /** What opening a row does: open its chat and find the message. */
  onOpen: (entry: MentionEntry) => void;
  /** Controls after the list's own (a close button). */
  headerEnd?: ReactNode;
  /** MultiChat: a slimmer header that fits under the window's own toolbar. */
  compact?: boolean;
}

export function MentionsList({ onOpen, headerEnd, compact = false }: MentionsListProps) {
  const [entries, setEntries] = useState<MentionEntry[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [unread, setUnread] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const loadingMore = useRef(false);
  const shownCount = useRef(PAGE);
  useEffect(() => { shownCount.current = Math.max(PAGE, entries.length); }, [entries.length]);

  const muted = useAppStore((s) => s.settings.mentions?.muted_channels);
  const mutedSet = useMemo(() => new Set((muted ?? []).map((r) => r.toLowerCase())), [muted]);

  // The first page again, as long as what is loaded, after any change.
  const reload = useCallback(() => {
    invoke<MentionPage>('get_mention_inbox', { limit: Math.min(shownCount.current, PAGE_MAX) })
      .then((p) => {
        setEntries(p.entries);
        setHasMore(p.has_more);
        setUnread(p.unread_total);
        setLoaded(true);
      })
      .catch((e) => {
        Logger.warn('[Mentions] get_mention_inbox failed:', e);
        setLoaded(true);
      });
  }, []);

  useEffect(() => {
    reload();
    const unlisten = listen(MENTION_INBOX_CHANGED_EVENT, reload);
    const tick = setInterval(() => setNow(Date.now()), 60_000);
    return () => {
      unlisten.then((fn) => fn());
      clearInterval(tick);
    };
  }, [reload]);

  const loadMore = useCallback(() => {
    const last = entries[entries.length - 1];
    if (!hasMore || !last || loadingMore.current) return;
    loadingMore.current = true;
    invoke<MentionPage>('get_mention_inbox', { beforeMs: last.at_ms, limit: PAGE })
      .then((p) => {
        setEntries((prev) => {
          const seen = new Set(prev.map((e) => e.id));
          return [...prev, ...p.entries.filter((e) => !seen.has(e.id))];
        });
        setHasMore(p.has_more);
      })
      .catch((e) => Logger.warn('[Mentions] older page failed:', e))
      .finally(() => { loadingMore.current = false; });
  }, [entries, hasMore]);

  const onScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 240) loadMore();
  };

  const remove = useCallback((entry: MentionEntry) => {
    setEntries((prev) => prev.filter((e) => e.id !== entry.id));
    invoke('remove_mention', { id: entry.id }).catch((e) => Logger.warn('[Mentions] remove failed:', e));
  }, []);

  const toggleMute = useCallback((entry: MentionEntry) => {
    void toggleMentionChatMuted(entry.room);
  }, []);

  const markAllRead = () => {
    setEntries((prev) => prev.map((e) => (e.read ? e : { ...e, read: true })));
    setUnread(0);
    invoke('mark_mentions_read', {}).catch((e) => Logger.warn('[Mentions] mark read failed:', e));
  };

  const clearAll = () => {
    setConfirmClear(false);
    setEntries([]);
    setHasMore(false);
    setUnread(0);
    invoke('clear_mention_inbox').catch((e) => Logger.warn('[Mentions] clear failed:', e));
  };

  // A platform mark only where platforms mix in the list.
  const mixed = useMemo(() => new Set(entries.map((e) => e.platform)).size > 1, [entries]);

  const iconButton =
    'flex h-8 w-8 items-center justify-center rounded-lg text-textSecondary transition-colors hover:bg-surface-hover hover:text-textPrimary disabled:pointer-events-none disabled:opacity-40';

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className={`flex shrink-0 items-center justify-between gap-3 border-b border-borderSubtle ${compact ? 'h-11 px-3' : 'h-[60px] px-4'}`}>
        <div className="flex min-w-0 items-center gap-2">
          <AtSign size={compact ? 16 : 19} className="shrink-0 text-accent" />
          <span className={`font-semibold text-textPrimary ${compact ? 'text-sm' : 'text-lg'}`}>Mentions</span>
          {unread > 0 && (
            <span className="glass-badge rounded-full px-2 py-0.5 text-xs font-medium tabular-nums text-textPrimary">
              {unread > 99 ? '99+' : unread}
            </span>
          )}
        </div>
        {confirmClear ? (
          <div className="flex shrink-0 items-center gap-2">
            <span className="text-[12.5px] text-textSecondary">Clear every mention?</span>
            <button
              type="button"
              onClick={() => setConfirmClear(false)}
              className="h-7 rounded-lg px-2.5 text-[12.5px] font-medium text-textSecondary transition-colors hover:bg-surface-hover hover:text-textPrimary"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={clearAll}
              className="h-7 rounded-lg bg-red-500/15 px-2.5 text-[12.5px] font-semibold text-red-300 transition-colors hover:bg-red-500/25"
            >
              Clear all
            </button>
          </div>
        ) : (
          <div className={`flex shrink-0 items-center gap-0.5 ${compact ? '-mr-1.5' : '-mr-2.5'}`}>
            <Tooltip content="Mark all read" side="bottom">
              <button type="button" aria-label="Mark all read" onClick={markAllRead} disabled={unread === 0} className={iconButton}>
                <CheckCheck size={17} />
              </button>
            </Tooltip>
            <Tooltip content="Clear all" side="bottom">
              <button
                type="button"
                aria-label="Clear all"
                onClick={() => setConfirmClear(true)}
                disabled={entries.length === 0}
                className={iconButton}
              >
                <Trash2 size={16} />
              </button>
            </Tooltip>
            {headerEnd}
          </div>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto scrollbar-thin" onScroll={onScroll}>
        {loaded && entries.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center px-10 text-center">
            <AtSign size={36} className="mb-3 text-accent/40" />
            <span className="text-sm font-medium text-textPrimary">No mentions yet</span>
            <span className="mt-1 max-w-[30ch] text-[12.5px] leading-relaxed text-textMuted [text-wrap:balance]">
              When someone mentions or replies to you in a chat, it shows up here.
            </span>
          </div>
        ) : (
          <div className="py-1">
            {entries.map((entry) => (
              <MentionRow
                key={entry.id}
                entry={entry}
                now={now}
                showLogo={mixed}
                muted={mutedSet.has(entry.room.toLowerCase())}
                onOpen={onOpen}
                onRemove={remove}
                onToggleMute={toggleMute}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

export default MentionsList;

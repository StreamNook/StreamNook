// Mentions: everyone who named you, replied to you or used one of your
// highlight phrases, in any chat this phone had open, newest first. Opened from
// the You tab and from the chat's menu; it sits over the watch layer so the
// chat menu can reach it without closing the stream.
//
// Rust holds the inbox (services/mention_inbox.rs) and decides what is in it;
// this pages through it, relays the row actions, and re-reads whenever Rust
// says it changed. A tap opens that chat and looks for the message there.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowLeft, BellSlash, Bell, ChatCircleText, DotsThreeVertical, Trash } from 'phosphor-react';
import { useMobileNavStore } from '../navStore';
import { useAppStore } from '../../stores/AppStore';
import { DrillInScreen } from '../ui/DrillInScreen';
import { MobileSheet } from '../ui/MobileSheet';
import { EmptyNote } from '../ui/EmptyNote';
import { ProviderMark } from '../../components/ProviderLogo';
import { agoLabel } from '../../utils/lastLive';
import {
  KIND_LABEL,
  PAGE_SIZE,
  clearMentionInbox,
  getMentionPage,
  markAllMentionsRead,
  openMention,
  removeMention,
  roomName,
  roomProvider,
  setRoomMuted,
  useMentionsStore,
  type MentionEntry,
} from '../mentions/mentionInbox';
import { Logger } from '../../utils/logger';

/** Rust's largest page: a re-read after a change refetches what is loaded. */
const REREAD_MAX = 200;

const ROW_ACTION =
  'sn-touch w-full flex items-center gap-3 px-2 text-[15px] text-textPrimary active:opacity-70 disabled:opacity-45';
const ICON_BOX = 'flex w-6 h-6 shrink-0 items-center justify-center';

/** The chat a mention came from: its name, with the platform's mark only when
 *  the list mixes platforms. */
const ChatName: React.FC<{ room: string; showMark: boolean }> = ({ room, showMark }) => (
  <span className="flex min-w-0 items-center gap-1">
    {showMark && <ProviderMark provider={roomProvider(room)} size={12} />}
    <span className="truncate">{roomName(room)}</span>
  </span>
);

const MentionRow: React.FC<{
  entry: MentionEntry;
  now: number;
  showMark: boolean;
  onOpen: (e: MentionEntry) => void;
  onMore: (e: MentionEntry) => void;
}> = React.memo(({ entry, now, showMark, onOpen, onMore }) => {
  // Rust hands over the readable color the chat row shows.
  const color = entry.sender_color ?? undefined;
  const when = agoLabel(new Date(entry.at_ms).toISOString(), now) ?? '';
  const unread = !entry.read && !entry.removed;
  return (
    <div
      className={`relative flex items-start ${unread ? '' : 'opacity-60'}`}
      data-mention-row={entry.id}
    >
      <button
        onClick={() => onOpen(entry)}
        className="flex-1 min-w-0 flex items-start gap-2.5 pl-4 pr-1 py-3 text-left active:bg-surface/60"
      >
        {/* The dot's column is always there, so read and unread rows keep one
            left edge; the dot sits on the first line's center. */}
        <span className="w-2 h-[19px] shrink-0 flex items-center" aria-hidden>
          {unread && <span className="block w-2 h-2 rounded-full bg-accent" />}
        </span>
        <span className="flex-1 min-w-0 flex flex-col">
          <span className="flex items-baseline gap-1.5 min-w-0 leading-[19px]">
            <span
              className="text-[15px] font-semibold truncate min-w-0 text-textPrimary"
              style={color ? { color } : undefined}
            >
              {entry.sender_name || entry.sender_login}
            </span>
            <span className="text-[13px] text-textMuted whitespace-nowrap shrink-0">
              {KIND_LABEL[entry.kind]}
            </span>
            <span className="flex-1" />
            <span className="text-[12px] text-textMuted whitespace-nowrap shrink-0 tabular-nums">
              {when}
            </span>
          </span>
          <span className="text-[12.5px] leading-[17px] text-textMuted mt-0.5 min-w-0">
            <ChatName room={entry.room} showMark={showMark} />
          </span>
          {entry.removed ? (
            <span className="text-[14px] leading-[20px] italic text-textMuted mt-1">
              Message deleted by a moderator
            </span>
          ) : (
            <span className="text-[14px] leading-[20px] text-textSecondary mt-1 line-clamp-3 break-words [overflow-wrap:anywhere]">
              {entry.text}
            </span>
          )}
        </span>
      </button>
      <button
        onClick={() => onMore(entry)}
        className="sn-touch shrink-0 flex items-start justify-center pt-3 pr-2 text-textMuted active:text-textPrimary"
        aria-label={`More for ${entry.sender_name}'s mention`}
      >
        <DotsThreeVertical size={18} weight="bold" />
      </button>
    </div>
  );
});
MentionRow.displayName = 'MentionRow';

export const MentionsScreen: React.FC = () => {
  const open = useMobileNavStore((s) => s.mentionsOpen);
  const setOpen = useMobileNavStore((s) => s.setMentionsOpen);
  const unreadTotal = useMentionsStore((s) => s.unreadTotal);
  const revision = useMentionsStore((s) => s.revision);
  const muted = useAppStore((s) => s.settings.mentions?.muted_channels);

  const [entries, setEntries] = useState<MentionEntry[] | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [rowMenu, setRowMenu] = useState<MentionEntry | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const loadedCount = useRef(PAGE_SIZE);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Read the first page on open, and again whenever Rust says the inbox
  // changed while the screen is up (a new mention, a read, a removal), keeping
  // as many rows as are already loaded so the list never shrinks under you.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    void getMentionPage(undefined, Math.min(REREAD_MAX, Math.max(PAGE_SIZE, loadedCount.current)))
      .then((page) => {
        if (cancelled) return;
        setEntries(page.entries);
        setHasMore(page.has_more);
        useMentionsStore.setState({ unreadTotal: page.unread_total });
      })
      .catch((err) => {
        Logger.warn('[Mentions] page failed:', err);
        if (!cancelled) setEntries((e) => e ?? []);
      });
    return () => {
      cancelled = true;
    };
  }, [open, revision]);

  // Relative times stay true while the screen is open.
  useEffect(() => {
    if (!open) return;
    const t = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(t);
  }, [open]);

  const loadMore = useCallback(() => {
    if (!entries || !hasMore || loadingMore) return;
    const last = entries[entries.length - 1];
    if (!last) return;
    setLoadingMore(true);
    void getMentionPage(last.at_ms, PAGE_SIZE)
      .then((page) => {
        setEntries((cur) => {
          const seen = new Set((cur ?? []).map((e) => e.id));
          const next = [...(cur ?? []), ...page.entries.filter((e) => !seen.has(e.id))];
          loadedCount.current = next.length;
          return next;
        });
        setHasMore(page.has_more);
      })
      .catch((err) => Logger.warn('[Mentions] older page failed:', err))
      .finally(() => setLoadingMore(false));
  }, [entries, hasMore, loadingMore]);

  const onScroll = useCallback(() => {
    const el = scrollRef.current;
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 400) loadMore();
  }, [loadMore]);

  const onOpen = useCallback((e: MentionEntry) => void openMention(e), []);
  const onMore = useCallback((e: MentionEntry) => setRowMenu(e), []);

  const list = entries ?? [];
  const showMark = new Set(list.map((e) => roomProvider(e.room))).size > 1;
  const rowMuted = !!rowMenu && (muted ?? []).includes(rowMenu.room.toLowerCase());

  return (
    <DrillInScreen
      open={open}
      layerKey="mentions"
      className="absolute inset-0 z-50 bg-background flex flex-col"
      style={{ paddingTop: 'var(--sn-safe-t, 0px)' }}
    >
      <div className="flex items-center gap-1 px-2 py-2 shrink-0 border-b border-borderSubtle">
        <button
          onClick={() => setOpen(false)}
          className="sn-touch flex items-center justify-center text-textSecondary"
          aria-label="Back"
        >
          <ArrowLeft size={22} />
        </button>
        <h1 className="flex-1 min-w-0 text-[17px] font-bold text-textPrimary">Mentions</h1>
        {unreadTotal > 0 && (
          <button
            onClick={() => void markAllMentionsRead()}
            className="sn-touch px-2 text-[14px] font-medium text-textSecondary active:text-textPrimary"
          >
            Mark all read
          </button>
        )}
        {list.length > 0 && (
          <button
            onClick={() => setConfirmClear(true)}
            className="sn-touch flex items-center justify-center text-textMuted active:text-textPrimary"
            aria-label="Clear all"
          >
            <Trash size={20} />
          </button>
        )}
      </div>

      <div
        ref={scrollRef}
        onScroll={onScroll}
        className="flex-1 min-h-0 overflow-y-auto overscroll-contain"
        style={{ paddingBottom: 'calc(var(--sn-safe-b, 0px) + 16px)' }}
      >
        {entries && list.length === 0 && (
          <EmptyNote
            text="No mentions yet."
            hint="When someone mentions or replies to you in a chat, it shows up here."
          />
        )}
        {list.length > 0 && (
          <div className="divide-y divide-borderSubtle">
            {list.map((e) => (
              <MentionRow key={e.id} entry={e} now={now} showMark={showMark} onOpen={onOpen} onMore={onMore} />
            ))}
          </div>
        )}
        {hasMore && (
          <div className="flex justify-center py-4">
            <button
              onClick={loadMore}
              disabled={loadingMore}
              className="sn-touch px-3 text-[13px] text-textMuted active:text-textPrimary disabled:opacity-50"
            >
              {loadingMore ? 'Loading' : 'Show older'}
            </button>
          </div>
        )}
      </div>

      {/* One mention's actions. */}
      <MobileSheet
        open={!!rowMenu}
        onClose={() => setRowMenu(null)}
        title={rowMenu ? roomName(rowMenu.room) : undefined}
        maxHeightFraction={0.5}
      >
        <div className="flex flex-col">
          <button
            className={ROW_ACTION}
            onClick={() => {
              const e = rowMenu;
              setRowMenu(null);
              if (e) void openMention(e);
            }}
          >
            <span className={ICON_BOX}>
              <ChatCircleText size={22} className="text-textSecondary" />
            </span>
            <span className="flex-1 text-left">Open chat</span>
          </button>
          <button
            className={ROW_ACTION}
            onClick={() => {
              const e = rowMenu;
              setRowMenu(null);
              if (e) void setRoomMuted(e.room, !rowMuted);
            }}
          >
            <span className={ICON_BOX}>
              {rowMuted ? (
                <Bell size={22} className="text-textSecondary" />
              ) : (
                <BellSlash size={22} className="text-textSecondary" />
              )}
            </span>
            <span className="flex-1 text-left">{rowMuted ? 'Unmute this chat' : 'Mute this chat'}</span>
          </button>
          <div className="my-1 mx-2 border-t border-borderSubtle" />
          <button
            className={`${ROW_ACTION} !text-error`}
            onClick={() => {
              const e = rowMenu;
              setRowMenu(null);
              if (e) {
                setEntries((cur) => cur?.filter((x) => x.id !== e.id) ?? cur);
                void removeMention(e.id);
              }
            }}
          >
            <span className={ICON_BOX}>
              <Trash size={22} />
            </span>
            <span className="flex-1 text-left">Remove</span>
          </button>
        </div>
      </MobileSheet>

      {/* Clear all, behind the header's trash and asked once more here: it
          cannot be undone. */}
      <MobileSheet
        open={confirmClear}
        onClose={() => setConfirmClear(false)}
        title="Clear all mentions?"
        maxHeightFraction={0.5}
      >
        <div className="flex flex-col gap-2.5 px-2 pb-1">
          <p className="text-[14px] leading-[20px] text-textSecondary mb-1">
            Every mention leaves this list. This cannot be undone.
          </p>
          <button
            className="glass-button sn-touch w-full text-[14px] font-semibold !text-error"
            onClick={() => {
              setConfirmClear(false);
              setEntries([]);
              setHasMore(false);
              void clearMentionInbox();
            }}
          >
            Clear all
          </button>
          <button
            className="sn-touch w-full text-[14px] text-textSecondary active:text-textPrimary"
            onClick={() => setConfirmClear(false)}
          >
            Cancel
          </button>
        </div>
      </MobileSheet>
    </DrillInScreen>
  );
};

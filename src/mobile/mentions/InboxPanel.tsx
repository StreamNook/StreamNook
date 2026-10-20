// The inbox: one timeline, newest first, of every mention, reply and
// highlight (and whispers, once their store joins), with chips across the top
// to narrow it. One component, two ways in: the bottom sheet the message bar's
// tray opens, and You > Mentions.
//
// Rust builds the view (`get_inbox_view`): it filters by chip, folds identical
// messages in a row into one, and attaches the senders' pictures it has. This
// draws it, relays taps, and marks what was on screen read when it closes.
import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ArrowLeft, BellSlash, Bell, Trash } from 'phosphor-react';
import { CheckCheck, ChevronDown } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { MobileSheet } from '../ui/MobileSheet';
import { SegmentedControl } from '../ui/SegmentedControl';
import { EmptyNote } from '../ui/EmptyNote';
import { ProviderMark } from '../../components/ProviderLogo';
import { Logger } from '../../utils/logger';
import {
  PAGE_SIZE,
  closeInboxSheet,
  getInboxView,
  markAllMentionsRead,
  markInboxSeen,
  openInboxRow,
  openMentionById,
  removeMention,
  roomProvider,
  setRoomMuted,
  useMentionsStore,
  type InboxFilterId,
  type InboxRowView,
  type InboxView,
} from './mentionInbox';
import { CHIP_LABEL, emptyWords, foldedWords, inboxTime, whereWords } from './inboxText';
import { InboxFace, InboxMessage } from './InboxParts';

/** Rust's largest page: a re-read after a change refetches what is loaded. */
const REREAD_MAX = 200;
const LONG_PRESS_MS = 450;

const ROW_ACTION =
  'sn-touch w-full flex items-center gap-3 px-2 text-[15px] text-textPrimary active:opacity-70 disabled:opacity-45';
const ICON_BOX = 'flex w-6 h-6 shrink-0 items-center justify-center';

const InboxRow: React.FC<{
  row: InboxRowView;
  now: number;
  mixed: boolean;
  onHold: (row: InboxRowView) => void;
}> = React.memo(({ row, now, mixed, onHold }) => {
  const [open, setOpen] = useState(false);
  const entry = row.entry;
  const where = whereWords(entry);
  const hold = useRef<{ timer: ReturnType<typeof setTimeout>; x: number; y: number; fired: boolean } | null>(null);
  const read = !row.unread;
  const cancelHold = () => {
    if (hold.current) clearTimeout(hold.current.timer);
  };
  return (
    <div data-inbox-row={entry.id} data-inbox-ids={[entry.id, ...row.folded.map((f) => f.id)].join(' ')}>
      <button
        type="button"
        className="w-full flex items-start gap-2.5 px-3 py-2.5 text-left active:bg-textPrimary/5 select-none [-webkit-touch-callout:none]"
        onPointerDown={(e) => {
          const timer = setTimeout(() => {
            if (hold.current) hold.current.fired = true;
            onHold(row);
          }, LONG_PRESS_MS);
          hold.current = { timer, x: e.clientX, y: e.clientY, fired: false };
        }}
        onPointerMove={(e) => {
          const h = hold.current;
          if (h && Math.hypot(e.clientX - h.x, e.clientY - h.y) > 8) clearTimeout(h.timer);
        }}
        onPointerUp={cancelHold}
        onPointerCancel={cancelHold}
        onContextMenu={(e) => e.preventDefault()}
        onClick={() => {
          if (hold.current?.fired) return;
          openInboxRow(row);
        }}
      >
        <span className="mt-1 shrink-0 block">
          <InboxFace entry={entry} avatar={row.avatar} size={32} dim={read} />
        </span>
        <span className="flex-1 min-w-0 flex flex-col">
          <span className="flex items-baseline gap-1.5 min-w-0 leading-[19px]">
            <span
              className="text-[14px] font-bold truncate min-w-0 max-w-[55%] text-textPrimary"
              style={entry.sender_color ? { color: entry.sender_color } : undefined}
            >
              {entry.sender_name || entry.sender_login}
            </span>
            <span className="shrink-0 text-[12.5px] text-textMuted">{where.lead}</span>
            {where.place && (
              <span className="min-w-0 flex items-baseline gap-1 text-[12.5px] text-textSecondary">
                {mixed && (
                  <span className="self-center shrink-0">
                    <ProviderMark provider={roomProvider(entry.room)} size={11} />
                  </span>
                )}
                <span className="truncate">{where.place}</span>
              </span>
            )}
            <span className="flex-1" />
            <span className="shrink-0 text-[11.5px] text-textMuted tabular-nums">{inboxTime(entry.at_ms, now)}</span>
            <span
              className="shrink-0 w-2 h-2 rounded-full self-center ml-1"
              style={{ background: row.unread ? 'var(--color-accent)' : 'transparent' }}
              aria-label={row.unread ? 'Unread' : undefined}
            />
          </span>
          <span
            className={`text-[14px] leading-[20px] mt-0.5 line-clamp-2 break-words [overflow-wrap:anywhere] ${
              read ? 'text-textSecondary' : 'text-textPrimary'
            }`}
          >
            <InboxMessage entry={entry} emote={21} />
          </span>
        </span>
      </button>
      {row.folded.length > 0 && (
        <div className="pl-[54px] pr-3 -mt-1.5 pb-2">
          <button
            type="button"
            onClick={() => setOpen((v) => !v)}
            aria-expanded={open}
            className="sn-touch-y flex items-center gap-1 text-[12px] font-medium text-textMuted active:text-textPrimary"
          >
            {foldedWords(entry, row.folded.length + 1)}
            <ChevronDown size={13} strokeWidth={2.4} className={`transition-transform ${open ? 'rotate-180' : ''}`} />
          </button>
          {open && (
            <div className="mt-1 flex flex-col">
              {row.folded.map((f) => (
                <button
                  key={f.id}
                  type="button"
                  onClick={() => {
                    closeInboxSheet();
                    void openMentionById(f.id, 1);
                  }}
                  className="flex items-center gap-2 py-1.5 text-left text-[12.5px] text-textSecondary active:text-textPrimary"
                >
                  <span className="w-1.5 h-1.5 rounded-full shrink-0" style={{ background: f.read ? 'var(--color-border)' : 'var(--color-accent)' }} />
                  <span className="tabular-nums text-textMuted w-12 shrink-0">{inboxTime(f.at_ms, now)}</span>
                  <span className="truncate">The same message</span>
                </button>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
});
InboxRow.displayName = 'InboxRow';

export const InboxPanel: React.FC<{
  /** `sheet` sits in the chat's bottom sheet (its grabber and title are the
   *  sheet's own); `screen` is You > Mentions, with a way back. */
  variant: 'sheet' | 'screen';
  onClose: () => void;
}> = ({ variant, onClose }) => {
  const revision = useMentionsStore((s) => s.revision);
  const muted = useAppStore((s) => s.settings.mentions?.muted_channels);
  const [filter, setFilter] = useState<InboxFilterId>('all');
  const [view, setView] = useState<InboxView | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [failed, setFailed] = useState(false);
  const [held, setHeld] = useState<InboxRowView | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const loaded = useRef(PAGE_SIZE);
  const scrollRef = useRef<HTMLDivElement>(null);
  const seen = useRef(new Set<string>());

  // The first page on open and on each chip, then again whenever Rust says
  // the inbox changed (or a picture landed), keeping as many rows as are
  // loaded so the list never shrinks under you.
  useEffect(() => {
    let cancelled = false;
    void getInboxView(filter, null, Math.min(REREAD_MAX, Math.max(PAGE_SIZE, loaded.current)))
      .then((v) => {
        if (cancelled) return;
        setView(v);
        setFailed(false);
        useMentionsStore.setState({ unreadTotal: v.unread_total });
      })
      .catch((err) => {
        Logger.warn('[Inbox] view failed:', err);
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [filter, revision]);

  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(t);
  }, []);

  // What was on screen is read once the inbox closes: the dots stay while you
  // look, and the count on the tray clears as you leave.
  const rows = view?.rows;
  useLayoutEffect(() => {
    const root = scrollRef.current;
    if (!root || !rows?.length) return;
    const observer = new IntersectionObserver(
      (items) => {
        for (const it of items) {
          if (!it.isIntersecting) continue;
          const ids = (it.target as HTMLElement).dataset.inboxIds?.split(' ') ?? [];
          ids.forEach((id) => seen.current.add(id));
        }
      },
      { root, threshold: 0.6 },
    );
    root.querySelectorAll('[data-inbox-ids]').forEach((el) => observer.observe(el));
    return () => observer.disconnect();
  }, [rows]);
  useEffect(() => {
    const ids = seen.current;
    return () => {
      const shown = [...ids];
      Logger.debug('[Inbox] closing, marking seen', shown.length);
      markInboxSeen(shown);
    };
  }, []);

  const loadMore = useCallback(() => {
    if (!view?.has_more || loadingMore || view.before_ms == null) return;
    setLoadingMore(true);
    const at = filter;
    void getInboxView(at, view.before_ms, PAGE_SIZE)
      .then((page) => {
        setView((cur) => {
          if (!cur || at !== filter) return cur;
          const have = new Set(cur.rows.map((r) => r.entry.id));
          const next = [...cur.rows, ...page.rows.filter((r) => !have.has(r.entry.id))];
          loaded.current = next.length;
          return { ...page, rows: next };
        });
      })
      .catch((err) => Logger.warn('[Inbox] older page failed:', err))
      .finally(() => setLoadingMore(false));
  }, [view, loadingMore, filter]);

  const onScroll = useCallback(() => {
    const el = scrollRef.current;
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 300) loadMore();
  }, [loadMore]);

  const pick = (f: InboxFilterId) => {
    if (f === filter) return;
    loaded.current = PAGE_SIZE;
    setFilter(f);
    scrollRef.current?.scrollTo({ top: 0 });
  };

  const list = view?.rows ?? [];
  const mixed = new Set(list.map((r) => roomProvider(r.entry.room))).size > 1;
  const unread = view?.unread_total ?? 0;
  const chips = view?.chips ?? [
    { id: 'all' as const, unread: 0 },
    { id: 'mentions' as const, unread: 0 },
    { id: 'highlights' as const, unread: 0 },
  ];
  const heldMuted = !!held && (muted ?? []).includes(held.entry.room.toLowerCase());
  const empty = emptyWords(filter);

  return (
    <div className="flex flex-col min-h-0 flex-1 h-full">
      {/* The screen's header: back, the title and count, mark all read. The
          sheet's grabber and title are MobileSheet's. */}
      {variant === 'screen' && (
        <div className="shrink-0 flex items-center h-10 pr-[2.5px] pl-1">
          <button
            onClick={onClose}
            className="sn-touch flex items-center justify-center text-textSecondary -mr-1"
            aria-label="Back"
          >
            <ArrowLeft size={22} />
          </button>
          <span className="flex items-baseline gap-2 min-w-0">
            <span className="text-[16px] font-bold text-textPrimary tracking-tight">Inbox</span>
            {unread > 0 && <span className="text-[12.5px] font-medium text-textMuted tabular-nums">{unread} new</span>}
          </span>
          <span className="flex-1" />
          {unread > 0 && (
            <button
              onClick={() => void markAllMentionsRead()}
              className="w-9 h-9 flex items-center justify-center text-textMuted active:text-textPrimary"
              aria-label="Mark all read"
            >
              <CheckCheck size={18} strokeWidth={2.1} />
            </button>
          )}
        </div>
      )}
      {/* The categories as the app's segmented control, each with its unread
          count in its label (no corner dot). */}
      <div className="shrink-0 flex items-center gap-1 px-3 pb-2">
        <div className="flex-1 min-w-0">
          <SegmentedControl<InboxFilterId>
            label="Show"
            fill
            value={filter}
            onChange={pick}
            segments={chips.map((c) => ({ value: c.id, label: CHIP_LABEL[c.id] ?? c.id, count: c.unread }))}
          />
        </div>
        {variant === 'sheet' && unread > 0 && (
          <button
            onClick={() => void markAllMentionsRead()}
            className="shrink-0 -mr-1.5 w-9 h-9 flex items-center justify-center text-textMuted active:text-textPrimary"
            aria-label={`Mark all read, ${unread} new`}
          >
            <CheckCheck size={18} strokeWidth={2.1} />
          </button>
        )}
      </div>
      <div
        ref={scrollRef}
        onScroll={onScroll}
        className="sn-inbox-list flex-1 min-h-0 overflow-y-auto overscroll-contain"
        style={variant === 'screen' ? { paddingBottom: 'calc(var(--sn-safe-b, 0px) + 16px)' } : { paddingBottom: 16 }}
      >
        {view && list.length === 0 && (
          <EmptyNote text={failed ? 'The inbox did not load.' : empty.text} hint={failed ? 'Try again in a moment.' : empty.hint} />
        )}
        {!view && failed && <EmptyNote text="The inbox did not load." hint="Try again in a moment." />}
        {list.map((r) => (
          <InboxRow key={r.entry.id} row={r} now={now} mixed={mixed} onHold={setHeld} />
        ))}
        {view?.has_more && (
          <div className="flex justify-center py-3">
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

      {/* A held row's own actions. */}
      <MobileSheet
        open={!!held}
        onClose={() => setHeld(null)}
        title={held ? (whereWords(held.entry).place ?? held.entry.sender_name) : undefined}
        maxHeightFraction={0.5}
      >
        <div className="flex flex-col">
          <button
            className={ROW_ACTION}
            onClick={() => {
              const h = held;
              setHeld(null);
              if (h) void setRoomMuted(h.entry.room, !heldMuted);
            }}
          >
            <span className={ICON_BOX}>
              {heldMuted ? <Bell size={22} className="text-textSecondary" /> : <BellSlash size={22} className="text-textSecondary" />}
            </span>
            <span className="flex-1 text-left">{heldMuted ? 'Unmute this chat' : 'Mute this chat'}</span>
          </button>
          <div className="my-1 mx-2 border-t border-borderSubtle" />
          <button
            className={`${ROW_ACTION} !text-error`}
            onClick={() => {
              const h = held;
              setHeld(null);
              if (!h) return;
              setView((cur) => (cur ? { ...cur, rows: cur.rows.filter((x) => x.entry.id !== h.entry.id) } : cur));
              void removeMention(h.entry.id);
              h.folded.forEach((f) => void removeMention(f.id));
            }}
          >
            <span className={ICON_BOX}>
              <Trash size={22} />
            </span>
            <span className="flex-1 text-left">Remove</span>
          </button>
        </div>
      </MobileSheet>
    </div>
  );
};

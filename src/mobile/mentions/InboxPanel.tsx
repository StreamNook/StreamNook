// The inbox: one timeline, newest first, of every mention, reply, highlight
// and whisper conversation, with chips across the top to narrow it. One
// component, two ways in: the bottom sheet the message bar's tray opens, and
// You > Inbox.
//
// Rust builds the view (`get_inbox_view`): it filters by chip, folds identical
// messages in a row into one, merges the whisper conversations in by time, and
// attaches the senders' pictures it has. This draws it, relays taps, and marks
// the mention rows that were on screen read when it closes. A whisper
// conversation is read only by opening it.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Archive, ArrowLeft, BellSlash, Bell, Check, Trash } from 'phosphor-react';
import { CheckCheck, ChevronDown } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { MobileSheet } from '../ui/MobileSheet';
import { SegmentedControl } from '../ui/SegmentedControl';
import { EmptyNote } from '../ui/EmptyNote';
import { PullToRefresh } from '../ui/PullToRefresh';
import { ProviderMark } from '../../components/ProviderLogo';
import { PinnedChatter } from '../../components/chat/PinnedChatter';
import { Logger } from '../../utils/logger';
import { SESSION_PROBLEM_TEXT } from '../../utils/whisperDelivery';
import type { InboxMentionItem, InboxWhisperItem, WhisperRowMeta } from '../../types';
import {
  PAGE_SIZE,
  bumpInbox,
  closeInboxSheet,
  getInboxView,
  markAllInboxRead,
  markInboxSeen,
  noteInboxOpened,
  openInboxRow,
  openMentionById,
  removeMention,
  roomProvider,
  setRoomMuted,
  takeCounts,
  useMentionsStore,
  type InboxFilterId,
  type InboxItem,
  type InboxViewPayload,
} from './mentionInbox';
import { CHIP_LABEL, emptyWords, foldedWords, inboxTime, whereWords } from './inboxText';
import { InboxFace, InboxMessage, InboxParts } from './InboxParts';
import { chipsOf, faceOfEntry, mentionItems, mergePage, seenIdsOf, whisperName, whisperPreview } from './inboxModel';
import {
  archiveWhisper,
  markWhisperRead,
  syncWhispersNow,
  useWhispersStore,
} from '../whispers/whisperInbox';

/** Rust's largest page: a re-read after a change refetches what is loaded. */
const REREAD_MAX = 200;
const LONG_PRESS_MS = 450;

const ROW_ACTION =
  'sn-touch w-full flex items-center gap-3 px-2 text-[15px] text-textPrimary active:opacity-70 disabled:opacity-45';
const ICON_BOX = 'flex w-6 h-6 shrink-0 items-center justify-center';
const ROW_BUTTON =
  'w-full flex items-start gap-2.5 px-3 py-2.5 text-left active:bg-textPrimary/5 select-none [-webkit-touch-callout:none]';

/** A press that is a tap, or a hold after `LONG_PRESS_MS` that is not. */
function usePressHold(onHold: () => void, onTap: () => void) {
  const hold = useRef<{ timer: ReturnType<typeof setTimeout>; x: number; y: number; fired: boolean } | null>(null);
  const cancel = () => {
    if (hold.current) clearTimeout(hold.current.timer);
  };
  return {
    onPointerDown: (e: React.PointerEvent) => {
      const timer = setTimeout(() => {
        if (hold.current) hold.current.fired = true;
        onHold();
      }, LONG_PRESS_MS);
      hold.current = { timer, x: e.clientX, y: e.clientY, fired: false };
    },
    onPointerMove: (e: React.PointerEvent) => {
      const h = hold.current;
      if (h && Math.hypot(e.clientX - h.x, e.clientY - h.y) > 8) clearTimeout(h.timer);
    },
    onPointerUp: cancel,
    onPointerCancel: cancel,
    onContextMenu: (e: React.MouseEvent) => e.preventDefault(),
    onClick: () => {
      if (hold.current?.fired) return;
      onTap();
    },
  };
}

const MentionRow: React.FC<{
  row: InboxMentionItem;
  now: number;
  mixed: boolean;
  onHold: (row: InboxItem) => void;
}> = React.memo(({ row, now, mixed, onHold }) => {
  const [open, setOpen] = useState(false);
  const entry = row.entry;
  const where = whereWords(entry);
  const read = !row.unread;
  const press = usePressHold(
    () => onHold(row),
    () => openInboxRow(row),
  );
  return (
    <div data-inbox-row={row.row_id} data-inbox-ids={seenIdsOf(row).join(' ')}>
      <button type="button" className={ROW_BUTTON} {...press}>
        <span className="mt-1 shrink-0 block">
          <InboxFace face={faceOfEntry(entry)} avatar={row.avatar} size={32} dim={read} />
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
MentionRow.displayName = 'MentionRow';

/** A whisper partner's picture: a plain round face (no corner mark), with the
 *  live ring while they stream. The first letter stands in while there is none. */
const WhisperFace: React.FC<{ m: WhisperRowMeta; size: number }> = ({ m, size }) => {
  const [broken, setBroken] = useState<string | null>(null);
  const url = m.profile_image_url && broken !== m.profile_image_url ? m.profile_image_url : null;
  const ring = m.live ? 'ring-2 ring-live/80' : '';
  return url ? (
    <img
      src={url}
      alt=""
      draggable={false}
      onError={() => setBroken(url)}
      className={`block shrink-0 rounded-full object-cover bg-surface ${ring}`}
      style={{ width: size, height: size }}
    />
  ) : (
    <span
      aria-hidden
      className={`flex shrink-0 items-center justify-center rounded-full bg-surface font-semibold text-textSecondary ${ring}`}
      style={{ width: size, height: size, fontSize: Math.round(size * 0.38) }}
    >
      {(m.user_name || m.user_login).slice(0, 1).toUpperCase()}
    </span>
  );
};

/** The accent count on a whisper row: no corner marks, "99+" past 99. */
const UnreadCapsule: React.FC<{ count: number }> = ({ count }) => (
  <span
    className="shrink-0 min-w-[22px] h-[22px] px-[7px] rounded-full text-[12px] font-bold leading-[22px] text-center tabular-nums"
    style={{ background: 'var(--color-accent)', color: 'color-mix(in srgb, var(--color-accent) 14%, black)' }}
  >
    {count > 99 ? '99+' : count}
  </span>
);

/** A whisper conversation at its newest message, in direction A's row: the
 *  person's face (live ring while streaming), their name in its paint or color
 *  (no badges: a run of them pushed the name out of the line; the
 *  conversation's header shows them), the time, what was said last ("You: " for yours), and the
 *  unread count as an accent capsule. Roomy (52 px face) on the Whispers chip;
 *  in All it keeps the mention rows' 32 px column so the timeline lines up. */
const WhisperRow: React.FC<{ row: InboxWhisperItem; now: number; roomy: boolean; onHold: (row: InboxItem) => void }> = React.memo(
  ({ row, now, roomy, onHold }) => {
    const m = row.whisper;
    const unread = m.unread_count > 0;
    const press = usePressHold(
      () => onHold(row),
      () => openInboxRow(row),
    );
    return (
      <div data-inbox-row={row.row_id} data-whisper-row={m.key}>
        <button
          type="button"
          className={roomy ? `${ROW_BUTTON} !items-center !gap-[13px] !py-[11px]` : ROW_BUTTON}
          aria-label={`${unread ? 'Unread. ' : ''}${whisperName(m)}: ${whisperPreview(m)}`}
          {...press}
        >
          <span className={roomy ? 'shrink-0 block' : 'mt-1 shrink-0 block'}>
            <WhisperFace m={m} size={roomy ? 52 : 32} />
          </span>
          <span className="flex-1 min-w-0 flex flex-col gap-1">
            <span className="flex items-center gap-1.5 min-w-0 leading-[20px]">
              <span className={`${roomy ? 'text-[15.5px]' : 'text-[14px]'} flex items-center gap-1 min-w-0`}>
                <PinnedChatter
                  variant="name"
                  provider="twitch"
                  userId={m.user_id}
                  name={m.user_name || m.user_login}
                  login={m.user_login}
                  color={m.color ?? undefined}
                  nameClassName="font-bold truncate min-w-0"
                />
              </span>
              <span className="flex-1" />
              <span className="shrink-0 text-[12px] text-textMuted tabular-nums">
                {inboxTime(m.last_message_timestamp, now)}
              </span>
            </span>
            <span className="flex items-center gap-2 min-w-0">
              <span
                className={`flex-1 min-w-0 ${roomy ? 'truncate' : 'line-clamp-2 break-words [overflow-wrap:anywhere]'} text-[14px] leading-[20px] ${
                  unread ? 'font-semibold text-textPrimary' : 'text-textSecondary'
                }`}
              >
                {m.last_is_sent && <span className="font-normal text-textMuted">You: </span>}
                {m.last_parts.length > 0 ? (
                  <InboxParts parts={m.last_parts} text={m.last_text} emote={21} />
                ) : (
                  m.last_text
                )}
              </span>
              {unread && <UnreadCapsule count={m.unread_count} />}
            </span>
          </span>
        </button>
      </div>
    );
  },
);
WhisperRow.displayName = 'WhisperRow';

export const InboxPanel: React.FC<{
  /** `sheet` sits in the chat's bottom sheet (its grabber and title are the
   *  sheet's own); `screen` is You > Inbox, with a way back. */
  variant: 'sheet' | 'screen';
  onClose: () => void;
}> = ({ variant, onClose }) => {
  const revision = useMentionsStore((s) => s.revision);
  const muted = useAppStore((s) => s.settings.mentions?.muted_channels);
  const status = useWhispersStore((s) => s.status);
  const [filter, setFilter] = useState<InboxFilterId>('all');
  const [view, setView] = useState<InboxViewPayload | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [failed, setFailed] = useState(false);
  const [held, setHeld] = useState<InboxItem | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const loaded = useRef(PAGE_SIZE);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const seen = useRef(new Set<string>());

  // Coming on screen asks Rust for a light whisper sync (it decides how often).
  useEffect(() => {
    noteInboxOpened();
  }, []);

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
        takeCounts(v.unread_total, v.counts_rev);
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
  // look, and the count on the tray clears as you leave. Only mention rows
  // carry ids to mark; a whisper conversation is read by opening it.
  const rows = view?.rows;
  useEffect(() => {
    const root = scrollRef.current;
    if (!root || !rows?.length) return;
    const observer = new IntersectionObserver(
      (items) => {
        for (const it of items) {
          if (!it.isIntersecting) continue;
          const ids = (it.target as HTMLElement).dataset.inboxIds?.split(' ').filter(Boolean) ?? [];
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
    if (!view?.has_more || loadingMore || view.cursor == null) return;
    setLoadingMore(true);
    const at = filter;
    void getInboxView(at, view.cursor, PAGE_SIZE)
      .then((page) => {
        setView((cur) => {
          if (!cur || at !== filter) return cur;
          const next = mergePage(cur, page);
          loaded.current = next.rows.length;
          return next;
        });
      })
      .catch((err) => Logger.warn('[Inbox] older page failed:', err))
      .finally(() => setLoadingMore(false));
  }, [view, loadingMore, filter]);

  // The scroller is this panel's own div, or the pull-to-refresh's on the
  // screen; either way it is in `scrollRef` by the time effects run.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onScroll = () => {
      if (el.scrollHeight - el.scrollTop - el.clientHeight < 300) loadMore();
    };
    el.addEventListener('scroll', onScroll, { passive: true });
    return () => el.removeEventListener('scroll', onScroll);
  }, [loadMore, variant]);

  const pick = (f: InboxFilterId) => {
    if (f === filter) return;
    loaded.current = PAGE_SIZE;
    setFilter(f);
    scrollRef.current?.scrollTo({ top: 0 });
  };

  const refresh = useCallback(async () => {
    await syncWhispersNow();
    bumpInbox();
  }, []);

  const list = view?.rows ?? [];
  const mixed = new Set(mentionItems(list).map((r) => roomProvider(r.entry.room))).size > 1;
  const unread = view?.unread_total ?? 0;
  const chips = chipsOf(view);
  const heldMention = held?.kind === 'mention' ? held : null;
  const heldWhisper = held?.kind === 'whisper' ? held : null;
  const heldMuted = !!heldMention && (muted ?? []).includes(heldMention.entry.room.toLowerCase());
  const empty = emptyWords(filter, filter === 'whispers' && !!status?.syncing);
  const sessionProblem = (filter === 'all' || filter === 'whispers') && status?.session_problem ? status.session_problem : null;

  const body = (
    <>
      {/* The twitch.tv session belongs to someone else: whispers cannot load
          until the person signs in there again. A steady state, so it sits at
          the top rather than coming and going. */}
      {sessionProblem && (
        <div className="mx-3 mb-2 px-3 py-2.5 glass-panel flex items-center gap-3" role="status">
          <span className="flex-1 min-w-0 text-[13px] leading-[18px] text-textSecondary">{SESSION_PROBLEM_TEXT}</span>
          <button
            type="button"
            onClick={() => void useAppStore.getState().loginToTwitch()}
            className="sn-touch shrink-0 px-2 text-[13px] font-semibold text-accent-on-surface active:opacity-70"
          >
            Sign in
          </button>
        </div>
      )}
      {view && list.length === 0 && (
        <EmptyNote text={failed ? 'The inbox did not load.' : empty.text} hint={failed ? 'Try again in a moment.' : empty.hint} />
      )}
      {!view && failed && <EmptyNote text="The inbox did not load." hint="Try again in a moment." />}
      {list.map((r) =>
        r.kind === 'mention' ? (
          <MentionRow key={r.row_id} row={r} now={now} mixed={mixed} onHold={setHeld} />
        ) : (
          <WhisperRow key={r.row_id} row={r} now={now} roomy={filter === 'whispers'} onHold={setHeld} />
        ),
      )}
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
    </>
  );

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
              onClick={() => void markAllInboxRead()}
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
            onClick={() => void markAllInboxRead()}
            className="shrink-0 -mr-1.5 w-9 h-9 flex items-center justify-center text-textMuted active:text-textPrimary"
            aria-label={`Mark all read, ${unread} new`}
          >
            <CheckCheck size={18} strokeWidth={2.1} />
          </button>
        )}
      </div>
      {/* The screen pulls down to sync whispers now; a sheet does not, since
          pulling a sheet down puts it away. */}
      {variant === 'screen' ? (
        <PullToRefresh onRefresh={refresh} scrollerRef={scrollRef} className="sn-inbox-list overscroll-contain">
          <div style={{ paddingBottom: 'calc(var(--sn-safe-b, 0px) + 16px)' }}>{body}</div>
        </PullToRefresh>
      ) : (
        <div
          ref={scrollRef}
          className="sn-inbox-list flex-1 min-h-0 overflow-y-auto overscroll-contain"
          style={{ paddingBottom: 16 }}
        >
          {body}
        </div>
      )}

      {/* A held mention row's own actions. */}
      <MobileSheet
        open={!!heldMention}
        onClose={() => setHeld(null)}
        title={heldMention ? (whereWords(heldMention.entry).place ?? heldMention.entry.sender_name) : undefined}
        maxHeightFraction={0.5}
      >
        <div className="flex flex-col">
          <button
            className={ROW_ACTION}
            onClick={() => {
              const h = heldMention;
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
              const h = heldMention;
              setHeld(null);
              if (!h) return;
              setView((cur) => (cur ? { ...cur, rows: cur.rows.filter((x) => x.row_id !== h.row_id) } : cur));
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

      {/* A held whisper conversation's own actions. */}
      <MobileSheet
        open={!!heldWhisper}
        onClose={() => setHeld(null)}
        title={heldWhisper ? whisperName(heldWhisper.whisper) : undefined}
        maxHeightFraction={0.5}
      >
        <div className="flex flex-col">
          {heldWhisper && heldWhisper.whisper.unread_count > 0 && (
            <>
              <button
                className={ROW_ACTION}
                onClick={() => {
                  const h = heldWhisper;
                  setHeld(null);
                  if (h) markWhisperRead(h.whisper.key);
                }}
              >
                <span className={ICON_BOX}>
                  <Check size={22} className="text-textSecondary" />
                </span>
                <span className="flex-1 text-left">Mark read</span>
              </button>
              <div className="my-1 mx-2 border-t border-borderSubtle" />
            </>
          )}
          <button
            className={ROW_ACTION}
            onClick={() => {
              const h = heldWhisper;
              setHeld(null);
              if (!h) return;
              setView((cur) => (cur ? { ...cur, rows: cur.rows.filter((x) => x.row_id !== h.row_id) } : cur));
              void archiveWhisper(h.whisper.key);
            }}
          >
            <span className={ICON_BOX}>
              <Archive size={22} className="text-textSecondary" />
            </span>
            <span className="flex-1 text-left">Archive</span>
          </button>
        </div>
      </MobileSheet>
    </div>
  );
};

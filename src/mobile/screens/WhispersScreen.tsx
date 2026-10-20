// Whispers: your Twitch private conversations, newest first, and one
// conversation with a composer. The list opens from the You tab (for now,
// `openWhispers`); a conversation opens in the bottom sheet the chat menu
// uses (`openWhisper`, which a tapped whisper notification uses too). Both sit
// over the watch layer like Mentions, so a stream keeps playing underneath.
//
// Rust holds the conversations (services/whisper_inbox.rs): this reads the list
// and a thread, marks the open one read, and sends through `whisper_send`. A
// message typed here shows as sending until Twitch answers, then as sent (Rust's
// own copy) or as not sent with Twitch's reason, with Retry and Discard.
import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeft, WarningCircle } from 'phosphor-react';
import { MobileSheet } from '../ui/MobileSheet';
import { useMobileNavStore } from '../navStore';
import { DrillInScreen } from '../ui/DrillInScreen';
import { EmptyNote } from '../ui/EmptyNote';
import { PullToRefresh } from '../ui/PullToRefresh';
import { SevenTvAvatar } from '../../components/SevenTvAvatar';
import { Logger } from '../../utils/logger';
import type { WhisperConversation } from '../../types';
import {
  discardWhisper,
  loadWhisperThread,
  refreshWhisperList,
  retryWhisper,
  sendWhisper,
  setOpenWhisper,
  useWhispersStore,
  type WhisperListRow,
} from '../whispers/whisperInbox';
import {
  WHISPER_WORDS,
  applyToThread,
  composerPlaceholder,
  previewLine,
  rowTime,
  shownName,
  threadItems,
  type PendingWhisper,
} from '../whispers/whisperModel';

/** Messages drawn at first in a long thread; "Show earlier" adds this many. */
const THREAD_PAGE = 200;

const Picture: React.FC<{ id: string; url: string | null | undefined; size: number; name: string }> = ({
  id,
  url,
  size,
  name,
}) =>
  url ? (
    <SevenTvAvatar
      cosmeticsId={id || null}
      src={url}
      px={size}
      alt=""
      className="shrink-0 rounded-full object-cover bg-surface"
      style={{ width: size, height: size }}
    />
  ) : (
    <span
      className="shrink-0 rounded-full bg-surface flex items-center justify-center text-textSecondary font-semibold"
      style={{ width: size, height: size, fontSize: Math.round(size * 0.4) }}
      aria-hidden
    >
      {name.slice(0, 1).toUpperCase()}
    </span>
  );

/** An unread count in the accent's capsule; nothing at zero; past 99, "99+". */
const UnreadCount: React.FC<{ count: number }> = ({ count }) =>
  count > 0 ? (
    <span className="shrink-0 min-w-[20px] h-[20px] px-1.5 rounded-full bg-accent text-[11.5px] font-semibold leading-[20px] text-center tabular-nums text-background [html[data-theme-tone=light]_&]:text-white">
      {count > 99 ? '99+' : count}
    </span>
  ) : null;

const ConversationRow: React.FC<{ row: WhisperListRow; now: number; onOpen: (key: string) => void }> = React.memo(
  ({ row, now, onOpen }) => {
    const unread = row.unread_count > 0;
    const name = shownName(row);
    return (
      <button
        onClick={() => onOpen(row.key)}
        data-whisper-row={row.key}
        className="w-full flex items-center gap-3 px-4 py-3 text-left active:bg-surface/60"
        aria-label={`${unread ? 'Unread. ' : ''}${name}: ${previewLine(row)}`}
      >
        <Picture id={row.user_id} url={row.profile_image_url} size={44} name={name} />
        {/* Read conversations are dimmed rather than unread ones decorated,
            as in Mentions; the name keeps one weight so it never changes width. */}
        <span className={`flex-1 min-w-0 flex flex-col gap-0.5 ${unread ? '' : 'opacity-60'}`}>
          <span className="flex items-baseline gap-2 min-w-0">
            <span className="flex-1 min-w-0 truncate text-[15px] leading-[20px] text-textPrimary font-semibold">
              {name}
            </span>
            <span className="shrink-0 text-[12px] text-textMuted tabular-nums">
              {rowTime(row.last_message_timestamp, now)}
            </span>
          </span>
          <span className="flex items-center gap-2 min-w-0">
            <span className="flex-1 min-w-0 truncate text-[13.5px] leading-[18px] text-textSecondary">
              {previewLine(row)}
            </span>
            <UnreadCount count={row.unread_count} />
          </span>
        </span>
      </button>
    );
  },
);
ConversationRow.displayName = 'ConversationRow';

// Yours in the accent's tint: strong enough in dark to part from the surface on
// a grey accent, lighter on light themes so the text keeps its contrast. A
// message still on its way takes a fainter fill with its text at full strength
// (the "Sending" line carries the state).
const MINE = 'bg-accent/35 [html[data-theme-tone=light]_&]:bg-accent/20';
const MINE_SENDING = 'bg-accent/20 [html[data-theme-tone=light]_&]:bg-accent/10';

const Bubble: React.FC<{ text: string; mine: boolean; sending?: boolean; failed?: boolean }> = ({
  text,
  mine,
  sending,
  failed,
}) => (
  <div className={`flex ${mine ? 'justify-end' : 'justify-start'}`}>
    <div
      className={`max-w-[78%] px-3 py-[7px] rounded-[18px] text-[15px] leading-[20px] whitespace-pre-wrap break-words [overflow-wrap:anywhere] text-textPrimary ${
        mine ? (sending ? MINE_SENDING : MINE) : 'bg-surface'
      } ${failed ? 'ring-1 ring-inset ring-error/60' : ''}`}
    >
      {text}
    </div>
  </div>
);

const PendingLine: React.FC<{ pending: PendingWhisper; row: WhisperListRow }> = ({ pending, row }) => (
  <div className="flex flex-col gap-1">
    <Bubble
      text={pending.text}
      mine
      sending={pending.status === 'sending'}
      failed={pending.status === 'failed'}
    />
    {pending.status === 'sending' ? (
      <span className="self-end pr-1 text-[11.5px] text-textMuted">{WHISPER_WORDS.sending}</span>
    ) : (
      // Under its bubble, right edges on the time line's: why, then what to do.
      <div className="self-end max-w-[78%] flex flex-col items-end gap-0.5 pr-1 text-[12.5px] text-right" role="alert">
        {/* The mark rides the first line, right beside the words it marks. */}
        <span className="text-textSecondary">
          <WarningCircle size={14} weight="fill" className="inline-block align-[-2px] mr-1.5 text-error" />
          {pending.reason}
        </span>
        <span className="flex justify-end gap-3">
          <button
            onClick={() => retryWhisper(pending.localId, row)}
            className="font-semibold text-accent-on-surface active:opacity-70"
          >
            {WHISPER_WORDS.retry}
          </button>
          <button onClick={() => discardWhisper(pending.localId)} className="text-textMuted active:text-textPrimary">
            {WHISPER_WORDS.discard}
          </button>
        </span>
      </div>
    )}
  </div>
);

/** One conversation: the thread, oldest at the top, and the composer, inside
 *  its bottom sheet (which brings the grabber, rides the keyboard and clears
 *  the gesture bar). */
const ConversationView: React.FC<{ conversationKey: string }> = ({ conversationKey }) => {
  const listRow = useWhispersStore((s) => s.rows.find((r) => r.key === conversationKey));
  const revision = useWhispersStore((s) => s.revision);
  const lastUpdate = useWhispersStore((s) => s.lastUpdate);
  const allPending = useWhispersStore((s) => s.pending);
  const [stored, setThread] = useState<WhisperConversation | null>(null);
  // The newest change shows at once; the full re-read below follows it.
  const thread = useMemo(
    () => (lastUpdate && stored ? applyToThread(stored, conversationKey, lastUpdate) : stored),
    [stored, lastUpdate, conversationKey],
  );
  const [loaded, setLoaded] = useState(false);
  const [shown, setShown] = useState(THREAD_PAGE);
  const [draft, setDraft] = useState('');
  const [now, setNow] = useState(() => Date.now());
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickToBottom = useRef(true);

  // Rust counts this conversation as open (and read) while it is on screen.
  useEffect(() => {
    setOpenWhisper(conversationKey);
    return () => setOpenWhisper(null);
  }, [conversationKey]);

  useEffect(() => {
    let cancelled = false;
    void loadWhisperThread(conversationKey)
      .then((t) => {
        if (cancelled) return;
        setThread(t);
        setLoaded(true);
      })
      .catch((err) => {
        Logger.warn('[Whispers] thread failed:', err);
        if (!cancelled) setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [conversationKey, revision]);

  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(t);
  }, []);

  // Who this is with: the list's row, else the thread's own record.
  const row: WhisperListRow | null = useMemo(() => {
    if (listRow) return listRow;
    if (!thread) return null;
    const last = thread.messages[thread.messages.length - 1];
    return {
      key: conversationKey,
      user_id: thread.user_id,
      user_login: thread.user_login,
      user_name: thread.user_name,
      profile_image_url: thread.profile_image_url ?? null,
      last_message_timestamp: thread.last_message_timestamp,
      unread_count: thread.unread_count,
      last_text: last?.message ?? '',
      last_is_sent: last?.is_sent ?? false,
    };
  }, [listRow, thread, conversationKey]);

  const messages = useMemo(() => thread?.messages ?? [], [thread]);
  const pending = useMemo(() => allPending.filter((p) => p.key === conversationKey), [allPending, conversationKey]);
  const hidden = Math.max(0, messages.length - shown);
  const items = useMemo(() => threadItems(messages.slice(hidden), pending, now), [messages, hidden, pending, now]);

  // Opened at the newest message, and kept there as messages arrive unless
  // you scrolled up to read.
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el && stickToBottom.current) el.scrollTop = el.scrollHeight;
  }, [items.length]);

  const onScroll = useCallback(() => {
    const el = scrollRef.current;
    if (el) stickToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  }, []);

  const send = () => {
    if (!row || !draft.trim()) return;
    stickToBottom.current = true;
    sendWhisper(row, draft);
    setDraft('');
  };

  const name = row ? shownName(row) : '';
  return (
    // The sheet is `flush`: this lays out and scrolls the thread itself.
    <div className="flex-1 min-h-0 flex flex-col">
      <div className="flex items-center gap-2 px-4 pb-2 shrink-0 border-b border-borderSubtle">
        {row && <Picture id={row.user_id} url={row.profile_image_url} size={30} name={name} />}
        <span className="flex-1 min-w-0 flex flex-col">
          <span className="truncate text-[16px] font-bold leading-[20px] text-textPrimary">{name}</span>
          {row && row.user_login && row.user_login.toLowerCase() !== name.toLowerCase() && (
            <span className="truncate text-[12px] leading-[15px] text-textMuted">@{row.user_login}</span>
          )}
        </span>
      </div>

      <div
        ref={scrollRef}
        onScroll={onScroll}
        // Messages fade out under the header instead of being cut by its line.
        className="flex-1 min-h-0 overflow-y-auto overscroll-contain px-3 pt-2 pb-2 [mask-image:linear-gradient(to_bottom,transparent,black_14px)]"
      >
        {hidden > 0 && (
          <div className="flex justify-center py-2">
            <button
              onClick={() => {
                stickToBottom.current = false;
                setShown((n) => n + THREAD_PAGE);
              }}
              className="sn-touch px-3 text-[13px] text-textMuted active:text-textPrimary"
            >
              Show earlier messages
            </button>
          </div>
        )}
        {loaded && messages.length === 0 && pending.length === 0 && (
          <EmptyNote text={WHISPER_WORDS.threadEmpty} hint={null} />
        )}
        <div className="flex flex-col">
          {items.map((item) => {
            if (item.kind === 'day') {
              return (
                <div key={item.id} className="py-3 text-center text-[12px] font-medium text-textMuted">
                  {item.label}
                </div>
              );
            }
            if (item.kind === 'pending') {
              return row ? (
                <div key={item.id} className="pt-[3px]">
                  <PendingLine pending={item.pending} row={row} />
                </div>
              ) : null;
            }
            return (
              <div key={item.id} className={`flex flex-col ${item.last ? 'pb-2' : 'pb-[3px]'}`}>
                <Bubble text={item.message.message} mine={item.mine} />
                {item.time && (
                  <span
                    className={`pt-1 px-1 text-[11.5px] text-textMuted tabular-nums ${item.mine ? 'self-end' : 'self-start'}`}
                  >
                    {item.time}
                  </span>
                )}
              </div>
            );
          })}
        </div>
      </div>

      {/* The sheet rides the keyboard and clears the gesture bar. */}
      <div className="shrink-0 border-t border-borderSubtle pb-2.5">
        <div className="flex items-end gap-1.5 px-2.5 pt-2">
          <div className="glass-input flex-1 min-w-0 min-h-10 flex items-center px-3">
            <textarea
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  send();
                }
              }}
              placeholder={row ? composerPlaceholder(row) : ''}
              disabled={!row}
              rows={1}
              maxLength={500}
              className="flex-1 min-w-0 resize-none bg-transparent py-2 text-[15px] leading-[1.4] text-textPrimary outline-none max-h-[96px] placeholder:text-textMuted"
              enterKeyHint="send"
            />
          </div>
          <button
            onPointerDown={(e) => e.preventDefault()}
            onClick={send}
            disabled={!row || !draft.trim()}
            className="chrome-glaze chrome-glaze--control shrink-0 flex items-center justify-center self-end w-10 h-10 text-white transition-colors duration-200 disabled:text-textMuted"
            aria-label="Send"
          >
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8" />
            </svg>
          </button>
        </div>
      </div>
    </div>
  );
};

export const WhispersScreen: React.FC = () => {
  const open = useMobileNavStore((s) => s.whispersOpen);
  const openKey = useMobileNavStore((s) => s.whisperKey);
  const closeWhispers = useMobileNavStore((s) => s.closeWhispers);
  const openWhisper = useMobileNavStore((s) => s.openWhisper);
  const closeWhisper = useMobileNavStore((s) => s.closeWhisper);
  const rows = useWhispersStore((s) => s.rows);
  const loaded = useWhispersStore((s) => s.loaded);
  const failed = useWhispersStore((s) => s.failed);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!open) return;
    void refreshWhisperList().then(() => setNow(Date.now()));
    const t = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(t);
  }, [open]);

  const onOpen = useCallback((key: string) => openWhisper(key), [openWhisper]);

  return (
    <>
      <DrillInScreen
        open={open}
        layerKey="whispers"
        className="absolute inset-0 z-50 bg-background flex flex-col"
        style={{ paddingTop: 'var(--sn-safe-t, 0px)' }}
      >
        <div className="flex items-center gap-1 px-2 py-2 shrink-0 border-b border-borderSubtle">
          <button
            onClick={closeWhispers}
            className="sn-touch flex items-center justify-center text-textSecondary"
            aria-label="Back"
          >
            <ArrowLeft size={22} />
          </button>
          <h1 className="flex-1 min-w-0 text-[17px] font-bold text-textPrimary">{WHISPER_WORDS.title}</h1>
        </div>
        <PullToRefresh onRefresh={refreshWhisperList}>
          <div style={{ paddingBottom: 'calc(var(--sn-safe-b, 0px) + 16px)' }}>
            {(loaded || failed) && rows.length === 0 && (
              <EmptyNote
                text={failed ? WHISPER_WORDS.failed : WHISPER_WORDS.empty}
                hint={failed ? WHISPER_WORDS.failedHint : WHISPER_WORDS.emptyHint}
              />
            )}
            {rows.length > 0 && (
              <div className="divide-y divide-borderSubtle">
                {rows.map((row) => (
                  <ConversationRow key={row.key} row={row} now={now} onOpen={onOpen} />
                ))}
              </div>
            )}
          </div>
        </PullToRefresh>
      </DrillInScreen>
      {/* `fill`: the thread arrives after it opens, so it never grows under the finger. */}
      <MobileSheet open={!!openKey} onClose={closeWhisper} maxHeightFraction={0.85} fill flush>
        {openKey && <ConversationView key={openKey} conversationKey={openKey} />}
      </MobileSheet>
    </>
  );
};

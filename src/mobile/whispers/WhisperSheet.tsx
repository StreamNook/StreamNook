// A whisper conversation, in the bottom sheet the chat menu uses, over
// whatever is on screen: an inbox row, a tapped whisper notification and the
// arrival card all open it (navStore `openWhisper`). Mounted once by the
// shell, so a notification tap finds it from anywhere.
//
// Rust holds the conversation (services/whisper_inbox.rs): this reads pages of
// it, applies Rust's updates as they come, and relays sends, Retry, Discard,
// Archive and older pages. Where a send stands (Sending, Checking, Sent,
// Delivered, Not delivered) is Rust's; the thread only shows it.
import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Archive, WarningCircle } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { liveLine } from '../../components/whispers/whisperView';
import { MobileSheet } from '../ui/MobileSheet';
import { EmptyNote } from '../ui/EmptyNote';
import { useMobileNavStore } from '../navStore';
import { SevenTvAvatar } from '../../components/SevenTvAvatar';
import { PinnedChatter } from '../../components/chat/PinnedChatter';
import { Logger } from '../../utils/logger';
import type { StoredWhisperMessage } from '../../types';
import { InboxParts } from '../mentions/InboxParts';
import {
  archiveWhisper,
  discardWhisper,
  loadOlderFromTwitch,
  loadWhisperPage,
  retryWhisper,
  sendWhisper,
  setOpenWhisper,
  useWhispersStore,
} from './whisperInbox';
import {
  PAGE,
  REREAD_MAX,
  WHISPER_WORDS,
  canSend,
  composerPlaceholder,
  counterText,
  fromPage,
  scalarLength,
  shownName,
  threadItems,
  updateNeedsReread,
  withOlder,
  withRefresh,
  withUpdate,
  type StatusLine,
  type ThreadState,
} from './whisperModel';

const Picture: React.FC<{ id: string; url: string | null; size: number; name: string; live?: boolean }> = ({
  id,
  url,
  size,
  name,
  live = false,
}) =>
  url ? (
    <SevenTvAvatar
      cosmeticsId={id || null}
      src={url}
      px={size}
      alt=""
      className={`shrink-0 rounded-full object-cover bg-surface ${live ? 'ring-2 ring-live/80' : ''}`}
      style={{ width: size, height: size }}
    />
  ) : (
    <span
      className={`shrink-0 rounded-full bg-surface flex items-center justify-center text-textSecondary font-semibold ${live ? 'ring-2 ring-live/80' : ''}`}
      style={{ width: size, height: size, fontSize: Math.round(size * 0.4) }}
      aria-hidden
    >
      {name.slice(0, 1).toUpperCase()}
    </span>
  );

// Direction A's glass bubbles. Theirs: a neutral film lit from the top. Yours:
// the same glass in the accent's tint, lighter on light themes so the text keeps
// its contrast. A message still on its way takes a fainter fill with its text
// at full strength (the status line carries the state); one that did not arrive
// takes the error color on its rim.
const LIT = 'inset 0 1px 0 color-mix(in srgb, white 12%, transparent)';
const THEIRS: React.CSSProperties = {
  background: 'color-mix(in srgb, var(--color-text-primary) 7%, transparent)',
  boxShadow: `${LIT}, inset 0 0 0 1px color-mix(in srgb, var(--color-text-primary) 6%, transparent)`,
};
const MINE_RIM = 'inset 0 0 0 1px color-mix(in srgb, var(--color-accent) 32%, transparent)';
const MINE = 'bg-accent/[0.32] [html[data-theme-tone=light]_&]:bg-accent/20';
const MINE_SENDING = 'bg-accent/[0.18] [html[data-theme-tone=light]_&]:bg-accent/10';

const Bubble: React.FC<{ message: StoredWhisperMessage; mine: boolean; status: StatusLine | null }> = ({
  message,
  mine,
  status,
}) => {
  const fainter = status?.delivery === 'sending' || status?.delivery === 'unknown';
  const refused = status?.delivery === 'not_delivered';
  const rim = refused ? 'inset 0 0 0 1px color-mix(in srgb, var(--color-error) 60%, transparent)' : MINE_RIM;
  return (
    <div className={`flex ${mine ? 'justify-end' : 'justify-start'}`}>
      <div
        className={`max-w-[78%] px-[15px] py-[11px] rounded-[20px] text-[15px] leading-[21px] whitespace-pre-wrap break-words [overflow-wrap:anywhere] text-textPrimary ${
          mine ? (fainter ? MINE_SENDING : MINE) : ''
        }`}
        style={mine ? { boxShadow: `${LIT}, ${rim}` } : THEIRS}
      >
        {message.deleted ? (
          <span className="italic text-textMuted">{WHISPER_WORDS.deleted}</span>
        ) : (
          <>
            <InboxParts parts={message.parts} text={message.message} emote={22} />
            {message.edited_ms != null && <span className="ml-1.5 text-[11.5px] text-textMuted">({WHISPER_WORDS.edited})</span>}
          </>
        )}
      </div>
    </div>
  );
};

/** Under one of our sends: where it stands, and for a refused one why and what
 *  to do. Buttons keep the keyboard where it is. */
const Status: React.FC<{ status: StatusLine; conversationKey: string; messageId: string }> = ({
  status,
  conversationKey,
  messageId,
}) =>
  status.actions ? (
    // Under its bubble, right edges on the time line's: why, then what to do.
    <div className="self-end max-w-[78%] flex flex-col items-end gap-0.5 pt-1 pr-1 text-[12.5px] text-right" role="alert">
      {/* The mark rides the first line, right beside the words it marks. */}
      <span className="text-textSecondary">
        <WarningCircle size={14} weight="fill" className="inline-block align-[-2px] mr-1.5 text-error" />
        {status.label}. {status.failure}
      </span>
      <span className="flex justify-end gap-3">
        <button
          type="button"
          onPointerDown={(e) => e.preventDefault()}
          onClick={() => retryWhisper(conversationKey, messageId)}
          className="font-semibold text-accent-on-surface active:opacity-70"
        >
          {WHISPER_WORDS.retry}
        </button>
        <button
          type="button"
          onPointerDown={(e) => e.preventDefault()}
          onClick={() => discardWhisper(conversationKey, messageId)}
          className="text-textMuted active:text-textPrimary"
        >
          {WHISPER_WORDS.discard}
        </button>
      </span>
    </div>
  ) : (
    <span className="self-end pt-1 pr-1 text-[11.5px] text-textMuted">{status.label}</span>
  );

/** One conversation: the thread, oldest at the top, and the composer, inside
 *  its bottom sheet (which brings the grabber, rides the keyboard and clears
 *  the gesture bar). */
const ConversationView: React.FC<{ conversationKey: string; onArchived: () => void }> = ({ conversationKey, onArchived }) => {
  const revision = useWhispersStore((s) => s.revision);
  const lastUpdate = useWhispersStore((s) => s.lastUpdate);
  const [thread, setThread] = useState<ThreadState | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [older, setOlder] = useState<'idle' | 'loading' | 'failed'>('idle');
  const [draft, setDraft] = useState('');
  const [sendError, setSendError] = useState<string | null>(null);
  const [reread, setReread] = useState(0);
  const [now, setNow] = useState(() => Date.now());
  const threadRef = useRef<ThreadState | null>(null);
  useEffect(() => {
    threadRef.current = thread;
  }, [thread]);
  const pendingText = useRef<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickToBottom = useRef(true);
  const keepFromBottom = useRef<number | null>(null);

  // Rust counts this conversation as open (and reads it through) while it is
  // on screen.
  useEffect(() => {
    setOpenWhisper(conversationKey);
    return () => setOpenWhisper(null);
  }, [conversationKey]);

  // The newest page on open, then again after a sync or an adoption, keeping
  // what is loaded so the thread never shrinks under you.
  useEffect(() => {
    let cancelled = false;
    const have = threadRef.current?.messages.length ?? 0;
    void loadWhisperPage(conversationKey, null, Math.min(REREAD_MAX, Math.max(PAGE, have)))
      .then((p) => {
        if (cancelled) return;
        setThread((cur) => (cur ? withRefresh(cur, p) : fromPage(p)));
        setLoaded(true);
      })
      .catch((err) => {
        Logger.warn('[Whispers] page failed:', err);
        if (!cancelled) setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [conversationKey, revision, reread]);

  // One change at a time from Rust, applied at once.
  useEffect(() => {
    if (!lastUpdate) return;
    if (updateNeedsReread(threadRef.current, conversationKey, lastUpdate)) setReread((n) => n + 1);
    setThread((cur) => withUpdate(cur, conversationKey, lastUpdate));
    const m = lastUpdate.message;
    if (m?.is_sent && pendingText.current !== null && m.message === pendingText.current) pendingText.current = null;
    if ((lastUpdate.key === conversationKey || lastUpdate.replaced_key === conversationKey) && lastUpdate.loading === false && lastUpdate.error) {
      setOlder('failed');
    }
  }, [lastUpdate, conversationKey]);

  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(t);
  }, []);

  const messages = useMemo(() => thread?.messages ?? [], [thread]);
  const items = useMemo(() => threadItems(messages, now), [messages, now]);

  // Opened at the newest message, kept there as messages arrive unless you
  // scrolled up to read; an older page added above keeps what you were looking at.
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (keepFromBottom.current !== null) {
      el.scrollTop = el.scrollHeight - keepFromBottom.current;
      keepFromBottom.current = null;
    } else if (stickToBottom.current) {
      el.scrollTop = el.scrollHeight;
    }
  }, [items]);

  const onScroll = useCallback(() => {
    const el = scrollRef.current;
    if (el) stickToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  }, []);

  const rememberPosition = () => {
    const el = scrollRef.current;
    if (el) keepFromBottom.current = el.scrollHeight - el.scrollTop;
  };

  const showEarlier = async () => {
    const t = threadRef.current;
    if (!t || older === 'loading' || !t.cursor) return;
    setOlder('loading');
    try {
      const p = await loadWhisperPage(conversationKey, t.cursor);
      rememberPosition();
      setThread((cur) => (cur ? withOlder(cur, p) : cur));
      setOlder('idle');
    } catch (err) {
      Logger.warn('[Whispers] earlier page failed:', err);
      setOlder('failed');
    }
  };

  const loadFromTwitch = async () => {
    const t = threadRef.current;
    if (!t || older === 'loading') return;
    setOlder('loading');
    try {
      await loadOlderFromTwitch(conversationKey);
      const cur = threadRef.current;
      const p = await loadWhisperPage(conversationKey, cur?.cursor ?? null);
      rememberPosition();
      setThread((c) => (c ? (c.cursor ? withOlder(c, p) : withRefresh(c, p)) : fromPage(p)));
      setOlder('idle');
    } catch (err) {
      Logger.warn('[Whispers] older from Twitch failed:', err);
      setOlder('failed');
    }
  };

  const meta = thread?.meta ?? null;
  const length = scalarLength(draft.trim());
  const counter = counterText(length);
  const sendable = !!meta && canSend(draft);

  const send = async () => {
    const text = draft.trim();
    if (!meta || !canSend(text)) return;
    stickToBottom.current = true;
    setDraft('');
    setSendError(null);
    pendingText.current = text;
    const err = await sendWhisper(meta, text);
    // Refused before Rust filed anything: the words come back to the composer.
    if (err && pendingText.current === text) {
      setDraft((d) => d || text);
      setSendError(err);
    }
    pendingText.current = null;
  };

  const archive = async () => {
    await archiveWhisper(conversationKey);
    onArchived();
  };

  const name = meta ? shownName(meta) : '';
  const olderControl = thread?.hasMoreLocal
    ? { label: WHISPER_WORDS.showEarlier, run: showEarlier }
    : thread?.olderOnTwitch
      ? { label: WHISPER_WORDS.olderOnTwitch, run: loadFromTwitch }
      : null;

  return (
    // The sheet is `flush`: this lays out and scrolls the thread itself.
    <div className="flex-1 min-h-0 flex flex-col">
      {/* Direction A's header: the face (live ring while they stream), badges
          and painted name, the live line (else @login, so the name never
          shifts), Watch while live, and Archive a step quieter. */}
      <div className="flex items-center gap-3 px-4 pt-1.5 pb-3.5 shrink-0 border-b border-borderSubtle">
        {meta && <Picture id={meta.user_id} url={meta.profile_image_url} size={44} name={name} live={!!meta.live} />}
        <span className="flex-1 min-w-0 flex flex-col gap-[3px]">
          <span className="text-[16.5px] leading-[21px] flex items-center gap-1 min-w-0">
            {meta && (
              <PinnedChatter
                variant="line"
                provider="twitch"
                userId={meta.user_id}
                name={meta.user_name || meta.user_login}
                login={meta.user_login}
                color={meta.color ?? undefined}
                badges={meta.badges}
                nameClassName="font-bold truncate min-w-0"
              />
            )}
          </span>
          {meta && liveLine(meta.live) ? (
            <span className="flex items-center gap-1.5 min-w-0 text-[12.5px] leading-4 text-textSecondary">
              <span className="w-1.5 h-1.5 shrink-0 rounded-full bg-live" aria-hidden />
              <span className="truncate">{liveLine(meta.live)}</span>
            </span>
          ) : (
            meta && <span className="truncate text-[12.5px] leading-4 text-textMuted">@{meta.user_login}</span>
          )}
        </span>
        {meta?.live && (
          <button
            type="button"
            onClick={() => {
              void useAppStore.getState().startStream(meta.user_login);
              useMobileNavStore.getState().closeWhisper();
            }}
            className="glaze-inset shrink-0 h-10 px-3.5 rounded-[13px] bg-white/[0.06] text-[14px] font-semibold text-textPrimary active:opacity-70"
          >
            Watch
          </button>
        )}
        {meta && (
          <button
            type="button"
            onClick={() => void archive()}
            aria-label={WHISPER_WORDS.archive}
            className="sn-touch shrink-0 w-10 h-10 flex items-center justify-center rounded-[13px] text-textMuted active:text-textPrimary"
          >
            <Archive size={20} />
          </button>
        )}
      </div>

      <div
        ref={scrollRef}
        onScroll={onScroll}
        // Messages fade out under the header instead of being cut by its line.
        className="flex-1 min-h-0 overflow-y-auto overscroll-contain px-3 pt-2 pb-2 [mask-image:linear-gradient(to_bottom,transparent,black_14px)]"
      >
        {olderControl && (
          <div className="flex flex-col items-center gap-0.5 py-2">
            <button
              type="button"
              onClick={() => {
                stickToBottom.current = false;
                void olderControl.run();
              }}
              disabled={older === 'loading'}
              className="sn-touch px-3 text-[13px] text-textMuted active:text-textPrimary disabled:opacity-60"
            >
              {older === 'loading' ? WHISPER_WORDS.loadingOlder : olderControl.label}
            </button>
            {older === 'failed' && <span className="text-[12px] text-textMuted">{WHISPER_WORDS.olderFailed}</span>}
          </div>
        )}
        {loaded && messages.length === 0 && <EmptyNote text={WHISPER_WORDS.threadEmpty} hint={null} />}
        <div className="flex flex-col">
          {items.map((item) =>
            item.kind === 'day' ? (
              <div key={item.id} className="py-3 text-center text-[12px] font-medium text-textMuted">
                {item.label}
              </div>
            ) : (
              <div key={item.id} className={`flex flex-col ${item.last ? 'pb-2' : 'pb-[3px]'}`}>
                <Bubble message={item.message} mine={item.mine} status={item.status} />
                {item.cut && <span className="self-end pt-1 pr-1 text-[11.5px] text-textMuted">{item.cut}</span>}
                {item.status ? (
                  <Status status={item.status} conversationKey={conversationKey} messageId={item.message.id} />
                ) : (
                  item.time && (
                    <span className={`pt-1 px-1 text-[11.5px] text-textMuted tabular-nums ${item.mine ? 'self-end' : 'self-start'}`}>
                      {item.time}
                    </span>
                  )
                )}
              </div>
            ),
          )}
        </div>
      </div>

      {/* The sheet rides the keyboard and clears the gesture bar. */}
      <div className="shrink-0 pb-2.5">
        {(sendError || counter) && (
          <div className="flex items-baseline gap-3 px-3.5 pt-1.5 text-[12px]">
            <span className="flex-1 min-w-0 text-error" role={sendError ? 'alert' : undefined}>
              {sendError}
            </span>
            {counter && (
              <span className={`shrink-0 tabular-nums ${length > 500 ? 'text-error' : 'text-textMuted'}`}>{counter}</span>
            )}
          </div>
        )}
        {/* Direction A's composer: one glazed rounded bar holding the field and
            the accent Send. */}
        <div
          className="chrome-glaze mx-2.5 mt-2 flex items-end gap-1.5 py-1 pl-3.5 pr-1"
          style={{ borderRadius: 22, '--glaze-frost': '24%' } as React.CSSProperties}
        >
          <div className="flex-1 min-w-0 min-h-10 flex items-center">
            <textarea
              value={draft}
              onChange={(e) => {
                setDraft(e.target.value);
                if (sendError) setSendError(null);
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  void send();
                }
              }}
              placeholder={meta ? composerPlaceholder(meta) : ''}
              disabled={!meta}
              rows={1}
              className="flex-1 min-w-0 resize-none bg-transparent py-2 text-[15px] leading-[1.4] text-textPrimary outline-none max-h-[96px] placeholder:text-textMuted"
              enterKeyHint="send"
            />
          </div>
          <button
            type="button"
            onPointerDown={(e) => e.preventDefault()}
            onClick={() => void send()}
            disabled={!sendable}
            className="glaze-inset shrink-0 flex items-center justify-center self-end w-10 h-10 rounded-full text-white transition-opacity duration-200 disabled:opacity-40"
            style={{ background: 'color-mix(in srgb, var(--color-accent) 55%, black)' }}
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

/** The sheet host: open while navStore names a conversation. */
export const WhisperSheet: React.FC = () => {
  const openKey = useMobileNavStore((s) => s.whisperKey);
  const closeWhisper = useMobileNavStore((s) => s.closeWhisper);
  return (
    // `fill`: the thread arrives after it opens, so it never grows under the finger.
    <MobileSheet open={!!openKey} onClose={closeWhisper} maxHeightFraction={0.85} fill flush>
      {openKey && <ConversationView key={openKey} conversationKey={openKey} onArchived={closeWhisper} />}
    </MobileSheet>
  );
};

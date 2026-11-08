// A conversation's messages, oldest first, as glass bubbles: theirs on the
// left, yours on the right in the accent's tint. Each side's messages run
// together under a day line, and where a send stands shows under the newest
// one of your run. Rust decides every state; this draws it.
import type { CSSProperties, ReactNode } from 'react';
import { motion } from 'framer-motion';
import type { StoredWhisperMessage } from '../../types';
import { Logger } from '../../utils/logger';
import { deliveryOf } from '../../utils/whisperDelivery';
import { WhisperParts } from './WhisperParts';
import { canRetry, cutWords, deliveryLabel, threadLayout } from './whisperView';

const URL_RE = /(https?:\/\/[^\s]+|www\.[^\s]+)/g;

async function openLink(url: string): Promise<void> {
  try {
    const { open } = await import('@tauri-apps/plugin-shell');
    await open(url);
  } catch (err) {
    Logger.error('[Whispers] Failed to open URL:', err);
  }
}

/** Words with their links made clickable; the app opens them outside. */
export function linkify(text: string, keyBase: string): ReactNode[] {
  return text.split(URL_RE).map((part, i) => {
    if (/^(https?:\/\/|www\.)/.test(part)) {
      const url = part.startsWith('http') ? part : `https://${part}`;
      return (
        <a
          key={`${keyBase}-${i}`}
          href={url}
          // No target="_blank": the webview opens _blank links externally on
          // its own, which would stack a second tab on top of the open() below.
          className="text-accent hover:text-accentHover underline cursor-pointer"
          onClick={(e) => {
            e.preventDefault();
            void openLink(url);
          }}
        >
          {part}
        </a>
      );
    }
    return part;
  });
}

/** Theirs: a neutral glass bubble lit from the top. */
const THEIRS_STYLE: CSSProperties = {
  background: 'color-mix(in srgb, var(--color-text-primary) 6%, transparent)',
  boxShadow:
    'inset 0 1px 0 color-mix(in srgb, white 8%, transparent), inset 0 0 0 1px color-mix(in srgb, var(--color-text-primary) 6%, transparent)',
};
/** Yours: the same glass tinted with the theme accent. */
const MINE_STYLE: CSSProperties = {
  background: 'color-mix(in srgb, var(--color-accent) 30%, transparent)',
  boxShadow:
    'inset 0 1px 0 color-mix(in srgb, white 16%, transparent), inset 0 0 0 1px color-mix(in srgb, var(--color-accent) 34%, transparent)',
};
/** A send still on its way: fainter fill, full-strength text. */
const MINE_PENDING_STYLE: CSSProperties = {
  ...MINE_STYLE,
  background: 'color-mix(in srgb, var(--color-accent) 18%, transparent)',
};
/** A send that did not arrive: the rim takes the error color. */
const MINE_FAILED_STYLE: CSSProperties = {
  ...MINE_STYLE,
  boxShadow:
    'inset 0 1px 0 color-mix(in srgb, white 16%, transparent), inset 0 0 0 1px color-mix(in srgb, var(--color-error) 60%, transparent)',
};

const EMOTE_24 = 'mx-px inline-block h-6 w-auto align-middle';

export interface WhisperMessagesProps {
  messages: StoredWhisperMessage[];
  nowMs: number;
  formatTime: (ms: number) => string;
  /** Motion Off or Performance mode: bubbles appear at once. */
  instant: boolean;
  onRetry: (m: StoredWhisperMessage) => void;
  onDiscard: (m: StoredWhisperMessage) => void;
}

/** One send's status line, or the reason, Retry and Discard when it did not
 *  arrive. Its slot exists from Sending on, so a send turning Delivered never
 *  moves anything. */
function StatusLine({
  m,
  onRetry,
  onDiscard,
}: {
  m: StoredWhisperMessage;
  onRetry: (m: StoredWhisperMessage) => void;
  onDiscard: (m: StoredWhisperMessage) => void;
}) {
  if (canRetry(m)) {
    return (
      <div className="flex max-w-[62%] flex-col items-end gap-1 self-end pr-1.5 pt-1 text-right" role="alert">
        <span className="text-[11.5px] leading-snug text-error">
          Not delivered{m.failure ? `. ${m.failure}` : ''}
        </span>
        <span className="flex gap-3 text-[12px]">
          <button type="button" onClick={() => onRetry(m)} className="font-semibold text-textPrimary hover:text-accent transition-colors">
            Retry
          </button>
          <button type="button" onClick={() => onDiscard(m)} className="text-textMuted hover:text-textPrimary transition-colors">
            Discard
          </button>
        </span>
      </div>
    );
  }
  return (
    <span data-delivery="" className="self-end pr-1.5 pt-1 text-[11.5px] leading-4 text-textMuted">
      {deliveryLabel(m)}
    </span>
  );
}

export function WhisperMessages({ messages, nowMs, formatTime, instant, onRetry, onDiscard }: WhisperMessagesProps) {
  const items = threadLayout(messages, nowMs);
  return (
    <div className="flex flex-col">
      {items.map((item) => {
        if (item.kind === 'day') {
          return (
            <div key={item.id} className="self-center pb-2 pt-4 text-[12px] font-medium text-textMuted first:pt-0">
              {item.label}
            </div>
          );
        }
        const m = item.message;
        const mine = item.mine;
        const state = mine ? deliveryOf(m) : null;
        const style = !mine
          ? THEIRS_STYLE
          : state === 'not_delivered' && canRetry(m)
            ? MINE_FAILED_STYLE
            : state === 'sending' || state === 'unknown'
              ? MINE_PENDING_STYLE
              : MINE_STYLE;
        const cut = cutWords(m);
        return (
          <motion.div
            key={item.id}
            data-whisper-message={m.id}
            initial={instant ? false : { opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: instant ? 0 : 0.14, ease: 'easeOut' }}
            className={`flex flex-col ${mine ? 'items-end' : 'items-start'} ${item.last ? 'pb-2.5' : 'pb-1'}`}
          >
            <div
              // tooltip-exempt: the bubble's own send time, no other hover target
              title={formatTime(m.timestamp)}
              className="max-w-[62%] rounded-[18px] px-[15px] py-[10px] text-[14.5px] leading-[21px] text-textPrimary break-words whitespace-pre-wrap [overflow-wrap:anywhere]"
              style={style}
            >
              {m.deleted ? (
                <span className="italic text-textMuted">Message deleted</span>
              ) : (
                <>
                  <WhisperParts parts={m.parts} text={m.message} linkify={linkify} emoteClass={EMOTE_24} />
                  {typeof m.edited_ms === 'number' && <span className="ml-1.5 text-[11.5px] text-textMuted">Edited</span>}
                </>
              )}
            </div>
            {cut && <span className="pr-1.5 pt-1 text-[11.5px] text-textMuted">{cut}</span>}
            {item.showStatus && <StatusLine m={m} onRetry={onRetry} onDiscard={onDiscard} />}
          </motion.div>
        );
      })}
    </div>
  );
}

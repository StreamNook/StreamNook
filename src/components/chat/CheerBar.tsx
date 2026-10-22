// The cheer bar: one line above the message box while what you are writing
// cheers. It says what Send will spend and anything in the way, live, from
// Rust's check of the box (check_cheer, cached balance, no network per key).
// Send is the yes: a cheer the bar already showed goes out at once
// (cheerPromptStore.hold). The x takes the cheer out and keeps your words.
// If Twitch refuses, the bar says why and the message comes back to the box.
import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { X } from 'lucide-react';
import { useCheerPromptStore, useCheerDraft } from '../../stores/cheerPromptStore';
import { checkCheer, formatBits, openBitsWindow, type CheerCheck } from '../../services/bitsService';
import { Logger } from '../../utils/logger';
import { ACCENT_BUTTON, ACCENT_FILL, GLAZE_PILL } from '../ui/glazeButtons';
import { Tooltip } from '../ui/Tooltip';

interface Props {
  /** Bare login of the channel this box sends to (Twitch only). */
  channelLogin: string | undefined;
  channelId: string | undefined;
  /** The box's text, for the live check. Absent where the box sends to several
   *  chats at once: then only a held or refused cheer shows. */
  text?: string;
  /** Replace the box's text (remove the cheer, put a refused message back). */
  onSetText: (text: string) => void;
}

const bitsWord = (n: number) => `${formatBits(n)} ${n === 1 ? 'bit' : 'bits'}`;

export function CheerBar({ channelLogin, channelId, text, onSetText }: Props) {
  const channel = channelLogin ? channelLogin.toLowerCase().replace(/^#/, '') : '';
  const setDraft = useCheerPromptStore((s) => s.setDraft);
  const status = useCheerPromptStore((s) => (s.status && channel && s.status.check.channel === channel ? s.status : null));
  const restore = useCheerPromptStore((s) => (s.restore && s.restore.channel === channel ? s.restore : null));
  const send = useCheerPromptStore((s) => s.send);
  const dismiss = useCheerPromptStore((s) => s.dismiss);
  const live = useCheerDraft(channel, text);

  // A purchase on Twitch's bits page changes what the check says.
  const [balanceNonce, setBalanceNonce] = useState(0);
  useEffect(() => {
    const off = listen('bits-balance-changed', () => setBalanceNonce((n) => n + 1));
    return () => {
      void off.then((un) => un());
    };
  }, []);

  // Rust checks the box a moment after typing settles. A cheer word always
  // ends in its amount, so a box with no digit is no cheer.
  useEffect(() => {
    if (!channel || !channelId || text === undefined) return;
    if (!text.trim() || !/\d/.test(text)) {
      setDraft(channel, null);
      return;
    }
    let cancelled = false;
    const t = window.setTimeout(() => {
      checkCheer(channel, channelId, text)
        .then((check) => {
          if (!cancelled) setDraft(channel, check);
        })
        .catch((err) => Logger.warn('[Cheer] live check failed:', err));
    }, 250);
    return () => {
      cancelled = true;
      window.clearTimeout(t);
    };
  }, [channel, channelId, text, balanceNonce, setDraft]);

  // A refused cheer comes back to the box, unless something new was typed.
  const textRef = useRef(text);
  const handledRef = useRef(restore?.seq ?? 0);
  useEffect(() => {
    textRef.current = text;
  }, [text]);
  useEffect(() => {
    if (!restore || restore.seq === handledRef.current) return;
    handledRef.current = restore.seq;
    if (!textRef.current?.trim()) onSetText(restore.text);
  }, [restore, onSetText]);

  const shown: CheerCheck | null = status?.check ?? live;
  if (!shown) return null;

  const problem =
    status?.kind === 'refused' ? status.problem : status ? null : live?.problems[0] ?? null;
  const needsBits = status?.kind === 'refused' ? status.needsBits : !status && !!live?.needs_bits;
  const left = shown.balance !== null ? shown.balance - shown.total : null;

  const close = () => {
    if (status?.kind === 'held') {
      // Held messages left the box when Send was pressed; put it back.
      onSetText(status.check.text);
      dismiss();
    } else if (status) {
      dismiss();
    } else {
      onSetText(shown.plain_text);
    }
  };

  return (
    <div
      role="status"
      className="glass-flyout rounded-xl absolute bottom-full left-0 right-0 z-[60] mb-2 flex items-center gap-2 !px-2.5 !py-1.5 text-[12.5px]"
    >
      {shown.icon_url && <img src={shown.icon_url} alt="" className="h-[18px] w-[18px] flex-shrink-0 object-contain" />}
      <span className="min-w-0 flex-1 truncate">
        {status?.kind === 'sending' ? (
          <span className="text-textSecondary">Cheering {bitsWord(shown.total)}...</span>
        ) : problem ? (
          <Tooltip content={problem}>
            <span className="text-warning">
              {problem}
            </span>
          </Tooltip>
        ) : (
          <>
            <span className="font-semibold text-textPrimary">Cheering {bitsWord(shown.total)}</span>
            {left !== null && left >= 0 && <span className="text-textSecondary"> · {formatBits(left)} left</span>}
          </>
        )}
      </span>
      {needsBits && (
        <button
          type="button"
          onClick={() => void openBitsWindow().catch((err) => Logger.warn('[Cheer] Get Bits failed:', err))}
          className={`${GLAZE_PILL} h-5 flex-shrink-0 px-2.5 text-[12px]`}
        >
          Get bits
        </button>
      )}
      {status?.kind === 'refused' && status.canContinue && (
        <button
          type="button"
          onClick={() => {
            onSetText('');
            void send(status.check, true);
          }}
          className={`${ACCENT_BUTTON} h-5 flex-shrink-0 px-2.5 !text-[12px]`}
          style={ACCENT_FILL}
        >
          Cheer without the message
        </button>
      )}
      {status?.kind === 'held' && (
        <button
          type="button"
          onClick={() => void send(status.check)}
          className={`${ACCENT_BUTTON} h-5 flex-shrink-0 px-2.5 !text-[12px]`}
          style={ACCENT_FILL}
        >
          Cheer {formatBits(status.check.total)}
        </button>
      )}
      {status?.kind !== 'sending' && (
        <Tooltip content={status ? undefined : 'Remove the cheer'}>
          <button
            type="button"
            onClick={close}
            aria-label={status ? 'Close' : 'Remove the cheer'}
            className={`${GLAZE_PILL} grid h-5 w-5 flex-shrink-0 place-items-center`}
          >
            <X size={14} />
          </button>
        </Tooltip>
      )}
    </div>
  );
}

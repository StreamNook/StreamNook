// The mention you opened, pinned over its chat, when the message is older than
// anything the chat could load. It says so, shows what was said, and goes away
// on Dismiss or when you leave that chat. In the top-of-chat stack (SN-284) it
// is the `mention_context` card: one line, or the whole message when opened.
import React from 'react';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { CaretDown, X } from 'phosphor-react';
import { agoLabel } from '../../utils/lastLive';
import { StackCard } from '../../components/chat/ChatOverlayStack';
import { useCardPresence, type StackCardControl, type StackKind } from '../../components/chat/chatTopStack';
import { dismissContextCard, KIND_LABEL, type MentionEntry } from './mentionInbox';

export const MentionContextCard: React.FC<{
  entry: MentionEntry;
  /** The card's slot in the stack; absent, it draws as the standalone card. */
  stack?: StackCardControl;
  measure?: (kind: StackKind, form: 'strip' | 'open' | 'line', height: number) => void;
}> = ({ entry, stack, measure }) => {
  useCardPresence(stack, stack ? entry.id : null);
  // The message the viewer just opened is what they came for: the card opens
  // once as it appears (a fold after that is theirs).
  const openedFor = React.useRef<string | null>(null);
  const placed = stack?.placed ?? false;
  const isOpen = stack?.open ?? false;
  const toggle = stack?.onToggle;
  React.useEffect(() => {
    if (!placed || openedFor.current === entry.id) return;
    openedFor.current = entry.id;
    if (!isOpen) toggle?.();
  }, [placed, isOpen, toggle, entry.id]);
  // Rust hands over the readable color the chat row shows.
  const color = entry.sender_color ?? undefined;
  const when = agoLabel(new Date(entry.at_ms).toISOString()) ?? '';
  const rootRef = React.useRef<HTMLDivElement>(null);
  const markInside = useOutsidePress(true, [rootRef], dismissContextCard);
  if (stack && (!stack.placed || stack.form === 'folded')) return null;

  const strip = stack && stack.form !== 'open';
  const card = strip ? (
    <div
      ref={rootRef}
      onPointerDownCapture={markInside}
      className="sn-popover sn-popover--static pointer-events-auto flex h-9 items-center gap-2 rounded-xl pl-3 pr-1"
    >
      <button type="button" onClick={stack.onToggle} aria-expanded={false} className="flex min-w-0 flex-1 items-baseline gap-1.5 text-left">
        <span className="min-w-0 truncate text-[13px] font-semibold text-textPrimary" style={color ? { color } : undefined}>
          {entry.sender_name || entry.sender_login}
        </span>
        <span className="shrink-0 whitespace-nowrap text-[12px] text-textMuted">{KIND_LABEL[entry.kind]}</span>
        <span className="flex-1" />
        <span className="shrink-0 text-[11px] text-textMuted">Older than chat history</span>
        <CaretDown size={12} weight="bold" className="shrink-0 self-center text-textSecondary" />
      </button>
      <button
        type="button"
        onClick={dismissContextCard}
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-textMuted active:text-textPrimary"
        aria-label="Dismiss"
      >
        <X size={15} weight="bold" />
      </button>
    </div>
  ) : (
    <div
      ref={rootRef}
      onPointerDownCapture={markInside}
      className="sn-popover pointer-events-auto rounded-xl px-3 pt-2 pb-2.5 flex items-start gap-2"
    >
      <div className="flex-1 min-w-0">
        <div className="text-[11.5px] font-semibold uppercase tracking-wide text-textMuted leading-[16px]">
          Older than chat history
        </div>
        <div className="flex items-baseline gap-1.5 min-w-0 mt-1 leading-[18px]">
          <span
            className="text-[14px] font-semibold truncate min-w-0 text-textPrimary"
            style={color ? { color } : undefined}
          >
            {entry.sender_name || entry.sender_login}
          </span>
          <span className="text-[12.5px] text-textMuted whitespace-nowrap shrink-0">
            {KIND_LABEL[entry.kind]}
          </span>
          <span className="flex-1" />
          <span className="text-[12px] text-textMuted whitespace-nowrap shrink-0 tabular-nums">{when}</span>
        </div>
        {entry.removed ? (
          <div className="text-[13.5px] leading-[19px] italic text-textMuted mt-0.5">
            Message deleted by a moderator
          </div>
        ) : (
          <div className="text-[13.5px] leading-[19px] text-textSecondary mt-0.5 line-clamp-3 break-words [overflow-wrap:anywhere]">
            {entry.text}
          </div>
        )}
      </div>
      <button
        onClick={dismissContextCard}
        className="shrink-0 -mr-1.5 -mt-2 w-8 h-8 flex items-center justify-center rounded-full text-textMuted active:text-textPrimary"
        aria-label="Dismiss"
      >
        <X size={15} weight="bold" />
      </button>
    </div>
  );
  if (!stack) return card;
  return (
    <StackCard kind="mention_context" control={stack} measure={measure}>
      {card}
    </StackCard>
  );
};

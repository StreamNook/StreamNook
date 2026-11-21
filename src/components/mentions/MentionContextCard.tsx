// Shown at the top of a chat when a mention it was asked to jump to is older
// than the chat's history: the message itself, so opening it still shows what
// was said, until the viewer dismisses it. In the top-of-chat stack (SN-284) it
// is the `mention_context` card: one line, or the whole message when opened.

import { useEffect, useRef } from 'react';
import { ChevronDown } from 'lucide-react';
import type { MentionEntry, MentionKind } from '../../types';
import { agoLabel } from '../../utils/lastLive';
import { BANNER_STYLE, GLAZE_BANNER, GLAZE_PILL } from '../ui/glazeButtons';
import { Tooltip } from '../ui/Tooltip';
import { StackCard } from '../chat/ChatOverlayStack';
import { useCardPresence, type StackCardControl, type StackKind } from '../chat/chatTopStack';

const KIND_WORDS: Record<MentionKind, string> = {
  mention: 'mentioned you',
  reply: 'replied to you',
  word: 'matched a highlight',
  gift: 'gifted you a sub',
};

export function MentionContextCard({
  entry,
  onDismiss,
  stack,
  measure,
}: {
  entry: MentionEntry;
  onDismiss: () => void;
  /** The card's slot in the stack; absent, it draws as the standalone card. */
  stack?: StackCardControl;
  measure?: (kind: StackKind, form: 'strip' | 'open' | 'line', height: number) => void;
}) {
  useCardPresence(stack, stack ? entry.id : null);
  // The message the viewer just opened is what they came for: the card opens
  // once as it appears (a fold after that is theirs).
  const openedFor = useRef<string | null>(null);
  const placed = stack?.placed ?? false;
  const isOpen = stack?.open ?? false;
  const toggle = stack?.onToggle;
  useEffect(() => {
    if (!placed || openedFor.current === entry.id) return;
    openedFor.current = entry.id;
    if (!isOpen) toggle?.();
  }, [placed, isOpen, toggle, entry.id]);
  // Rust's text color for the sender, drawn as is.
  const color = entry.sender_color || undefined;
  const ago = agoLabel(new Date(entry.at_ms).toISOString());
  const dismiss = (
    <button type="button" onClick={onDismiss} className={`${GLAZE_PILL} h-6 shrink-0 px-2.5 text-[12px]`}>
      Dismiss
    </button>
  );

  if (stack) {
    if (!stack.placed || stack.form === 'folded') return null;
    // One line: what it is, who, and a press to read it.
    if (stack.form !== 'open') {
      return (
        <StackCard kind="mention_context" control={stack} measure={measure}>
          <div
            className={`${GLAZE_BANNER} flex h-9 w-full items-center gap-2 overflow-hidden pl-3 pr-1.5`}
            style={BANNER_STYLE}
            data-mention-context={entry.id}
          >
            <button
              type="button"
              onClick={stack.onToggle}
              aria-expanded={false}
              className="flex min-w-0 flex-1 items-center gap-1.5 text-left text-[12.5px]"
            >
              <span className="min-w-0 truncate font-semibold" style={{ color }}>{entry.sender_name}</span>
              <span className="shrink-0 text-textMuted">{KIND_WORDS[entry.kind]}</span>
              <span className="ml-auto shrink-0 text-[11px] text-textMuted">Older than chat history</span>
              <ChevronDown className="h-4 w-4 shrink-0 text-textSecondary" />
            </button>
            {dismiss}
          </div>
        </StackCard>
      );
    }
  }

  const card = (
    <div
      className={`${GLAZE_BANNER} w-full overflow-hidden p-3.5 ${stack ? '' : 'pointer-events-auto mt-2 max-w-sm'}`}
      style={BANNER_STYLE}
      data-mention-context={entry.id}
    >
      <div className="flex h-5 items-center justify-between gap-3">
        <span className="text-[11px] font-medium uppercase tracking-wide text-textSecondary">
          Older than chat history
        </span>
        <div className="flex items-center gap-1.5">
          {stack && (
            <button
              type="button"
              onClick={stack.onToggle}
              aria-label="Fold to one line"
              className="flex h-6 w-6 items-center justify-center rounded-md text-textSecondary hover:bg-white/10"
            >
              <ChevronDown className="h-4 w-4 rotate-180" />
            </button>
          )}
          {dismiss}
        </div>
      </div>
      <div className="mt-2 flex items-center gap-1.5 text-[12.5px]">
        <span className="min-w-0 truncate font-semibold" style={{ color }}>{entry.sender_name}</span>
        <span className="shrink-0 text-textMuted">{KIND_WORDS[entry.kind]}</span>
        {ago && <span className="ml-auto shrink-0 text-[11px] text-textMuted">{ago}</span>}
      </div>
      {entry.removed ? (
        <p className="mt-1 text-[13px] italic leading-snug text-textMuted">Message deleted by a moderator</p>
      ) : (
        <Tooltip content={entry.text} delay={500}>
          <p className="mt-1 line-clamp-4 break-words text-[13px] leading-snug text-textPrimary">
            {entry.text}
          </p>
        </Tooltip>
      )}
    </div>
  );
  if (!stack) return card;
  return (
    <StackCard kind="mention_context" control={stack} measure={measure}>
      {card}
    </StackCard>
  );
}

export default MentionContextCard;

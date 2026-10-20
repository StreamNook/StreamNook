// The mention you opened, pinned over its chat, when the message is older than
// anything the chat could load. It says so, shows what was said, and goes away
// on Dismiss or when you leave that chat.
import React from 'react';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { X } from 'phosphor-react';
import { agoLabel } from '../../utils/lastLive';
import { dismissContextCard, KIND_LABEL, type MentionEntry } from './mentionInbox';

export const MentionContextCard: React.FC<{ entry: MentionEntry }> = ({ entry }) => {
  // Rust hands over the readable color the chat row shows.
  const color = entry.sender_color ?? undefined;
  const when = agoLabel(new Date(entry.at_ms).toISOString()) ?? '';
  const rootRef = React.useRef<HTMLDivElement>(null);
  const markInside = useOutsidePress(true, [rootRef], dismissContextCard);
  return (
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
};

// Shown at the top of a chat when a mention it was asked to jump to is older
// than the chat's history: the message itself, so opening it still shows what
// was said, until the viewer dismisses it.

import type { MentionEntry, MentionKind } from '../../types';
import { agoLabel } from '../../utils/lastLive';

const KIND_WORDS: Record<MentionKind, string> = {
  mention: 'mentioned you',
  reply: 'replied to you',
  word: 'matched a highlight',
};

export function MentionContextCard({ entry, onDismiss }: { entry: MentionEntry; onDismiss: () => void }) {
  // Rust stores the readable color the chat row shows; drawn as is.
  const color = entry.sender_color || undefined;
  const ago = agoLabel(new Date(entry.at_ms).toISOString());
  return (
    <div className="sn-popover pointer-events-auto mt-2 w-full max-w-sm overflow-hidden p-3.5" data-mention-context={entry.id}>
      <div className="flex h-5 items-center justify-between gap-3">
        <span className="text-[11px] font-medium uppercase tracking-wide text-textSecondary">
          Older than chat history
        </span>
        <button
          type="button"
          onClick={onDismiss}
          className="-mr-1.5 h-6 rounded-md px-1.5 text-[12px] font-medium text-textMuted transition-colors hover:bg-white/[0.06] hover:text-textPrimary"
        >
          Dismiss
        </button>
      </div>
      <div className="mt-2 flex items-center gap-1.5 text-[12.5px]">
        <span className="min-w-0 truncate font-semibold" style={{ color }}>{entry.sender_name}</span>
        <span className="shrink-0 text-textMuted">{KIND_WORDS[entry.kind]}</span>
        {ago && <span className="ml-auto shrink-0 text-[11px] text-textMuted">{ago}</span>}
      </div>
      {entry.removed ? (
        <p className="mt-1 text-[13px] italic leading-snug text-textMuted">Message deleted by a moderator</p>
      ) : (
        <p className="mt-1 line-clamp-4 break-words text-[13px] leading-snug text-textPrimary" title={entry.text}>
          {entry.text}
        </p>
      )}
    </div>
  );
}

export default MentionContextCard;

import { Ban, Timer, Trash2 } from 'lucide-react';
import type { ModerationContext } from '../../stores/chatConnectionStore';
import { formatDuration } from '../../utils/timeoutRamp';
import type { DeletedMessageStyle } from './deletedMessage';

const REASON_ICON = { deleted: Trash2, timeout: Timer, ban: Ban } as const;

/** What happened to a removed message, set after its text. Strikethrough keeps
 *  its terse bracket; Italic gets an upright chip in its own tone (neutral for
 *  a deletion, amber for a timeout, red for a ban) so the reason reads apart
 *  from the slanted, muted message. Other styles show no reason. */
export const ModerationTag = ({
  context,
  style,
}: {
  context: ModerationContext;
  style: DeletedMessageStyle;
}) => {
  if (style === 'strikethrough') {
    return (
      <span className="ml-1.5 text-xs text-error/70 font-medium">
        {context.type === 'timeout'
          ? context.duration
            ? `[timed out for ${formatDuration(context.duration)}]`
            : '[timed out]'
          : context.type === 'ban'
            ? '[banned]'
            : '[deleted by mod]'}
      </span>
    );
  }
  if (style !== 'italic') return null;
  const Icon = REASON_ICON[context.type];
  const label =
    context.type === 'timeout' ? 'Timed out' : context.type === 'ban' ? 'Banned' : 'Deleted';
  // A real space before the pill, not a margin: it collapses when the pill
  // wraps to a new line, so a wrapped pill starts flush with the text.
  return (
    <>
      {' '}
      <span className={`sn-mod-tag sn-mod-tag--${context.type}`}>
        <Icon aria-hidden strokeWidth={2.25} />
        {label}
        {context.type === 'timeout' && context.duration ? (
          <span className="sn-mod-tag__detail">{formatDuration(context.duration)}</span>
        ) : null}
      </span>
    </>
  );
};

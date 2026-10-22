import { Ban, Timer, Trash2 } from 'lucide-react';
import type { ModerationContext } from '../../stores/chatConnectionStore';
import { formatDuration } from '../../utils/timeoutRamp';
import { Tooltip } from '../ui/Tooltip';
import type { DeletedMessageStyle } from './deletedMessage';
import { modTagSpoken, modTagTooltipLines, shortModName, strikeTagText } from './moderationTagText';

const REASON_ICON = { deleted: Trash2, timeout: Timer, ban: Ban } as const;

/** What happened to a removed message, set after its text. Strikethrough keeps
 *  its terse bracket; Italic gets an upright chip in its own tone (neutral for
 *  a deletion, amber for a timeout, red for a ban) so the reason reads apart
 *  from the slanted, muted message. Other styles show no reason. A moderator
 *  sees who acted after the length; the reason waits in the tooltip so the
 *  row never grows a line. */
export const ModerationTag = ({
  context,
  style,
}: {
  context: ModerationContext;
  style: DeletedMessageStyle;
}) => {
  // Who on one line, why on the next.
  const lines = modTagTooltipLines(context);
  const tip = lines.length ? (
    <span className="block space-y-0.5 text-left">
      {lines.map((l) => (
        <span key={l} className="block">
          {l}
        </span>
      ))}
    </span>
  ) : null;
  const spoken = tip ? modTagSpoken(context) : undefined;
  if (style === 'strikethrough') {
    return (
      <Tooltip content={tip ?? ''} disabled={!tip} requirePointerMove side="bottom">
        <span className="ml-1.5 whitespace-nowrap text-xs text-error/70 font-medium" aria-label={spoken}>
          {strikeTagText(context)}
        </span>
      </Tooltip>
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
      <Tooltip content={tip ?? ''} disabled={!tip} requirePointerMove side="bottom">
        <span className={`sn-mod-tag sn-mod-tag--${context.type}`} aria-label={spoken}>
          <Icon aria-hidden strokeWidth={2.25} />
          {label}
          {context.type === 'timeout' && context.duration ? (
            <span className="sn-mod-tag__detail">{formatDuration(context.duration)}</span>
          ) : null}
          {context.by ? <span className="sn-mod-tag__detail">{shortModName(context.by)}</span> : null}
        </span>
      </Tooltip>
    </>
  );
};

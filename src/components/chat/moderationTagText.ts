import type { ModerationContext } from '../../stores/chatConnectionStore';
import { formatDuration } from '../../utils/timeoutRamp';

/** Longest moderator name the tag shows whole; longer ones end in an ellipsis
 *  and the full name moves to the tooltip, so the pill never wraps the row. */
const MAX_NAME = 14;

export function shortModName(name: string): string {
  return name.length > MAX_NAME ? `${name.slice(0, MAX_NAME - 1)}…` : name;
}

/** The Strikethrough style's bracket: "[timed out for 10m by kay_lee]". */
export function strikeTagText(context: ModerationContext): string {
  const by = context.by ? ` by ${shortModName(context.by)}` : '';
  if (context.type === 'timeout') {
    return context.duration
      ? `[timed out for ${formatDuration(context.duration)}${by}]`
      : `[timed out${by}]`;
  }
  if (context.type === 'ban') return `[banned${by}]`;
  return context.by ? `[deleted${by}]` : '[deleted by mod]';
}

/** The tag's hover text: the reason, and the full name when it was cut.
 *  Null when the tag already says everything. */
export function modTagTooltip(context: ModerationContext): string | null {
  const lines = modTagTooltipLines(context);
  return lines.length ? lines.join(' · ') : null;
}

/** The tooltip one fact per line: who, then why. */
export function modTagTooltipLines(context: ModerationContext): string[] {
  const lines: string[] = [];
  if (context.by && shortModName(context.by) !== context.by) lines.push(`By ${context.by}`);
  if (context.reason) lines.push(`Reason: ${context.reason}`);
  return lines;
}

/** The whole tag in a sentence, for screen readers. */
export function modTagSpoken(context: ModerationContext): string {
  const what =
    context.type === 'timeout'
      ? context.duration
        ? `Timed out for ${formatDuration(context.duration)}`
        : 'Timed out'
      : context.type === 'ban'
        ? 'Banned'
        : 'Deleted';
  const by = context.by ? ` by ${context.by}` : '';
  const why = context.reason ? `. Reason: ${context.reason}` : '';
  return `${what}${by}${why}`;
}

/** Whether two rows' moderation draws the same, for the chat row memos. */
export function sameModerationContext(
  a: ModerationContext | null | undefined,
  b: ModerationContext | null | undefined,
): boolean {
  return (
    a?.type === b?.type &&
    a?.duration === b?.duration &&
    a?.by === b?.by &&
    a?.reason === b?.reason
  );
}

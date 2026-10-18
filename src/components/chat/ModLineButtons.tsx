import type { ModRequest } from '../../services/moderation';
import { lineTargetAttrs } from './modLine';

/**
 * The line buttons at the start of a chat line, for moderators: Delete, the
 * timeouts, Ban, one click each (see `chat/modLine.ts`).
 *
 * One empty element on purpose, because every row a moderator can act on
 * draws one: the buttons are a picture the list draws once (`lineArt.ts`),
 * and the list's listeners light the button under the pointer and send the
 * click through `moderate_chatter`. No text, so a selection or Copy never
 * picks up "10m"; not focusable, since keyboard moderation has its own keys.
 */
export function ModLineButtons({
  target,
  messageId,
}: {
  target: Omit<ModRequest, 'action'>;
  messageId: string;
}) {
  return <span className="sn-mod-line" aria-hidden="true" {...lineTargetAttrs(target, messageId)} />;
}

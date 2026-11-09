// Which chat's composer has a riser up (the suggestions strip, the emote or wallet panel). A new
// mention's card waits only on its own chat's composer: a panel left open on a chat out of sight
// never holds the card back in the chat on screen, and the two composers never overwrite one flag.
import type { InboxSurface } from './mentionInbox';

export type ComposerBusy = Readonly<Record<InboxSurface, boolean>>;

// `home` has no composer: it stays false, and the browsing card never waits.
export const NO_COMPOSER_BUSY: ComposerBusy = { watch: false, chats: false, home: false };

/** The flags with `surface` set to `busy`, or null when that changes nothing. */
export function withComposerBusy(current: ComposerBusy, surface: InboxSurface, busy: boolean): ComposerBusy | null {
  return current[surface] === busy ? null : { ...current, [surface]: busy };
}

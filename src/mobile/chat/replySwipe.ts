// Swipe left on a message to reply (right with Left-handed swipes, the same rules read the other
// way round): the rules both phones follow, run on every finger move (so
// they stay out of Rust, as the mod slider's per-move lookups do). The iPhone's twin is SNKit
// ReplySwipe; both are tested against replySwipeCases.json.
import { pickFittingText } from './placeholderFit';

/** A leftward slide starts after this many px (the mod slider's own distance). */
export const REPLY_ENGAGE = 10;
/** The pull that arms the reply... */
export const REPLY_ARM = 64;
/** ...and the one an armed pull must fall under to let go of it again. */
export const REPLY_DISARM = 52;
/** How far the row moves at most. */
export const REPLY_REACH = 88;
/** Past the arm point the row follows a quarter of the finger. */
export const REPLY_RESISTANCE = 0.25;
/** The arrow starts to show once the row has moved this far. */
export const REPLY_ICON_FROM = 16;
/** How long a swiped row takes to slide back (none when Motion drops movement). */
export const SNAP_BACK_MS = 220;
/** The paused hint shows on this many separate pauses at most. */
export const REPLY_HINT_SHOWS = 5;
export const REPLY_HINT = 'Swipe left on a message to reply';
export const REPLY_HINT_SHORT = 'Swipe left to reply';
/** The same with Left-handed swipes, where right replies. */
export const REPLY_HINT_FLIPPED = 'Swipe right on a message to reply';
export const REPLY_HINT_SHORT_FLIPPED = 'Swipe right to reply';
/** The platforms whose sends thread a reply (YouTube and TikTok send it as a plain line). */
export const REPLY_PROVIDERS: readonly string[] = ['twitch', 'kick'];
/** Ids of your own rows the platform has not answered yet: nothing can reply to them. */
export const PROVISIONAL_PREFIXES: readonly string[] = ['local-', 'kick-local-'];

/** One of your own lines still under its local id: nothing can reply to it or delete it yet. */
export const notSentYet = (messageId: string): boolean => PROVISIONAL_PREFIXES.some((p) => messageId.startsWith(p));

/** What a reply starts with. */
export interface ReplyTarget {
  messageId: string;
  username: string;
}

/** How far the row shows moved for a leftward pull of `pull` px. */
export function replyShown(pull: number): number {
  if (!(pull > 0)) return 0;
  if (pull <= REPLY_ARM) return pull;
  return Math.min(REPLY_REACH, REPLY_ARM + (pull - REPLY_ARM) * REPLY_RESISTANCE);
}

/** Whether a pull arms the reply, given whether it was armed. */
export function replyArmed(pull: number, was: boolean): boolean {
  return was ? pull >= REPLY_DISARM : pull >= REPLY_ARM;
}

/** How much of the arrow shows, 0 to 1. */
export function replyProgress(pull: number): number {
  const p = (replyShown(pull) - REPLY_ICON_FROM) / (REPLY_ARM - REPLY_ICON_FROM);
  return Math.min(1, Math.max(0, p));
}

export interface ReplyDrag {
  armed: boolean;
}
export const REPLY_DRAG_START: ReplyDrag = { armed: false };

/** One finger move of an engaged reply swipe. */
export function replyStep(
  drag: ReplyDrag,
  pull: number,
): { drag: ReplyDrag; shown: number; progress: number; armedChanged: boolean } {
  const armed = replyArmed(pull, drag.armed);
  return { drag: { armed }, shown: replyShown(pull), progress: replyProgress(pull), armedChanged: armed !== drag.armed };
}

/** The slide back's length under Interface > Motion (`html[data-motion]`). */
export function snapBackMs(motion: string | null | undefined): number {
  return motion === 'reduced' || motion === 'off' ? 0 : SNAP_BACK_MS;
}

/** A row as the reply rule reads it. */
export interface ReplyRow {
  id: string;
  username: string;
  /** The room's own notices (sender "System"). */
  system: boolean;
  /** An event row (sub, raid, announcement) Twitch tags with `system-msg`. */
  systemMsg: boolean;
  /** A message AutoMod is holding. */
  held: boolean;
  provider: string;
}

/** A parsed chat line, as far as `replyRowOf` reads it. */
export interface ParsedLine {
  username?: string | null;
  tags: { get(key: string): string | undefined };
}

/**
 * The reply rule's view of a live (never held) line. `provider` is the message's own platform when
 * it carries one; a bare IRC line takes the room's.
 */
export function replyRowOf(id: string, parsed: ParsedLine, provider: string | undefined, roomProvider: string): ReplyRow {
  const username = parsed.username ?? '';
  return {
    id,
    username,
    system: username === 'System',
    systemMsg: !!parsed.tags.get('system-msg'),
    held: false,
    provider: provider ?? roomProvider,
  };
}

/** The room's composer as the reply rule reads it. */
export interface ComposerState {
  /** The landscape overlay or VOD replay: no composer. */
  readOnly: boolean;
  /** The mode that certainly stops a send (Rust `room_modes`), or null. */
  blocked: string | null;
  /** A moderator's warning not acknowledged yet. */
  warning: boolean;
  kickSignedOut: boolean;
}

/** Whether a reply can start in this room. Emote-only counts: the emote menu types there. */
export function composerTakesText(c: ComposerState): boolean {
  return !c.readOnly && (c.blocked == null || c.blocked === 'emote_only') && !c.warning && !c.kickSignedOut;
}

/** The one reply rule: the swipe, the hold menu's Reply and the hint all follow it. */
export function canReplyTo(row: ReplyRow, c: ComposerState): boolean {
  if (row.held || row.system || row.systemMsg || !row.username) return false;
  if (PROVISIONAL_PREFIXES.some((p) => row.id.startsWith(p))) return false;
  if (!REPLY_PROVIDERS.includes(row.provider)) return false;
  return composerTakesText(c);
}

/** Whether a finished send clears the reply draft: only the reply it went out with. */
export function replyAfterSend(draftId: string | null, sentWith: string | null): boolean {
  return sentWith != null && draftId === sentWith;
}

/** The paused hint's words for a column `width` px wide (`measure` gives a text's width); `flipped`
 *  is Left-handed swipes. */
export function replyHintWords(width: number, measure: (text: string) => number | null, flipped = false): string {
  return pickFittingText(flipped ? [REPLY_HINT_FLIPPED, REPLY_HINT_SHORT_FLIPPED] : [REPLY_HINT, REPLY_HINT_SHORT], width, measure);
}

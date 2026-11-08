// The swipe slider's data and step rules, shared by the Android swipe and its
// tests. Rust lays the slider out (services/touch_mod_slider.rs); these are
// the per-move lookups that must not cross into Rust on every finger move,
// written to match Rust's `step_at` and `with_hysteresis` and tested on the
// same pulls. `slideStart` also decides the reply swipe's start (swipe left, or
// right with Left-handed swipes).
import type { ModAction } from '../../services/moderation';
import { REPLY_ENGAGE } from './replySwipe';

export interface TouchStep {
  action: 'delete' | 'timeout' | 'ban';
  seconds: number | null;
  label: string;
  chip: string;
  tone: number | null;
}

export interface TouchSlider {
  enabled: boolean;
  style: 'preview' | 'rail' | 'capsule';
  steps: TouchStep[];
  dead: number;
  spacing: number;
  starts: number[];
  end: number;
  ban_dwell_ms: number;
  ban_still: number;
  hysteresis: number;
  leash: number;
  engage: number;
  cancel_words: string;
  hold_ban_words: string;
  nudge_words: string;
  /** Where the thumb landed, from the row's left edge: the ruler starts here. */
  start: number;
  /** The pull runs left (Left-handed swipes): a pull of `p` sits at `start - p` on the row. */
  flipped: boolean;
  quick_seconds: number;
}

export interface SwipeTarget {
  userId: string;
  name: string;
  /** The sender's badges as Twitch tags them ("moderator/1"). */
  badges: string[];
}

/** What the viewer may do to one message, by Twitch's rules: the core's
 *  `moderation::message_reach`, mirrored because the hold menu decides while
 *  the finger rests. Helix refuses deleting the broadcaster's messages or
 *  another moderator's, and banning or timing out the broadcaster, yourself,
 *  or (unless the broadcaster does it) a moderator. */
export type MessageReach = 'all' | 'delete_only' | 'no_delete' | 'nothing';

export function messageReach(
  viewer: 'broadcaster' | 'moderator' | null,
  viewerId: string,
  targetId: string,
  badges: string[],
): MessageReach {
  if (!viewer || !targetId) return 'nothing';
  const has = (name: string) => badges.some((b) => b === name || b.startsWith(`${name}/`));
  const own = !!viewerId && targetId === viewerId;
  if (has('broadcaster') || (own && viewer === 'broadcaster') || has('staff') || has('admin') || has('global_mod')) {
    return 'nothing';
  }
  if (own) return 'delete_only';
  if (has('moderator') || has('lead_moderator')) return viewer === 'broadcaster' ? 'no_delete' : 'nothing';
  return 'all';
}

export const reachCanDelete = (r: MessageReach) => r === 'all' || r === 'delete_only';
export const reachCanTimeout = (r: MessageReach) => r === 'all' || r === 'no_delete';

/** The step under a pull of `slide` px, null in the cancel zone (Rust `step_at`). */
export function stepAt(s: TouchSlider, slide: number): number | null {
  if (slide < s.dead || s.steps.length === 0 || s.spacing <= 0) return null;
  const unit = Math.floor((slide - s.dead) / s.spacing);
  const last = s.steps.length - 1;
  if (s.steps[last].action !== 'ban') return Math.min(unit, last);
  if (unit >= last + 1) return last;
  if (last === 0) return null;
  return Math.min(unit, last - 1);
}

/** The step to show, keeping `cur` until the pull is `hysteresis` past its edge (Rust `with_hysteresis`). */
export function stepKeeping(s: TouchSlider, slide: number, cur: number | null): number | null {
  const next = stepAt(s, slide);
  if (cur == null || next == null || next === cur) return next;
  if (next > cur) return slide >= s.starts[next] + s.hysteresis ? next : cur;
  return slide <= s.starts[cur] - s.hysteresis ? next : cur;
}

/** A step's color: Delete a calm slate, timeouts amber toward red, Ban red. */
export function stepTint(step: TouchStep): string {
  if (step.action === 'ban') return 'var(--color-error)';
  if (step.tone == null) return 'color-mix(in srgb, var(--color-text-secondary) 75%, #334155)';
  return `color-mix(in srgb, var(--color-error) ${Math.round(step.tone * 100)}%, var(--color-warning))`;
}

export function stepAction(step: TouchStep, messageId: string): ModAction {
  if (step.action === 'delete') return { kind: 'delete', messageId };
  if (step.action === 'ban') return { kind: 'ban' };
  return { kind: 'timeout', seconds: step.seconds ?? 600 };
}

/** The grab zone: this far in from where the screen's left edge stops being the system's (the
 *  back-gesture strip), a slide starts on a looser rule. About a thumb's width, past the 44 pt
 *  minimum touch target and about twice the back strip, so a thumb reaching in from the edge
 *  lands in it. The iPhone keeps the same numbers (ChatListView). */
export const GRAB_ZONE = 56;
/** In the grab zone a slide starts once the finger has moved this far right... */
export const GRAB_ENGAGE = 6;
/** ...and at least this much sideways for each unit up or down (elsewhere 1: mostly
 *  sideways), so a thumb's arc from the edge still counts while a plain scroll does not. */
export const GRAB_RATIO = 0.5;

/** Whether a press at `clientX` (screen points) is in the grab zone, given where the system's
 *  left edge strip (or the list's own left edge) ends. */
export function inGrabZone(clientX: number, systemEdge: number): boolean {
  return clientX >= systemEdge && clientX < systemEdge + GRAB_ZONE;
}

/** Whether a press is in the reply swipe's grab zone along the right edge, given where the usable
 *  right edge is (the list's own right edge, or where the system's right strip starts). */
export function inRightGrabZone(clientX: number, rightEdge: number): boolean {
  return clientX <= rightEdge && clientX > rightEdge - GRAB_ZONE;
}

/** What the press can start, for `slideStart`: a mod target, a reply, and whether it landed in the
 *  reply's grab zone (the right edge's; the left edge's with Left-handed swipes). */
export interface SlideOptions {
  mod: boolean;
  reply: boolean;
  replyGrab: boolean;
}

/** What a finger `dx`, `dy` from its press does before a slide has started: starts the mod slide,
 *  starts a reply (leftward), is the list's scroll, is a sideways move nothing here can take, or is
 *  not decided yet. `dx` is measured toward the mod side, so Left-handed swipes pass it negated
 *  and every rule mirrors. `engage` is the core's distance (10 by default); a press in the mod's
 *  grab zone starts a mod slide sooner and on a steeper angle. Without `options` it is the mod
 *  slide's rule alone (leftward waits). The reply mirrors the mod rule; in its grab zone it starts
 *  sooner but never on a steeper angle, so a thumb's read-back scroll is never a reply. */
export function slideStart(
  dx: number,
  dy: number,
  engage: number,
  grab: boolean,
  options?: SlideOptions,
): 'slide' | 'reply' | 'scroll' | 'none' | 'wait' {
  if (options && dx < 0) {
    const reach = options.replyGrab ? Math.min(GRAB_ENGAGE, REPLY_ENGAGE) : REPLY_ENGAGE;
    if (Math.abs(dy) > reach && -dx <= Math.abs(dy)) return 'scroll';
    if (-dx >= reach && -dx >= Math.abs(dy)) return options.reply ? 'reply' : 'none';
    return 'wait';
  }
  const reach = grab ? Math.min(GRAB_ENGAGE, engage) : engage;
  const ratio = grab ? GRAB_RATIO : 1;
  if (Math.abs(dy) > reach && Math.abs(dx) <= ratio * Math.abs(dy)) return 'scroll';
  if (dx >= reach && dx >= ratio * Math.abs(dy)) return !options || options.mod ? 'slide' : 'none';
  return 'wait';
}

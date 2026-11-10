// Vertical view on the phones (SN-40): which layout a stream gets, which
// picture a start asks for, when a turn restarts it, and how the tall layout's
// chat moves between its three states. Not in Rust: these are synchronous
// layout and gesture answers read on every layout pass and touch (as
// dialZone.ts and chatSwipe.ts). The iPhone's twin is SNKit `VerticalView`;
// both are tested against src/mobile/verticalViewCases.json.
import { resumeCountText } from '../chat/resumeLabel';
import { chatSwipeIntent, FLING_MIN, FLING_SPEED, SWIPE_MIN } from './chatSwipe';
import { DIAL_EDGE_GUARD } from './dialZone';

/** The picture a start asks for or was served. */
export type StreamView = 'landscape' | 'vertical';

/** How the phone is held: only an upright phone can want the tall picture.
 *  Mini, Picture in Picture and the keyboard are not postures and never change
 *  the wanted view. */
export type Posture = 'upright' | 'landscape' | 'large' | 'tabletop' | 'split';

/** The tall layout's chat: the lower third over the picture, cleared away
 *  (pure video), or grown up the screen as the full chat. */
export type VerticalChatState = 'lower' | 'cleared' | 'expanded';

/** What moves the chat between its states. */
export type VerticalChatInput =
  | 'swipeRight'
  | 'swipeLeft'
  | 'dragUp'
  | 'dragDown'
  | 'tapChatArea'
  | 'tapChatButton'
  | 'tapPicture'
  | 'back'
  | 'rotate'
  | 'mini'
  | 'pip'
  | 'keyboardUp';

/** Where a touch in the tall layout started: the picture, the lower third's
 *  rows (inert: the chat area owns every touch there), the message box (the
 *  composer's own), or the grown chat's handle. */
export type TallRegion = 'picture' | 'lowerThird' | 'composer' | 'handle';

/** What a tall-layout drag became at the slop, latched for the rest of the
 *  drag: sideways clears or restores chat, up grows it, down from the grown
 *  chat folds it back (never on into mini), down elsewhere shrinks the layer. */
export type TallDrag = 'swipe' | 'expand' | 'collapse' | 'mini' | 'none';

/** The raw facts the tall layout is decided from. A missing field is what an
 *  upright phone playing a live stream here would say. */
export interface LayoutInputs {
  w: number;
  h: number;
  largeScreen?: boolean;
  splitY?: number | null;
  twoPane?: boolean;
  fullDisplay?: boolean;
  playerMode?: 'full' | 'mini' | string;
  pip?: boolean;
  playingHere?: boolean;
  offline?: boolean;
  media?: 'live' | 'vod' | 'clip' | 'room' | string;
  tvCard?: boolean;
  /** The core's answer for the served ladder; null when it knows no size. */
  portrait?: boolean | null;
  /** The playing item's own frame, used only when the core knows none. */
  measuredPortrait?: boolean | null;
}

/** The picture is taller than wide: the core's answer first, the measured
 *  frame only when the core has none. */
export function tallPicture(portrait: boolean | null | undefined, measured: boolean | null | undefined): boolean {
  return portrait ?? measured ?? false;
}

/** The tall layout shows: a tall picture on an upright phone whose window
 *  fills the display at 16:9 or taller, in the full player, playing here, live
 *  or a VOD. */
export function tallLayout(i: LayoutInputs): boolean {
  if (!(i.w > 0 && i.h > 0 && i.h / i.w >= 16 / 9 - 0.0001)) return false;
  if (i.largeScreen || i.splitY != null || i.twoPane || i.fullDisplay === false) return false;
  if ((i.playerMode ?? 'full') !== 'full' || i.pip || i.playingHere === false || i.offline || i.tvCard) return false;
  const media = i.media ?? 'live';
  if (media !== 'live' && media !== 'vod') return false;
  return tallPicture(i.portrait, i.measuredPortrait);
}

/** Where the message box needs room under the picture (the field and its
 *  breathing room). */
export const COMPOSER_ROOM = 56;
/** The lower third's share of the picture's height. */
export const LOWER_SHARE = 0.36;
/** The room the chat button takes beside the message box: its 44 px and the
 *  composer row's own gap. */
export const CHAT_BUTTON_SLOT = 50;

/** The picture's height in the tall layout: the width at 9:16, capped so the
 *  message box still fits under it (a 16:9 phone shows the picture a little
 *  narrower than wide). */
export function bandHeight(width: number, height: number, topInset: number, bottomKeep: number): number {
  const natural = (width * 16) / 9;
  const room = height - topInset - COMPOSER_ROOM - bottomKeep;
  return Math.max(0, Math.min(natural, room));
}

/** The lower third's height: a fixed share of the picture, so large text shows
 *  fewer rows, never more area. */
export function lowerHeight(band: number): number {
  return Math.round(band * LOWER_SHARE);
}

/** The mini box: today's long edge (200 px) on whichever side the picture is
 *  long. */
export function miniSize(tall: boolean): { width: number; height: number } {
  return tall ? { width: 112.5, height: 200 } : { width: 200, height: 112.5 };
}

/** The view a start asks for: Landscape when the posture or TV screen mode
 *  wants it; else this session's pick; else null, which sends nothing and lets
 *  the core read the remembered pick. */
export function wantedView(c: {
  posture: Posture;
  tv: boolean;
  airplay: boolean;
  sessionPick: StreamView | null;
}): StreamView | null {
  if (c.posture !== 'upright' || c.tv || c.airplay) return 'landscape';
  return c.sessionPick;
}

/** A posture or TV change restarts the stream only when it has both pictures
 *  and the view now wanted (the remembered one resolved) is not the one
 *  playing. A stream with one picture never restarts on a turn. */
export function restartOnPosture(c: {
  verticalAvailable: boolean;
  served: StreamView | null;
  wantedNow: StreamView | null;
}): boolean {
  if (!c.verticalAvailable || !c.served || !c.wantedNow) return false;
  return c.served !== c.wantedNow;
}

/** `vertical-available` as the core sends it. */
export interface VerticalAvailable {
  ticket: number;
  available: boolean;
}

/** A late `vertical-available` for the start holding `ticket` sets the pill
 *  either way: a start that did not wait for v2 shows what the core last knew
 *  of the channel, and v2's own answer then confirms or corrects it. Any other
 *  start's answer changes nothing. */
export function availableAfter(event: VerticalAvailable, current: boolean, ticket: number | null): boolean {
  if (ticket == null || event.ticket !== ticket) return current;
  return event.available;
}

/** The core can send a late answer just before its start's own result reaches
 *  the shell: an answer for a newer ticket than the one playing (or while no
 *  start is held) waits until that start's result arrives. An older one is
 *  dropped. */
export function holdsLate(event: VerticalAvailable, ticket: number | null): boolean {
  if (ticket == null) return true;
  return event.ticket > ticket;
}

/** The chat's next state. */
export function nextChat(state: VerticalChatState, input: VerticalChatInput): VerticalChatState {
  switch (state) {
    case 'lower':
      if (input === 'swipeRight' || input === 'tapChatButton') return 'cleared';
      if (input === 'dragUp' || input === 'tapChatArea') return 'expanded';
      return 'lower';
    case 'cleared':
      if (input === 'swipeLeft' || input === 'tapChatButton') return 'lower';
      return 'cleared';
    case 'expanded':
      // Back, a tap on the picture above it, and leaving the tall layout fold
      // it down. Sideways swipes belong to the rows here.
      if (
        input === 'dragDown' ||
        input === 'tapPicture' ||
        input === 'back' ||
        input === 'rotate' ||
        input === 'mini' ||
        input === 'pip'
      ) {
        return 'lower';
      }
      if (input === 'tapChatButton') return 'cleared';
      return 'expanded';
  }
}

/** The state the chat starts in on entering the tall layout
 *  (`fullscreen_chat.vertical_open`, read again on every entry). The grown
 *  chat is never remembered. */
export function entryState(open: boolean): VerticalChatState {
  return open ? 'lower' : 'cleared';
}

/** What a drag in the tall layout became at the slop. The state is the one at
 *  touch-down. */
export function latch(region: TallRegion, state: VerticalChatState, dx: number, dy: number): TallDrag {
  if (region === 'composer') return 'none';
  const sideways = Math.abs(dx) > Math.abs(dy);
  if (state === 'expanded') return !sideways && dy > 0 ? 'collapse' : 'none';
  if (sideways) return 'swipe';
  if (dy < 0) return state === 'lower' && (region === 'lowerThird' || region === 'handle') ? 'expand' : 'none';
  return 'mini';
}

/** The chat's state once a latched drag lets go (swipes on the landscape chat
 *  swipe's thresholds, with chat on the right). Speeds are CSS px per ms, as
 *  every Android gesture measures them. A mini drag is the layer's, never the
 *  chat's. */
export function release(
  drag: TallDrag,
  state: VerticalChatState,
  dx: number,
  dy: number,
  vx: number,
  vy: number,
): VerticalChatState {
  switch (drag) {
    case 'swipe': {
      const intent = chatSwipeIntent({ dx, dy, vx, side: 'right', open: state !== 'cleared' });
      if (!intent) return state;
      return nextChat(state, intent === 'close' ? 'swipeRight' : 'swipeLeft');
    }
    case 'expand':
      return past(-dy, -vy) ? nextChat(state, 'dragUp') : state;
    case 'collapse':
      return past(dy, vy) ? nextChat(state, 'dragDown') : state;
    default:
      return state;
  }
}

function past(travel: number, speed: number): boolean {
  return travel >= SWIPE_MIN || (speed >= FLING_SPEED && travel >= FLING_MIN);
}

/** Android only: a tall-layout swipe never starts in the strip along a side
 *  edge, where Back is pulled (a fingertip, or the system's own gesture strip
 *  when that is larger); a drag up never starts in the bottom gesture strip. */
export function tallSwipeCanStart(x: number, width: number, edgeLeft: number, edgeRight: number): boolean {
  return x >= Math.max(DIAL_EDGE_GUARD, edgeLeft) && x <= width - Math.max(DIAL_EDGE_GUARD, edgeRight);
}

/** The count inside the chat button while chat is cleared: the resume
 *  button's words. */
export function missedWords(count: number): string {
  return resumeCountText(count);
}

/** What TalkBack reads for the chat button. */
export function missedSpoken(count: number): string {
  if (count <= 0) return 'Show chat';
  return `Show chat, ${count} new ${count === 1 ? 'message' : 'messages'}`;
}

/** The pill's word (the picture it switches to) and what TalkBack reads. */
export function pill(served: StreamView): { text: string; spoken: string } {
  return served === 'vertical'
    ? { text: 'Landscape', spoken: 'Watch landscape' }
    : { text: 'Vertical', spoken: 'Watch vertical' };
}

/** The one-time note by the pill when vertical gave up the ad-free relay. */
export const ADS_NOTE = 'Vertical plays straight from Twitch, so ads can show.';

/** The Ad-free page's line while the start's `reason` is `vertical`. */
export const AD_FREE_DETAIL = 'Vertical view plays straight from Twitch. Ads can appear.';

/** Rows that arrived since chat was cleared, per room. Each room's counter only
 *  grows (the slice's countable live rows); the anchor is taken on entering
 *  cleared, a room first seen is anchored where it stands, and a counter below
 *  its anchor (a new room under the same key) is anchored again. */
export class MissedCount {
  private anchors = new Map<string, number>();

  /** Chat was cleared: count from here. */
  clear(key: string, value: number): void {
    this.anchors.set(key, value);
  }

  /** A room's counter as it is now: anchors a room first seen, or one whose
   *  counter fell. */
  see(key: string, value: number): void {
    const anchor = this.anchors.get(key);
    if (anchor !== undefined && value >= anchor) return;
    this.anchors.set(key, value);
  }

  /** How many rows arrived in `key` since its anchor; 0 for a room never
   *  anchored. */
  missed(key: string, value: number): number {
    const anchor = this.anchors.get(key);
    if (anchor === undefined || value < anchor) return 0;
    return value - anchor;
  }
}

/** The start queue's skip rule: a restart waiting behind another step skips
 *  itself when a step ran since it was queued, that step serves the view now
 *  wanted, and playback is healthy. */
export function queueSkips(c: {
  ranSinceQueued: boolean;
  served: StreamView | null;
  wanted: StreamView | null;
  healthy: boolean;
}): boolean {
  if (!c.ranSinceQueued || !c.healthy) return false;
  if (!c.wanted) return true;
  return c.served === c.wanted;
}

/** Reads a `view` off a start's answer. */
export function parseView(v: unknown): StreamView | null {
  return v === 'landscape' || v === 'vertical' ? v : null;
}

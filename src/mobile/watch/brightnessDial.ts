// The landscape brightness dial's rule. The dial runs from full down to a 2
// percent floor; a swipe that keeps going past the floor reaches Auto, which
// hands the screen back to the phone's own brightness (Android drops the
// window override; the iPhone's twin, SNKit `BrightnessDial`, puts the
// borrowed level back). Kept beside the gesture rather than in Rust: the
// answer is needed synchronously on every pointer move, and it is arithmetic
// on the finger's position, like `dialZone`.
import type { DialReading } from './DialIndicator';

/** The darkest level the dial sets, so a dimmed screen can always be dialed
 *  back up. MainActivity clamps to the same floor. */
export const BRIGHTNESS_FLOOR = 0.02;

/** How far past the bottom of the dial (CSS px) the finger goes to reach
 *  Auto. The floor holds over that stretch, so the darkest level is easy to
 *  stop on. */
export const AUTO_PUSH_PX = 24;

/** How far back up (CSS px) a finger in Auto comes before the dial leaves
 *  it, so a resting finger never flickers between Auto and the floor. */
export const AUTO_RELEASE_PX = 12;

/** A dial's hundredth: what the readout prints and what is pushed. */
export function dialHundredth(level: number): number {
  return Math.round(level * 100);
}

/** A dial's haptic step: the tens digit of the hundredth on show, so the
 *  tick lands where the readout's tens digit changes. */
export function dialTenth(level: number): number {
  return Math.floor(dialHundredth(level) / 10);
}

/** One brightness swipe, from finger down to finger up. */
export interface BrightnessDial {
  /** The level when the finger landed, 0..1: ours, or the phone's in Auto. */
  start: number;
  /** Pixels of travel for the full 0..1 swing. */
  travel: number;
  /** Whether the finger is in the Auto stretch (with the release margin). */
  inAuto: boolean;
  /** What was last applied: -1 for Auto, else the level's hundredth. */
  pushed: number;
  /** Haptic step of the last applied level: -1 for Auto, null until the
   *  first push of a swipe that started in Auto. */
  step: number | null;
  /** How many times the layer had handed the screen back to the phone when
   *  the swipe began; any hand-back after that ends the swipe. */
  handBacks: number;
}

/** What one move asks the gesture to do. */
export interface BrightnessMove {
  /** The bridge argument to send: -1 for Auto, else the level; null to send
   *  nothing. */
  push: number | null;
  /** detent: the dial entered Auto; step: a new tens digit. */
  haptic: 'detent' | 'step' | null;
  /** The readout for where the dial stands now. */
  reading: DialReading;
  /** Something else handed the screen back to the phone mid-swipe (playback
   *  ended, PiP): the swipe ends here rather than undo it. */
  stop: boolean;
}

/** A swipe's starting state. `auto` says the phone's own level is on the
 *  screen (no level of ours in force); `handBacks` is the layer's count of
 *  hand-backs so far. */
export function startBrightnessDial(
  start: number,
  travel: number,
  auto: boolean,
  handBacks = 0,
): BrightnessDial {
  const level = Math.min(1, Math.max(0, start));
  return {
    start: level,
    travel,
    inAuto: false,
    pushed: auto ? -1 : dialHundredth(level),
    step: auto ? null : dialTenth(level),
    handBacks,
  };
}

/**
 * Moves the dial to a drag of `dy` px (down is positive) and says what to
 * apply. Updates `dial` in place. The move says stop instead of putting a
 * level back when something else handed the screen back since the swipe
 * began (playback ended, PiP): the layer's `handBacks` count moved on, or
 * the dial's own level is no longer in force (`overrideInForce`). The count
 * also catches a hand-back while the finger sits in Auto.
 */
export function moveBrightnessDial(
  dial: BrightnessDial,
  dy: number,
  overrideInForce: boolean,
  handBacks: number = dial.handBacks,
): BrightnessMove {
  if (handBacks !== dial.handBacks || (!overrideInForce && dial.pushed !== -1)) {
    return { push: null, haptic: null, reading: { axis: 'brightness', value: 0, auto: true }, stop: true };
  }
  // How far past the bottom of the dial the finger is, in px.
  const below = dy - dial.start * dial.travel;
  dial.inAuto = dial.inAuto ? below > AUTO_PUSH_PX - AUTO_RELEASE_PX : below >= AUTO_PUSH_PX;
  const level = dial.inAuto
    ? null
    : Math.min(1, Math.max(BRIGHTNESS_FLOOR, dial.start - dy / dial.travel));
  const reading: DialReading =
    level === null ? { axis: 'brightness', value: 0, auto: true } : { axis: 'brightness', value: level };
  const key = level === null ? -1 : dialHundredth(level);
  if (key === dial.pushed) return { push: null, haptic: null, reading, stop: false };
  dial.pushed = key;
  const bucket = level === null ? -1 : dialTenth(level);
  let haptic: BrightnessMove['haptic'] = null;
  if (dial.step === null) {
    dial.step = bucket;
  } else if (bucket !== dial.step) {
    dial.step = bucket;
    haptic = level === null ? 'detent' : 'step';
  }
  return { push: level === null ? -1 : level, haptic, reading, stop: false };
}

/** The player layer's hold on the window brightness, between swipes. */
export interface LayerBrightness {
  /** A dialed level is in force on the window. */
  touched: boolean;
  /** The dialed level set aside while the stream is in PiP; null when none. */
  saved: number | null;
}

/**
 * What the layer does with a dialed brightness when playback or PiP changes.
 * Playback over, never started, or an offline room: hand the screen back and
 * forget anything set aside. In PiP: set the level aside and hand the screen
 * back, since Android applies a window's brightness to the whole display
 * while the PiP window is on screen. Back from PiP while watching: the level
 * set aside comes back, but only once the page is visible again, so a PiP
 * window being dismissed (the stream about to end) never flashes it back.
 * `push` is the bridge argument to send, or null; `level` reads the level in
 * force, asked only when one is set aside.
 */
export function layerBrightness(
  state: LayerBrightness,
  now: { watching: boolean; pip: boolean; visible: boolean },
  level: () => number,
): LayerBrightness & { push: number | null } {
  if (!now.watching) return { touched: false, saved: null, push: state.touched ? -1 : null };
  if (now.pip) {
    if (!state.touched) return { ...state, push: null };
    return { touched: false, saved: level(), push: -1 };
  }
  if (state.saved !== null && now.visible) return { touched: true, saved: null, push: state.saved };
  return { ...state, push: null };
}

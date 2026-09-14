// Mouse-driven volume for the video player: wheel to change volume, middle click
// to mute. Shared by the single player (VideoPlayer) and each MultiNook tile so
// both surfaces feel identical.
//
// The wheel over the player is contested — Plyr binds its own wheel handler to
// its volume slider, and ChannelAboutReveal binds one to the wrapper above the
// player — so the guard below matters as much as the math.

/** Anything with volume/muted: a Plyr instance or a bare <video> element. */
export interface VolumeTarget {
  volume: number;
  muted: boolean;
}

/** Volume moved per wheel notch when the user hasn't picked a step. */
export const WHEEL_VOLUME_STEP = 0.05;

/** The two scroll gestures that compete for the wheel over the player. */
export interface PlayerScrollPrefs {
  scroll_volume?: boolean;
  scroll_about_reveal?: boolean;
}

export const scrollVolumeOn = (s?: PlayerScrollPrefs) => s?.scroll_volume ?? true;
export const scrollAboutRevealOn = (s?: PlayerScrollPrefs) => s?.scroll_about_reveal ?? true;

/**
 * True when Shift is what picks out the About reveal. Both gestures want the
 * wheel, so when both are enabled the plain wheel stays with volume (the far
 * more frequent action) and the reveal moves to Shift + scroll down. With only
 * one of them on, that one owns the plain wheel and no modifier is involved.
 *
 * Every surface that reads the wheel goes through this, so the player and the
 * reveal can never disagree about who owns a given event.
 */
export function aboutRevealNeedsShift(s?: PlayerScrollPrefs): boolean {
  return scrollVolumeOn(s) && scrollAboutRevealOn(s);
}

// One volume step per this many pixels of accumulated scroll. A wheel notch is
// ~100px, so a notch is one step; a precision trackpad emits dozens of tiny
// deltas per second that accumulate to the same place instead of slamming the
// volume to a rail.
const WHEEL_NOTCH_PX = 100;
// Scroll left alone for this long starts a fresh accumulation, so a half-notch
// nudge doesn't sit around and add itself to an unrelated scroll a minute later.
const IDLE_RESET_MS = 300;

/**
 * True when a player-area mouse event landed on something that owns its own
 * scroll or its own meaning, so we should leave the event completely alone.
 *
 * `.plyr__volume` is the important one: Plyr already runs a wheel-volume handler
 * on that slider, so handling it here too would move the volume twice per notch.
 */
export function ignoresPlayerMouse(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el?.closest) return false;
  return !!(
    el.closest('.plyr__volume') ||
    el.closest('.plyr__menu') ||
    el.closest('.plyr__menu__container') ||
    el.closest('[data-no-wheel-volume]')
  );
}

/**
 * Turns raw wheel deltas into whole volume steps. One accumulator per player —
 * two tiles scrolled in turn shouldn't pool their deltas.
 *
 * Returns how many steps to move (positive = louder), which is 0 for the small
 * sub-notch events a trackpad produces. The caller still consumes those events.
 */
export function createWheelAccumulator() {
  let accum = 0;
  let last = 0;

  return (event: WheelEvent, now: number): number => {
    // deltaMode: 0 = pixels, 1 = lines, 2 = pages. Normalize everything to pixels.
    const scale = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? 100 : 1;
    // Natural-scroll inversion. Reported by WebKit only, so it matters on macOS
    // builds and is simply undefined everywhere else.
    const inverted = (event as WheelEvent & { webkitDirectionInvertedFromDevice?: boolean })
      .webkitDirectionInvertedFromDevice;
    // Scrolling UP (negative deltaY) means louder.
    const delta = (inverted ? event.deltaY : -event.deltaY) * scale;

    if (now - last > IDLE_RESET_MS || Math.sign(delta) !== Math.sign(accum)) accum = 0;
    last = now;
    accum += delta;

    const steps = Math.trunc(accum / WHEEL_NOTCH_PX);
    accum -= steps * WHEEL_NOTCH_PX;
    return steps;
  };
}

/**
 * Move volume by `steps * step`, identically for a Plyr instance and a bare
 * <video>.
 *
 * Plyr's volume setter clears mute on any assignment above zero, which would
 * mean scrolling DOWN on a muted player unmutes it. So the mute state is
 * asserted explicitly afterwards: scrolling up off a muted player unmutes it
 * (that's what the gesture means), scrolling down leaves it muted with its
 * level intact.
 */
export function stepVolume(target: VolumeTarget, steps: number, step = WHEEL_VOLUME_STEP) {
  const raw = target.volume + steps * step;
  const volume = Math.min(1, Math.max(0, Math.round(raw * 100) / 100));
  const muted = steps > 0 && volume > 0 ? false : target.muted;

  target.volume = volume;
  if (target.muted !== muted) target.muted = muted;

  return { volume, muted };
}

/** Flip mute and report the resulting state, for the volume readout. */
export function toggleVolumeMute(target: VolumeTarget) {
  target.muted = !target.muted;
  return { volume: target.volume, muted: target.muted };
}

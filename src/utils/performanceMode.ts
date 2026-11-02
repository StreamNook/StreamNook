// Desktop Performance mode (Settings > Interface > Performance mode). Rust
// decides when it is active (services::performance_mode: "In the background"
// depends on whether ANY StreamNook window is in front, which no single page
// can see) and tells every window through `performance-state`. This module
// applies it to the window it runs in: decorative loops rest and CSS
// animations finish at once (utils/lowPower), the backdrop blur goes (themes),
// and with Freeze emotes the chat draws first frames. The video is untouched.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { isPerformanceQuiet, setLowPowerSource } from './lowPower';
import { setPerformanceFlat } from '../themes';
import { peekPreloadedSettings } from '../bootPreload';
import { Logger } from './logger';
import type { Settings } from '../types';

/** What Rust sends (`performance_mode::PerformanceState`). */
export interface PerformanceState {
  active: boolean;
  still_chat: boolean;
}

/** Rust has answered (an event or the first read), so the settings seed below
 *  no longer applies. */
let answered = false;
let installing: Promise<void> | null = null;

/** The finite CSS animations Performance mode is holding at zero duration.
 *  Read while the instant look is still on; finished right after it lifts, so
 *  an entrance that mounted inside its delay or duration never replays when
 *  its real timing comes back. Exempt one-shot cues keep their own duration
 *  and are left alone. */
export function cappedAnimations(): Animation[] {
  if (typeof document === 'undefined' || typeof document.getAnimations !== 'function') return [];
  if (typeof CSSAnimation === 'undefined') return [];
  const held: Animation[] = [];
  for (const a of document.getAnimations()) {
    if (!(a instanceof CSSAnimation)) continue;
    const timing = a.effect?.getComputedTiming();
    if (!timing || timing.iterations === Infinity || timing.duration !== 0) continue;
    held.push(a);
  }
  return held;
}

/** Finish each animation in `list` (see cappedAnimations). Reading the timing
 *  first makes the engine apply the style change that just restored the real
 *  duration; finishing before that would end it at zero and let it replay. */
export function finishFiniteAnimations(list: Animation[]): void {
  for (const a of list) {
    try {
      a.effect?.getComputedTiming();
      a.finish();
    } catch {
      // Gone from the page, or no longer finite: nothing to hold.
    }
  }
}

export function applyPerformanceState(state: PerformanceState): void {
  const active = state.active === true;
  const leaving = !active && isPerformanceQuiet();
  const held = leaving ? cappedAnimations() : [];
  setLowPowerSource('performance', { quiet: active, still: active && state.still_chat === true, flat: active });
  setPerformanceFlat(active);
  if (held.length > 0) finishFiniteAnimations(held);
}

/** Until Rust answers, a window whose settings say Always draws the cheap look
 *  from its first frame. Anything else waits for Rust. */
function seed(settings: Partial<Settings> | null | undefined): void {
  if (answered || !settings) return;
  if (settings.performance_mode !== 'always') return;
  applyPerformanceState({ active: true, still_chat: settings.performance_still_chat === true });
}

function fromRust(state: PerformanceState): void {
  answered = true;
  applyPerformanceState(state);
}

async function install(): Promise<void> {
  // The main window has the boot preload; popouts load their own settings
  // into the store, so follow that until Rust has answered.
  peekPreloadedSettings()
    ?.then((s) => seed(s as Partial<Settings> | null))
    .catch(() => {});
  void import('../stores/AppStore')
    .then(({ useAppStore }) => {
      seed(useAppStore.getState().settings);
      const stop = useAppStore.subscribe((s) => {
        if (answered) {
          stop();
          return;
        }
        seed(s.settings);
      });
    })
    .catch(() => {});

  let gotEvent = false;
  try {
    await listen<PerformanceState>('performance-state', (event) => {
      gotEvent = true;
      fromRust(event.payload);
    });
  } catch (err) {
    Logger.debug('[Performance] listen failed:', err);
  }
  try {
    const state = await invoke<PerformanceState>('get_performance_state');
    if (!gotEvent) fromRust(state);
  } catch (err) {
    // The full look stays (or the settings seed, which is the same rule).
    Logger.debug('[Performance] get_performance_state failed:', err);
  }
}

/** Once per window (desktop): a second call returns the same promise. */
export function installPerformanceMode(): Promise<void> {
  if (!installing) installing = install();
  return installing;
}

/** Resolves when `p` settles or after `ms`, whichever is first. Never rejects:
 *  the first paint must happen whatever Rust does. */
export function paintAfter(p: Promise<unknown>, ms: number): Promise<void> {
  return new Promise<void>((resolve) => {
    const timer = setTimeout(resolve, ms);
    p.then(
      () => {},
      () => {},
    ).finally(() => {
      clearTimeout(timer);
      resolve();
    });
  });
}

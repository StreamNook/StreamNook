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
import { CHEAP_EXEMPT } from './cheapExempt';
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

function timelineNow(): number | null {
  if (typeof document === 'undefined') return null;
  const t = document.timeline?.currentTime;
  return typeof t === 'number' ? t : null;
}

/** Right after the instant look lifts: finish every finite CSS animation that
 *  started before `uncapAt`, so an entrance that mounted inside its delay or
 *  duration never replays when its real timing comes back (a delayed one too,
 *  which was already gone from the page's list while capped). Exempt one-shot
 *  cues kept their own timing all along and are left alone. Reading playState
 *  first makes the engine apply the style change that restored the real
 *  durations; getComputedTiming() does not, and finishing before that ends
 *  each animation at zero and lets it replay. */
export function finishReplays(uncapAt: number | null): void {
  if (uncapAt === null || typeof document === 'undefined') return;
  if (typeof document.getAnimations !== 'function' || typeof CSSAnimation === 'undefined') return;
  const first = document.getAnimations()[0];
  if (first) void first.playState;
  for (const a of document.getAnimations()) {
    try {
      if (!(a instanceof CSSAnimation)) continue;
      if (a.effect?.getComputedTiming().iterations === Infinity) continue;
      const target = (a.effect as KeyframeEffect | null)?.target;
      if (target?.matches?.(CHEAP_EXEMPT)) continue;
      if (typeof a.startTime === 'number' && a.startTime <= uncapAt) a.finish();
    } catch {
      // Gone from the page meanwhile: nothing to hold.
    }
  }
}

export function applyPerformanceState(state: PerformanceState): void {
  const active = state.active === true;
  const leaving = !active && isPerformanceQuiet();
  const uncapAt = leaving ? timelineNow() : null;
  setLowPowerSource('performance', { quiet: active, still: active && state.still_chat === true, flat: active });
  setPerformanceFlat(active);
  if (leaving) finishReplays(uncapAt);
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

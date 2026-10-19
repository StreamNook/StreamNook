// The phones' sleep timer, as the core runs it (services/sleep_timer.rs). Rust
// owns the clock so it keeps going with the screen off and the app in the
// background, and it words every label; this module holds its last state for
// the player's menu and the moon in the header, fades the audio for the last
// seconds, and stops playback like the close button when it fires. Rust has
// already stopped the relay and the watch session by then; exitStream lets go
// of the player, which releases the media session and the foreground service.
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../../stores/AppStore';
import { getActiveVideo } from '../../utils/activeVideo';
import { Logger } from '../../utils/logger';
import {
  customMinutes,
  sleepTimerRunning,
  type SleepTimerPreset,
  type SleepTimerState,
} from './sleepTimerModel';

export { customMinutes, sleepTimerRunning };
export type { SleepTimerPreset, SleepTimerState };

export const useSleepTimer = create<{ state: SleepTimerState | null }>(() => ({ state: null }));

const take = (state: SleepTimerState | null | undefined) => {
  if (state) useSleepTimer.setState({ state });
};

export async function startSleepTimer(minutes: number): Promise<void> {
  take(await invoke<SleepTimerState>('sleep_timer_start', { minutes }).catch(() => null));
}

export async function cancelSleepTimer(): Promise<void> {
  take(await invoke<SleepTimerState>('sleep_timer_cancel').catch(() => null));
  cancelFade();
}

export async function refreshSleepTimer(): Promise<void> {
  take(await invoke<SleepTimerState>('sleep_timer_state').catch(() => null));
}

let fadeTimer: number | null = null;
let fadeVideo: HTMLVideoElement | null = null;
let volumeBefore: number | null = null;

/** The last seconds: the volume falls to nothing; the stop that follows puts it back. */
function fadeOut(seconds: number): void {
  const video = getActiveVideo();
  if (!video) return;
  if (fadeTimer !== null) window.clearInterval(fadeTimer);
  fadeVideo = video;
  volumeBefore ??= video.volume;
  const from = volumeBefore;
  const steps = Math.max(1, seconds * 10);
  let step = 0;
  fadeTimer = window.setInterval(() => {
    step += 1;
    video.volume = Math.max(0, (from * (steps - step)) / steps);
    if (step >= steps && fadeTimer !== null) {
      window.clearInterval(fadeTimer);
      fadeTimer = null;
    }
  }, 100);
}

/** Turned off mid-fade, or stopped: the volume comes back. */
export function cancelFade(): void {
  if (fadeTimer !== null) window.clearInterval(fadeTimer);
  fadeTimer = null;
  if (fadeVideo && volumeBefore !== null) fadeVideo.volume = volumeBefore;
  fadeVideo = null;
  volumeBefore = null;
}

/** Follow the core's pushes. Returns the unlisten. */
export function listenSleepTimer(): () => void {
  let unlisten: (() => void) | null = null;
  let disposed = false;
  void listen<SleepTimerState>('sleep-timer://state', (event) => {
    const state = event.payload;
    take(state);
    if (state.phase === 'fading') {
      fadeOut(state.fadeSecs);
    } else if (state.phase === 'fired') {
      Logger.info('[SleepTimer] fired; stopping playback');
      const store = useAppStore.getState();
      void Promise.resolve(store.currentStream ? store.exitStream() : undefined).finally(cancelFade);
    } else {
      cancelFade();
    }
  }).then((u) => {
    if (disposed) u();
    else unlisten = u;
  });
  void refreshSleepTimer();
  return () => {
    disposed = true;
    unlisten?.();
  };
}

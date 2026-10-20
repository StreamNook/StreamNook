// The sleep timer's shapes and pure helpers, with no app imports, so tests and
// views can read them without loading the store. The state lives in sleepTimer.ts.

export interface SleepTimerPreset {
  minutes: number;
  label: string;
}

export interface SleepTimerState {
  phase: 'off' | 'running' | 'fading' | 'fired';
  endsAtMs?: number | null;
  remainingSecs?: number | null;
  /** "Stops in 42 min" while one runs. */
  rowLabel?: string | null;
  fadeSecs: number;
  presets: SleepTimerPreset[];
}

export const sleepTimerRunning = (state: SleepTimerState | null): boolean =>
  state?.phase === 'running' || state?.phase === 'fading';

/** Minutes for a custom pick of hours and minutes; null when it comes to nothing. */
export const customMinutes = (hours: number, minutes: number): number | null => {
  const total = Math.max(0, hours) * 60 + Math.max(0, minutes);
  return total > 0 ? Math.min(total, 24 * 60) : null;
};

// The sleep timer in the player's menu, under the qualities: the presets and
// Custom while it is off; while it runs, when it stops playback and Off. The
// core runs the timer and words every label, the same words as the iPhone.
import React, { useEffect, useState } from 'react';
import { MoonStars } from 'phosphor-react';
import { MobileSheet } from '../ui/MobileSheet';
import { Button } from '../ui/Button';
import {
  cancelSleepTimer,
  customMinutes,
  refreshSleepTimer,
  sleepTimerRunning,
  startSleepTimer,
  useSleepTimer,
  type SleepTimerPreset,
} from './sleepTimer';

// Shown for the moment before the core answers with its own presets.
const FALLBACK_PRESETS: SleepTimerPreset[] = [
  { minutes: 15, label: '15 min' },
  { minutes: 30, label: '30 min' },
  { minutes: 60, label: '1 h' },
  { minutes: 120, label: '2 h' },
];
const HOURS = Array.from({ length: 13 }, (_, i) => i);
const MINUTES = Array.from({ length: 60 }, (_, i) => i);

export const SleepTimerSection: React.FC = () => {
  const state = useSleepTimer((s) => s.state);
  const [customOpen, setCustomOpen] = useState(false);
  const running = sleepTimerRunning(state);

  // The row counts down while the menu is open.
  useEffect(() => {
    void refreshSleepTimer();
    const id = window.setInterval(() => void refreshSleepTimer(), 20_000);
    return () => window.clearInterval(id);
  }, []);

  const chip = (label: string, onClick: () => void) => (
    <button
      key={label}
      onClick={onClick}
      className="h-[34px] shrink-0 whitespace-nowrap rounded-full bg-white/[0.08] px-3 text-[13px] font-semibold text-textPrimary active:opacity-70"
    >
      {label}
    </button>
  );

  return (
    <div className="mt-1 flex flex-col gap-2">
      <div className="px-2 text-[12px] font-semibold uppercase tracking-wide text-textMuted">Sleep timer</div>
      {running ? (
        <div className="flex min-h-[44px] items-center gap-2.5 px-2">
          <MoonStars size={16} weight="fill" className="shrink-0 text-accent" aria-hidden />
          <span className="flex-1 text-[15px] tabular-nums text-textPrimary">{state?.rowLabel}</span>
          <button
            onClick={() => void cancelSleepTimer()}
            className="sn-touch px-2 text-[14px] font-semibold text-textSecondary active:opacity-70"
          >
            Off
          </button>
        </div>
      ) : (
        <div className="flex gap-2 px-2">
          {(state?.presets?.length ? state.presets : FALLBACK_PRESETS).map((p) =>
            chip(p.label, () => void startSleepTimer(p.minutes)),
          )}
          {chip('Custom', () => setCustomOpen(true))}
        </div>
      )}
      <SleepTimerPicker open={customOpen} onClose={() => setCustomOpen(false)} />
    </div>
  );
};

/** A custom sleep timer: hours and minutes, then Start. */
const SleepTimerPicker: React.FC<{ open: boolean; onClose: () => void }> = ({ open, onClose }) => {
  const [hours, setHours] = useState(0);
  const [minutes, setMinutes] = useState(45);
  const total = customMinutes(hours, minutes);
  const select =
    'h-[44px] flex-1 rounded-xl bg-white/[0.08] px-3 text-[15px] text-textPrimary outline-none';
  return (
    <MobileSheet open={open} onClose={onClose} title="Sleep timer">
      <div className="flex flex-col gap-4 px-2 pb-2">
        <div className="flex gap-3">
          <select aria-label="Hours" className={select} value={hours} onChange={(e) => setHours(Number(e.target.value))}>
            {HOURS.map((h) => (
              <option key={h} value={h}>{`${h} h`}</option>
            ))}
          </select>
          <select aria-label="Minutes" className={select} value={minutes} onChange={(e) => setMinutes(Number(e.target.value))}>
            {MINUTES.map((m) => (
              <option key={m} value={m}>{`${m} min`}</option>
            ))}
          </select>
        </div>
        <Button
          kind="primary"
          disabled={total === null}
          onClick={() => {
            if (total === null) return;
            void startSleepTimer(total);
            onClose();
          }}
        >
          Start
        </Button>
      </div>
    </MobileSheet>
  );
};

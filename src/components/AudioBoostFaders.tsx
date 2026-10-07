import type { ReactNode } from 'react';
import type { AudioBoostSettings } from '../types';
import { audioBoostFaderDefs, audioBoostResetPatch } from '../utils/audioBoost';
import { Tooltip } from './ui/Tooltip';

// Presentational building blocks for the Audio Boost controls, shared by the
// Player settings tab and the in-player popover so the two stay identical. The
// fader descriptors and reset patch live in utils/audioBoost (kept out of this
// file so it only exports components, which Fast Refresh requires).

// Small on/off switch (matches the settings toggles).
export const Toggle = ({
  enabled,
  onChange,
}: {
  enabled: boolean;
  onChange: () => void;
}) => (
  <button
    onClick={onChange}
    aria-pressed={enabled}
    className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors flex-shrink-0 ${
      enabled ? 'bg-accent' : 'bg-gray-600'
    }`}
  >
    <span
      className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
        enabled ? 'translate-x-6' : 'translate-x-1'
      }`}
    />
  </button>
);

// Vertical, EQ-style fader: value readout on top, the upright slider, then a
// label (with an optional hover hint). Reads like a mixer/EQ.
export const Fader = ({
  label,
  display,
  value,
  min,
  max,
  step,
  hint,
  onChange,
}: {
  label: string;
  display: string;
  value: number;
  min: number;
  max: number;
  step: number;
  hint?: string;
  onChange: (v: number) => void;
}) => (
  <div className="flex flex-col items-center gap-2">
    <span className="text-[12px] font-semibold text-textPrimary tabular-nums">{display}</span>
    <input
      type="range"
      min={min}
      max={max}
      step={step}
      value={value}
      onChange={(e) => onChange(parseFloat(e.target.value))}
      className="eq-fader accent-accent"
      aria-label={label}
    />
    {hint ? (
      <Tooltip content={hint} side="bottom" delay={150}>
        <span className="text-[11px] text-textSecondary text-center cursor-help">{label}</span>
      </Tooltip>
    ) : (
      <span className="text-[11px] text-textSecondary text-center">{label}</span>
    )}
  </div>
);

// The whole in-player Audio Boost panel body: title + on/off, the faders, and a
// footer that always offers "Reset to defaults" plus whatever the host adds.
// Shared by the solo player's popover and each MultiNook tile's, so the two
// never drift.
export const AudioBoostControls = ({
  boost,
  onPatch,
  note,
  footer,
}: {
  boost: AudioBoostSettings;
  onPatch: (patch: Partial<AudioBoostSettings>) => void;
  note?: ReactNode;
  footer?: ReactNode;
}) => (
  <>
    <div className="mb-3 flex items-center justify-between gap-3">
      <span className="text-[13px] font-semibold text-textPrimary">Audio Boost</span>
      <Toggle enabled={boost.enabled} onChange={() => onPatch({ enabled: !boost.enabled })} />
    </div>
    {note && <p className="-mt-1 mb-3 text-[11px] text-textSecondary">{note}</p>}
    <div className={boost.enabled ? '' : 'opacity-50 pointer-events-none'}>
      <div className="flex flex-wrap items-end justify-center gap-x-5 gap-y-4">
        {audioBoostFaderDefs(boost).map((d) => (
          <Fader
            key={d.key}
            label={d.label}
            display={d.display}
            value={d.value}
            min={d.min}
            max={d.max}
            step={d.step}
            hint={d.hint}
            onChange={(v) => onPatch(d.apply(v))}
          />
        ))}
      </div>
    </div>
    <div className="mt-3 flex flex-wrap items-center justify-center gap-x-4 gap-y-2">
      <button
        onClick={() => onPatch(audioBoostResetPatch())}
        style={{ borderRadius: 8 }}
        className="glass-button text-textSecondary hover:text-textPrimary text-xs px-3 py-1.5"
      >
        Reset to defaults
      </button>
      {footer}
    </div>
  </>
);

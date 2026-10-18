import { useEffect, useState, type ReactNode } from 'react';
import { Plus, Prohibit, Trash, X } from 'phosphor-react';

import { useAppStore } from '../../stores/AppStore';
import {
  TIMEOUT_LENGTH_CHOICES,
  useLineConfig,
  useModSliderSteps,
  useModTimeoutSteps,
  useSliderConfig,
} from '../../services/moderation';
import { Dropdown } from '../ui/Dropdown';
import { fitHeaderDensity } from '../chat/fitHeaderDensity';

/** More timeouts than this stop fitting a narrow chat. */
const MAX_TIMEOUTS = 8;

// One height for every chip: each carries a 1px border, solid or dashed.
const chipBase = 'inline-flex items-center gap-1.5 rounded-full border py-1 text-[12.5px] transition-colors';

/** Where each list keeps its Delete, timeouts and Ban in `chat_design`. */
const LIST_FIELDS = {
  slider: { del: 'mod_slider_delete', timeouts: 'mod_slider_timeouts', ban: 'mod_slider_ban' },
  line: { del: 'mod_line_delete', timeouts: 'mod_line_timeouts', ban: 'mod_line_ban' },
} as const;

/**
 * The slider's steps (or, with `list="line"`, the line buttons') as chips, in
 * the order they are used: Delete, the timeouts shortest first, Ban. Delete
 * and Ban switch on and off; timeouts are removed with their x and added from
 * the menu. The order is not editable on purpose: further right is always
 * harsher.
 */
export function ModSliderStepsEditor({ list = 'slider' }: { list?: keyof typeof LIST_FIELDS }) {
  const settings = useAppStore((s) => s.settings);
  const updateSettings = useAppStore((s) => s.updateSettings);
  const sliderConfig = useSliderConfig();
  const lineConfig = useLineConfig();
  const config = list === 'line' ? lineConfig : sliderConfig;
  const fields = LIST_FIELDS[list];
  const steps = useModSliderSteps('twitch', config);
  // Compress in place (tighter chips, then icon-only Delete and Ban, a bare
  // Add) rather than wrap Ban onto a line of its own.
  const [row, setRow] = useState<HTMLDivElement | null>(null);
  useEffect(() => (row ? fitHeaderDensity(row, 2, 0) : undefined), [row]);
  const timeouts = steps?.filter((s) => s.action === 'timeout') ?? [];

  const write = (patch: { del?: boolean; timeouts?: number[]; ban?: boolean }) =>
    updateSettings({
      ...settings,
      chat_design: {
        ...settings.chat_design!,
        ...(patch.del !== undefined ? { [fields.del]: patch.del } : {}),
        ...(patch.timeouts !== undefined ? { [fields.timeouts]: patch.timeouts } : {}),
        ...(patch.ban !== undefined ? { [fields.ban]: patch.ban } : {}),
      },
    });

  const remaining = TIMEOUT_LENGTH_CHOICES.filter((c) => !config.timeouts.includes(c.value));
  const full = config.timeouts.length >= MAX_TIMEOUTS;

  const toggleChip = (on: boolean, label: string, icon: ReactNode, onToggle: () => void) => (
    <button
      type="button"
      aria-pressed={on}
      aria-label={label}
      title={label}
      onClick={onToggle}
      className={`${chipBase} sn-step-toggle px-2.5 ${
        on
          ? 'border-white/[0.12] bg-white/[0.08] text-textPrimary hover:bg-white/[0.12]'
          : 'border-dashed border-white/15 text-textMuted hover:text-textSecondary'
      }`}
    >
      {icon}
      <span className="sn-step-word">{label}</span>
    </button>
  );

  return (
    <div ref={setRow} className="sn-steps flex min-w-0 flex-nowrap items-center gap-1.5">
      {toggleChip(config.delete, 'Delete', <Trash size={12} weight="bold" />, () => write({ del: !config.delete }))}
      {timeouts.map((step) => (
        <span
          key={step.seconds}
          className={`${chipBase} sn-step-time border-white/[0.12] bg-white/[0.08] pl-2.5 pr-1 text-textPrimary`}
        >
          <span>{step.label}</span>
          <button
            type="button"
            aria-label={`Remove ${step.label}`}
            onClick={() => write({ timeouts: config.timeouts.filter((t) => t !== step.seconds) })}
            className="rounded-full p-0.5 text-textMuted transition-colors hover:bg-white/10 hover:text-textPrimary"
          >
            <X size={12} weight="bold" />
          </button>
        </span>
      ))}
      {!full && remaining.length > 0 && (
        <Dropdown<number>
          bare
          value={0}
          options={remaining}
          placeholder="Add"
          leadingIcon={<Plus size={12} weight="bold" />}
          ariaLabel="Add a timeout length"
          onChange={(value) => write({ timeouts: [...config.timeouts, value] })}
          className={`${chipBase} border-dashed border-white/15 pl-2 pr-1.5 text-textMuted hover:border-white/25 hover:text-textSecondary`}
        />
      )}
      {toggleChip(config.ban, 'Ban', <Prohibit size={12} weight="bold" />, () => write({ ban: !config.ban }))}
    </div>
  );
}

/**
 * The hover buttons' other timeout lengths (what the arrow beside the quick
 * timeout opens) as the same chips, shortest first. Removed with their x,
 * added from the menu. The list is Rust's answer for the saved lengths, so a
 * fresh install shows Rust's defaults without the page keeping a copy.
 */
export function HoverTimeoutsEditor() {
  const settings = useAppStore((s) => s.settings);
  const updateSettings = useAppStore((s) => s.updateSettings);
  const steps = useModTimeoutSteps('twitch', true);
  const timeouts = steps?.filter((s) => s.action === 'timeout' && s.seconds != null) ?? [];
  const seconds = timeouts.map((s) => s.seconds as number);
  const [row, setRow] = useState<HTMLDivElement | null>(null);
  useEffect(() => (row ? fitHeaderDensity(row, 2, 0) : undefined), [row]);

  const write = (next: number[]) =>
    updateSettings({ ...settings, moderation: { ...settings.moderation, timeout_presets: next } });
  const remaining = TIMEOUT_LENGTH_CHOICES.filter((c) => !seconds.includes(c.value));

  return (
    <div ref={setRow} className="sn-steps flex min-w-0 flex-nowrap items-center gap-1.5">
      {timeouts.map((step) => (
        <span
          key={step.seconds}
          className={`${chipBase} sn-step-time border-white/[0.12] bg-white/[0.08] pl-2.5 pr-1 text-textPrimary`}
        >
          <span>{step.label}</span>
          {timeouts.length > 1 && (
            <button
              type="button"
              aria-label={`Remove ${step.label}`}
              onClick={() => write(seconds.filter((t) => t !== step.seconds))}
              className="rounded-full p-0.5 text-textMuted transition-colors hover:bg-white/10 hover:text-textPrimary"
            >
              <X size={12} weight="bold" />
            </button>
          )}
        </span>
      ))}
      {steps && seconds.length < MAX_TIMEOUTS && remaining.length > 0 && (
        <Dropdown<number>
          bare
          value={0}
          options={remaining}
          placeholder="Add"
          leadingIcon={<Plus size={12} weight="bold" />}
          ariaLabel="Add a timeout length"
          onChange={(value) => write([...seconds, value])}
          className={`${chipBase} border-dashed border-white/15 pl-2 pr-1.5 text-textMuted hover:border-white/25 hover:text-textSecondary`}
        />
      )}
    </div>
  );
}

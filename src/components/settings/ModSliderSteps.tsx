import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { Plus, Prohibit, Trash, X } from 'phosphor-react';

import { useAppStore } from '../../stores/AppStore';
import {
  TIMEOUT_LENGTH_CHOICES,
  parseModTimeout,
  useLineConfig,
  useModSliderSteps,
  useModTimeoutSteps,
  useSliderConfig,
} from '../../services/moderation';
import { Dropdown } from '../ui/Dropdown';
import { fitHeaderDensity } from '../chat/fitHeaderDensity';
import { Tooltip } from '../ui/Tooltip';

/** More timeouts than this stop fitting a narrow chat. */
const MAX_TIMEOUTS = 8;

// One height for every chip: each carries a 1px border, solid or dashed.
const chipBase = 'inline-flex items-center gap-1.5 rounded-full border py-1 text-[12.5px] transition-colors';

/** The Add menu's first item: type a length instead of picking one. */
const CUSTOM = -1;

/**
 * The dashed Add chip: "Custom length…", then the usual lengths not in the
 * list yet. Custom turns the chip into a small field. Enter adds what was
 * typed (Rust reads it: "45m", "1h30m", "2d", a bare number of minutes),
 * Escape or leaving it empty puts the chip back, and anything Rust cannot
 * read keeps the field open with a red edge and says how to write one.
 */
function AddTimeoutChip({ taken, onAdd }: { taken: number[]; onAdd: (seconds: number) => void }) {
  const [typing, setTyping] = useState(false);
  const [text, setText] = useState('');
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const close = () => {
    setTyping(false);
    setText('');
    setError(null);
  };
  const submit = async (leaving: boolean) => {
    if (!text.trim()) {
      close();
      return;
    }
    try {
      const step = await parseModTimeout(text);
      if (step.seconds != null && !taken.includes(step.seconds)) onAdd(step.seconds);
      close();
    } catch (err) {
      // Leaving the field with something unreadable drops it rather than
      // leaving a stuck red box behind.
      if (leaving) close();
      else {
        setError(String(err));
        // Selected, so the next try replaces it.
        inputRef.current?.select();
      }
    }
  };
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      void submit(false);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  };

  if (typing) {
    return (
      <Tooltip content={error ?? 'Type a length like 45m, 2h or 3d, then press Enter'} describe>
        <input
          ref={inputRef}
          autoFocus
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            setError(null);
          }}
          onKeyDown={onKeyDown}
          onBlur={() => void submit(true)}
          placeholder="45m"
          aria-label="Custom timeout length"
          aria-invalid={error ? true : undefined}
          spellCheck={false}
          className={`${chipBase} w-[76px] bg-transparent px-2.5 text-textPrimary outline-none placeholder:text-textMuted ${
            error ? 'border-error' : 'border-white/25 focus:border-white/40'
          }`}
        />
      </Tooltip>
    );
  }
  const presets = TIMEOUT_LENGTH_CHOICES.filter((c) => !taken.includes(c.value));
  return (
    <Dropdown<number>
      bare
      value={0}
      // Custom first: the presets run long enough to scroll.
      options={[{ value: CUSTOM, label: 'Custom length…' }, ...presets]}
      placeholder="Add"
      leadingIcon={<Plus size={12} weight="bold" />}
      ariaLabel="Add a timeout length"
      onChange={(value) => (value === CUSTOM ? setTyping(true) : onAdd(value))}
      className={`${chipBase} border-dashed border-white/15 pl-2 pr-1.5 text-textMuted hover:border-white/25 hover:text-textSecondary`}
    />
  );
}

/** Where each list keeps its Delete, timeouts and Ban in `chat_design`. */
const LIST_FIELDS = {
  slider: { del: 'mod_slider_delete', timeouts: 'mod_slider_timeouts', ban: 'mod_slider_ban' },
  line: { del: 'mod_line_delete', timeouts: 'mod_line_timeouts', ban: 'mod_line_ban' },
} as const;

/**
 * The slider's steps (or, with `list="line"`, the line buttons') as chips, in
 * the order they are used: Delete, the timeouts shortest first, Ban. Delete
 * and Ban switch on and off; timeouts are removed with their x and added from
 * the menu or typed. The order is not editable on purpose: further right is
 * always harsher.
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

  const full = config.timeouts.length >= MAX_TIMEOUTS;

  const toggleChip = (on: boolean, label: string, icon: ReactNode, onToggle: () => void) => (
    <Tooltip content={label}>
      <button
        type="button"
        aria-pressed={on}
        aria-label={label}
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
    </Tooltip>
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
      {!full && (
        <AddTimeoutChip taken={config.timeouts} onAdd={(value) => write({ timeouts: [...config.timeouts, value] })} />
      )}
      {toggleChip(config.ban, 'Ban', <Prohibit size={12} weight="bold" />, () => write({ ban: !config.ban }))}
    </div>
  );
}

/**
 * The hover buttons' other timeout lengths (what the arrow beside the quick
 * timeout opens) as the same chips, shortest first. Removed with their x,
 * added from the menu or typed. The list is Rust's answer for the saved lengths, so a
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
      {steps && seconds.length < MAX_TIMEOUTS && (
        <AddTimeoutChip taken={seconds} onAdd={(value) => write([...seconds, value])} />
      )}
    </div>
  );
}

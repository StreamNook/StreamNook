// The building blocks the automations page shares: a labeled part that sits on
// its own surface inside a rule's card, a labeled field with its error under
// it, a switch line, a number field, a names field, a labeled disclosure for
// the less common choices, and the small marks (run, Test or Live, a face).
import { useId, useState, type ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';
import { Toggle } from '../../ui/Toggle';
import { Tooltip } from '../../ui/Tooltip';
import { IS_MOBILE } from '../../../utils/platform';
import { splitNames } from '../../../utils/automations';
import type { AutomationRunState } from '../../../types';

export const FIELD = 'glass-input px-2.5 py-1.5 text-sm text-textPrimary';
export const INPUT = `w-full ${FIELD}`;
export const SELECT = 'glass-input max-w-full px-2 py-1.5 text-sm text-textPrimary';
export const HINT = 'text-[11.5px] leading-relaxed text-textMuted';
export const QUIET_BUTTON =
  'inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-[13px] text-textSecondary transition-colors hover:bg-white/[0.06] hover:text-textPrimary disabled:opacity-50';
export const PRIMARY_BUTTON =
  'glass-button inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-[13px] font-medium text-textPrimary disabled:opacity-50';
export const DANGER_BUTTON =
  'inline-flex items-center gap-1.5 rounded-lg border border-error/40 bg-error/10 px-3 py-1.5 text-[13px] font-medium text-textPrimary transition-colors hover:bg-error/20';

/** One part of a rule (When, If, Do, Send as) on its own surface, a step
 *  above the rule's card, with a quiet heading. */
export function Block({ title, aside, children }: { title: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="space-y-3 rounded-lg border border-borderSubtle/70 bg-white/[0.03] p-3.5">
      <div className="flex min-h-6 items-center justify-between gap-3">
        <h4 className="text-[11px] font-semibold uppercase tracking-[0.12em] text-textSecondary">{title}</h4>
        {aside}
      </div>
      {children}
    </section>
  );
}

/** A labeled input: its name above, a hint and its error under it. */
export function Field({
  label,
  hint,
  error,
  children,
}: {
  label: string;
  hint?: string;
  error?: string | null;
  children: ReactNode;
}) {
  return (
    <div className="space-y-1.5">
      <div className="text-[12px] font-medium text-textPrimary">{label}</div>
      {children}
      {hint && <p className={HINT}>{hint}</p>}
      {error && (
        <p role="alert" className="text-[12px] leading-relaxed text-error">
          {error}
        </p>
      )}
    </div>
  );
}

/** One on/off choice: its name and what it does, its switch at the end. */
export function SwitchLine({
  label,
  description,
  on,
  onChange,
  disabled = false,
}: {
  label: string;
  description?: string;
  on: boolean;
  onChange: () => void;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <div
      className={`flex items-center justify-between gap-4 ${IS_MOBILE ? 'min-h-[44px]' : 'min-h-[34px]'} ${disabled ? 'opacity-50' : ''}`}
    >
      <label htmlFor={id} className="min-w-0">
        <span className="block text-[13px] text-textPrimary">{label}</span>
        {description && <span className="mt-0.5 block text-[11.5px] leading-relaxed text-textMuted">{description}</span>}
      </label>
      <Toggle id={id} enabled={on} onChange={onChange} disabled={disabled} />
    </div>
  );
}

/** A whole number kept inside its range, with its unit after it. */
export function NumberField({
  value,
  min,
  max,
  label,
  suffix,
  onChange,
}: {
  value: number;
  min: number;
  max: number;
  label: string;
  suffix?: string;
  onChange: (value: number) => void;
}) {
  return (
    <span className="inline-flex items-center gap-2 text-[13px] text-textSecondary">
      <input
        type="number"
        aria-label={label}
        min={min}
        max={max}
        value={value}
        onChange={(e) => {
          const n = parseInt(e.target.value, 10);
          onChange(Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : min);
        }}
        className="w-24 glass-input px-2 py-1 text-center text-sm text-textPrimary"
      />
      {suffix && <span>{suffix}</span>}
    </span>
  );
}

/** Names typed as one line, kept as typed until the field is left. */
export function NamesInput({
  values,
  label,
  placeholder,
  onCommit,
}: {
  values: readonly string[];
  label: string;
  placeholder?: string;
  onCommit: (next: string[]) => void;
}) {
  const [text, setText] = useState<string | null>(null);
  return (
    <input
      value={text ?? values.join(', ')}
      aria-label={label}
      placeholder={placeholder}
      spellCheck={false}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => {
        if (text === null) return;
        onCommit(splitNames(text));
        setText(null);
      }}
      className={INPUT}
    />
  );
}

/** The less common choices, behind their name. */
export function Disclosure({ label, children }: { label: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="space-y-3">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="inline-flex items-center gap-1.5 text-[12px] font-medium text-textSecondary transition-colors hover:text-textPrimary"
      >
        <ChevronRight size={13} className={`transition-transform ${open ? 'rotate-90' : ''}`} />
        {label}
      </button>
      {open && <div className="space-y-3 border-l border-borderSubtle pl-3.5">{children}</div>}
    </div>
  );
}

/** Off gray, Test a ring, On filled. */
export function RunDot({ run }: { run: AutomationRunState }) {
  const look = run === 'on' ? 'bg-accent' : run === 'test' ? 'border-[1.5px] border-accent' : 'bg-textMuted/50';
  return <span aria-hidden className={`inline-block h-2 w-2 rounded-full align-middle ${look}`} />;
}

/** A rule's Test or Live, as a dot and a word. */
export function LiveWord({ live }: { live: boolean }) {
  return (
    <span className="inline-flex items-center gap-1.5 text-[11.5px] text-textSecondary">
      <span aria-hidden className={`h-1.5 w-1.5 rounded-full ${live ? 'bg-accent' : 'border border-textMuted'}`} />
      {live ? 'Live' : 'Test'}
    </span>
  );
}

/** An account's picture, or its first letter. */
export function AccountFace({
  account,
  size = 16,
}: {
  account: { login: string; name?: string };
  size?: number;
}) {
  // The core knows no account pictures: the first letter stands in.
  const label = account.name || account.login;
  return (
    <Tooltip content={label}>
      <span
        aria-label={label}
        className="flex shrink-0 items-center justify-center rounded-full bg-surface text-[9px] font-semibold text-textMuted"
        style={{ width: size, height: size }}
      >
        {label.charAt(0).toUpperCase()}
      </span>
    </Tooltip>
  );
}

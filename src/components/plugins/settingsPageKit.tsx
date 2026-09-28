// The native settings-page look, shared with plugins through `api.components`:
// a scrolling page that carries the "On this page" rail, and the compact
// controls a long page needs (an inline pill list, a slider with its value, a
// row's own sub-controls). With the Settings window's own sections, rows and
// segmented control beside them in the kit, a plugin composes its whole page
// from host components. It has to: the host stylesheet only holds the classes
// the host itself uses, so markup a plugin styles on its own renders bare.

import { useRef, useState, type ReactNode } from 'react';
import { Plus, X } from 'lucide-react';
import SectionNav from '../settings/SectionNav';

/** A long settings page that scrolls on its own, with the section rail beside
 *  it on wide windows. Fill it with PageSections; `railKey` names the page. */
export function SettingsPage({ railKey, children }: { railKey: string; children: ReactNode }) {
  const scrollRef = useRef<HTMLDivElement>(null);
  return (
    <div ref={scrollRef} className="h-full overflow-y-auto custom-scrollbar animate-in fade-in">
      <div className="mx-auto flex max-w-[1024px] justify-center gap-10 px-6 pb-10 pt-6">
        <div className="w-full min-w-0 max-w-3xl space-y-8">{children}</div>
        <SectionNav containerRef={scrollRef} tabKey={railKey} />
      </div>
    </div>
  );
}

/** A short list of names edited in place: removable pills, then an add field.
 *  Adding ignores case, so "rust" does not sit beside "Rust". */
export function PillList({
  items,
  numbered = false,
  placeholder = 'Add',
  onChange,
}: {
  items: string[];
  /** Show each pill's position, for a list whose order matters. */
  numbered?: boolean;
  placeholder?: string;
  onChange: (next: string[]) => void;
}) {
  const [input, setInput] = useState('');
  const add = () => {
    const name = input.trim();
    if (!name) return;
    if (!items.some((item) => item.toLowerCase() === name.toLowerCase())) onChange([...items, name]);
    setInput('');
  };
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {items.map((item, i) => (
        <span
          key={item}
          className="inline-flex max-w-full items-center gap-1.5 rounded-full bg-white/[0.05] py-1 pl-2.5 pr-1 text-[12.5px] text-textPrimary ring-1 ring-inset ring-white/[0.07]"
        >
          {numbered && <span className="font-mono text-[10.5px] tabular-nums text-textMuted">{i + 1}</span>}
          <span className="min-w-0 truncate">{item}</span>
          <button
            type="button"
            aria-label={`Remove ${item}`}
            onClick={() => onChange(items.filter((_, j) => j !== i))}
            className="rounded-full p-0.5 text-textMuted transition-colors hover:bg-white/10 hover:text-textPrimary"
          >
            <X size={12} />
          </button>
        </span>
      ))}
      <div className="inline-flex items-center rounded-full border border-dashed border-white/15 py-0.5 pl-2.5 pr-1 transition-colors focus-within:border-solid focus-within:border-accent/40 focus-within:bg-white/[0.03]">
        <Plus size={12} className="mr-1 shrink-0 text-textMuted" />
        <input
          type="text"
          value={input}
          placeholder={placeholder}
          aria-label={placeholder}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') add();
            if (e.key === 'Escape') setInput('');
          }}
          className="w-28 bg-transparent py-0.5 text-[12.5px] text-textPrimary placeholder:text-textMuted focus:outline-none"
        />
        {input.trim() && (
          <button
            type="button"
            onClick={add}
            className="ml-1 rounded-full bg-accent/15 px-2 py-0.5 text-[11.5px] font-medium text-accent transition-colors hover:bg-accent/25"
          >
            Add
          </button>
        )}
      </div>
    </div>
  );
}

/** A slider with its value beside it, sized for a row's control slot. */
export function InlineSlider({
  value,
  min,
  max,
  step = 1,
  label,
  format = String,
  disabled = false,
  onChange,
}: {
  value: number;
  min: number;
  max: number;
  step?: number;
  /** Announced to screen readers. */
  label: string;
  /** How the value reads beside the track, e.g. seconds shown as minutes. */
  format?: (value: number) => string;
  disabled?: boolean;
  onChange: (value: number) => void;
}) {
  return (
    <div className="flex items-center gap-2">
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={Math.min(max, Math.max(min, value))}
        disabled={disabled}
        aria-label={label}
        onChange={(e) => onChange(Number(e.target.value))}
        className="w-32 accent-accent"
      />
      <span className="w-12 text-right text-[12px] tabular-nums text-textMuted">{format(value)}</span>
    </div>
  );
}

/** A row's own sub-controls (the settings under a choice like Custom), set in
 *  under a rule so they read as part of that row. Put SubControls inside. */
export function SubControls({ children }: { children: ReactNode }) {
  return <div className="mt-4 space-y-3.5 border-l border-borderSubtle pl-4">{children}</div>;
}

/** One line in SubControls: its name, and its control on the right. */
export function SubControl({
  title,
  control,
  disabled = false,
}: {
  title: string;
  control: ReactNode;
  disabled?: boolean;
}) {
  return (
    <div className={`flex items-center justify-between gap-4 ${disabled ? 'pointer-events-none opacity-50' : ''}`}>
      <div className="min-w-0 text-[12.5px] font-medium text-textPrimary">{title}</div>
      <div className="shrink-0">{control}</div>
    </div>
  );
}

/** Autopilot 0.4.x drew the app's own Drops settings page through this name.
 *  The page ships with the plugin now, so an older Autopilot gets a pointer. */
export function DropsSettingsTab(_props: Record<string, unknown>) {
  return (
    <div className="flex h-full items-center justify-center px-6 text-center text-[13px] text-textSecondary">
      Update Autopilot in the Marketplace to see its settings here.
    </div>
  );
}

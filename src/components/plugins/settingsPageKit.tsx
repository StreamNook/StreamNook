// The native settings-page look, shared with plugins through `api.components`:
// a scrolling page that carries the "On this page" rail, and the compact
// controls a long page needs (an inline pill list, a slider with its value, a
// row's own sub-controls). With the Settings window's own sections, rows and
// segmented control beside them in the kit, a plugin composes its whole page
// from host components. It has to: the host stylesheet only holds the classes
// the host itself uses, so markup a plugin styles on its own renders bare.

import {
  cloneElement,
  createContext,
  isValidElement,
  useCallback,
  useContext,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { Plus, X } from 'lucide-react';
import SectionNav from '../settings/SectionNav';
import { ResetArrow, SegmentedSelect } from '../settings/_primitives';
import { IS_MOBILE } from '../../utils/platform';

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
    // On the phone the track flexes: beside a name it keeps a modest natural
    // width, and on a line of its own it spans the line, with the value in a
    // fixed column so the two never overlap.
    <div className={IS_MOBILE ? 'flex w-full items-center gap-2' : 'flex items-center gap-2'}>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={Math.min(max, Math.max(min, value))}
        disabled={disabled}
        aria-label={label}
        onChange={(e) => onChange(Number(e.target.value))}
        className={IS_MOBILE ? 'min-w-0 flex-1 basis-28 accent-accent' : 'w-32 accent-accent'}
      />
      <span className="w-12 text-right text-[12px] tabular-nums text-textMuted">{format(value)}</span>
    </div>
  );
}

/** A row's own sub-controls (the settings under a choice like Custom), set in
 *  under a rule so they read as part of that row. Put SubControls inside. */
export function SubControls({ children }: { children: ReactNode }) {
  // On the phone the group's sliders decide together, and so do its segmented
  // choices: when one has to go under its name, the others of its kind do too,
  // so their controls start at the same place and the group never mixes the
  // two layouts.
  const [stacked, setStacked] = useState<ReadonlyMap<string, SubControlKind>>(new Map());
  const report = useCallback((id: string, kind: SubControlKind, under: boolean) => {
    setStacked((prev) => {
      if (prev.has(id) === under) return prev;
      const next = new Map(prev);
      if (under) next.set(id, kind);
      else next.delete(id);
      return next;
    });
  }, []);
  const group = useMemo(
    () => ({ report, anyStacked: (kind: SubControlKind) => [...stacked.values()].includes(kind) }),
    [report, stacked],
  );
  return (
    <SubControlGroup.Provider value={group}>
      <div className="mt-4 space-y-3.5 border-l border-borderSubtle pl-4">{children}</div>
    </SubControlGroup.Provider>
  );
}

type SubControlKind = 'slider' | 'segmented';

const SubControlGroup = createContext<{
  report: (id: string, kind: SubControlKind, under: boolean) => void;
  anyStacked: (kind: SubControlKind) => boolean;
} | null>(null);

/** One line in SubControls: its name, and its control on the right. `stacked`
 *  puts the control under the name at full width, for a control too wide to
 *  share the line (a wrapping set of chips). */
export function SubControl({
  title,
  control,
  disabled = false,
  stacked = false,
  onReset,
}: {
  title: string;
  control: ReactNode;
  disabled?: boolean;
  stacked?: boolean;
  /** Restores this line's setting. Pass it only while the setting differs
   *  from its default, as on SettingsRow. */
  onReset?: () => void;
}) {
  const name = (
    // A step under its row's name, as every nested line on both phones.
    <div className="min-w-0 text-[12.5px] font-medium text-textPrimary">
      {title}
      {onReset && <ResetArrow onReset={onReset} />}
    </div>
  );
  if (stacked) {
    return (
      <div
        className={`space-y-2 ${disabled ? 'pointer-events-none opacity-50' : ''}`}
        data-setting-row={title}
      >
        {name}
        {control}
      </div>
    );
  }
  if (IS_MOBILE) {
    return <PhoneSubControl title={title} control={control} disabled={disabled} onReset={onReset} />;
  }
  return (
    <div
      className={`flex items-center justify-between gap-4 ${disabled ? 'pointer-events-none opacity-50' : ''}`}
      // A settings search result can land on a nested line, as on a row.
      data-setting-row={title}
    >
      {name}
      <div className="shrink-0">{control}</div>
    </div>
  );
}

/** A sub-line on the phone: its name never shrinks or wraps, and a control
 *  that cannot share the line with it goes under the name, a segmented one at
 *  full width. The control's natural width is measured while it sits beside
 *  the name and remembered, so the layout cannot flip back and forth. */
function PhoneSubControl({
  title,
  control,
  disabled,
  onReset,
}: {
  title: string;
  control: ReactNode;
  disabled: boolean;
  onReset?: () => void;
}) {
  const rowRef = useRef<HTMLDivElement>(null);
  const nameRef = useRef<HTMLDivElement>(null);
  const partRef = useRef<HTMLSpanElement>(null);
  const natural = useRef(0);
  const under = useRef(false);
  const [ownStack, setOwnStack] = useState(false);
  const id = useId();
  const group = useContext(SubControlGroup);
  const isSlider = isValidElement(control) && control.type === InlineSlider;
  const isSegmented = isValidElement(control) && control.type === SegmentedSelect;
  const report = group?.report;
  useLayoutEffect(() => {
    const row = rowRef.current;
    if (!row) return;
    const fit = () => {
      const nameEl = nameRef.current;
      const part = partRef.current;
      if (!nameEl || !part) return;
      // The control's natural width is only readable while it sits beside the
      // name (its group can put it under too); the name sizes to its own text
      // either way (w-fit). A segmented control is read by its track's own box:
      // its sliding thumb can still sit where a full-width layout put it, which
      // would inflate scrollWidth.
      if (!under.current) {
        const track = part.firstElementChild;
        natural.current = isSegmented && track ? track.getBoundingClientRect().width : part.scrollWidth;
      }
      setOwnStack(nameEl.scrollWidth + 16 + natural.current > row.clientWidth);
    };
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(row);
    return () => ro.disconnect();
  }, [isSegmented]);
  const kind: SubControlKind | null = isSlider ? 'slider' : isSegmented ? 'segmented' : null;
  useLayoutEffect(() => {
    if (!report || !kind) return;
    report(id, kind, ownStack);
    return () => report(id, kind, false);
  }, [report, kind, id, ownStack]);
  const stack = ownStack || (!!kind && !!group?.anyStacked(kind));
  useLayoutEffect(() => {
    under.current = stack;
  }, [stack]);
  const shown =
    stack && isValidElement<{ fullWidth?: boolean }>(control) && control.type === SegmentedSelect
      ? cloneElement(control, { fullWidth: true })
      : control;
  return (
    <div
      ref={rowRef}
      className={`${stack ? 'space-y-2' : 'flex items-center gap-4'} ${disabled ? 'pointer-events-none opacity-50' : ''}`}
      // A settings search result can land on a nested line, as on a row.
      data-setting-row={title}
    >
      <div ref={nameRef} className="w-fit shrink-0 whitespace-nowrap text-[12.5px] font-medium text-textPrimary">
        {title}
        {onReset && <ResetArrow onReset={onReset} />}
      </div>
      <div className={stack ? 'flex w-full' : 'flex min-w-0 flex-1 justify-end'}>
        <span ref={partRef} className={stack ? 'flex w-full' : 'inline-flex'}>
          {shown}
        </span>
      </div>
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

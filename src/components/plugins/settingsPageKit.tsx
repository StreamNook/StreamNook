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
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { ChevronDown, Plus, TriangleAlert, X } from 'lucide-react';
import SectionNav from '../settings/SectionNav';
import { ResetArrow, SegmentedSelect } from '../settings/_primitives';
import { IS_MOBILE } from '../../utils/platform';
import { progressPercent, readDisclosureOpen, writeDisclosureOpen } from './settingsKitPure';

// The pure helper lives beside the other plain functions so a test can import
// it without the components; it is exported here too, as the kit's own name for it.
// eslint-disable-next-line react-refresh/only-export-components
export { progressPercent };

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
  description,
  control,
  disabled = false,
  stacked = false,
  onReset,
  tone,
}: {
  title: string;
  /** One plain sentence under the name, saying what the line does. */
  description?: string;
  control: ReactNode;
  disabled?: boolean;
  stacked?: boolean;
  /** Restores this line's setting. Pass it only while the setting differs
   *  from its default, as on SettingsRow. */
  onReset?: () => void;
  /** `'warning'` draws the description as a warning, with a small triangle. */
  tone?: 'warning';
}) {
  const name = (
    // A step under its row's name, as every nested line on both phones.
    <div className="min-w-0 text-[12.5px] font-medium text-textPrimary">
      <span className={disabled ? 'opacity-50' : ''}>{title}</span>
      {onReset && <ResetArrow onReset={onReset} />}
      {description && <SubDescription tone={tone}>{description}</SubDescription>}
    </div>
  );
  if (stacked) {
    return (
      <div className={`space-y-2 ${disabled ? 'pointer-events-none' : ''}`} data-setting-row={title}>
        {name}
        <div className={disabled ? 'opacity-40 grayscale' : ''}>{control}</div>
      </div>
    );
  }
  if (IS_MOBILE) {
    return (
      <PhoneSubControl
        title={title}
        description={description}
        control={control}
        disabled={disabled}
        onReset={onReset}
        tone={tone}
      />
    );
  }
  return (
    <div
      className={`flex items-center justify-between gap-4 ${disabled ? 'pointer-events-none' : ''}`}
      // A settings search result can land on a nested line, as on a row.
      data-setting-row={title}
    >
      {name}
      <div className={`shrink-0 ${disabled ? 'opacity-40 grayscale' : ''}`}>{control}</div>
    </div>
  );
}

/** The sentence under a sub-line's name: plain, or a warning with a triangle. */
function SubDescription({ tone, children }: { tone?: 'warning'; children: ReactNode }) {
  if (tone === 'warning') {
    return (
      <p className="mt-0.5 flex items-start gap-1.5 text-[12px] font-normal leading-relaxed text-warning [html[data-theme-tone=light]_&]:text-[color-mix(in_srgb,var(--color-warning)_58%,black)]">
        <TriangleAlert size={12} className="mt-[3px] shrink-0" />
        <span>{children}</span>
      </p>
    );
  }
  return <p className="mt-0.5 text-[12px] font-normal leading-relaxed text-textSecondary">{children}</p>;
}

/** A sub-line on the phone: its name never shrinks or wraps, and a control
 *  that cannot share the line with it goes under the name, a segmented one at
 *  full width. The control's natural width is measured while it sits beside
 *  the name and remembered, so the layout cannot flip back and forth. */
function PhoneSubControl({
  title,
  description,
  control,
  disabled,
  onReset,
  tone,
}: {
  title: string;
  description?: string;
  control: ReactNode;
  disabled: boolean;
  onReset?: () => void;
  tone?: 'warning';
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
      className={disabled ? 'pointer-events-none' : ''}
      // A settings search result can land on a nested line, as on a row.
      data-setting-row={title}
    >
      <div ref={rowRef} className={stack ? 'space-y-2' : 'flex items-center gap-4'}>
        <div ref={nameRef} className="w-fit shrink-0 whitespace-nowrap text-[12.5px] font-medium text-textPrimary">
          <span className={disabled ? 'opacity-50' : ''}>{title}</span>
          {onReset && <ResetArrow onReset={onReset} />}
        </div>
        <div className={`${stack ? 'flex w-full' : 'flex min-w-0 flex-1 justify-end'} ${disabled ? 'opacity-40 grayscale' : ''}`}>
          <span ref={partRef} className={stack ? 'flex w-full' : 'inline-flex'}>
            {shown}
          </span>
        </div>
      </div>
      {/* Under the whole line, so the name keeps its single line above. */}
      {description && <SubDescription tone={tone}>{description}</SubDescription>}
    </div>
  );
}

// A visible step above the page on every theme: the page color nudged toward
// the text color (lighter on a dark theme), a soft edge on its top-left, and a
// shadow under it. Opaque, so Glassiness 0% leaves it solid; on a light theme
// the card's class swaps in a color lifted toward white.
const CARD_STYLE = {
  background: 'color-mix(in oklab, var(--color-background) 86%, var(--color-text-primary) 14%)',
  boxShadow:
    'inset 1px 1px 0 color-mix(in srgb, var(--color-text-primary) 10%, transparent), 0 6px 20px rgba(0, 0, 0, 0.18)',
} as const;

const TAG_STYLE = { background: 'color-mix(in srgb, var(--color-accent) 12%, transparent)' } as const;

/** What the plugin is doing right now, on a raised card at the top of its
 *  page: a small label (and an optional tag), a title, a line under it, an
 *  optional progress bar, and an action. Wraps on a narrow window: the action
 *  drops under the words, and a long title takes two lines. */
export function StatusCard({
  label,
  tag,
  title,
  subtitle,
  progress,
  action,
}: {
  /** The small caps line ("Right now"). */
  label: string;
  /** A pill beside the label. */
  tag?: { icon?: ReactNode; text: string };
  title: string;
  subtitle?: string;
  /** The inset bar: `left` and `right` are the words under its two ends. */
  progress?: { value: number; max: number; left: string; right?: string };
  /** A KitButton, or any control, on the right. */
  action?: ReactNode;
}) {
  // The last bar stays drawn while its row closes.
  const [lastProgress, setLastProgress] = useState(progress);
  if (progress && progress !== lastProgress) setLastProgress(progress);
  const bar = progress ?? lastProgress;
  return (
    <div
      className="rounded-2xl px-5 pb-5 pt-6 [html[data-theme-tone=light]_&]:!bg-[color-mix(in_oklab,var(--color-background)_40%,white)]"
      style={CARD_STYLE}
      data-status-card
    >
      <div className="flex flex-wrap items-start justify-between gap-x-6 gap-y-3">
        <div className="min-w-0 flex-1 basis-[330px]">
          <div className="flex min-h-[22px] flex-wrap items-center gap-2">
            <span className="text-[12px] font-semibold uppercase tracking-[0.12em] text-textMuted">{label}</span>
            {tag && (
              <span
                className="inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[12px] font-semibold text-textPrimary"
                style={TAG_STYLE}
              >
                {tag.icon && <span className="text-accent-on-surface">{tag.icon}</span>}
                {tag.text}
              </span>
            )}
            {action && <div className="-my-[7px] ml-auto shrink-0 pl-4">{action}</div>}
          </div>
          <div className="mt-2 line-clamp-2 break-words text-[19px] font-semibold leading-snug text-textPrimary">
            {title}
          </div>
          {subtitle && <div className="mt-1 break-words text-[13.5px] leading-snug text-textSecondary">{subtitle}</div>}
        </div>
      </div>
      <div
        className="grid transition-[grid-template-rows] duration-[220ms] motion-reduce:transition-none"
        style={{ gridTemplateRows: progress ? '1fr' : '0fr' }}
      >
        <div className="min-h-0 overflow-hidden">
          {bar && (
            <div className="mt-4">
              <div
                role="progressbar"
                aria-label={bar.left}
                aria-valuemin={0}
                aria-valuemax={bar.max > 0 ? bar.max : 1}
                aria-valuenow={Math.min(Math.max(bar.value, 0), bar.max > 0 ? bar.max : 1)}
                className="h-2 rounded-full p-[1.5px] bg-black/40 shadow-[inset_0_1px_2px_rgba(0,0,0,0.5)] [html[data-theme-tone=light]_&]:bg-black/10 [html[data-theme-tone=light]_&]:shadow-[inset_0_1px_1.5px_rgba(0,0,0,0.18)]"
              >
                <div
                  className="h-full rounded-full bg-accent transition-[width] duration-500 ease-out motion-reduce:transition-none"
                  style={{ width: `${progressPercent(bar.value, bar.max)}%` }}
                />
              </div>
              <div className="mt-2 flex flex-wrap justify-between gap-x-4 gap-y-0.5 text-[12.5px] text-textSecondary">
                <span className="tabular-nums">{bar.left}</span>
                {bar.right && <span>{bar.right}</span>}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/** A button in the page's own look: filled with the accent for the one thing
 *  the page wants you to do, or a quiet raised one for the rest. */
export function KitButton({
  children,
  onClick,
  icon,
  variant = 'quiet',
  disabled = false,
}: {
  children: ReactNode;
  onClick: () => void;
  icon?: ReactNode;
  variant?: 'primary' | 'quiet';
  disabled?: boolean;
}) {
  const look =
    variant === 'primary'
      ? 'bg-accent text-background hover:bg-accent-hover [html[data-theme-tone=light]_&]:text-white'
      : 'bg-[color-mix(in_srgb,var(--color-text-primary)_9%,transparent)] text-textPrimary ring-1 ring-inset ring-[color-mix(in_srgb,var(--color-text-primary)_12%,transparent)] hover:bg-[color-mix(in_srgb,var(--color-text-primary)_15%,transparent)]';
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={`inline-flex min-h-9 items-center gap-2 rounded-[10px] px-3.5 py-2 text-[13px] font-semibold transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[color:var(--color-text-primary)] active:translate-y-px disabled:pointer-events-none disabled:opacity-50 ${look}`}
    >
      {icon}
      {children}
    </button>
  );
}

const DISCLOSURE_MS = 220;

/** A fold on a page: its title and a one-line summary stay visible, and it
 *  opens in place with a height animation. Whether it is open is remembered
 *  per `id` for the visit. What it holds sits on a settings card, so put page
 *  rows (PageRow) straight inside. Shut, its content is not mounted. */
export function Disclosure({
  id,
  title,
  summary,
  children,
}: {
  id: string;
  title: string;
  /** What is set inside, in one line, shown while shut. */
  summary: string;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(() => readDisclosureOpen(id));
  // `mounted` keeps the content in the tree through the closing animation;
  // `shown` is the animated height; `settled` lets a fully open fold stop
  // clipping, so a focus ring or popover at its edge is not cut off.
  const [mounted, setMounted] = useState(open);
  const [shown, setShown] = useState(open);
  const [settled, setSettled] = useState(open);
  const bodyId = useId();
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const frame = useRef(0);
  useEffect(
    () => () => {
      clearTimeout(timer.current);
      cancelAnimationFrame(frame.current);
    },
    [],
  );
  const toggle = () => {
    const next = !open;
    setOpen(next);
    writeDisclosureOpen(id, next);
    clearTimeout(timer.current);
    cancelAnimationFrame(frame.current);
    setSettled(false);
    if (next) {
      setMounted(true);
      // Two frames: the content must be laid out at height 0 before it grows.
      frame.current = requestAnimationFrame(() => {
        frame.current = requestAnimationFrame(() => setShown(true));
      });
      timer.current = setTimeout(() => setSettled(true), DISCLOSURE_MS + 80);
    } else {
      setShown(false);
      timer.current = setTimeout(() => setMounted(false), DISCLOSURE_MS + 60);
    }
  };
  return (
    <div data-disclosure={id}>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={bodyId}
        onClick={toggle}
        className="flex w-full items-center justify-between gap-4 rounded-xl bg-[color-mix(in_srgb,var(--color-text-primary)_4%,transparent)] px-4 py-3.5 text-left ring-1 ring-inset ring-[color-mix(in_srgb,var(--color-text-primary)_8%,transparent)] transition-colors hover:bg-[color-mix(in_srgb,var(--color-text-primary)_7%,transparent)] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[color:var(--color-text-primary)]"
      >
        <span className="min-w-0">
          <span className="block text-[14px] font-semibold text-textPrimary">{title}</span>
          <span className="mt-0.5 block text-[12px] leading-snug text-textSecondary">{summary}</span>
        </span>
        <ChevronDown
          size={18}
          className="shrink-0 text-textMuted transition-transform duration-[220ms] motion-reduce:transition-none"
          style={{ transform: open ? 'rotate(180deg)' : 'none' }}
        />
      </button>
      <div
        id={bodyId}
        aria-hidden={!open}
        className="transition-[grid-template-rows] duration-[220ms] ease-[ease] motion-reduce:transition-none"
        style={{
          display: 'grid',
          gridTemplateRows: shown ? '1fr' : '0fr',
        }}
      >
        <div style={{ minHeight: 0, overflow: settled && open ? 'visible' : 'hidden' }}>
          {mounted && (
            <div
              className="transition-opacity duration-[220ms] ease-out motion-reduce:transition-none"
              style={{ paddingTop: 8, opacity: shown ? 1 : 0 }}
            >
              <div className="settings-card px-4">{children}</div>
            </div>
          )}
        </div>
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

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
  type CSSProperties,
  type ReactNode,
} from 'react';
import { motion } from 'framer-motion';
import { Award, ChevronDown, Play, Plus, TriangleAlert, X } from 'lucide-react';
import { ACCENT_BUTTON, ACCENT_FILL, GLAZE_PILL } from '../ui/glazeButtons';
import SectionNav from '../settings/SectionNav';
import { ResetArrow, SegmentedSelect } from '../settings/_primitives';
import { IS_MOBILE } from '../../utils/platform';
import { Tooltip } from '../ui/Tooltip';
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
        <div className={disabled ? 'opacity-60 [&_[role=switch]]:!bg-textPrimary/25' : ''}>{control}</div>
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
      <div className={`shrink-0 ${disabled ? 'opacity-60 [&_[role=switch]]:!bg-textPrimary/25' : ''}`}>{control}</div>
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
        <div className={`${stack ? 'flex w-full' : 'flex min-w-0 flex-1 justify-end'} ${disabled ? 'opacity-60 [&_[role=switch]]:!bg-textPrimary/25' : ''}`}>
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

// The status card wears the glaze (the chrome's lit glass) at a card's corner,
// with a little more frost than the chrome, since a page's words sit on it.
// The glaze's own rule rounds to a pill, so the corner is set inline.
const STATUS_GLAZE = { borderRadius: 20, '--glaze-frost': '30%' } as CSSProperties;

const TAG_STYLE = { background: 'color-mix(in srgb, var(--color-accent) 12%, transparent)' } as const;

/** The hairline between a glazed card's parts, in the theme's ink. */
const CARD_RULE = 'border-t border-[color-mix(in_srgb,var(--color-text-primary)_9%,transparent)]';

/** A picture on a card: a badge, drawn whole on a dark tile, or a game's box
 *  art, filling its tile. */
export interface CardArtSpec {
  src?: string;
  /** `'badge'` (the default) fits the image inside the tile; `'cover'` fills it. */
  shape?: 'badge' | 'cover';
}

function CardArt({ art, size }: { art: CardArtSpec; size: number }) {
  const [failed, setFailed] = useState(false);
  const cover = art.shape === 'cover';
  return (
    <span
      className="glaze-inset glaze-chip flex shrink-0 items-center justify-center overflow-hidden bg-black/30 [html[data-theme-tone=light]_&]:bg-black/[0.08]"
      style={{ width: size, height: size }}
      aria-hidden
    >
      {art.src && !failed ? (
        <img
          src={art.src}
          alt=""
          draggable={false}
          onError={() => setFailed(true)}
          className={cover ? 'h-full w-full object-cover' : 'h-[76%] w-[76%] object-contain'}
        />
      ) : (
        <Award size={Math.round(size * 0.42)} className="text-textMuted" />
      )}
    </span>
  );
}

/** One choice on a ModeSwitch. `live` marks a choice that starts work, which
 *  the chosen pill shows in the accent; the rest (Off) show neutral. */
export interface ModeOption<T extends string> {
  value: T;
  label: string;
  icon?: ReactNode;
  live?: boolean;
}

/** A few modes of one thing, one of which runs: a recessed track with the
 *  chosen mode a lit pill that glides between them. Picking a live mode
 *  starts it, so the chosen live pill is the accent; Off stays neutral. */
export function ModeSwitch<T extends string>({
  value,
  options,
  onChange,
  label,
  disabled = false,
}: {
  value: T;
  options: ModeOption<T>[];
  onChange: (next: T) => void;
  /** What the group chooses, for screen readers. */
  label: string;
  disabled?: boolean;
}) {
  // One pill per switch: two switches on a page must never trade pills.
  const pillId = useId();
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={`flex w-full gap-0.5 rounded-full bg-black/25 p-1 shadow-[inset_0_1px_2px_rgba(0,0,0,0.35)] sm:inline-flex sm:w-auto [html[data-theme-tone=light]_&]:bg-black/[0.07] [html[data-theme-tone=light]_&]:shadow-[inset_0_1px_1.5px_rgba(0,0,0,0.14)] ${disabled ? 'pointer-events-none opacity-60' : ''}`}
    >
      {options.map((opt) => {
        const on = opt.value === value;
        const lit = on && opt.live;
        return (
          <button
            key={opt.value}
            type="button"
            role="radio"
            aria-checked={on}
            onClick={() => {
              if (!on) onChange(opt.value);
            }}
            // In a narrow window each choice keeps its label's width plus an
            // equal share of what is left, so a long label is never cut.
            className={`relative flex min-w-0 flex-auto items-center justify-center gap-1.5 whitespace-nowrap rounded-full px-3.5 py-1.5 text-[13px] font-semibold transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[color:var(--color-text-primary)] sm:flex-none ${
              lit ? 'text-white' : on ? 'text-textPrimary' : 'text-textSecondary hover:text-textPrimary'
            }`}
          >
            {on && (
              <motion.span
                layoutId={`mode-pill-${pillId}`}
                className="absolute inset-0"
                transition={{ type: 'spring', stiffness: 380, damping: 32 }}
              >
                <span
                  className="glaze-inset block h-full w-full rounded-full transition-[background-color] duration-200"
                  style={lit ? ACCENT_FILL : NEUTRAL_PILL}
                />
              </motion.span>
            )}
            {opt.icon && <span className={`relative z-[1] inline-flex ${lit ? 'text-white' : ''}`}>{opt.icon}</span>}
            <span className="relative z-[1] truncate">{opt.label}</span>
          </button>
        );
      })}
    </div>
  );
}

const NEUTRAL_PILL = { background: 'color-mix(in srgb, var(--color-text-primary) 14%, transparent)' } as const;

/** What the plugin is doing right now, on a glazed card at the top of its
 *  page: a small label (and an optional tag), a title, a line under it, an
 *  optional progress bar, and an action on the label row. One height in every
 *  state: a long title ends in an ellipsis (the full text in a tooltip) and
 *  the subtitle takes at most two lines.
 *
 *  Optional parts, for a plugin whose work runs in modes: `modes` puts a
 *  ModeSwitch on top, `art` a picture beside the title, `queue` the next
 *  things in line under the bar, and `choices` explains each mode while it is
 *  off, as tiles that pick it. */
export function StatusCard({
  label,
  tag,
  title,
  subtitle,
  progress,
  action,
  art,
  modes,
  queue,
  choices,
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
  art?: CardArtSpec;
  modes?: {
    value: string;
    options: ModeOption<string>[];
    onChange: (next: string) => void;
    label: string;
    disabled?: boolean;
  };
  queue?: {
    heading: string;
    /** Quiet words on the heading's line, at its right ("5 in all"). */
    note?: string;
    items: { key: string; art?: CardArtSpec; title: string; subtitle?: string; trailing?: string }[];
  };
  choices?: { key: string; icon?: ReactNode; title: string; text: string; onPick: () => void }[];
}) {
  // The card keeps one height in every state, so nothing below it ever moves:
  // a hidden sizer lays out the tallest shape (label row, one title line, two
  // subtitle lines, the bar row), and the real content sits centered over it,
  // so an idle card reads calm instead of holding an empty band. A state
  // change fades the content in (opacity only: nothing slides).
  const nbsp = String.fromCharCode(0xa0);
  const titleRef = useRef<HTMLDivElement>(null);
  const [titleClipped, setTitleClipped] = useState(false);
  // The content layer remounts between its bar and idle forms, so the check
  // follows the title node that is mounted now.
  const hasBar = !!progress;
  const hasArt = !!art;
  useLayoutEffect(() => {
    const el = titleRef.current;
    if (!el) return;
    const measure = () => setTitleClipped(el.scrollWidth > el.clientWidth);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [title, hasBar, hasArt]);
  const titleBlock = (
    <>
      <Tooltip content={title} describe disabled={!titleClipped}>
        <div ref={titleRef} className="truncate text-[19px] font-semibold leading-snug text-textPrimary">
          {title}
        </div>
      </Tooltip>
      {subtitle && (
        <div className="mt-1 line-clamp-2 break-words text-[13.5px] leading-snug text-textSecondary">{subtitle}</div>
      )}
    </>
  );
  return (
    <div className="chrome-glaze flex flex-col" style={STATUS_GLAZE} data-status-card>
      {modes && (
        <div className="px-5 pb-4 pt-5">
          <ModeSwitch
            value={modes.value}
            options={modes.options}
            onChange={modes.onChange}
            label={modes.label}
            disabled={modes.disabled}
          />
        </div>
      )}
      <div className={`grid px-5 pb-5 ${modes ? `${CARD_RULE} pt-5` : 'pt-6'}`}>
      {/* The off state's choices follow it, and it never shows a bar, so it
          keeps no room for one. */}
      {!choices?.length && (
      <div className="invisible [grid-area:1/1]" aria-hidden>
        <div className="min-h-[22px]" />
        <div className="mt-2 text-[19px] font-semibold leading-snug">{nbsp}</div>
        <div className="mt-1 min-h-[2lh] text-[13.5px] leading-snug" />
        <div className="mt-4">
          <div className="h-2" />
          <div className="mt-2 text-[12.5px]">{nbsp}</div>
        </div>
      </div>
      )}
      <div
        key={hasBar ? 'bar' : 'idle'}
        className="min-w-0 -translate-y-1 self-center [grid-area:1/1]"
      >
        <div className="animate-enter-fade motion-reduce:animate-none">
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
        {art ? (
          <div className="mt-2.5 flex items-center gap-3.5">
            <CardArt key={art.src ?? ''} art={art} size={54} />
            <div className="min-w-0 flex-1">{titleBlock}</div>
          </div>
        ) : (
          <div className="mt-2">{titleBlock}</div>
        )}
        {progress && (
          <div className="mt-4">
            <div
              role="progressbar"
              aria-label={progress.left}
              aria-valuemin={0}
              aria-valuemax={progress.max > 0 ? progress.max : 1}
              aria-valuenow={Math.min(Math.max(progress.value, 0), progress.max > 0 ? progress.max : 1)}
              className="h-2 rounded-full p-[1.5px] bg-black/40 shadow-[inset_0_1px_2px_rgba(0,0,0,0.5)] [html[data-theme-tone=light]_&]:bg-black/10 [html[data-theme-tone=light]_&]:shadow-[inset_0_1px_1.5px_rgba(0,0,0,0.18)]"
            >
              <div
                className="h-full rounded-full bg-accent transition-[width] duration-500 ease-out motion-reduce:transition-none"
                style={{ width: `${progressPercent(progress.value, progress.max)}%` }}
              />
            </div>
            <div className="mt-2 flex flex-wrap justify-between gap-x-4 gap-y-0.5 text-[12.5px] text-textSecondary">
              <span className="tabular-nums">{progress.left}</span>
              {progress.right && <span>{progress.right}</span>}
            </div>
          </div>
        )}
        </div>
      </div>
      </div>
      {queue && queue.items.length > 0 && (
        <div className={`${CARD_RULE} px-5 pb-4 pt-3.5`}>
          <div className="mb-1.5 flex items-baseline justify-between gap-4">
            <span className="text-[11.5px] font-semibold uppercase tracking-[0.12em] text-textMuted">{queue.heading}</span>
            {queue.note && <span className="text-[12px] tabular-nums text-textMuted">{queue.note}</span>}
          </div>
          <ul>
            {queue.items.map((item) => (
              <li key={item.key} className="flex items-center gap-3 py-1.5">
                {item.art && <CardArt key={item.art.src ?? ''} art={item.art} size={34} />}
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13px] font-medium text-textPrimary">{item.title}</div>
                  {item.subtitle && <div className="truncate text-[12px] text-textSecondary">{item.subtitle}</div>}
                </div>
                {item.trailing && (
                  <span className="shrink-0 text-right text-[12px] tabular-nums text-textMuted">{item.trailing}</span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
      {choices && choices.length > 0 && (
        <div className={`${CARD_RULE} grid gap-2.5 px-5 pb-5 pt-4 sm:grid-cols-2`}>
          {choices.map((choice) => (
            <button
              key={choice.key}
              type="button"
              onClick={choice.onPick}
              className="glaze-inset glaze-chip group flex items-start gap-3 bg-[color-mix(in_srgb,var(--color-text-primary)_5%,transparent)] p-3.5 text-left transition-colors hover:bg-[color-mix(in_srgb,var(--color-text-primary)_9%,transparent)] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[color:var(--color-text-primary)]"
            >
              {choice.icon && <span className="mt-px inline-flex shrink-0 text-accent-on-surface">{choice.icon}</span>}
              <span className="min-w-0 flex-1">
                <span className="block text-[13.5px] font-semibold text-textPrimary">{choice.title}</span>
                <span className="mt-0.5 block text-[12px] leading-snug text-textSecondary">{choice.text}</span>
              </span>
              <span className="mt-px inline-flex shrink-0 items-center gap-1 text-[12px] font-semibold text-accent-on-surface opacity-80 transition-opacity group-hover:opacity-100">
                <Play size={11} className="fill-current" />
                Start
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** A capsule in the glaze's own look: the dark accent fill for the one thing
 *  the page wants you to do, or the quieter glazed pill for the rest. */
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
  const look = variant === 'primary' ? ACCENT_BUTTON : GLAZE_PILL;
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      style={variant === 'primary' ? ACCENT_FILL : undefined}
      className={`${look} inline-flex min-h-9 items-center gap-2 px-4 py-2 !text-[13px] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[color:var(--color-text-primary)] disabled:pointer-events-none`}
    >
      {icon}
      {children}
    </button>
  );
}

const DISCLOSURE_MS = 220;

/** The fold's bar: a settings glaze card's corner and frost. */
const FOLD_GLAZE = { borderRadius: 16, '--glaze-frost': '22%' } as CSSProperties;

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
        // The glaze, lit a little brighter under the pointer, like the chrome's own controls.
        className="chrome-glaze chrome-glaze--control flex w-full items-center justify-between gap-4 px-4 py-3.5 text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[color:var(--color-text-primary)]"
        style={FOLD_GLAZE}
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
              <div className="chrome-glaze settings-glaze px-4">{children}</div>
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

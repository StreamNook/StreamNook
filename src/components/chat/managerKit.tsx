// The stream manager family's one look: the panel frame over the message box,
// the window and sheet bodies, the footer with its main button, and the cards
// and rows the Tools menu, the Manage channel window and the phone's sheet
// share. Layout only; every line and every rule comes from Rust.
import { useCallback, useEffect, useRef } from 'react';
import type { CSSProperties, KeyboardEvent as ReactKeyboardEvent, ReactNode } from 'react';
import { CaretRight, Check, CircleNotch, CornersOut, WarningCircle, X } from 'phosphor-react';
import { Tooltip } from '../ui/Tooltip';
import { Dropdown } from '../ui/Dropdown';
import { useAppStore } from '../../stores/AppStore';
import { usePerformanceQuiet } from '../../utils/lowPower';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { useFitAbove } from '../../hooks/useFitAbove';
import { IS_MOBILE } from '../../utils/platform';
import { ACCENT_BUTTON, ACCENT_FILL, DANGER_FILL, GLAZE_PILL } from '../ui/glazeButtons';
import { SAVED_HOLD_MS, type ToolPhase } from './managerStore';

/** Where a form is drawn: the panel over the message box, a page of the
 *  Manage channel window, or the phone's Manage channel sheet. */
export type Layout = 'panel' | 'window' | 'sheet';

export const LABEL = 'text-[11px] font-semibold uppercase tracking-wider text-textSecondary';
export const KEY_CAP =
  'font-sans rounded-md bg-surface/50 px-1.5 py-[3px] text-[10.5px] font-bold leading-none text-textSecondary ring-1 ring-inset ring-textPrimary/10';
/** Close, expand, Discard, Back: a step quieter than the content and the main button. */
export const QUIET_PILL = `${GLAZE_PILL} !bg-textPrimary/[0.05] hover:!bg-textPrimary/[0.1] !text-textSecondary hover:!text-textPrimary`;
/** Done: the button's own glass tinted green, never a flat fill. */
export const SAVED_FILL: CSSProperties = {
  background: 'color-mix(in srgb, var(--color-success) 30%, var(--color-background-tertiary))',
};
export const TONE_CLASS = { quiet: 'text-textSecondary', near: 'text-warning', over: 'text-error' } as const;
/** A scrolling body's bottom edge fades into the footer instead of cutting. */
export const FADE_BOTTOM: CSSProperties = { maskImage: 'linear-gradient(to bottom, black calc(100% - 16px), transparent)' };
/** Both edges fade (a body under a header divider); the top fade is shorter
 *  than the body's top padding, so the first line is crisp at rest. */
export const FADE_BOTH: CSSProperties = {
  maskImage: 'linear-gradient(to bottom, transparent, black 10px, black calc(100% - 16px), transparent)',
};
const FADE_RIGHT: CSSProperties = { maskImage: 'linear-gradient(to right, black calc(100% - 20px), transparent)' };
/** A full-size action on the phone: finger-sized. */
export const BIG = 'min-h-[44px] !text-[15px]';
/** A text field on any layout. */
export const FIELD = 'glass-input w-full !rounded-xl px-3 text-textPrimary placeholder-textSecondary outline-none focus-within:!border-accent/40';

/** Motion Off or Performance mode: every move is instant. */
export function useInstant(): boolean {
  const motionOff = useAppStore((s) => s.settings.motion_mode) === 'off';
  const perfQuiet = usePerformanceQuiet();
  return motionOff || perfQuiet;
}

/** The family's ease, or none. */
export const easeFor = (instant: boolean) =>
  instant ? { duration: 0 } : { duration: 0.2, ease: [0.2, 0.8, 0.2, 1] as const };

/** Esc for a surface: skipped when something inside already answered it. */
export function useEscape(active: boolean, onEscape: () => void): void {
  const latest = useRef(onEscape);
  useEffect(() => {
    latest.current = onEscape;
  });
  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented || e.isComposing) return;
      e.preventDefault();
      e.stopPropagation();
      latest.current();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [active]);
}

/**
 * A panel that closes itself after showing its result. `closeSoon` waits
 * SAVED_HOLD_MS, and does nothing once this mount is gone: a write that
 * finishes after the panel closed, opened larger or reopened never closes
 * whatever shows now.
 */
export function useCloseSoon(onClose: () => void): () => void {
  const latest = useRef(onClose);
  const alive = useRef(true);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    latest.current = onClose;
  });
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      if (timer.current) clearTimeout(timer.current);
    };
  }, []);
  return useCallback(() => {
    if (!alive.current) return;
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      if (alive.current) latest.current();
    }, SAVED_HOLD_MS);
  }, []);
}

export function IconButton({
  label,
  onClick,
  children,
  size = 'h-[26px] w-[26px]',
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
  size?: string;
}) {
  return (
    <Tooltip content={label} side="top">
      <button type="button" onClick={onClick} aria-label={label} className={`${QUIET_PILL} grid ${size} flex-shrink-0 place-items-center`}>
        {children}
      </button>
    </Tooltip>
  );
}

interface FrameProps {
  title: string;
  icon: ReactNode;
  /** A short mark after the title ("Running"). */
  badge?: ReactNode;
  onClose: () => void;
  /** Open this tool in the Manage channel window. */
  onExpand?: () => void;
  footer?: ReactNode;
  problem?: string | null;
  children: ReactNode;
}

/**
 * The panel over the message box: header (icon, title, badge, open larger,
 * close), a scrolling body with a soft bottom edge, one problem line and the
 * footer. Esc and a press outside close it (the app's one outside rule); it
 * answers its own keys, so a key typed or pressed in it never reaches the
 * player.
 */
export function ManagerFrame({ title, icon, badge, onClose, onExpand, footer, problem, children }: FrameProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  useFitAbove(rootRef, true, 640);
  const markInside = useOutsidePress(true, [rootRef], onClose);
  useEscape(true, onClose);
  return (
    <div
      ref={rootRef}
      onPointerDownCapture={markInside}
      role="dialog"
      aria-label={title}
      data-sn-owns-keys=""
      className="glass-flyout rounded-xl !p-0 absolute bottom-full left-0 right-0 mb-2 z-[60] flex max-h-[70vh] flex-col overflow-hidden"
    >
      <div className="flex items-center gap-2 px-3.5 pb-2 pt-3">
        <span className="flex items-center justify-center text-textSecondary" aria-hidden>
          {icon}
        </span>
        <span className="min-w-0 truncate text-sm font-semibold text-textPrimary">{title}</span>
        {badge}
        <span className="flex-1" />
        {onExpand && (
          <IconButton label="Open larger" onClick={onExpand}>
            <CornersOut size={16} />
          </IconButton>
        )}
        <IconButton label="Close" onClick={onClose}>
          <X size={15} />
        </IconButton>
      </div>
      <div className="min-h-0 flex-1 space-y-3.5 overflow-y-auto scrollbar-thin px-3.5 pb-4 pt-0.5" style={FADE_BOTTOM}>
        {children}
      </div>
      {problem && (
        <div className="px-3.5 pb-2.5">
          <ProblemLine line={problem} />
        </div>
      )}
      {footer && <div className="border-t border-textPrimary/[0.08] px-3.5 py-2.5">{footer}</div>}
    </div>
  );
}

/** One form's body and footer in whichever layout it is drawn in. The window
 *  brings its own header; the sheet brings its own frame. */
export function FormShell({
  layout,
  frame,
  footer,
  problem,
  children,
}: {
  layout: Layout;
  frame: Omit<FrameProps, 'children' | 'footer' | 'problem'>;
  footer?: ReactNode;
  problem?: string | null;
  children: ReactNode;
}) {
  if (layout === 'panel') {
    return (
      <ManagerFrame {...frame} footer={footer} problem={problem}>
        {children}
      </ManagerFrame>
    );
  }
  if (layout === 'window') {
    return (
      <>
        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto scrollbar-thin px-5 pb-5 pt-1" style={FADE_BOTTOM}>
          {children}
        </div>
        {problem && (
          <div className="px-5 pb-3">
            <ProblemLine line={problem} />
          </div>
        )}
        {footer && <div className="border-t border-textPrimary/[0.08] px-5 py-3">{footer}</div>}
      </>
    );
  }
  return (
    <div className="space-y-3.5">
      {children}
      {problem && <ProblemLine line={problem} />}
      {footer && <div className="pt-1">{footer}</div>}
    </div>
  );
}

/** The footer's row: the keys this form answers (not on the phone), then
 *  the quieter action and the main button. */
export function FooterRow({
  layout,
  hints,
  children,
}: {
  layout: Layout;
  hints?: [key: string, what: string][];
  children: ReactNode;
}) {
  return (
    <div className="flex min-h-[34px] items-center gap-2">
      {layout !== 'sheet' && hints?.length ? (
        <span
          className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden whitespace-nowrap text-[11.5px] text-textSecondary/80"
          style={FADE_RIGHT}
        >
          {hints.map(([key, what], i) => (
            <span key={key} className={`flex items-center gap-1.5 ${i ? 'ml-1' : ''}`}>
              <kbd className={KEY_CAP}>{key}</kbd>
              {what}
            </span>
          ))}
        </span>
      ) : (
        <span className="flex-1" />
      )}
      {children}
    </div>
  );
}

/**
 * The one filled button, carrying its write's result: working (spinner),
 * done (green tinted glass and its done word), failed (Try again).
 */
export function MainButton({
  layout,
  phase,
  label,
  working,
  done,
  onClick,
  disabled = false,
  danger = false,
}: {
  layout: Layout;
  phase: ToolPhase;
  label: string;
  working: string;
  done: string;
  onClick: () => void;
  disabled?: boolean;
  danger?: boolean;
}) {
  const text = phase === 'working' ? working : phase === 'done' ? done : phase === 'failed' ? 'Try again' : label;
  return (
    <button
      type="button"
      onClick={() => {
        if (phase !== 'done' && phase !== 'working') onClick();
      }}
      disabled={phase !== 'done' && (phase === 'working' || disabled)}
      aria-live="polite"
      className={`${ACCENT_BUTTON} flex items-center justify-center gap-1.5 disabled:cursor-not-allowed ${
        layout === 'sheet' ? `${BIG} flex-1 px-5` : 'h-[34px] min-w-[104px] px-5 !text-[13.5px]'
      }`}
      style={phase === 'done' ? SAVED_FILL : danger ? DANGER_FILL : ACCENT_FILL}
    >
      {phase === 'working' && <CircleNotch size={14} weight="bold" className="animate-spin" aria-hidden />}
      {phase === 'done' && <Check size={15} weight="bold" aria-hidden />}
      {text}
    </button>
  );
}

/** Discard, Back, Snooze: beside the main button, a step quieter. */
export function SecondaryButton({
  layout,
  onClick,
  disabled,
  children,
}: {
  layout: Layout;
  onClick: () => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={`${QUIET_PILL} ${layout === 'sheet' ? `${BIG} px-5` : 'h-[34px] px-4 text-[13.5px]'}`}
    >
      {children}
    </button>
  );
}

/** Why a write did not go through, in Rust's words. */
export function ProblemLine({ line }: { line: string }) {
  return (
    <div
      role="alert"
      className="sn-error-text flex items-start gap-2 rounded-lg bg-error/10 px-3 py-2 text-[12.5px] leading-snug ring-1 ring-inset ring-error/25"
    >
      <WarningCircle size={15} className="mt-px flex-shrink-0" aria-hidden />
      <span>{line}</span>
    </div>
  );
}

/** A row's own result under it: quiet when it went through, red when not. */
export function ResultLine({ line, failed }: { line: string | null; failed: boolean }) {
  if (!line) return null;
  return (
    <p role="status" className={`text-[11.5px] leading-snug ${failed ? 'sn-error-text' : 'text-textSecondary'}`}>
      {line}
    </p>
  );
}

/** A titled group of rows (Chat modes, Moderation, Your settings). In the
 *  window, a card that is the whole page drops its title: the page's own
 *  title already says it (`hideTitle`). */
export function GroupCard({
  title,
  layout,
  hideTitle = false,
  children,
}: {
  title: string;
  layout: Layout | 'menu';
  hideTitle?: boolean;
  children: ReactNode;
}) {
  const titleClass =
    layout === 'sheet'
      ? 'px-3 text-[12px] font-semibold uppercase tracking-wide text-textMuted'
      : layout === 'window'
        ? LABEL
        : `${LABEL} px-3`;
  return (
    <section aria-label={hideTitle ? title : undefined} className={layout === 'sheet' ? 'space-y-1.5' : 'space-y-1'}>
      {!hideTitle && <h3 className={titleClass}>{title}</h3>}
      <div
        className={
          layout === 'sheet'
            ? 'divide-y divide-borderSubtle overflow-hidden rounded-2xl bg-white/[0.05]'
            : layout === 'menu'
              ? // The Tools menu's rows sit straight on the flyout's frost, one
                // continuous glass surface: boxes, outlines and dividers here
                // break it into flat grey blocks.
                'flex flex-col'
              : 'divide-y divide-textPrimary/[0.06] overflow-hidden rounded-xl bg-surface/25 ring-1 ring-inset ring-textPrimary/[0.06]'
        }
      >
        {children}
      </div>
    </section>
  );
}

const ROW_PAD = { menu: 'min-h-[40px] px-3 py-2', panel: 'min-h-[40px] px-3 py-2', window: 'min-h-[46px] px-3.5 py-2.5', sheet: 'min-h-[48px] px-3 py-2' };

/**
 * One row of a card: icon, label, an optional quieter second line, what sits
 * at its end (a count, a switch, a value, its buttons), then its own result
 * line and any second line of buttons (`below`) under the words, so the
 * row's top line never moves when one appears. With `onClick` the whole row
 * opens something and ends in a chevron.
 */
export function MenuRow({
  layout,
  icon,
  label,
  labelFor,
  detail,
  wrapDetail = false,
  trailing,
  line,
  failed = false,
  below,
  onClick,
  danger = false,
}: {
  layout: Layout | 'menu';
  icon?: ReactNode;
  label: ReactNode;
  /** The switch this row's label names, so a press on the words flips it. */
  labelFor?: string;
  detail?: ReactNode;
  /** The second line wraps instead of cutting off (a confirm's reason). */
  wrapDetail?: boolean;
  trailing?: ReactNode;
  line?: string | null;
  failed?: boolean;
  below?: ReactNode;
  onClick?: () => void;
  danger?: boolean;
}) {
  const text = layout === 'sheet' ? 'text-[15px]' : 'text-[13.5px]';
  const ink = danger ? 'sn-error-text' : 'text-textPrimary';
  const words = (
    <span className="min-w-0 flex-1">
      {labelFor ? (
        <label htmlFor={labelFor} className={`block cursor-pointer truncate ${text} ${ink}`}>
          {label}
        </label>
      ) : (
        <span className={`block truncate ${text} ${ink}`}>{label}</span>
      )}
      {detail && <span className={`block text-[12px] text-textSecondary ${wrapDetail ? 'leading-snug' : 'truncate'}`}>{detail}</span>}
    </span>
  );
  const iconEl = icon && (
    <span className={`flex w-4 flex-shrink-0 items-center justify-center ${danger ? 'sn-error-text' : 'text-textSecondary'}`} aria-hidden>
      {icon}
    </span>
  );
  // Under the words: the icon's column (16 px) plus the gap (10 px).
  const indent = icon ? 'pl-[26px]' : '';
  const under = (line || below) && (
    <div className={`mt-1 space-y-1.5 ${indent}`}>
      {below && <div className="flex flex-wrap gap-1.5">{below}</div>}
      {line && <ResultLine line={line} failed={failed} />}
    </div>
  );
  if (onClick) {
    return (
      <button
        type="button"
        onClick={onClick}
        className={`group flex w-full items-center gap-2.5 text-left transition-colors hover:bg-textPrimary/[0.05] ${
          IS_MOBILE ? 'sn-touch active:opacity-70' : ''
        } ${layout === 'menu' ? 'rounded-lg' : ''} ${ROW_PAD[layout]}`}
      >
        {iconEl}
        {words}
        {trailing}
        <CaretRight size={13} weight="bold" className="flex-shrink-0 text-textSecondary/70 group-hover:text-textSecondary" aria-hidden />
      </button>
    );
  }
  return (
    <div className={ROW_PAD[layout]}>
      <div className="flex items-center gap-2.5">
        {iconEl}
        {words}
        {trailing && <span className="flex flex-shrink-0 items-center gap-1.5">{trailing}</span>}
      </div>
      {under}
    </div>
  );
}

/**
 * A small action inside a row (End, Lock, Turn on). `quiet` is the plain
 * glass; `danger` is red-tinted glass for one that can't be taken back
 * (Refund); `commit` is the filled red that says yes to a confirm in place.
 */
export function RowButton({
  layout,
  onClick,
  disabled,
  tone = 'quiet',
  autoFocus = false,
  children,
}: {
  layout: Layout | 'menu';
  onClick: () => void;
  disabled?: boolean;
  tone?: 'quiet' | 'danger' | 'commit';
  /** Takes focus when it appears (a confirm in place), so Esc and Enter answer it. */
  autoFocus?: boolean;
  children: ReactNode;
}) {
  const look =
    tone === 'commit'
      ? `${ACCENT_BUTTON} focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-error/50`
      : tone === 'danger'
        ? `${GLAZE_PILL} sn-error-text !bg-error/[0.12] hover:!bg-error/[0.2] ring-1 ring-inset ring-error/25`
        : GLAZE_PILL;
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      autoFocus={autoFocus}
      className={`${look} ${layout === 'sheet' ? 'h-9 px-3.5 !text-[14px]' : 'h-7 px-3 !text-[12px]'}`}
      style={tone === 'commit' ? DANGER_FILL : undefined}
    >
      {children}
    </button>
  );
}

/** A choice of a few: the picked one is tinted glass, never a flat fill. */
export function Segmented<T extends string | number>({
  layout,
  value,
  options,
  onChange,
  ariaLabel,
}: {
  layout: Layout;
  value: T;
  options: { value: T; label: string; icon?: ReactNode }[];
  onChange: (value: T) => void;
  ariaLabel: string;
}) {
  const at = Math.max(
    0,
    options.findIndex((o) => o.value === value),
  );
  // One stop for Tab, on the pick; the arrow keys move the pick.
  const onKey = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    const step = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0;
    if (!step || !options.length) return;
    e.preventDefault();
    const next = options[(at + step + options.length) % options.length];
    onChange(next.value);
    const el = e.currentTarget.querySelector<HTMLButtonElement>(`[data-seg="${String(next.value)}"]`);
    requestAnimationFrame(() => el?.focus());
  };
  return (
    <div
      role="radiogroup"
      aria-label={ariaLabel}
      onKeyDown={onKey}
      className="flex gap-1 rounded-full bg-surface/30 p-[3px] ring-1 ring-inset ring-textPrimary/[0.06]"
    >
      {options.map((o, i) => {
        const on = i === at;
        return (
          <button
            key={String(o.value)}
            data-seg={String(o.value)}
            type="button"
            role="radio"
            aria-checked={on}
            tabIndex={on ? 0 : -1}
            onClick={() => onChange(o.value)}
            className={`flex min-w-0 flex-1 items-center justify-center gap-1.5 rounded-full font-semibold transition-colors ${
              layout === 'sheet' ? 'h-9 text-[14px]' : 'h-7 text-[12.5px]'
            } ${on ? 'glaze-inset bg-textPrimary/[0.12] text-textPrimary' : 'text-textSecondary hover:text-textPrimary'}`}
          >
            {o.icon}
            <span className="truncate">{o.label}</span>
          </button>
        );
      })}
    </div>
  );
}

/** A length on a row (Followers-only 10 min, Slow mode 30s): opens a list. */
export function ValuePicker({
  value,
  options,
  onChange,
  ariaLabel,
  shown,
}: {
  value: number;
  options: { value: number; label: string }[];
  onChange: (value: number) => void;
  ariaLabel: string;
  /** Rust's words for a length not in the list (set on twitch.tv). */
  shown?: string | null;
}) {
  return (
    <Dropdown
      bare
      align="right"
      value={value}
      options={options}
      onChange={onChange}
      ariaLabel={ariaLabel}
      placeholder={shown ?? undefined}
      className="h-6 rounded-full bg-textPrimary/[0.06] px-2.5 text-[12.5px] font-semibold text-textSecondary hover:text-textPrimary"
    />
  );
}

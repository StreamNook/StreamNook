// The building blocks the automations page shares: a group (its heading over a
// card whose rows are split by hairlines), a labeled field with its error under
// it, a switch line, a number field, a names field, a labeled disclosure for
// the less common choices, the ⋯ and Add menus, and the small marks (run, the
// Live or Test tag, a note, a warning, a face).
import { useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion } from 'framer-motion';
import { ChevronDown, CircleAlert, Ellipsis, Info, TriangleAlert, type LucideIcon } from 'lucide-react';
import { Toggle } from '../../ui/Toggle';
import { Tooltip } from '../../ui/Tooltip';
import { IS_MOBILE } from '../../../utils/platform';
import { splitNames } from '../../../utils/automations';
import { useOutsidePress } from '../../../hooks/useOutsidePress';
import { MobileSheet } from '../../../mobile/ui/MobileSheet';
import { GroupLabel } from '../../../mobile/ui/GroupedList';
import type { AutomationRunState } from '../../../types';
import { CARD } from './surfaces';

export { CARD, RAISED_CARD } from './surfaces';

export const FIELD = 'glass-input px-2.5 py-1.5 text-sm text-textPrimary';
export const INPUT = `w-full ${FIELD}`;
export const SELECT = 'glass-input max-w-full px-2 py-1.5 text-sm text-textPrimary';
/** A helper line under a name: what it does. */
export const HINT = 'text-[12px] leading-relaxed text-textSecondary';
/** Facts about a row (counts, times): a step quieter than a helper. */
export const META = 'text-[11.5px] leading-relaxed text-textMuted';
export const QUIET_BUTTON =
  'inline-flex items-center gap-1.5 rounded-lg bg-textPrimary/[0.06] px-2.5 py-1.5 text-[13px] text-textSecondary transition-colors hover:bg-textPrimary/[0.10] hover:text-textPrimary disabled:opacity-50';
/** A button with no surface of its own, for an accessory beside a heading. */
export const GHOST_BUTTON =
  'inline-flex items-center gap-1.5 rounded-lg px-2 py-1.5 text-[13px] text-textSecondary transition-colors hover:bg-textPrimary/[0.06] hover:text-textPrimary disabled:opacity-50';
/** Warning text darkened enough to read on a light theme. */
export const LIGHT_WARN =
  '[html[data-theme-tone=light]_&]:text-[color-mix(in_srgb,var(--color-warning)_52%,black)]';
/** Success text darkened enough to read on a light theme. */
export const LIGHT_OK =
  '[html[data-theme-tone=light]_&]:text-[color-mix(in_srgb,var(--color-success)_62%,black)]';
export const PRIMARY_BUTTON =
  'glass-button inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-[13px] font-medium text-textPrimary disabled:opacity-50';
export const DANGER_BUTTON =
  'inline-flex items-center gap-1.5 rounded-lg border border-error/40 bg-error/10 px-3 py-1.5 text-[13px] font-medium text-textPrimary transition-colors hover:bg-error/20';
/** The ⋯ on a row: quieter than the row's own words. */
const DOTS_BUTTON = `inline-flex shrink-0 items-center justify-center rounded-md text-textMuted transition-colors hover:bg-textPrimary/[0.06] hover:text-textPrimary ${
  IS_MOBILE ? 'h-9 w-9' : 'h-7 w-7'
}`;

/** A group heading, in the settings section style of each platform. */
export function GroupHeading({ title, aside }: { title: string; aside?: ReactNode }) {
  return (
    <div className={`flex items-center justify-between gap-3 ${IS_MOBILE ? 'px-4 pb-2' : 'px-1 pb-2.5'}`}>
      {IS_MOBILE ? (
        <GroupLabel as="h3">{title}</GroupLabel>
      ) : (
        <h3 className="text-[12px] font-semibold uppercase tracking-[0.14em] text-textPrimary">{title}</h3>
      )}
      {aside}
    </div>
  );
}

/** One part of a rule (When, Only if, Do, Send as): a heading over a card.
 *  `rows` splits the card's children into rows with a hairline between each,
 *  the way a settings section does; without it the children sit on the card
 *  as they are (the Do steps, each on its own surface). */
export function Block({
  title,
  aside,
  rows = true,
  children,
}: {
  title: string;
  aside?: ReactNode;
  rows?: boolean;
  children: ReactNode;
}) {
  return (
    <section>
      <GroupHeading title={title} aside={aside} />
      <div className={`${CARD} ${rows ? 'automation-rows' : 'space-y-2.5 p-2.5'}`}>{children}</div>
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
      <div className="text-[13px] font-medium text-textPrimary">{label}</div>
      {children}
      {hint && !error && <p className={HINT}>{hint}</p>}
      {error && <ErrorLine>{error}</ErrorLine>}
    </div>
  );
}

/** A row with its name (and a line under it) on the left and its control on
 *  the right edge, like a switch line; on a phone the control goes under the name. */
export function RowField({
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
      <div className={IS_MOBILE ? 'space-y-2' : 'flex items-center justify-between gap-4'}>
        <div className="min-w-0">
          <div className="text-[13px] font-medium text-textPrimary">{label}</div>
          {hint && !error && <p className={`mt-0.5 ${HINT}`}>{hint}</p>}
        </div>
        <div className={IS_MOBILE ? '' : 'flex max-w-[62%] shrink-0 justify-end'}>{children}</div>
      </div>
      {error && <ErrorLine>{error}</ErrorLine>}
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
        <span className="block text-[13px] font-medium text-textPrimary">{label}</span>
        {description && <span className={`mt-0.5 block ${HINT}`}>{description}</span>}
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

/** The less common choices, behind their name and a line saying what is set.
 *  Open, they sit on their own surface inside the row. */
export function Disclosure({
  label,
  summary,
  defaultOpen,
  children,
}: {
  label: string;
  summary?: string;
  defaultOpen?: boolean;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(defaultOpen ?? false);
  return (
    <div>
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className={`group flex w-full items-center gap-3 text-left ${IS_MOBILE ? 'min-h-[44px]' : 'min-h-[34px]'}`}
      >
        <span className="min-w-0 flex-1">
          <span className="block text-[13px] font-medium text-textSecondary transition-colors group-hover:text-textPrimary">
            {label}
          </span>
          {summary && <span className={`mt-0.5 block truncate ${META}`}>{summary}</span>}
        </span>
        <ChevronDown size={15} className={`shrink-0 text-textMuted transition-transform ${open ? 'rotate-180' : ''}`} />
      </button>
      {open && <div className="mt-2.5 space-y-3.5 rounded-lg bg-textPrimary/[0.045] p-3.5">{children}</div>}
    </div>
  );
}

/** A note: the info mark in the accent, then the words. */
export function Note({ children, className = '' }: { children: ReactNode; className?: string }) {
  return (
    <div className={`flex items-start gap-2 ${HINT} ${className}`}>
      <Info size={14} className="mt-[3px] shrink-0 text-accent-on-surface" aria-hidden />
      <div className="min-w-0">{children}</div>
    </div>
  );
}

/** Something holding a rule back: the warning mark and color. */
export function WarnLine({ children, className = '' }: { children: ReactNode; className?: string }) {
  return (
    <p className={`flex items-start gap-1.5 text-[12px] leading-relaxed text-warning ${LIGHT_WARN} ${className}`}>
      <TriangleAlert size={13} className="mt-[3px] shrink-0" aria-hidden />
      <span className="min-w-0">{children}</span>
    </p>
  );
}

/** Something that failed: the error mark and color. */
export function ErrorLine({ children, className = '' }: { children: ReactNode; className?: string }) {
  return (
    <p role="alert" className={`flex items-start gap-1.5 text-[12px] leading-relaxed text-error ${className}`}>
      <CircleAlert size={13} className="mt-[3px] shrink-0" aria-hidden />
      <span className="min-w-0">{children}</span>
    </p>
  );
}

/** On a green dot, Test an amber one, Off gray; each with a soft halo. */
export function RunDot({ run }: { run: AutomationRunState }) {
  const look =
    run === 'on'
      ? 'bg-success shadow-[0_0_0_3px_color-mix(in_srgb,var(--color-success)_24%,transparent)]'
      : run === 'test'
        ? 'bg-warning shadow-[0_0_0_3px_color-mix(in_srgb,var(--color-warning)_24%,transparent)]'
        : 'bg-textMuted/60 shadow-[0_0_0_3px_color-mix(in_srgb,var(--color-text-muted)_16%,transparent)]';
  return <span aria-hidden className={`inline-block h-2 w-2 shrink-0 rounded-full ${look}`} />;
}

/** A rule's Live or Test, as a small tag beside its name. */
export function RuleTag({ live }: { live: boolean }) {
  return (
    <span
      className={`inline-flex shrink-0 items-center rounded-md px-1.5 py-px text-[11px] font-semibold leading-[18px] ${
        live ? `bg-success/15 text-success ${LIGHT_OK}` : `bg-warning/15 text-warning ${LIGHT_WARN}`
      }`}
    >
      {live ? 'Live' : 'Test'}
    </span>
  );
}

/** A quiet tag beside a name ("Now a rule"). */
export function QuietTag({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex shrink-0 items-center rounded-md bg-textPrimary/[0.07] px-1.5 py-px text-[11px] font-semibold leading-[18px] text-textSecondary">
      {children}
    </span>
  );
}

/** A count beside a group's heading. */
export function CountChip({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex items-center rounded-full bg-textPrimary/[0.07] px-2 py-px text-[11px] font-semibold tabular-nums text-textSecondary">
      {children}
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
        className="flex shrink-0 items-center justify-center rounded-full bg-surface font-semibold text-textSecondary"
        style={{ width: size, height: size, fontSize: Math.round(size * 0.56) }}
      >
        {label.charAt(0).toUpperCase()}
      </span>
    </Tooltip>
  );
}

export interface MenuItem {
  id: string;
  label: string;
  /** One line under the name, in the Add menu. */
  help?: string;
  icon?: LucideIcon;
  danger?: boolean;
  disabled?: boolean;
  /** The group it sits in; items of one group stay together, in order. */
  section?: string;
  onSelect: () => void;
}

/** Items in their groups, in the order the groups first appear. */
function grouped(items: readonly MenuItem[]): { section: string | undefined; items: MenuItem[] }[] {
  const out: { section: string | undefined; items: MenuItem[] }[] = [];
  for (const item of items) {
    const last = out[out.length - 1];
    if (last && last.section === item.section) last.items.push(item);
    else out.push({ section: item.section, items: [item] });
  }
  return out;
}

function MenuRows({ items, onPick }: { items: readonly MenuItem[]; onPick: (item: MenuItem) => void }) {
  return (
    <>
      {grouped(items).map((g, gi) => (
        <div key={g.section ?? `g${gi}`} className={gi > 0 && !g.section ? 'mt-1 border-t border-borderSubtle/50 pt-1' : ''}>
          {g.section && (
            <div className={`px-2.5 pb-1 text-[11.5px] font-semibold text-textMuted ${gi > 0 ? 'pt-2.5' : 'pt-1.5'}`}>{g.section}</div>
          )}
          {g.items.map((item) => {
            const Icon = item.icon;
            return (
              <button
                key={item.id}
                type="button"
                role="menuitem"
                disabled={item.disabled}
                onClick={() => onPick(item)}
                className={`flex w-full items-start gap-2.5 rounded-lg px-2.5 text-left transition-colors hover:bg-textPrimary/[0.07] disabled:pointer-events-none disabled:opacity-40 ${
                  IS_MOBILE ? 'min-h-[46px] py-2.5' : 'py-2'
                }`}
              >
                {Icon && (
                  <Icon
                    size={15}
                    aria-hidden
                    className={`mt-[2px] shrink-0 ${item.danger ? 'text-error' : 'text-accent-on-surface'}`}
                  />
                )}
                <span className="min-w-0">
                  <span className={`block text-[13.5px] ${item.danger ? 'text-error' : 'text-textPrimary'}`}>{item.label}</span>
                  {item.help && <span className="mt-px block text-[12px] leading-snug text-textSecondary">{item.help}</span>}
                </span>
              </button>
            );
          })}
        </div>
      ))}
    </>
  );
}

/** A button that opens a short menu: a flyout under it on the desktop, the
 *  platform sheet on a phone. The default face is the ⋯ of a row. */
export function MenuButton({
  label,
  items,
  children,
  className,
  align = 'right',
}: {
  /** What the button does, for a screen reader and the phone sheet's title. */
  label: string;
  items: readonly MenuItem[];
  /** The button's face; the ⋯ when left out. */
  children?: ReactNode;
  className?: string;
  align?: 'left' | 'right';
}) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [style, setStyle] = useState<CSSProperties>({});
  const width = items.some((i) => i.help) ? 280 : 200;

  const place = () => {
    const el = triggerRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const raw = align === 'right' ? r.right - width : r.left;
    setStyle({
      position: 'fixed',
      top: Math.round(r.bottom + 6),
      left: Math.round(Math.max(8, Math.min(raw, window.innerWidth - width - 8))),
      width,
      // Lifted over the player's full screen (globals.css --sn-z-over-player-lift).
      zIndex: 'calc(var(--sn-z-over-player-lift, 0) + 9999)',
    });
  };

  useLayoutEffect(() => {
    if (open && !IS_MOBILE) place();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  useEffect(() => {
    if (!open || IS_MOBILE) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      e.stopPropagation();
      setOpen(false);
    };
    const onMove = (e: Event) => {
      // A scroll inside the menu's own list leaves it where it is; a resize's target is the window.
      if (!(e.target instanceof Node) || !menuRef.current?.contains(e.target)) place();
    };
    document.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', onMove);
    window.addEventListener('scroll', onMove, true);
    return () => {
      document.removeEventListener('keydown', onKey, true);
      window.removeEventListener('resize', onMove);
      window.removeEventListener('scroll', onMove, true);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const pressedInside = useOutsidePress(open && !IS_MOBILE, [triggerRef, menuRef], () => setOpen(false));

  const pick = (item: MenuItem) => {
    setOpen(false);
    item.onSelect();
  };

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={children ? undefined : label}
        onPointerDownCapture={pressedInside}
        onClick={() => setOpen((v) => !v)}
        className={className ?? DOTS_BUTTON}
      >
        {children ?? <Ellipsis size={16} />}
      </button>
      {IS_MOBILE ? (
        <MobileSheet open={open} onClose={() => setOpen(false)} title={label}>
          <div role="menu" className="pb-2">
            <MenuRows items={items} onPick={pick} />
          </div>
        </MobileSheet>
      ) : (
        createPortal(
          <AnimatePresence>
            {open && (
              <motion.div
                ref={menuRef}
                role="menu"
                aria-label={label}
                data-sn-owns-keys=""
                style={style}
                onPointerDownCapture={pressedInside}
                initial={{ opacity: 0, y: -4, scale: 0.97 }}
                animate={{ opacity: 1, y: 0, scale: 1 }}
                exit={{ opacity: 0, y: -4, scale: 0.97 }}
                transition={{ duration: 0.14, ease: 'easeOut' }}
                className="glass-flyout max-h-[70vh] overflow-y-auto rounded-xl p-1.5 custom-scrollbar"
              >
                <MenuRows items={items} onPick={pick} />
              </motion.div>
            )}
          </AnimatePresence>,
          document.body,
        )
      )}
    </>
  );
}

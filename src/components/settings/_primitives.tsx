import { createContext, createElement, useContext, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { CircleHelp, RotateCcw, type LucideIcon } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import { IS_MOBILE } from '../../utils/platform';
import { sectionIdFromLabel } from './sectionId';
import {
  GROUP_ICON,
  GROUP_ICON_STROKE,
  GroupCard,
  GroupFooter,
  GroupIcon,
  GroupLabel,
} from '../../mobile/ui/GroupedList';
import { useGroupRowFrame } from '../../mobile/ui/groupSurface';
import { phoneSettingGlyph } from './phoneGlyphs';

/** The section a phone row sits in, so a short row name that repeats across
 *  sections ("Style", "Spacing") finds its own glyph. */
const SectionNameContext = createContext<string | undefined>(undefined);

interface SettingsSectionProps {
  label: string;
  description?: string;
  children: ReactNode;
  id?: string;
  bare?: boolean;
  /** A brand mark leading the heading, for a section about one platform. */
  logo?: ReactNode;
  /** The words drawn when the logo already names the brand ("Chat" beside
   *  YouTube's mark). `label` stays the section's name for anchors and search. */
  shownLabel?: string;
  /** The phone's small action on the heading's line (Custom Commands' Show
   *  reference), as the iPhone places it. The desktop does not draw it. */
  accessory?: ReactNode;
}

export { sectionIdFromLabel };

/** The phone's section, in the chat menu's grouped style: the name as a quiet
 *  label over the icon column, the rows on a solid card, and the one-line
 *  description as the quiet footer under it. */
const PhoneSettingsSection = ({
  label,
  description,
  children,
  id,
  bare = false,
  logo,
  shownLabel,
  accessory,
}: SettingsSectionProps) => (
  <section id={id ?? sectionIdFromLabel(label)} data-settings-section={label}>
    {/* An accessory sits on the heading's line, centered on it. */}
    <div className={accessory ? 'flex items-center justify-between gap-3 pr-2' : undefined}>
      <GroupLabel
        as="h3"
        className={`px-4 ${accessory ? '' : 'pb-2'} ${logo ? 'flex items-center gap-1.5' : ''}`}
        aria-label={shownLabel ? label : undefined}
      >
        {logo}
        {shownLabel ?? label}
      </GroupLabel>
      {accessory}
    </div>
    {accessory && <div className="h-2" />}
    <SectionNameContext.Provider value={label}>
      {bare ? <div className="space-y-3">{children}</div> : <GroupCard>{children}</GroupCard>}
    </SectionNameContext.Provider>
    {description && <GroupFooter className="px-4 pt-2">{description}</GroupFooter>}
  </section>
);

export const SettingsSection = (props: SettingsSectionProps) =>
  IS_MOBILE ? <PhoneSettingsSection {...props} /> : <DesktopSettingsSection {...props} />;

const DesktopSettingsSection = ({
  label,
  description,
  children,
  id,
  bare = false,
  logo,
  shownLabel,
}: SettingsSectionProps) => (
  <section id={id ?? sectionIdFromLabel(label)} data-settings-section={label}>
    <div className="px-1 pb-2.5">
      <h3
        className={`text-[12px] font-semibold uppercase tracking-[0.14em] text-textPrimary ${logo ? 'flex items-center gap-1.5' : ''}`}
        aria-label={shownLabel ? label : undefined}
      >
        {logo}
        {shownLabel ?? label}
      </h3>
      {description && (
        <p className="mt-1 text-[12px] leading-relaxed text-textMuted">
          {description}
        </p>
      )}
    </div>
    {bare ? (
      <div className="space-y-3">{children}</div>
    ) : (
      <div className="settings-card px-4">{children}</div>
    )}
  </section>
);

/** The arrow beside a changed setting's title that puts its default back.
 *  Shared by SettingsRow and the nested SubControl lines. */
export const ResetArrow = ({ onReset }: { onReset: () => void }) => (
  <Tooltip content="Reset to default">
    <button
      type="button"
      onClick={onReset}
      aria-label="Reset to default"
      // Touch has no hover tooltip and needs a bigger target than the glyph.
      className={`inline-flex align-middle text-textMuted hover:text-textPrimary transition-colors ${IS_MOBILE ? '-my-1.5 p-1.5' : 'ml-1.5'}`}
    >
      {/* A step up on the phone, beside its 15px row names. */}
      <RotateCcw size={IS_MOBILE ? 13 : 11} />
    </button>
  </Tooltip>
);

interface SettingsRowProps {
  title: ReactNode;
  /** A brand mark leading the title, for a row about one service. Kept apart
   *  from `title` so the title stays the plain string search lands on. */
  logo?: ReactNode;
  /** Small inline element after the title (e.g. a source-scope indicator). */
  titleBadge?: ReactNode;
  description?: string;
  /** The longer explanation, behind a small help glyph beside the title. Keeps
   *  the row to one short line while the detail stays one hover away, so a
   *  tab full of rows reads as a list of settings rather than a wall of prose. */
  help?: ReactNode;
  control?: ReactNode;
  children?: ReactNode;
  disabled?: boolean;
  /** Restores this row's setting to its default. Pass it ONLY while the setting
   *  actually differs from the default — the affordance's presence is the signal
   *  that something was changed here, so an always-present one says nothing. */
  onReset?: () => void;
  /** The phone's leading glyph, for a row whose title the glyph table cannot
   *  name (a date, a value built at runtime). Other rows take theirs from the
   *  table by section and title. The desktop draws no glyph. */
  icon?: LucideIcon;
}

export const SettingsRow = (props: SettingsRowProps) =>
  IS_MOBILE ? <PhoneSettingsRow {...props} /> : <DesktopSettingsRow {...props} />;

/** The phone's row, in the chat menu's grouped style: a leading glyph, the
 *  name at 15px with its control on the same line, the description under the
 *  name, and the nested settings under that, all in the column after the
 *  glyph so the hairlines start where the text does. */
const PhoneSettingsRow = ({
  title,
  logo,
  titleBadge,
  description,
  help,
  control,
  children,
  disabled = false,
  onReset,
  icon,
}: SettingsRowProps) => {
  const section = useContext(SectionNameContext);
  const frame = useGroupRowFrame();
  // Touch has no hover, so a tap opens the explanation in place, under the
  // description, and a second tap closes it.
  const [helpOpen, setHelpOpen] = useState(false);
  const glyph = icon ?? phoneSettingGlyph(section, typeof title === 'string' ? title : '');
  // A control wider than about half the text column (a pair of buttons) would
  // crush the name and its description into a sliver, so it goes under them,
  // on the right. Measured before paint and again whenever the row resizes.
  const gridRef = useRef<HTMLDivElement>(null);
  const controlRef = useRef<HTMLSpanElement>(null);
  const [controlUnder, setControlUnder] = useState(false);
  const hasControl = control != null && control !== false;
  useLayoutEffect(() => {
    const grid = gridRef.current;
    if (!hasControl || !grid) return;
    const fit = () => {
      const part = controlRef.current;
      if (part) setControlUnder(part.scrollWidth > (grid.clientWidth - 36) * 0.45);
    };
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(grid);
    return () => ro.disconnect();
  }, [hasControl]);
  const controlBeside = hasControl && !controlUnder;
  return (
    <div
      className={`settings-row ${frame} py-2.5 ${disabled ? 'opacity-50 pointer-events-none' : ''}`}
      // What a search result scrolls to. A title that carries a live value
      // ("Text size: 14px") still starts with the name the search index uses.
      data-setting-row={typeof title === 'string' ? title : undefined}
    >
      {/* One grid so the glyph, the name and the control share the first
          line's midline even when the control is taller than the name, and
          the lines under the name keep to its column. */}
      <div
        ref={gridRef}
        className={`grid ${controlBeside ? 'grid-cols-[1.5rem_minmax(0,1fr)_auto]' : 'grid-cols-[1.5rem_minmax(0,1fr)]'} items-center gap-x-3`}
      >
        <GroupIcon className="col-start-1 row-start-1 text-textSecondary">
          {/* An element, not a JSX tag: the glyph is looked up per row, and a
              looked-up tag reads to the compiler as a component made in render. */}
          {createElement(glyph, { size: GROUP_ICON, strokeWidth: GROUP_ICON_STROKE })}
        </GroupIcon>
        <div className="col-start-2 row-start-1 min-w-0 text-[14px] leading-6 text-textPrimary">
          {/* Dropped 2px under the baseline so a 14px mark centers on the
              title's capitals rather than riding above them. */}
          {logo && (
            <span className="mr-1.5 inline-flex" style={{ verticalAlign: '-2px' }} aria-hidden>
              {logo}
            </span>
          )}
          {title}
          {titleBadge && <span className="ml-1.5 align-middle">{titleBadge}</span>}
          {help && (
            <button
              type="button"
              aria-expanded={helpOpen}
              aria-label="More about this setting"
              onClick={() => setHelpOpen((v) => !v)}
              className={`ml-1.5 inline-flex align-middle transition-colors ${helpOpen ? 'text-accent' : 'text-textMuted'}`}
            >
              <CircleHelp size={14} />
            </button>
          )}
          {onReset && <ResetArrow onReset={onReset} />}
        </div>
        {controlBeside && (
          <div className="col-start-3 row-start-1 shrink-0">
            <span ref={controlRef} className="inline-flex">{control}</span>
          </div>
        )}
        {description && (
          <p className="col-start-2 mt-0.5 text-[12px] leading-4 text-textSecondary">{description}</p>
        )}
        {help && helpOpen && <p className="col-start-2 mt-1 text-[12px] leading-4 text-textMuted">{help}</p>}
        {hasControl && controlUnder && (
          <div className="col-start-2 mt-2 flex min-w-0 justify-end">
            <span ref={controlRef} className="inline-flex max-w-full flex-wrap justify-end gap-2">{control}</span>
          </div>
        )}
        {children && <div className={`col-start-2 mt-3 ${controlBeside ? 'col-span-2' : ''}`}>{children}</div>}
      </div>
    </div>
  );
};

const DesktopSettingsRow = ({
  title,
  logo,
  titleBadge,
  description,
  help,
  control,
  children,
  disabled = false,
  onReset,
}: SettingsRowProps) => {
  // Touch has no hover, so the shared Tooltip renders nothing on the phone
  // and the help glyph was decoration. A tap opens the explanation in place,
  // under the description, and a second tap closes it.
  const [helpOpen, setHelpOpen] = useState(false);
  return (
  <div
    className={`settings-row -mx-4 px-4 py-3 ${
      disabled ? 'opacity-50 pointer-events-none' : ''
    }`}
    // What a search result scrolls to. A title that carries a live value
    // ("Text size: 14px") still starts with the name the search index uses.
    data-setting-row={typeof title === 'string' ? title : undefined}
  >
    <div className="flex items-start justify-between gap-4">
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium text-textPrimary">
          {/* Dropped 2px under the baseline so a 14px mark centers on the
              title's capitals rather than riding above them. */}
          {logo && (
            <span className="mr-1.5 inline-flex" style={{ verticalAlign: '-2px' }} aria-hidden>
              {logo}
            </span>
          )}
          {title}
          {titleBadge && <span className="ml-1.5 align-middle">{titleBadge}</span>}
          {help && IS_MOBILE ? (
            <button
              type="button"
              aria-expanded={helpOpen}
              aria-label="More about this setting"
              onClick={() => setHelpOpen((v) => !v)}
              className={`ml-1.5 inline-flex align-middle transition-colors ${helpOpen ? 'text-accent' : 'text-textMuted'}`}
            >
              <CircleHelp size={12} />
            </button>
          ) : help ? (
            <Tooltip content={<span className="block max-w-[34ch] text-left leading-relaxed">{help}</span>}>
              <span
                tabIndex={0}
                aria-label="More about this setting"
                className="ml-1.5 inline-flex align-middle text-textMuted hover:text-textSecondary transition-colors cursor-help"
              >
                <CircleHelp size={12} />
              </span>
            </Tooltip>
          ) : null}
          {onReset && <ResetArrow onReset={onReset} />}
        </div>
        {description && (
          <p className="mt-0.5 text-[12px] leading-relaxed text-textSecondary">
            {description}
          </p>
        )}
      </div>
      {control && <div className="flex-shrink-0">{control}</div>}
    </div>
    {help && IS_MOBILE && helpOpen && (
      <p className="mt-1 text-[12px] leading-relaxed text-textMuted">{help}</p>
    )}
    {children && <div className="mt-3">{children}</div>}
  </div>
  );
};

/** Visually nests the rows that exist only to configure the row above them
 *  (a toggle's color/animation/etc.): a thin left rule + inset, so they read as
 *  that setting's sub-settings instead of more top-level rows. The pl-4 exactly
 *  absorbs each row's -mx-4 bleed, so row separators start at the rule. */
export const SettingsSubGroup = ({ children }: { children: ReactNode }) => (
  // The marker lets a search result for a nested row find it: the group sits
  // after its parent row, not inside it.
  <div className="ml-1 border-l border-borderSubtle pl-4" data-setting-subgroup="">{children}</div>
);

interface SegmentedOption<T extends string> {
  value: T;
  label: string;
}

interface SegmentedSelectProps<T extends string> {
  value: T;
  options: SegmentedOption<T>[];
  onChange: (value: T) => void;
  /** Stretch to fill the row. Off by default so the control sizes to its labels
   *  instead of spanning the whole width as detached full-width buttons. */
  fullWidth?: boolean;
  /** Called with the option under the pointer (or keyboard focus), and null
   *  when it leaves, so a preview beside the control can show an option
   *  before it is picked. */
  onPreview?: (value: T | null) => void;
}

// A unified segmented control: one recessed track with the segments packed
// inside it and a single raised "thumb" that slides between them on change.
// The thumb is measured off the active segment, so it works with the
// variable-width labels of the default (content-sized) layout.
export const SegmentedSelect = <T extends string>({
  value,
  options,
  onChange,
  fullWidth = false,
  onPreview,
}: SegmentedSelectProps<T>) => {
  const trackRef = useRef<HTMLDivElement>(null);
  const btnRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  const measuredOnce = useRef(false);
  const [thumb, setThumb] = useState<{ left: number; width: number; ready: boolean; animate: boolean }>({
    left: 0,
    width: 0,
    ready: false,
    animate: false,
  });

  // Position the thumb over the active segment; re-measure on selection change,
  // option change, and any reflow (font load, container resize). The very first
  // placement is instant so it never slides in from the corner on mount.
  useLayoutEffect(() => {
    const measure = () => {
      const el = btnRefs.current[value];
      if (!el) return;
      const animate = measuredOnce.current;
      measuredOnce.current = true;
      setThumb({ left: el.offsetLeft, width: el.offsetWidth, ready: true, animate });
    };
    measure();
    const ro = new ResizeObserver(measure);
    if (trackRef.current) ro.observe(trackRef.current);
    return () => ro.disconnect();
  }, [value, options]);

  const reduceMotion =
    typeof window !== 'undefined' &&
    window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

  return (
    <div
      ref={trackRef}
      onMouseLeave={onPreview ? () => onPreview(null) : undefined}
      onBlur={onPreview ? () => onPreview(null) : undefined}
      className={`${fullWidth ? 'flex w-full' : 'inline-flex'} relative gap-0.5 p-0.5 rounded-lg`}
      style={{
        background: 'rgba(151,177,185,0.06)',
        boxShadow: 'inset 0 1px 2px rgba(0,0,0,0.32), inset 0 0 0 1px rgba(151,177,185,0.12)',
      }}
    >
      {/* The sliding thumb — one shared element that animates between segments. */}
      <div
        aria-hidden
        style={{
          position: 'absolute',
          top: 2,
          bottom: 2,
          left: thumb.left,
          width: thumb.width,
          borderRadius: 6,
          background: 'rgba(151,177,185,0.15)',
          boxShadow: '0 1px 1px rgba(0,0,0,0.22), inset 0 0 0 1px rgba(255,255,255,0.06)',
          opacity: thumb.ready ? 1 : 0,
          transition: thumb.animate && !reduceMotion
            ? 'left 280ms cubic-bezier(0.34,1.4,0.5,1), width 280ms cubic-bezier(0.34,1.4,0.5,1), opacity 140ms ease'
            : 'opacity 140ms ease',
          pointerEvents: 'none',
        }}
      />
      {options.map((opt) => {
        const isActive = value === opt.value;
        return (
          <button
            key={opt.value}
            ref={(el) => { btnRefs.current[opt.value] = el; }}
            onClick={() => onChange(opt.value)}
            onMouseEnter={onPreview ? () => onPreview(opt.value) : undefined}
            onFocus={onPreview ? () => onPreview(opt.value) : undefined}
            className={`${fullWidth ? 'flex-1 px-2' : 'px-3'} relative z-[1] whitespace-nowrap py-1 text-[13px] font-medium rounded-md transition-colors ${
              isActive ? 'text-textPrimary' : 'text-textMuted hover:text-textSecondary'
            }`}
          >
            {opt.label}
          </button>
        );
      })}
    </div>
  );
};

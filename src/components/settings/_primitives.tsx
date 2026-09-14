import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { CircleHelp, RotateCcw } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';

interface SettingsSectionProps {
  label: string;
  description?: string;
  children: ReactNode;
  id?: string;
  bare?: boolean;
}

/** Deterministic id from a section label, so the "On this page" strip can
 *  target sections that never declared one. Explicit ids still win (the
 *  search index deep-links to those). */
export const sectionIdFromLabel = (label: string): string =>
  `settings-section-${label.toLowerCase().replace(/&/g, 'and').replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '')}`;

export const SettingsSection = ({
  label,
  description,
  children,
  id,
  bare = false,
}: SettingsSectionProps) => (
  <section id={id ?? sectionIdFromLabel(label)} data-settings-section={label}>
    <div className="px-1 pb-2.5">
      <h3 className="text-[12px] font-semibold uppercase tracking-[0.14em] text-textPrimary">
        {label}
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

interface SettingsRowProps {
  title: ReactNode;
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
}

export const SettingsRow = ({
  title,
  titleBadge,
  description,
  help,
  control,
  children,
  disabled = false,
  onReset,
}: SettingsRowProps) => (
  <div
    className={`settings-row -mx-4 px-4 py-3 ${
      disabled ? 'opacity-50 pointer-events-none' : ''
    }`}
  >
    <div className="flex items-start justify-between gap-4">
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium text-textPrimary">
          {title}
          {titleBadge && <span className="ml-1.5 align-middle">{titleBadge}</span>}
          {help && (
            <Tooltip content={<span className="block max-w-[34ch] text-left leading-relaxed">{help}</span>}>
              <span
                tabIndex={0}
                aria-label="More about this setting"
                className="ml-1.5 inline-flex align-middle text-textMuted hover:text-textSecondary transition-colors cursor-help"
              >
                <CircleHelp size={12} />
              </span>
            </Tooltip>
          )}
          {onReset && (
            <Tooltip content="Reset to default">
              <button
                type="button"
                onClick={onReset}
                aria-label="Reset to default"
                className="ml-1.5 inline-flex align-middle text-textMuted hover:text-textPrimary transition-colors"
              >
                <RotateCcw size={11} />
              </button>
            </Tooltip>
          )}
        </div>
        {description && (
          <p className="mt-0.5 text-[12px] leading-relaxed text-textSecondary">
            {description}
          </p>
        )}
      </div>
      {control && <div className="flex-shrink-0">{control}</div>}
    </div>
    {children && <div className="mt-3">{children}</div>}
  </div>
);

/** Visually nests the rows that exist only to configure the row above them
 *  (a toggle's color/animation/etc.): a thin left rule + inset, so they read as
 *  that setting's sub-settings instead of more top-level rows. The pl-4 exactly
 *  absorbs each row's -mx-4 bleed, so row separators start at the rule. */
export const SettingsSubGroup = ({ children }: { children: ReactNode }) => (
  <div className="ml-1 border-l border-borderSubtle pl-4">{children}</div>
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
            className={`${fullWidth ? 'flex-1' : ''} relative z-[1] px-3 py-1 text-[13px] font-medium rounded-md transition-colors ${
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

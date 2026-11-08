// The MultiChat title bar's View menu: how the chats are shown, in one place.
//
// It replaces a row of seven unlabeled toggles (@, two circles, four column
// glyphs, a split glyph) that a new window's 402 px could not hold: the row
// pushed the window's own minimize / maximize / close buttons out of the
// window. One button with a named, described menu fits every width and says
// what each choice does, the same way MultiNook's Layout menu does.
//
// The choices are one radio group: a layout (Tabs, Columns, Custom splits) or
// a single merged feed (Blended, Mentions). Picking the one already shown
// keeps it; leaving a feed is picking a layout.
import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Activity, Check, ShieldCheck } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import { Toggle } from '../ui/Toggle';
import { useOutsidePress } from '../../hooks/useOutsidePress';

export type MultiChatView = 'tabs' | 'columns' | 'splits' | 'blend' | 'mentions';
type ColumnCount = 2 | 3 | 4;

/** A rectangle split into `cols` equal columns, sized to sit beside 17 px
 *  lucide glyphs. The stroke is a shade under lucide's effective weight,
 *  because a pure rectangle reads heavier than an open-path glyph. */
export function ColumnsIcon({ cols }: { cols: number }) {
  const width = 17;
  const height = 13;
  const colWidth = width / cols;
  return (
    <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} fill="none" aria-hidden>
      <rect x="0.5" y="0.5" width={width - 1} height={height - 1} rx="1.5" stroke="currentColor" strokeWidth="1.25" />
      {Array.from({ length: cols - 1 }, (_, i) => (
        <line
          key={i}
          x1={(i + 1) * colWidth}
          y1="0.5"
          x2={(i + 1) * colWidth}
          y2={height - 0.5}
          stroke="currentColor"
          strokeWidth="1.25"
        />
      ))}
    </svg>
  );
}

function SplitsIcon() {
  return (
    <svg className="h-[17px] w-[17px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" aria-hidden>
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <path d="M11 3v18M11 13h10" />
    </svg>
  );
}

function BlendIcon() {
  return (
    <svg className="h-[17px] w-[17px]" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2} aria-hidden>
      <circle cx="9" cy="12" r="6" />
      <circle cx="15" cy="12" r="6" />
    </svg>
  );
}

function MentionsIcon() {
  return (
    <span className="flex h-[17px] w-[17px] items-center justify-center text-[15px] font-bold leading-none" aria-hidden>
      @
    </span>
  );
}

function ViewIcon({ view, columns }: { view: MultiChatView; columns: ColumnCount }) {
  if (view === 'columns') return <ColumnsIcon cols={columns} />;
  if (view === 'splits') return <SplitsIcon />;
  if (view === 'blend') return <BlendIcon />;
  if (view === 'mentions') return <MentionsIcon />;
  return <ColumnsIcon cols={1} />;
}

const MENU_WIDTH = 320;
const MENU_MARGIN = 8;

const VIEW_NAME: Record<MultiChatView, string> = {
  tabs: 'Tabs',
  columns: 'Columns',
  splits: 'Custom splits',
  blend: 'Blended',
  mentions: 'Inbox',
};

interface ViewRowProps {
  icon: ReactNode;
  label: string;
  detail: string;
  on: boolean;
  onPick: () => void;
  /** Extra controls on the row's right (the column counts). */
  accessory?: ReactNode;
}

function ViewRow({ icon, label, detail, on, onPick, accessory }: ViewRowProps) {
  return (
    <div
      className={`flex items-center gap-3 rounded-lg px-2.5 py-2 transition-colors ${
        on ? 'bg-white/[0.07]' : 'hover:bg-white/[0.04]'
      }`}
    >
      <button
        type="button"
        role="radio"
        aria-checked={on}
        onClick={onPick}
        className="flex min-w-0 flex-1 items-center gap-3 text-left"
      >
        <span
          className={`flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg bg-white/[0.05] ${
            on ? 'text-accent' : 'text-textSecondary'
          }`}
        >
          {icon}
        </span>
        <span className="min-w-0 flex-1">
          <span className={`block text-[13px] font-medium leading-5 ${on ? 'text-textPrimary' : 'text-textSecondary'}`}>
            {label}
          </span>
          <span className="block text-[12px] leading-4 text-textMuted">{detail}</span>
        </span>
      </button>
      {accessory}
      {/* The check keeps its slot when off, so a row never shifts as the pick moves. */}
      <Check size={15} strokeWidth={2.5} className={`flex-shrink-0 text-accent ${on ? '' : 'invisible'}`} aria-hidden />
    </div>
  );
}

interface PanelRowProps {
  icon: ReactNode;
  label: string;
  detail: string;
  on: boolean;
  onToggle: () => void;
}

function PanelRow({ icon, label, detail, on, onToggle }: PanelRowProps) {
  return (
    <div className="flex items-center gap-3 rounded-lg px-2.5 py-2">
      <span className={`flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg bg-white/[0.05] ${on ? 'text-accent' : 'text-textSecondary'}`}>
        {icon}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-[13px] font-medium leading-5 text-textSecondary">{label}</span>
        <span className="block text-[12px] leading-4 text-textMuted">{detail}</span>
      </span>
      <Toggle enabled={on} onChange={onToggle} ariaLabel={label} />
    </div>
  );
}

/** The panel switches a narrow window moves out of the title bar. */
export interface ViewPanels {
  modLogs: boolean;
  activity: boolean;
  onToggleModLogs: () => void;
  onToggleActivity: () => void;
}

interface MultiChatViewPickerProps {
  view: MultiChatView;
  /** The column count Columns uses (the last one picked when not in Columns). */
  columns: ColumnCount;
  chatCount: number;
  onTabs: () => void;
  onColumns: (count: ColumnCount) => void;
  onSplits: () => void;
  onBlend: () => void;
  onMentions: () => void;
  /** Set on a narrow window, whose title bar has no room for the panel buttons. */
  panels?: ViewPanels;
  /** Total live viewers, shown in the header when the title bar's own viewer
   *  count has no room (a narrow window); null hides it. */
  watching?: number | null;
}

export default function MultiChatViewPicker({
  view,
  columns,
  chatCount,
  onTabs,
  onColumns,
  onSplits,
  onBlend,
  onMentions,
  panels,
  watching = null,
}: MultiChatViewPickerProps) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number; width: number } | null>(null);
  // The menu is portaled out but stays inside the wrapper in React.
  const markInside = useOutsidePress(open, [wrapRef], () => setOpen(false));

  // Hang the menu under the button, right edges lined up, and slide it back
  // inside the window when that would run past its left edge (on a narrow
  // window the button sits near the left).
  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      const r = wrapRef.current?.getBoundingClientRect();
      if (!r) return;
      const width = Math.min(MENU_WIDTH, window.innerWidth - 2 * MENU_MARGIN);
      const left = Math.min(Math.max(MENU_MARGIN, r.right - width), window.innerWidth - width - MENU_MARGIN);
      setPos({ top: r.bottom + 8, left, width });
    };
    place();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    window.addEventListener('resize', place);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('resize', place);
      window.removeEventListener('keydown', onKey);
    };
  }, [open]);

  const pick = (fn: () => void) => () => {
    fn();
    setOpen(false);
  };

  return (
    <div ref={wrapRef} onPointerDownCapture={markInside} className="chrome-glaze chrome-glaze--flat titlebar-icon-group" data-tauri-drag-region="false">
      <Tooltip content={`View: ${VIEW_NAME[view]}`} delay={200} disabled={open}>
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          aria-haspopup="dialog"
          aria-expanded={open}
          aria-label={`View: ${VIEW_NAME[view]}`}
          data-tauri-drag-region="false"
          className={`titlebar-icon-btn gap-1 ${open ? 'is-active' : ''}`}
        >
          <ViewIcon view={view} columns={columns} />
          <svg width={10} height={10} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.75} strokeLinecap="round" strokeLinejoin="round" className="opacity-60" aria-hidden>
            <path d="M6 9l6 6 6-6" />
          </svg>
        </button>
      </Tooltip>

      {open && pos && createPortal(
        <div
          role="dialog"
          aria-label="View"
          className="glass-flyout fixed z-[1001] !p-2"
          style={{ top: pos.top, left: pos.left, width: pos.width }}
        >
          <div className="flex items-baseline justify-between px-2.5 pb-1 pt-1">
            <p className="text-[13px] font-semibold text-textPrimary">View</p>
            <span className="flex items-center gap-1.5 text-[12px] tabular-nums text-textMuted">
              {chatCount} {chatCount === 1 ? 'chat' : 'chats'}
              {watching != null && (
                <>
                  <span aria-hidden>·</span>
                  <span className="h-1.5 w-1.5 rounded-full bg-red-500" aria-hidden />
                  {watching.toLocaleString()} watching
                </>
              )}
            </span>
          </div>

          <div role="radiogroup" aria-label="View">
            <p className="px-2.5 pb-1 pt-2.5 text-[11px] font-semibold uppercase tracking-wider text-textMuted">Layout</p>
            <ViewRow
              icon={<ColumnsIcon cols={1} />}
              label="Tabs"
              detail="One chat at a time"
              on={view === 'tabs'}
              onPick={pick(onTabs)}
            />
            <ViewRow
              icon={<ColumnsIcon cols={columns} />}
              label="Columns"
              detail="Chats side by side"
              on={view === 'columns'}
              onPick={pick(() => onColumns(columns))}
              accessory={
                <span className="flex flex-shrink-0 items-center gap-1" role="group" aria-label="Number of columns">
                  {([2, 3, 4] as const).map((n) => {
                    const chosen = view === 'columns' && columns === n;
                    return (
                      <button
                        key={n}
                        type="button"
                        onClick={pick(() => onColumns(n))}
                        aria-pressed={chosen}
                        aria-label={`${n} columns`}
                        className={`h-7 w-7 rounded-md text-[12px] font-semibold tabular-nums transition-colors ${
                          chosen
                            ? 'bg-accent/20 text-accent'
                            : 'bg-white/[0.05] text-textSecondary hover:bg-white/[0.1] hover:text-textPrimary'
                        }`}
                      >
                        {n}
                      </button>
                    );
                  })}
                </span>
              }
            />
            <ViewRow
              icon={<SplitsIcon />}
              label="Custom splits"
              detail="Split any pane, drag the dividers"
              on={view === 'splits'}
              onPick={pick(onSplits)}
            />

            <div className="mx-2.5 my-2 h-px bg-white/[0.06]" />

            <p className="px-2.5 pb-1 pt-0.5 text-[11px] font-semibold uppercase tracking-wider text-textMuted">One feed</p>
            <ViewRow
              icon={<BlendIcon />}
              label="Blended"
              detail="Every chat merged into one"
              on={view === 'blend'}
              onPick={pick(onBlend)}
            />
            <ViewRow
              icon={<MentionsIcon />}
              label="Inbox"
              detail="Mentions, replies and whispers"
              on={view === 'mentions'}
              onPick={pick(onMentions)}
            />
          </div>

          {panels && (
            <>
              <div className="mx-2.5 my-2 h-px bg-white/[0.06]" />
              <p className="px-2.5 pb-1 pt-0.5 text-[11px] font-semibold uppercase tracking-wider text-textMuted">Panels</p>
              <PanelRow
                icon={<ShieldCheck size={17} />}
                label="Mod logs"
                detail="Bans, timeouts and deletions"
                on={panels.modLogs}
                onToggle={panels.onToggleModLogs}
              />
              <PanelRow
                icon={<Activity size={17} />}
                label="Activity"
                detail="Subs, gifts and raids"
                on={panels.activity}
                onToggle={panels.onToggleActivity}
              />
            </>
          )}
        </div>,
        document.body,
      )}
    </div>
  );
}

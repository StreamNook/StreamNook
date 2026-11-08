// The headings Profile > Customize is built from: a group label over a run of
// cards, a card title (mark, name, how many you own, an action on the right),
// and a sub-title inside a card. One look for every card. Plus the search that
// narrows every wall at once.
import { useRef, type ReactNode } from 'react';
import { Search, X } from 'lucide-react';

/** The search box over the walls (its matching is cosmeticSearch.ts). */
export function CosmeticSearch({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <div className="relative w-full max-w-[260px]">
      <Search
        size={13}
        strokeWidth={2.25}
        className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-textMuted"
      />
      <input
        ref={input}
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Escape' && value) {
            e.stopPropagation();
            onChange('');
          }
        }}
        placeholder="Search badges and paints"
        aria-label="Search your badges and paints"
        className="w-full rounded-md border border-white/[0.06] bg-white/[0.03] py-1.5 pl-7 pr-7 text-[12px] text-textPrimary placeholder:text-textMuted focus:border-white/[0.12] focus:bg-white/[0.05] focus:outline-none"
      />
      {value && (
        <button
          type="button"
          aria-label="Clear the search"
          onClick={() => {
            onChange('');
            input.current?.focus();
          }}
          className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-textMuted hover:bg-white/[0.06] hover:text-textPrimary"
        >
          <X size={12} />
        </button>
      )}
    </div>
  );
}

export function CustomizeGroup({ title, note, action }: { title: string; note?: string; action?: ReactNode }) {
  return (
    <div className="flex items-end justify-between gap-4 px-1 pt-2 first:pt-0">
      <div className="min-w-0">
        <h3 className="text-[11px] font-semibold uppercase tracking-[0.14em] text-textMuted">{title}</h3>
        {note && <p className="mt-0.5 text-[11px] leading-relaxed text-textSecondary">{note}</p>}
      </div>
      {action}
    </div>
  );
}

export function CardTitle({
  title,
  count,
  icon,
  badge,
  action,
}: {
  title: string;
  count?: number;
  icon?: ReactNode;
  /** A small state pill after the title (7TV "Connected"). */
  badge?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="mb-3 flex min-h-8 items-center justify-between gap-3">
      <div className="flex min-w-0 items-center gap-2">
        {icon && <span className="flex shrink-0 items-center">{icon}</span>}
        <h4 className="text-sm font-semibold uppercase tracking-wide text-textPrimary">{title}</h4>
        {count != null && <span className="text-xs tabular-nums text-textMuted">{count}</span>}
        {badge}
      </div>
      {action && <div className="flex shrink-0 items-center">{action}</div>}
    </div>
  );
}

export function SubTitle({ title, count, wearing }: { title: string; count?: number; wearing?: string | null }) {
  return (
    <p className="mb-2 flex min-w-0 items-baseline gap-1.5 text-xs font-medium text-textSecondary">
      {title}
      {count != null && <span className="tabular-nums text-textMuted">{count}</span>}
      {wearing && (
        <span className="min-w-0 truncate font-normal text-textMuted">
          · Wearing <span className="text-textSecondary">{wearing}</span>
        </span>
      )}
    </p>
  );
}

/** One tile size for every badge wall, so the walls line up card to card. */
export const BADGE_GRID = 'grid grid-cols-[repeat(auto-fill,3rem)] gap-2';
/** Paint names in even columns; a long name truncates (its tooltip has it whole). */
export const PAINT_GRID = 'grid grid-cols-[repeat(auto-fill,minmax(8.5rem,1fr))] gap-1.5';

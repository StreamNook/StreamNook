// The You tab's settings: a search over every settings row, and the page rows
// while nothing is typed. Rust ranks the rows (`search_settings`, the same
// catalog the iPhone reads: titles, aliases, sections and descriptions), so
// this only draws the answer and opens the page a hit lives on.
import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { create } from 'zustand';
import { ChevronRight } from 'lucide-react';
import { useMobileNavStore } from '../navStore';
import { SearchField } from '../ui/SearchField';
import EmoteProviderLogo, { type EmoteProviderId } from '../../components/chat/EmoteProviderLogo';
import { SETTINGS_ROWS } from './SettingsScreen';
import { catalogRowKey, openCatalogRow, pageViewOf, rowPlace, type CatalogRow } from './settingsCatalog';
import { Logger } from '../../utils/logger';

const EMOTE_LOGOS: readonly string[] = ['7tv', 'ffz', 'bttv'];

/** The brand mark that stands in for a service's name in a row title. FFZ
 *  takes the text color because its brand white vanishes on a light theme. */
export const CatalogLogo: React.FC<{ logo?: string | null; size?: number }> = ({ logo, size = 14 }) => {
  if (!logo || !EMOTE_LOGOS.includes(logo)) return null;
  return (
    // Dropped 2px under the baseline so the mark centers on the title's
    // capitals rather than riding above them.
    <span
      className="mr-1.5 inline-flex shrink-0"
      style={{ width: size, height: size, verticalAlign: '-2px' }}
      aria-hidden
    >
      <EmoteProviderLogo provider={logo as EmoteProviderId} tinted={logo !== 'ffz'} className="h-full w-full" />
    </span>
  );
};

let measureCanvas: HTMLCanvasElement | null = null;

/** The longest start of `text` that fits `width` with an ellipsis after it,
 *  trimmed first, so the ellipsis never trails a space ("go large ..."), which
 *  the browser's own truncation leaves. */
function fitLine(text: string, width: number, font: string): string {
  measureCanvas ??= document.createElement('canvas');
  const ctx = measureCanvas.getContext('2d');
  if (!ctx || width <= 0) return text;
  ctx.font = font;
  if (ctx.measureText(text).width <= width) return text;
  let lo = 0;
  let hi = text.length;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (ctx.measureText(`${text.slice(0, mid).trimEnd()}…`).width <= width) lo = mid;
    else hi = mid - 1;
  }
  return `${text.slice(0, lo).trimEnd()}…`;
}

/** One line of quiet text, cut to fit with a trimmed ellipsis. The CSS
 *  truncation stays as the backstop for the frame before it has measured. */
const FittedLine: React.FC<{ text: string; className: string }> = ({ text, className }) => {
  const ref = useRef<HTMLSpanElement>(null);
  const [shown, setShown] = useState(text);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    let live = true;
    // Canvas and layout disagree on this font (features and axes the canvas
    // ignores), so the budget is scaled by how wide layout really draws what
    // is shown now (a Range measures the text, overflow included), and kept a
    // pixel short, so the browser never cuts the fitted line a second time.
    const fit = () => {
      if (!live) return;
      const font = getComputedStyle(el).font;
      const range = document.createRange();
      range.selectNodeContents(el);
      const drawn = range.getBoundingClientRect().width;
      const ctx = (measureCanvas ??= document.createElement('canvas')).getContext('2d');
      let scale = 1;
      if (ctx && drawn > 0) {
        ctx.font = font;
        const measured = ctx.measureText(el.textContent ?? '').width;
        if (measured > 0) scale = drawn / measured;
      }
      setShown(fitLine(text, (el.clientWidth - 1) / scale, font));
    };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(el);
    // A web font arriving after the first measure changes every width.
    void document.fonts?.ready.then(fit);
    return () => {
      live = false;
      observer.disconnect();
    };
  }, [text]);
  return (
    <span ref={ref} className={`block truncate ${className}`} title={shown === text ? undefined : text}>
      {shown}
    </span>
  );
};

interface SearchAnswer {
  query: string;
  rows: CatalogRow[];
}

/** What the search holds, kept while you open a hit and come back: the query,
 *  and the last answer so the hits are there on return without a blank beat.
 *  Each answer is kept with the query it answers, so a late reply for an older
 *  query is never shown under a newer one. */
const useSettingsSearchStore = create<{
  query: string;
  answer: SearchAnswer | null;
  setQuery: (query: string) => void;
  setAnswer: (answer: SearchAnswer) => void;
}>((set) => ({
  query: '',
  answer: null,
  setQuery: (query) => set({ query }),
  setAnswer: (answer) => set({ answer }),
}));

/** Waits this long after the last keystroke before asking Rust. */
const SEARCH_DEBOUNCE_MS = 120;

export const SettingsDirectory: React.FC = () => {
  const openSettings = useMobileNavStore((s) => s.openSettings);
  const query = useSettingsSearchStore((s) => s.query);
  const setQuery = useSettingsSearchStore((s) => s.setQuery);
  const answer = useSettingsSearchStore((s) => s.answer);
  const setAnswer = useSettingsSearchStore((s) => s.setAnswer);
  const trimmed = query.trim();
  const [focused, setFocused] = useState(false);
  const fieldRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    // Already answered (coming back from a hit): nothing to ask again.
    if (!trimmed || useSettingsSearchStore.getState().answer?.query === trimmed) return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      invoke<CatalogRow[]>('search_settings', { query: trimmed })
        .then((rows) => {
          if (!cancelled) setAnswer({ query: trimmed, rows });
        })
        .catch((e) => {
          Logger.warn('[SettingsSearch] search_settings failed:', e);
          if (!cancelled) setAnswer({ query: trimmed, rows: [] });
        });
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [trimmed, setAnswer]);

  // The field sits low on the You tab; on focus it rises to the top of the
  // screen so the hits show above the keyboard instead of under it.
  const raiseField = () => {
    const field = fieldRef.current;
    const scroller = field?.closest<HTMLElement>('.sn-mobile-screen');
    if (!field || !scroller) return;
    const top = field.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop - 8;
    scroller.scrollTo({ top: Math.max(0, top), behavior: 'smooth' });
  };

  const openHit = (row: CatalogRow) => {
    inputRef.current?.blur();
    openCatalogRow(row);
  };

  // While the next answer is on its way, the last hits stay up rather than the
  // list blinking empty on every keystroke; "no match" is only ever said for
  // the query actually typed.
  const answered = answer?.query === trimmed;
  const shown = answer && (answered || answer.rows.length > 0) ? answer.rows : null;
  // Room under the hits while searching, so the field can rise to the top even
  // when only a few rows answer. Reserved space, never content moved down.
  const searching = focused || !!trimmed;

  return (
    <div>
      <div ref={fieldRef} className="px-4 mb-2">
        <SearchField
          ref={inputRef}
          value={query}
          onChange={(next) => {
            setQuery(next);
            // Cleared with the X: keep the keyboard up for the next search.
            if (!next) inputRef.current?.focus();
          }}
          onFocus={() => {
            setFocused(true);
            // After the keyboard has started to open, so the rise lands where
            // the visible area ends up.
            window.setTimeout(raiseField, 150);
          }}
          onBlur={() => setFocused(false)}
          placeholder="Search settings"
          aria-label="Search settings"
        />
      </div>

      <div className="px-3" style={searching ? { minHeight: '100dvh' } : undefined}>
        {!trimmed ? (
          SETTINGS_ROWS.map(({ id, label, icon: Icon, description }) => (
            <button
              key={id}
              onClick={() => openSettings(id)}
              className="w-full flex items-center gap-3 px-3 py-3 rounded-lg active:bg-surface-active text-left"
            >
              <Icon size={20} className="text-textSecondary shrink-0" />
              <span className="flex-1 min-w-0">
                <span className="block text-[15px] font-medium text-textPrimary">{label}</span>
                <FittedLine text={description} className="text-[12.5px] text-textMuted" />
              </span>
              <ChevronRight size={16} className="text-textMuted shrink-0" />
            </button>
          ))
        ) : shown === null ? null : shown.length === 0 && answered ? (
          <p className="px-6 py-6 text-center text-[14px] font-medium text-textSecondary">
            No setting matches &ldquo;{trimmed}&rdquo;.
          </p>
        ) : (
          shown.map((row) => <SearchHit key={catalogRowKey(row)} row={row} onOpen={openHit} />)
        )}
      </div>
    </div>
  );
};

/** One hit: its page's icon, the title (a brand mark leading it where the row
 *  is about one service), where it lives, and its one-line description. */
const SearchHit: React.FC<{ row: CatalogRow; onOpen: (row: CatalogRow) => void }> = ({ row, onOpen }) => {
  const view = pageViewOf(row.page);
  const Icon = SETTINGS_ROWS.find((r) => r.id === view)?.icon;
  return (
    <button
      onClick={() => onOpen(row)}
      className="w-full flex items-start gap-3 px-3 py-2.5 rounded-lg active:bg-surface-active text-left"
    >
      {/* Icon and chevron sit on the title's line, not the block's middle. */}
      <span className="flex h-5 w-5 shrink-0 items-center justify-center">
        {Icon && <Icon size={20} className="text-textSecondary" />}
      </span>
      <span className="flex-1 min-w-0">
        <span className="block truncate text-[15px] font-medium leading-5 text-textPrimary">
          <CatalogLogo logo={row.logo} />
          {row.title}
        </span>
        {/* Where it lives, in the description's quiet color: the title is the
            one bright line of a hit. */}
        <FittedLine text={rowPlace(row)} className="text-[12.5px] font-medium text-textMuted" />
        {row.detail && <FittedLine text={row.detail} className="text-[12.5px] text-textMuted" />}
      </span>
      <span className="flex h-5 shrink-0 items-center">
        <ChevronRight size={16} className="text-textMuted" />
      </span>
    </button>
  );
};

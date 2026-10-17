// The You tab's settings: a search over every settings row, and the page rows
// while nothing is typed. Rust ranks the rows (`search_settings`, the same
// catalog the iPhone reads: titles, aliases, sections and descriptions), so
// this only draws the answer and opens the page a hit lives on.
import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { create } from 'zustand';
import { MagnifyingGlass, X } from 'phosphor-react';
import { ChevronRight } from 'lucide-react';
import { useMobileNavStore } from '../navStore';
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
      <div ref={fieldRef} className="relative px-4 mb-2">
        <MagnifyingGlass
          size={17}
          className="absolute left-7 top-1/2 -translate-y-1/2 text-textMuted pointer-events-none z-10"
        />
        <input
          ref={inputRef}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onFocus={() => {
            setFocused(true);
            // After the keyboard has started to open, so the rise lands where
            // the visible area ends up.
            window.setTimeout(raiseField, 150);
          }}
          onBlur={() => setFocused(false)}
          placeholder="Search settings"
          aria-label="Search settings"
          className="glass-input w-full h-11 pl-10 pr-10 text-[15px] text-textPrimary placeholder:text-textMuted bg-transparent outline-none"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          enterKeyHint="search"
        />
        {query && (
          <button
            onClick={() => {
              setQuery('');
              inputRef.current?.focus();
            }}
            className="absolute right-5 top-1/2 -translate-y-1/2 sn-touch flex items-center justify-center text-textMuted"
            aria-label="Clear search"
          >
            <X size={16} />
          </button>
        )}
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
                <span className="block text-[12.5px] text-textMuted truncate">{description}</span>
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
        <span className="block truncate text-[12.5px] font-medium text-textSecondary">{rowPlace(row)}</span>
        {row.detail && <span className="block truncate text-[12.5px] text-textMuted">{row.detail}</span>}
      </span>
      <span className="flex h-5 shrink-0 items-center">
        <ChevronRight size={16} className="text-textMuted" />
      </span>
    </button>
  );
};

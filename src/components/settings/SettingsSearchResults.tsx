import { useMemo } from 'react';
import { ChevronRight, SearchX } from 'lucide-react';
import { searchSettings, type SettingsIndexEntry } from './searchIndex';

interface SettingsSearchResultsProps {
  query: string;
  onSelect: (entry: SettingsIndexEntry) => void;
  /** Precomputed results. When omitted, the full settings index is searched.
   *  A scoped surface (e.g. MultiChat) passes its own merged result set. */
  results?: SettingsIndexEntry[];
}

const Highlight = ({ text, query }: { text: string; query: string }) => {
  if (!query.trim()) return <>{text}</>;
  const tokens = query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  if (tokens.length === 0) return <>{text}</>;
  const pattern = new RegExp(`(${tokens.join('|')})`, 'gi');
  const parts = text.split(pattern);
  return (
    <>
      {parts.map((part, i) =>
        pattern.test(part) ? (
          <mark
            key={i}
            className="bg-accent/20 text-textPrimary rounded px-0.5"
          >
            {part}
          </mark>
        ) : (
          <span key={i}>{part}</span>
        ),
      )}
    </>
  );
};

const SettingsSearchResults = ({
  query,
  onSelect,
  results: provided,
}: SettingsSearchResultsProps) => {
  const results = useMemo(() => provided ?? searchSettings(query), [provided, query]);

  if (results.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <SearchX size={32} className="text-textMuted mb-3" strokeWidth={1.5} />
        <p className="text-sm text-textPrimary">No matches for "{query}"</p>
        <p className="mt-1 text-[12px] text-textMuted">
          Try fewer or different words.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-1.5">
      {results.map((entry, i) => (
        <button
          key={`${entry.tab}-${entry.section}-${entry.parent ?? ''}-${entry.title}-${i}`}
          onClick={() => onSelect(entry)}
          className="settings-card group flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-white/[0.03]"
        >
          <div className="min-w-0 flex-1">
            <div className="mb-1 flex items-center gap-1.5 text-[11px] uppercase tracking-[0.08em] text-textMuted">
              <span>{entry.tab}</span>
              <ChevronRight size={11} className="opacity-60" />
              <span>{entry.section}</span>
              {entry.parent && (
                <>
                  <ChevronRight size={11} className="opacity-60" />
                  <span>{entry.parent}</span>
                </>
              )}
            </div>
            <div className="text-[13px] font-medium text-textPrimary">
              <Highlight text={entry.title} query={query} />
            </div>
            {entry.description && (
              <p className="mt-0.5 text-[12px] leading-relaxed text-textSecondary line-clamp-2">
                <Highlight text={entry.description} query={query} />
              </p>
            )}
          </div>
          <ChevronRight
            size={14}
            className="flex-shrink-0 text-textMuted opacity-0 transition-opacity group-hover:opacity-100"
          />
        </button>
      ))}
    </div>
  );
};

export default SettingsSearchResults;

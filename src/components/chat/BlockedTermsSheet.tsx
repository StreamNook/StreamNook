// The Tools menu's Blocked terms page: block a phrase, then the channel's
// whole list with Remove on each row. It takes the menu's body while open, so
// the menu's header, outside press and sizing still apply. Rust writes the
// rows' words; blockedTerms.ts holds the list.
import React, { useId, useMemo, useState } from 'react';
import { Search } from 'lucide-react';
import { isImeComposing } from '../../utils/imeComposing';
import { BLOCKED_TERM_MAX, filterBlockedTerms, useBlockedTerms } from './blockedTerms';
import { ACCENT_BUTTON, ACCENT_FILL, GLAZE_PILL } from '../ui/glazeButtons';

const LABEL = 'text-[11px] font-semibold text-white/50 tracking-wider uppercase';
const QUIET = 'px-2 py-4 text-center text-[12px] text-white/40';

const BlockedTermsSheet: React.FC<{ broadcasterId: string }> = ({ broadcasterId }) => {
  const { rows, error, busy, note, retry, add, remove } = useBlockedTerms(broadcasterId);
  const [draft, setDraft] = useState('');
  const [query, setQuery] = useState('');
  const fieldId = useId();
  const shown = useMemo(() => (rows ? filterBlockedTerms(rows, query) : []), [rows, query]);

  const submit = async () => {
    if (!draft.trim() || busy) return;
    if (await add(draft)) {
      setDraft('');
      setQuery('');
    }
  };

  return (
    <div className="flex-1 min-h-0 flex flex-col">
      <div className="px-4 pt-3.5 pb-4 border-b border-white/5 shrink-0">
        <label htmlFor={fieldId} className={`${LABEL} block mb-2`}>
          Block a phrase
        </label>
        <div className="flex items-center gap-2">
          <input
            id={fieldId}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key !== 'Enter' || isImeComposing(e)) return;
              e.preventDefault();
              void submit();
            }}
            maxLength={BLOCKED_TERM_MAX}
            placeholder="A word or phrase"
            autoComplete="off"
            spellCheck={false}
            className="glass-input flex-1 min-w-0 h-9 rounded-lg px-3 text-[13px] text-textPrimary placeholder-textSecondary"
          />
          <button
            onClick={() => void submit()}
            disabled={!draft.trim() || busy !== null}
            className={`${ACCENT_BUTTON} h-9 px-4 !text-[12.5px]`}
            style={ACCENT_FILL}
          >
            Block
          </button>
        </div>
        <p className="mt-2 text-[12px] leading-relaxed text-white/50">
          Use * to catch every form of a word, like spam*.
          <span className="block">2 to 500 characters.</span>
        </p>
        {note && <p className="mt-1.5 text-xs leading-relaxed text-error">{note}</p>}
      </div>

      {rows && rows.length > 0 && (
        <div className="px-4 pt-3 pb-1 shrink-0">
          <div className="relative">
            <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-white/30 pointer-events-none" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              aria-label="Search blocked terms"
              placeholder={`Search ${rows.length} ${rows.length === 1 ? 'term' : 'terms'}`}
              autoComplete="off"
              spellCheck={false}
              className="glass-input w-full h-9 rounded-lg pl-8 pr-3 text-[13px] text-textPrimary placeholder-textSecondary"
            />
          </div>
        </div>
      )}

      <div className="flex-1 min-h-0 overflow-y-auto custom-scrollbar px-2 pt-1.5 pb-4 [mask-image:linear-gradient(to_bottom,black_calc(100%-20px),transparent)]">
        {error ? (
          <div className="px-3 py-6 text-center">
            <p className="text-[13px] leading-relaxed text-white/65">{error}</p>
            <button
              onClick={retry}
              className={`${GLAZE_PILL} mt-3 h-8 px-4 text-[12px]`}
            >
              Try again
            </button>
          </div>
        ) : rows === null ? (
          <p className={QUIET}>Loading blocked terms…</p>
        ) : rows.length === 0 ? (
          <div className="px-3 py-6 text-center">
            <p className="text-[13px] text-white/70">No blocked terms in this channel yet.</p>
            <p className="mt-1.5 text-[12px] leading-relaxed text-white/45">Messages with a blocked phrase never reach chat.</p>
          </div>
        ) : shown.length === 0 ? (
          <p className={QUIET}>Nothing matches “{query.trim()}”.</p>
        ) : (
          shown.map((row) => (
            <div
              key={row.id}
              className="group px-2.5 py-2.5 rounded-lg flex items-center gap-3 border-b border-white/[0.06] last:border-b-0 hover:bg-white/[0.06] transition-colors"
            >
              <div className="min-w-0 flex-1">
                <span className="block text-sm text-white/80 [overflow-wrap:anywhere]">{row.text}</span>
                {(row.added || row.expires) && (
                  <span className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-[12px] text-white/45">
                    {row.added && <span>{row.added}</span>}
                    {row.expires && (
                      <span className="px-1.5 py-px rounded bg-warning/15 text-warning">{row.expires}</span>
                    )}
                  </span>
                )}
              </div>
              <button
                onClick={() => void remove(row)}
                disabled={busy !== null}
                aria-label={`Remove ${row.text}`}
                // Quiet until the row is pointed at, then a glazed pill.
                className={`${GLAZE_PILL} h-7 flex-shrink-0 px-3 text-[12px] opacity-60 group-hover:opacity-100 focus-visible:opacity-100 hover:!text-red-300`}
              >
                Remove
              </button>
            </div>
          ))
        )}
      </div>
    </div>
  );
};

export default BlockedTermsSheet;

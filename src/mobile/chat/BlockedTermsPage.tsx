// The phone's Blocked terms page: block a phrase, then the channel's whole
// list with Remove on each row. Self-contained, so any sheet can mount it as
// a page (Manage channel does). The list and its words come from Rust through
// the desktop sheet's hook, so both apps say the same thing.
import React, { useMemo, useState } from 'react';
import { MagnifyingGlass } from 'phosphor-react';
import { BLOCKED_TERM_MAX, filterBlockedTerms, useBlockedTerms } from '../../components/chat/blockedTerms';

interface Props {
  channelId: string;
}

export const BlockedTermsPage: React.FC<Props> = ({ channelId }) => {
  const { rows, error, busy, note, retry, add, remove } = useBlockedTerms(channelId);
  const [draft, setDraft] = useState('');
  const [query, setQuery] = useState('');
  const shown = useMemo(() => (rows ? filterBlockedTerms(rows, query) : []), [rows, query]);

  const submit = async () => {
    if (!draft.trim() || busy) return;
    if (await add(draft)) {
      setDraft('');
      setQuery('');
    }
  };

  const group = 'rounded-2xl bg-white/[0.05] overflow-hidden';
  const groupLabel = 'px-3 pb-1.5 text-[12px] font-semibold uppercase tracking-wide text-textMuted';
  const quiet = 'px-3 py-4 text-center text-[14px] text-textMuted';

  return (
    <div className="flex flex-col gap-4 pt-1">
      <div>
        <label htmlFor="sn-block-phrase" className={`${groupLabel} block`}>
          Block a phrase
        </label>
        <div className={`${group} flex items-center gap-2 px-3`}>
          <input
            id="sn-block-phrase"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && void submit()}
            maxLength={BLOCKED_TERM_MAX}
            placeholder="A word or phrase"
            enterKeyHint="done"
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            className="h-12 min-w-0 flex-1 bg-transparent text-[15px] text-textPrimary outline-none placeholder:text-textMuted"
          />
          <button
            type="button"
            disabled={!draft.trim() || busy !== null}
            onClick={() => void submit()}
            className="sn-touch shrink-0 px-1 text-[15px] font-semibold text-accent active:opacity-70 disabled:opacity-40"
          >
            Block
          </button>
        </div>
        <p className="px-3 pt-1.5 text-[12.5px] leading-snug text-textMuted">
          Use * to catch every form of a word, like spam*.
          <span className="block">2 to 500 characters.</span>
        </p>
        {note && <p className="px-3 pt-1 text-[12.5px] leading-snug text-error">{note}</p>}
      </div>

      <div>
        <div className={groupLabel}>{rows && rows.length > 0 ? `Blocked · ${rows.length}` : 'Blocked'}</div>
        {rows && rows.length > 0 && (
          <div className={`${group} mb-2 flex items-center gap-2 px-3`}>
            <MagnifyingGlass size={16} className="shrink-0 text-textMuted" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              aria-label="Search blocked terms"
              placeholder="Search"
              enterKeyHint="search"
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              className="h-10 min-w-0 flex-1 bg-transparent text-[15px] text-textPrimary outline-none placeholder:text-textMuted"
            />
          </div>
        )}
        <div className={`${group} divide-y divide-borderSubtle`}>
          {error ? (
            <div className="px-3 py-4 text-center">
              <p className="text-[14px] text-textSecondary">{error}</p>
              <button
                type="button"
                onClick={retry}
                className="sn-touch mt-2 h-9 px-3.5 rounded-xl bg-white/10 text-[14px] font-semibold text-textPrimary active:opacity-70"
              >
                Try again
              </button>
            </div>
          ) : rows === null ? (
            <p className={quiet}>Loading…</p>
          ) : rows.length === 0 ? (
            <div className="px-3 py-4 text-center">
              <p className="text-[14px] text-textPrimary">No blocked terms in this channel yet.</p>
              <p className="mt-1 text-[12.5px] text-textMuted">Messages with a blocked phrase never reach chat.</p>
            </div>
          ) : shown.length === 0 ? (
            <p className={quiet}>Nothing matches “{query.trim()}”.</p>
          ) : (
            shown.map((row) => (
              <div key={row.id} className="flex items-center gap-3 px-3 py-2.5 min-h-[48px]">
                <div className="min-w-0 flex-1">
                  <div className="text-[15px] text-textPrimary [overflow-wrap:anywhere]">{row.text}</div>
                  {(row.added || row.expires) && (
                    <div className="mt-0.5 flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-[12.5px] text-textMuted">
                      {row.added && <span>{row.added}</span>}
                      {row.expires && <span className="px-1.5 rounded-md bg-warning/15 text-warning">{row.expires}</span>}
                    </div>
                  )}
                </div>
                <button
                  type="button"
                  disabled={busy !== null}
                  onClick={() => void remove(row)}
                  aria-label={`Remove ${row.text}`}
                  className="sn-touch shrink-0 px-1 py-1 text-[14px] font-semibold text-error active:opacity-70 disabled:opacity-50"
                >
                  Remove
                </button>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
};

export default BlockedTermsPage;

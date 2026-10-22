// Ignored phrases: messages containing one never reach any chat surface.
// Evaluated in Rust (src-tauri/src/services/chat_rules.rs) before the message
// is broadcast, so every window inherits the list. Regex patterns are checked
// by the same engine that runs them.

import { useEffect, useMemo, useState } from 'react';
import { EntryInput } from './entryInput';
import { invoke } from '@tauri-apps/api/core';
import { Plus, Trash2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import type { IgnoredPhrase } from '../../types';
import { Tooltip } from '../ui/Tooltip';

/** One edit to the stored ignored-phrase list; Rust makes new entries.
 *  Answers the list as stored, or null when the edit failed. */
async function editIgnored(edit: Record<string, unknown>): Promise<IgnoredPhrase[] | null> {
  try {
    return await invoke<IgnoredPhrase[]>('edit_settings_list', { path: 'chat_filters.ignored_phrases', edit });
  } catch (err) {
    Logger.warn('[IgnoredPhrases] edit failed:', err);
    return null;
  }
}

const Chip = ({
  on,
  label,
  title,
  onClick,
}: {
  on: boolean;
  label: string;
  title: string;
  onClick: () => void;
}) => (
  <Tooltip content={title} describe>
    <button
      type="button"
      onClick={onClick}
      className={`rounded px-1.5 py-0.5 text-[11px] font-semibold transition-colors ${
        on ? 'bg-accent/25 text-accent' : 'bg-surface text-textSecondary hover:text-textPrimary'
      }`}
    >
      {label}
    </button>
  </Tooltip>
);

const IgnoredPhrasesSettings = () => {
  const stored = useAppStore((s) => s.settings.chat_filters?.ignored_phrases);
  const phrases = useMemo(() => stored ?? [], [stored]);

  const update = (id: string, patch: Partial<IgnoredPhrase>) => void editIgnored({ op: 'update', id, patch });
  const savePattern = (id: string) => async (pattern: string) => {
    const list = await editIgnored({ op: 'update', id, patch: { pattern } });
    return list?.find((p) => p.id === id)?.pattern;
  };
  const remove = (id: string) => void editIgnored({ op: 'remove', id });
  const add = () => void editIgnored({ op: 'add' });

  const [errors, setErrors] = useState<Record<string, string>>({});
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const next: Record<string, string> = {};
      for (const p of phrases) {
        if (!p.is_regex || !p.pattern.trim()) continue;
        try {
          const err = await invoke<string | null>('validate_chat_phrase', {
            pattern: p.pattern,
            isRegex: true,
            wholeWord: !!p.whole_word,
            caseSensitive: !!p.case_sensitive,
          });
          if (err) next[p.id] = err;
        } catch {
          /* advisory */
        }
      }
      if (!cancelled) setErrors(next);
    })();
    return () => {
      cancelled = true;
    };
  }, [phrases]);

  return (
    <div className="flex w-full flex-col gap-2">
      {phrases.map((p) => (
        <div key={p.id} className="flex flex-col gap-1 rounded-lg bg-glass/30 p-2">
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => update(p.id, { enabled: !p.enabled })}
              className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors ${
                p.enabled ? 'bg-accent' : 'bg-gray-600'
              }`}
              aria-label={p.enabled ? 'Disable phrase' : 'Enable phrase'}
            >
              <span
                className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                  p.enabled ? 'translate-x-5' : 'translate-x-1'
                }`}
              />
            </button>
            <EntryInput
              type="text"
              value={p.pattern}
              save={savePattern(p.id)}
              placeholder={p.is_regex ? 'Regular expression' : 'Word or phrase to hide'}
              className="glass-input min-w-0 flex-1 px-2.5 py-1.5 text-sm text-textPrimary"
              spellCheck={false}
            />
            <Chip on={!!p.is_regex} label="regex" title="Treat as a regular expression" onClick={() => update(p.id, { is_regex: !p.is_regex })} />
            <Chip on={!!p.whole_word} label="word" title="Match whole words only" onClick={() => update(p.id, { whole_word: !p.whole_word })} />
            <Chip on={!!p.case_sensitive} label="Aa" title="Case sensitive" onClick={() => update(p.id, { case_sensitive: !p.case_sensitive })} />
            <button
              type="button"
              onClick={() => remove(p.id)}
              className="p-1 text-textSecondary transition-colors hover:text-error"
              aria-label="Remove phrase"
            >
              <Trash2 size={14} />
            </button>
          </div>
          {errors[p.id] && <p className="pl-11 text-xs text-error">{errors[p.id]}</p>}
        </div>
      ))}
      <button
        type="button"
        onClick={add}
        className="glass-button inline-flex w-fit items-center gap-2 rounded-lg px-3 py-1.5 text-sm font-medium text-textPrimary"
      >
        <Plus size={14} />
        Add phrase
      </button>
    </div>
  );
};

export default IgnoredPhrasesSettings;

// Ignored phrases: messages containing one never reach any chat surface.
// Evaluated in Rust (src-tauri/src/services/chat_rules.rs) before the message
// is broadcast, so every window inherits the list. Regex patterns are checked
// by the same engine that runs them.

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Plus, Trash2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import type { IgnoredPhrase } from '../../types';

function newId(): string {
  return typeof crypto !== 'undefined' && 'randomUUID' in crypto
    ? crypto.randomUUID()
    : Math.random().toString(36).slice(2);
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
  <button
    type="button"
    onClick={onClick}
    title={title}
    className={`rounded px-1.5 py-0.5 text-[11px] font-semibold transition-colors ${
      on ? 'bg-accent/25 text-accent' : 'bg-surface text-textSecondary hover:text-textPrimary'
    }`}
  >
    {label}
  </button>
);

const IgnoredPhrasesSettings = () => {
  const { settings, updateSettings } = useAppStore();
  const phrases = settings.chat_filters?.ignored_phrases ?? [];

  const write = (next: IgnoredPhrase[]) => {
    updateSettings({
      ...settings,
      chat_filters: { ...settings.chat_filters, ignored_phrases: next },
    });
  };
  const update = (id: string, patch: Partial<IgnoredPhrase>) =>
    write(phrases.map((p) => (p.id === id ? { ...p, ...patch } : p)));
  const remove = (id: string) => write(phrases.filter((p) => p.id !== id));
  const add = () =>
    write([
      ...phrases,
      { id: newId(), pattern: '', enabled: true, case_sensitive: false, whole_word: false, is_regex: false },
    ]);

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
            <input
              type="text"
              value={p.pattern}
              onChange={(e) => update(p.id, { pattern: e.target.value })}
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

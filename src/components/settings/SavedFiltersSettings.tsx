// Saved message filters, in Chatterino's expression syntax, evaluated in
// Rust for every message (src-tauri/src/services/chat_rules.rs). A chat pane
// bound to a filter shows only the rows the engine stamped with its id, so
// toggling a filter never drops history. Presets cover the common asks;
// the expression editor accepts anything Chatterino's filter page does.

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Plus, Trash2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import type { SavedChatFilter } from '../../types';

function newId(): string {
  return typeof crypto !== 'undefined' && 'randomUUID' in crypto
    ? crypto.randomUUID()
    : Math.random().toString(36).slice(2);
}

const PRESETS: Array<{ name: string; expr: string }> = [
  { name: 'Mods only', expr: 'author.badges contains "moderator" || author.badges contains "broadcaster"' },
  { name: 'Subs only', expr: 'author.subbed' },
  { name: 'VIPs and mods', expr: 'author.badges contains "vip" || author.badges contains "moderator"' },
  { name: 'Mentions and highlights', expr: 'flags.mention || flags.highlight_rule' },
  { name: 'Redemptions', expr: 'flags.points_redeemed || flags.highlighted' },
  { name: 'Links', expr: 'message.content contains "http"' },
  { name: 'Events only', expr: 'flags.sub_message || flags.cheer_message || flags.system_message' },
  { name: 'No commands', expr: '!(message.content startswith "!")' },
];

interface Validation {
  ok: boolean;
  error?: string;
  unsupported: string[];
}

const SavedFiltersSettings = () => {
  const { settings, updateSettings } = useAppStore();
  const filters = settings.chat_query?.filters ?? [];

  const write = (next: SavedChatFilter[]) => {
    updateSettings({
      ...settings,
      chat_query: { ...settings.chat_query, filters: next },
    });
  };
  const update = (id: string, patch: Partial<SavedChatFilter>) =>
    write(filters.map((f) => (f.id === id ? { ...f, ...patch } : f)));
  const remove = (id: string) => write(filters.filter((f) => f.id !== id));
  const add = (preset?: { name: string; expr: string }) =>
    write([...filters, { id: newId(), name: preset?.name ?? '', expr: preset?.expr ?? '', enabled: true }]);

  const [checks, setChecks] = useState<Record<string, Validation>>({});
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const next: Record<string, Validation> = {};
      for (const f of filters) {
        if (!f.expr.trim()) continue;
        try {
          next[f.id] = await invoke<Validation>('validate_chat_filter', { expr: f.expr });
        } catch {
          /* advisory */
        }
      }
      if (!cancelled) setChecks(next);
    })();
    return () => {
      cancelled = true;
    };
  }, [filters]);

  return (
    <div className="flex w-full flex-col gap-3">
      {filters.length === 0 && (
        <p className="text-sm text-textSecondary">
          No filters yet. Start from a preset, or write your own as a short expression, for
          example <code className="text-xs">author.badges contains &quot;moderator&quot;</code>,{' '}
          <code className="text-xs">message.length &gt; 200</code>,{' '}
          <code className="text-xs">flags.reply &amp;&amp; !author.subbed</code>.
        </p>
      )}
      {filters.map((f) => {
        const v = checks[f.id];
        return (
          <div key={f.id} className="flex flex-col gap-1.5 rounded-lg bg-glass/30 p-2">
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => update(f.id, { enabled: !f.enabled })}
                className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors ${
                  f.enabled ? 'bg-accent' : 'bg-gray-600'
                }`}
                aria-label={f.enabled ? 'Disable filter' : 'Enable filter'}
              >
                <span
                  className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                    f.enabled ? 'translate-x-5' : 'translate-x-1'
                  }`}
                />
              </button>
              <input
                type="text"
                value={f.name}
                onChange={(e) => update(f.id, { name: e.target.value })}
                placeholder="Filter name"
                className="glass-input w-40 flex-shrink-0 px-2.5 py-1.5 text-sm text-textPrimary"
                spellCheck={false}
              />
              <input
                type="text"
                value={f.expr}
                onChange={(e) => update(f.id, { expr: e.target.value })}
                placeholder='author.badges contains "moderator"'
                className="glass-input min-w-0 flex-1 px-2.5 py-1.5 font-mono text-xs text-textPrimary"
                spellCheck={false}
              />
              <button
                type="button"
                onClick={() => remove(f.id)}
                className="p-1 text-textSecondary transition-colors hover:text-error"
                aria-label="Remove filter"
              >
                <Trash2 size={14} />
              </button>
            </div>
            {v && !v.ok && <p className="pl-11 text-xs text-error">{v.error}</p>}
            {v && v.ok && v.unsupported.length > 0 && (
              <p className="pl-11 text-xs text-textSecondary">
                StreamNook has no data for {v.unsupported.join(', ')} yet; those read as empty.
              </p>
            )}
          </div>
        );
      })}
      <div className="flex flex-wrap items-center gap-1.5">
        <button
          type="button"
          onClick={() => add()}
          className="glass-button inline-flex items-center gap-2 rounded-lg px-3 py-1.5 text-sm font-medium text-textPrimary"
        >
          <Plus size={14} />
          New filter
        </button>
        <span className="mx-1 text-xs text-textSecondary">Presets:</span>
        {PRESETS.map((p) => (
          <button
            key={p.name}
            type="button"
            onClick={() => add(p)}
            className="rounded-full bg-surface px-2.5 py-1 text-xs text-textSecondary transition-colors hover:text-textPrimary"
            title={p.expr}
          >
            {p.name}
          </button>
        ))}
      </div>
    </div>
  );
};

export default SavedFiltersSettings;

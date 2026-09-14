import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Dropdown } from '../ui/Dropdown';
import { ChevronDown, ChevronUp, Plus, Trash2, Volume2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { SOUND_LABELS, playSound, type SoundId } from '../../utils/notificationSound';
import { SettingsSection } from './_primitives';
import type { HighlightPhrase } from '../../types';

const DEFAULT_PHRASE_COLOR = '#fbbf24';
const DEFAULT_COOLDOWN_SECONDS = 3;
const SOUND_OPTIONS: SoundId[] = ['boop', 'tick', 'soft', 'whisper', 'gentle'];

function makeDefaultPhrase(): HighlightPhrase {
  return {
    id: typeof crypto !== 'undefined' && 'randomUUID' in crypto
      ? crypto.randomUUID()
      : Math.random().toString(36).slice(2),
    pattern: '',
    enabled: true,
    case_sensitive: false,
    whole_word: true,
    is_regex: false,
    color: DEFAULT_PHRASE_COLOR,
    sound_id: null,
    cooldown_seconds: DEFAULT_COOLDOWN_SECONDS,
  };
}

const HighlightPhrasesSettings = () => {
  const { settings, updateSettings } = useAppStore();
  const phrases = useMemo(
    () => settings.chat_highlights?.phrases ?? [],
    [settings.chat_highlights],
  );

  // Merge, never replace: the same object carries users, badges, built_in
  // and appearance, which this panel must not wipe.
  const writePhrases = (next: HighlightPhrase[]) => {
    updateSettings({
      ...settings,
      chat_highlights: { ...settings.chat_highlights, phrases: next },
    });
  };

  // Regex phrases are validated by Rust, the dialect that actually runs them
  // (no lookaround, no backreferences). Errors keyed by phrase id.
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
            wholeWord: p.whole_word,
            caseSensitive: p.case_sensitive,
          });
          if (err) next[p.id] = err;
        } catch {
          // Validation is advisory; the engine skips a bad pattern anyway.
        }
      }
      if (!cancelled) setErrors(next);
    })();
    return () => {
      cancelled = true;
    };
  }, [phrases]);

  const updatePhrase = (id: string, patch: Partial<HighlightPhrase>) => {
    writePhrases(phrases.map((p) => (p.id === id ? { ...p, ...patch } : p)));
  };

  const removePhrase = (id: string) => {
    writePhrases(phrases.filter((p) => p.id !== id));
  };

  const movePhrase = (id: string, direction: -1 | 1) => {
    const idx = phrases.findIndex((p) => p.id === id);
    if (idx === -1) return;
    const target = idx + direction;
    if (target < 0 || target >= phrases.length) return;
    const next = phrases.slice();
    [next[idx], next[target]] = [next[target], next[idx]];
    writePhrases(next);
  };

  const addPhrase = () => {
    writePhrases([...phrases, makeDefaultPhrase()]);
  };

  return (
    <SettingsSection
      label="Highlight Phrases"
      description="Flash chat messages that match specific words, names, or patterns. Mentions of your own name and replies to you are always highlighted; these are extra."
      bare
    >
      <div className="space-y-3">
        {phrases.length === 0 && (
          <div className="bg-glass/30 rounded-lg px-4 py-6 text-center">
            <p className="text-sm text-textSecondary mb-3">
              No highlight phrases yet.
            </p>
            <button
              onClick={addPhrase}
              className="glass-button inline-flex items-center gap-2 px-3 py-1.5 rounded-lg text-textPrimary text-sm font-medium"
            >
              <Plus size={14} />
              Add your first phrase
            </button>
          </div>
        )}

        {phrases.map((phrase, idx) => {
          const error = errors[phrase.id] ?? null;
          return (
            <div
              key={phrase.id}
              className="bg-glass/30 rounded-lg p-3 space-y-2"
            >
              <div className="flex items-center gap-2">
                <button
                  onClick={() => updatePhrase(phrase.id, { enabled: !phrase.enabled })}
                  className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors flex-shrink-0 ${
                    phrase.enabled ? 'bg-accent' : 'bg-gray-600'
                  }`}
                  aria-label={phrase.enabled ? 'Disable phrase' : 'Enable phrase'}
                >
                  <span
                    className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                      phrase.enabled ? 'translate-x-5' : 'translate-x-1'
                    }`}
                  />
                </button>

                <input
                  type="text"
                  value={phrase.pattern}
                  onChange={(e) => updatePhrase(phrase.id, { pattern: e.target.value })}
                  placeholder={phrase.is_regex ? 'Regular expression' : 'Word or phrase'}
                  className="flex-1 glass-input text-textPrimary text-sm px-2.5 py-1.5"
                  spellCheck={false}
                />

                <input
                  type="color"
                  value={phrase.color}
                  onChange={(e) => updatePhrase(phrase.id, { color: e.target.value })}
                  className="w-8 h-8 rounded cursor-pointer bg-transparent border border-borderSubtle"
                  aria-label="Highlight color"
                />

                <div className="flex items-center gap-1 flex-shrink-0">
                  <button
                    onClick={() => movePhrase(phrase.id, -1)}
                    disabled={idx === 0}
                    className="p-1 text-textSecondary hover:text-textPrimary disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
                    aria-label="Move up"
                  >
                    <ChevronUp size={14} />
                  </button>
                  <button
                    onClick={() => movePhrase(phrase.id, 1)}
                    disabled={idx === phrases.length - 1}
                    className="p-1 text-textSecondary hover:text-textPrimary disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
                    aria-label="Move down"
                  >
                    <ChevronDown size={14} />
                  </button>
                  <button
                    onClick={() => removePhrase(phrase.id)}
                    className="p-1 text-textSecondary hover:text-red-400 transition-colors"
                    aria-label="Delete phrase"
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>

              <div className="flex items-center gap-4 pl-11 text-xs text-textSecondary">
                <label className="inline-flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={phrase.case_sensitive}
                    onChange={(e) => updatePhrase(phrase.id, { case_sensitive: e.target.checked })}
                    className="accent-accent"
                  />
                  Case sensitive
                </label>
                <label className={`inline-flex items-center gap-1.5 ${phrase.is_regex ? 'opacity-50' : 'cursor-pointer'}`}>
                  <input
                    type="checkbox"
                    checked={phrase.whole_word && !phrase.is_regex}
                    onChange={(e) => updatePhrase(phrase.id, { whole_word: e.target.checked })}
                    disabled={phrase.is_regex}
                    className="accent-accent"
                  />
                  Whole word
                </label>
                <label className="inline-flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={phrase.is_regex}
                    onChange={(e) => updatePhrase(phrase.id, { is_regex: e.target.checked })}
                    className="accent-accent"
                  />
                  Regular expression
                </label>
              </div>

              <div className="flex items-center gap-3 pl-11 text-xs text-textSecondary">
                <label className="inline-flex items-center gap-2">
                  <span>Sound</span>
                  <Dropdown
                    value={phrase.sound_id ?? ''}
                    onChange={(v) =>
                      updatePhrase(phrase.id, {
                        sound_id: v === '' ? null : (v as SoundId),
                      })
                    }
                    ariaLabel="Highlight sound"
                    options={[
                      { value: '', label: 'None' },
                      ...SOUND_OPTIONS.map((id) => ({ value: id, label: SOUND_LABELS[id] })),
                      ...(settings.chat_highlights?.custom_sounds ?? []).map((s) => ({ value: s.id, label: s.name || 'Custom sound' })),
                    ]}
                  />
                </label>

                {phrase.sound_id && (
                  <>
                    <button
                      onClick={() => playSound(phrase.sound_id as SoundId)}
                      className="inline-flex items-center gap-1 text-textSecondary hover:text-textPrimary transition-colors"
                      aria-label="Preview sound"
                      type="button"
                    >
                      <Volume2 size={12} />
                      <span>Preview</span>
                    </button>

                    <label className="inline-flex items-center gap-2 ml-auto">
                      <span>Cooldown</span>
                      <input
                        type="number"
                        min={0}
                        max={60}
                        step={1}
                        value={phrase.cooldown_seconds ?? DEFAULT_COOLDOWN_SECONDS}
                        onChange={(e) => {
                          const raw = parseInt(e.target.value, 10);
                          const clamped = Number.isFinite(raw) ? Math.max(0, Math.min(60, raw)) : DEFAULT_COOLDOWN_SECONDS;
                          updatePhrase(phrase.id, { cooldown_seconds: clamped });
                        }}
                        className="glass-input text-textPrimary text-xs px-2 py-1 w-16 text-right"
                      />
                      <span>s</span>
                    </label>
                  </>
                )}
              </div>

              {error && (
                <div className="pl-11 text-xs text-red-400">
                  {error}
                </div>
              )}
            </div>
          );
        })}

        {phrases.length > 0 && (
          <button
            onClick={addPhrase}
            className="inline-flex items-center gap-2 px-3 py-1.5 text-textSecondary hover:text-textPrimary text-sm transition-colors"
          >
            <Plus size={14} />
            Add another phrase
          </button>
        )}
      </div>
    </SettingsSection>
  );
};

export default HighlightPhrasesSettings;

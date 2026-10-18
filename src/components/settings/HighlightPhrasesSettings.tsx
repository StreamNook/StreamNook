import { useEffect, useMemo, useState } from 'react';
import { EntryInput } from './entryInput';
import { invoke } from '@tauri-apps/api/core';
import { Dropdown } from '../ui/Dropdown';
import { ChevronDown, ChevronUp, Plus, Trash2, Volume2 } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { BUILT_IN_SOUNDS, SOUND_LABELS, playSound, type SoundId } from '../../utils/notificationSound';
import { Logger } from '../../utils/logger';
import { SettingsSection } from './_primitives';
import type { HighlightPhrase } from '../../types';

const DEFAULT_COOLDOWN_SECONDS = 3;
const SOUND_OPTIONS: SoundId[] = BUILT_IN_SOUNDS;

/** One edit to the stored phrase list; Rust makes new entries and clamps the
 *  cooldown. Answers the list as stored, or null when the edit failed. */
async function editPhrases(edit: Record<string, unknown>): Promise<HighlightPhrase[] | null> {
  try {
    return await invoke<HighlightPhrase[]>('edit_settings_list', { path: 'chat_highlights.phrases', edit });
  } catch (err) {
    Logger.warn('[HighlightPhrases] edit failed:', err);
    return null;
  }
}

const HighlightPhrasesSettings = () => {
  const highlights = useAppStore((s) => s.settings.chat_highlights);
  const phrases = useMemo(() => highlights?.phrases ?? [], [highlights]);

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
    void editPhrases({ op: 'update', id, patch });
  };

  // A typed field: the text as stored comes back for the field to show.
  const saveField = (id: string, field: 'pattern' | 'color') => async (text: string) => {
    const stored = await editPhrases({ op: 'update', id, patch: { [field]: text } });
    return stored?.find((p) => p.id === id)?.[field];
  };

  const saveCooldown = (id: string) => async (text: string) => {
    const raw = parseInt(text, 10);
    const stored = await editPhrases({
      op: 'update',
      id,
      patch: { cooldown_seconds: Number.isFinite(raw) ? raw : DEFAULT_COOLDOWN_SECONDS },
    });
    const seconds = stored?.find((p) => p.id === id)?.cooldown_seconds;
    return seconds === undefined ? undefined : String(seconds);
  };

  const removePhrase = (id: string) => {
    void editPhrases({ op: 'remove', id });
  };

  const movePhrase = (id: string, direction: -1 | 1) => {
    const idx = phrases.findIndex((p) => p.id === id);
    if (idx === -1) return;
    const target = idx + direction;
    if (target < 0 || target >= phrases.length) return;
    void editPhrases({ op: 'move', id, to: target });
  };

  const addPhrase = () => {
    void editPhrases({ op: 'add' });
  };

  return (
    <SettingsSection
      label="Highlight Phrases"
      description="Highlight chat messages that match specific words, names, or patterns, with a flash or, with Keep tinted, a color that stays. Mentions of your own name and replies to you are always highlighted; these are extra."
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

                <EntryInput
                  type="text"
                  value={phrase.pattern}
                  save={saveField(phrase.id, 'pattern')}
                  placeholder={phrase.is_regex ? 'Regular expression' : 'Word or phrase'}
                  className="flex-1 glass-input text-textPrimary text-sm px-2.5 py-1.5"
                  spellCheck={false}
                />

                <EntryInput
                  type="color"
                  value={phrase.color}
                  save={saveField(phrase.id, 'color')}
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
                <label className="inline-flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={phrase.hold === true}
                    onChange={(e) => updatePhrase(phrase.id, { hold: e.target.checked })}
                    className="accent-accent"
                  />
                  Keep tinted
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
                      ...(highlights?.custom_sounds ?? []).map((s) => ({ value: s.id, label: s.name || 'Custom sound' })),
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
                      <EntryInput
                        type="number"
                        min={0}
                        max={60}
                        step={1}
                        value={String(phrase.cooldown_seconds ?? DEFAULT_COOLDOWN_SECONDS)}
                        save={saveCooldown(phrase.id)}
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

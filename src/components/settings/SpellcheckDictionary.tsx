import { useState } from 'react';
import { Plus, X } from 'lucide-react';

import { useAppStore } from '../../stores/AppStore';

/**
 * The words the spell checker has been taught, with a way to add and remove
 * them. Most entries arrive from "Add to dictionary" in the composer's
 * right-click menu; this panel is how you take one back out after a mis-click.
 */
export const SpellcheckDictionary: React.FC = () => {
  const settings = useAppStore((s) => s.settings);
  const updateSettings = useAppStore((s) => s.updateSettings);
  const [draft, setDraft] = useState('');

  const words = settings.chat_input?.spellcheck_custom_words ?? [];

  const save = (next: string[]) =>
    void updateSettings({
      ...settings,
      chat_input: { ...settings.chat_input, spellcheck_custom_words: next },
    });

  const add = () => {
    // One word per entry: the checker works a word at a time, so a phrase would
    // never match anything.
    const word = draft.trim().split(/\s+/)[0]?.toLowerCase() ?? '';
    if (!word) return;
    setDraft('');
    if (words.some((w) => w.toLowerCase() === word)) return;
    save([...words, word]);
  };

  return (
    <div className="px-1 pb-3">
      <div className="mb-2 text-xs text-textSecondary">
        Words you&apos;ve taught the spell checker. Added automatically when you pick
        &quot;Add to dictionary&quot; on a word in chat.
      </div>

      <div className="flex items-center gap-2">
        <input
          type="text"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              add();
            }
          }}
          placeholder="Add a word"
          className="flex-1 glass-input text-textPrimary text-sm px-2.5 py-1.5"
          spellCheck={false}
        />
        <button
          onClick={add}
          disabled={!draft.trim()}
          className="glass-button flex h-8 w-8 items-center justify-center rounded disabled:opacity-40 disabled:cursor-not-allowed"
          aria-label="Add word"
        >
          <Plus size={16} />
        </button>
      </div>

      {words.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-1.5">
          {words.map((word) => (
            <span
              key={word}
              className="flex items-center gap-1 rounded-lg bg-glass px-2 py-1 text-sm text-textSecondary"
            >
              {word}
              <button
                onClick={() => save(words.filter((w) => w !== word))}
                className="text-textMuted transition-colors hover:text-textPrimary"
                aria-label={`Remove ${word}`}
              >
                <X size={12} />
              </button>
            </span>
          ))}
        </div>
      )}
    </div>
  );
};

export default SpellcheckDictionary;

import { useRef, useState, type InputHTMLAttributes, type TextareaHTMLAttributes } from 'react';

/** Saves one typed value to Rust and answers the value as stored. */
export type SaveText = (text: string) => Promise<string | undefined>;

/**
 * Local text for a field over one field of a stored list entry. What is typed
 * shows at once; writes go to Rust one at a time with the newest text
 * winning, and once they settle the field shows the text as stored. Off focus
 * it shows the store's value.
 */
function useEntryDraft(value: string) {
  const [draft, setDraft] = useState<string | null>(null);
  const queued = useRef<string | null>(null);
  const busy = useRef(false);

  const change = async (text: string, save: SaveText) => {
    setDraft(text);
    queued.current = text;
    if (busy.current) return;
    busy.current = true;
    let stored: string | undefined;
    try {
      while (queued.current !== null) {
        const next = queued.current;
        queued.current = null;
        stored = await save(next);
      }
    } finally {
      busy.current = false;
    }
    const settled = stored;
    if (settled !== undefined) setDraft((d) => (d === null ? null : settled));
  };

  return { shown: draft ?? value, change, blur: () => setDraft(null) };
}

export const EntryInput = ({
  value,
  save,
  ...props
}: Omit<InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange' | 'onBlur'> & { value: string; save: SaveText }) => {
  const draft = useEntryDraft(value);
  return (
    <input {...props} value={draft.shown} onChange={(e) => void draft.change(e.target.value, save)} onBlur={draft.blur} />
  );
};

export const EntryTextarea = ({
  value,
  save,
  ...props
}: Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, 'value' | 'onChange' | 'onBlur'> & { value: string; save: SaveText }) => {
  const draft = useEntryDraft(value);
  return (
    <textarea {...props} value={draft.shown} onChange={(e) => void draft.change(e.target.value, save)} onBlur={draft.blur} />
  );
};

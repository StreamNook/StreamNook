import { useEffect, useState } from 'react';

import { checkText, type SpellContext } from '../utils/spellcheck';
import { useAppStore } from '../stores/AppStore';

/** Long enough that a fast typist doesn't trigger a check per keystroke, short
 *  enough that the underline lands before they've moved on. */
const DEBOUNCE_MS = 250;

/** Stable identity so callers can use the result in a dependency array. */
const NO_RANGES: Array<[number, number]> = [];

export interface UseSpellcheckOptions extends SpellContext {
  enabled: boolean;
}

/**
 * Misspelled ranges in `text`, as [start, end) pairs, for a chat composer.
 *
 * Results are stored alongside the text they were computed from, and only
 * returned while that text is still current. Offsets from a previous keystroke
 * point at the wrong characters — typing one letter at the start of the box
 * shifts every range — so a mismatch means no underlines rather than underlines
 * in the wrong place. In practice that reads as the squiggles briefly lifting
 * while you type and settling once you pause.
 */
export function useSpellcheck(
  text: string,
  { enabled, emoteKey }: UseSpellcheckOptions,
): Array<[number, number]> {
  const [result, setResult] = useState<{
    text: string;
    ranges: Array<[number, number]>;
  }>({ text: '', ranges: NO_RANGES });

  // Adding a word to the dictionary has to clear its underline immediately,
  // including in a MultiChat popout that received the change by broadcast.
  const customWords = useAppStore((s) => s.settings?.chat_input?.spellcheck_custom_words);

  useEffect(() => {
    if (!enabled || text.length === 0) return;

    // The check is async, so a slow reply for an earlier keystroke could land
    // after a newer one. This flag drops anything that arrives out of date.
    let current = true;

    const timer = setTimeout(() => {
      void checkText(text, { emoteKey }).then((ranges) => {
        if (current) setResult({ text, ranges });
      });
    }, DEBOUNCE_MS);

    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [text, enabled, emoteKey, customWords]);

  return enabled && result.text === text ? result.ranges : NO_RANGES;
}

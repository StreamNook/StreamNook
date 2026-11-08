import type { ReactNode } from 'react';
import type { ChatDesignSettings } from '../../types';

/** How the typed word sits beside an emote (`chat_design.emote_words`). */
export type EmoteWordsLook = NonNullable<ChatDesignSettings['emote_words']>;

/** One drawn emote (or its stack) with the word typed for it, in the chosen
 *  look. The chat row and the settings preview both draw through this, so the
 *  preview is the row. The look classes live in globals.css (`sn-emote-word-*`). */
export const withEmoteWord = (look: EmoteWordsLook, node: ReactNode, word: string, key: string): ReactNode => {
  if (look === 'off') return node;
  if (look === 'after') {
    // One unbreakable pair, so a wrap never leaves the word alone on the next line.
    return (
      <span key={key} className="sn-emote-word-pair">
        {node}
        <span className="sn-emote-word-after" aria-hidden="true">{word}</span>
      </span>
    );
  }
  if (look === 'chips') {
    return (
      <span key={key} className="sn-emote-word-chip">
        <span className="sn-emote-word-chip-art">{node}</span>
        <span className="sn-emote-word-label" aria-hidden="true">{word}</span>
      </span>
    );
  }
  return (
    <span key={key} className="sn-emote-word-under">
      {node}
      <span className="sn-emote-word-label" aria-hidden="true">{word}</span>
    </span>
  );
};

// A whisper's words as Rust filed them: text, and Twitch emotes (with 7TV,
// BTTV and FFZ globals added at read time) as small pictures. No parts means
// the plain text says everything.
import type { ReactNode } from 'react';
import type { MentionPart } from '../../types';

const EMOTE_CLASS = 'mx-px inline-block h-[1.35em] w-auto align-middle';

export interface WhisperPartsProps {
  parts: MentionPart[];
  text: string;
  emoteClass?: string;
  /** Turns plain words into nodes (links), when the surface wants that. */
  linkify?: (text: string, keyBase: string) => ReactNode;
}

export function WhisperParts({ parts, text, emoteClass = EMOTE_CLASS, linkify }: WhisperPartsProps) {
  if (!parts || parts.length === 0) return <>{linkify ? linkify(text, 't') : text}</>;
  return (
    <>
      {parts.map((part, i) =>
        part.type === 'emote' ? (
          // tooltip-exempt: an emote's name on a small inline picture
          <img key={i} src={part.url} alt={part.name} title={part.name} draggable={false} className={emoteClass} />
        ) : (
          <span key={i}>{linkify ? linkify(part.text, `t${i}`) : part.text}</span>
        ),
      )}
    </>
  );
}

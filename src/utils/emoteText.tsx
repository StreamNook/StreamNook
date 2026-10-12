import type { Emote, EmoteSet } from '../services/emoteService';
import { Tooltip } from '../components/ui/Tooltip';

// Flatten an EmoteSet into a single name -> emote lookup. Insertion order is
// reverse priority (ffz, bttv, 7tv, twitch, later wins): a Twitch emote is
// never replaced by a same-name third-party one, and 7TV beats the other
// third-party sets, which matches the precedence the chat renderer uses.
export function buildEmoteNameMap(emotes: EmoteSet | null | undefined): Map<string, Emote> {
  const map = new Map<string, Emote>();
  if (!emotes) return map;
  for (const list of [emotes.ffz, emotes.bttv, emotes['7tv'], emotes.twitch]) {
    for (const e of list) map.set(e.name, e);
  }
  return map;
}

/** A plain-text message split into its whitespace-delimited words and the
 *  whitespace between them, each word with the emote its name resolves to. */
export function emoteWords(text: string, emoteMap: Map<string, Emote>): Array<{ text: string; emote?: Emote }> {
  return text.split(/(\s+)/).map((tok) => {
    const emote = emoteMap.get(tok);
    return emote ? { text: tok, emote } : { text: tok };
  });
}

interface EmoteTextProps {
  text: string;
  emoteMap: Map<string, Emote>;
  keyPrefix?: string;
  imgClassName?: string;
}

// Render plain text, swapping whitespace-delimited words that match an emote
// name for inline emote images. For surfaces that arrive as plain text with no
// IRC emote ranges, such as prediction outcomes and pinned messages (the open
// pin and the collapsed pin bar in ChatWidget).
export function EmoteText({
  text,
  emoteMap,
  keyPrefix = 'et',
  imgClassName = 'inline-block align-middle object-contain h-5 max-h-5',
}: EmoteTextProps) {
  return (
    <>
      {emoteWords(text, emoteMap).map(({ text: tok, emote }, i) => {
        if (!emote) return <span key={`${keyPrefix}-${i}`}>{tok}</span>;
        return (
          <Tooltip key={`${keyPrefix}-${i}`} content={emote.name}>
            <img
              src={emote.url}
              alt={emote.name}
              className={imgClassName}
              loading="lazy"
              draggable={false}
            />
          </Tooltip>
        );
      })}
    </>
  );
}

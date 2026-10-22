import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import type { Emote, EmoteSet } from '../services/emoteService';
import type { MessageSegment } from '../services/twitchChat';
import type { ProviderId } from '../types/providers';
import { Tooltip } from '../components/ui/Tooltip';
import { listsByPrecedence } from './emotePrecedence';

/** Which emote each word of one text shows, by word. */
export type EmoteWordMap = ReadonlyMap<string, Emote>;

const NO_WORDS: EmoteWordMap = new Map();

// Flatten an EmoteSet into a single name -> emote lookup, a shared name going
// to the emote chat draws for it (listsByPrecedence, first wins). Only for
// platforms Rust does not cut plain text for yet (Kick, YouTube): a Twitch
// text's emotes come from `parse_text_segments`, through `useTextEmotes`.
export function buildEmoteNameMap(emotes: EmoteSet | null | undefined): Map<string, Emote> {
  const map = new Map<string, Emote>();
  if (!emotes) return map;
  for (const list of listsByPrecedence(emotes, { withPlatformEmotes: false })) {
    for (const e of list) if (!map.has(e.name)) map.set(e.name, e);
  }
  return map;
}

/** A plain-text message split into its whitespace-delimited words and the
 *  whitespace between them, each word with the emote its name resolves to. */
export function emoteWords(text: string, emoteMap: EmoteWordMap): Array<{ text: string; emote?: Emote }> {
  return text.split(/(\s+)/).map((tok) => {
    const emote = emoteMap.get(tok);
    return emote ? { text: tok, emote } : { text: tok };
  });
}

const providerOfUrl = (url: string): Emote['provider'] => {
  if (url.includes('7tv.app')) return '7tv';
  if (url.includes('betterttv.net')) return 'bttv';
  if (url.includes('frankerfacez.com')) return 'ffz';
  return 'twitch';
};

const setKey = (name: string, id: string) => `${name}\u0000${id}`;
const setIndexCache = new WeakMap<EmoteSet, Map<string, Emote>>();

/** The loaded set's emotes by name and id, so a word Rust resolved draws the
 *  same image the set would (its tier and URL), whichever provider it is from. */
function setIndex(emotes: EmoteSet): Map<string, Emote> {
  let index = setIndexCache.get(emotes);
  if (!index) {
    index = new Map();
    for (const list of [emotes.twitch, emotes['7tv'], emotes.bttv, emotes.ffz, emotes.kick, emotes.youtube]) {
      for (const e of list ?? []) if (!index.has(setKey(e.name, e.id))) index.set(setKey(e.name, e.id), e);
    }
    setIndexCache.set(emotes, index);
  }
  return index;
}

/** The emote per word in Rust's segments for one text, as the loaded set's
 *  emote where it holds the same one (Rust chose which; the set says how it
 *  looks), else as Rust described it. */
export function emoteWordMap(segments: readonly MessageSegment[], emotes: EmoteSet | null | undefined): EmoteWordMap {
  const index = emotes ? setIndex(emotes) : null;
  const map = new Map<string, Emote>();
  for (const seg of segments) {
    if (seg.type !== 'emote' || map.has(seg.content)) continue;
    const id = seg.emote_id ?? seg.content;
    const known = index?.get(setKey(seg.content, id));
    map.set(
      seg.content,
      known ?? { id, name: seg.content, url: seg.emote_url, provider: providerOfUrl(seg.emote_url), isZeroWidth: seg.is_zero_width },
    );
  }
  return map;
}

/** Rust's segments for each text as said in a Twitch channel, cut by the same
 *  parser and channel table as a chat row. Null when the ask failed. */
export async function parseTextSegments(channel: string, texts: string[]): Promise<MessageSegment[][] | null> {
  try {
    return await invoke<MessageSegment[][]>('parse_text_segments', { channel, texts });
  } catch {
    return null;
  }
}

// Answers per loaded set: a set that has not changed reuses them, so a card
// opened again draws its emotes on the first frame.
const answerCache = new WeakMap<EmoteSet, Map<string, EmoteWordMap>>();
const answerKey = (channel: string, text: string) => `${channel}\u0000${text}`;

export interface TextEmotesTarget {
  provider: ProviderId;
  /** The channel's login (Twitch) or slug. */
  channel: string | null | undefined;
}

/**
 * The emote words of each of `texts` (plain text with no emote positions: a
 * pinned message, a prediction outcome, a recent message on the profile card)
 * as the channel's chat would show them. Twitch asks Rust, which chooses the
 * emote for a name exactly as it does for a chat row; Kick and YouTube still
 * match names against the loaded set. Returns a lookup by text; a text Rust
 * has not answered yet shows as plain words, and an answer for a set that just
 * changed keeps the previous one until the new one lands.
 */
export function useTextEmotes(
  target: TextEmotesTarget,
  texts: readonly string[],
  emotes: EmoteSet | null | undefined,
): (text: string) => EmoteWordMap {
  const { provider } = target;
  const channel = target.channel?.toLowerCase() ?? '';
  const viaRust = provider === 'twitch' && !!channel;
  const nameMap = useMemo(() => (viaRust ? null : buildEmoteNameMap(emotes)), [viaRust, emotes]);

  const textsKey = useMemo(() => [...new Set(texts)].filter((t) => t.trim()).join('\u0001'), [texts]);
  const [held, setHeld] = useState<{ channel: string; answers: Map<string, EmoteWordMap> } | null>(null);

  useEffect(() => {
    if (!viaRust || !emotes || !textsKey) return;
    const cached = answerCache.get(emotes) ?? new Map<string, EmoteWordMap>();
    answerCache.set(emotes, cached);
    const missing = textsKey.split('\u0001').filter((t) => !cached.has(answerKey(channel, t)));
    if (missing.length === 0) return;
    let live = true;
    void parseTextSegments(channel, missing).then((lists) => {
      if (!lists) return;
      missing.forEach((t, i) => cached.set(answerKey(channel, t), emoteWordMap(lists[i] ?? [], emotes)));
      if (!live) return;
      setHeld((prev) => {
        const answers = new Map(prev?.channel === channel ? prev.answers : undefined);
        for (const t of missing) answers.set(t, cached.get(answerKey(channel, t))!);
        return { channel, answers };
      });
    });
    return () => {
      live = false;
    };
  }, [viaRust, channel, emotes, textsKey]);

  return (text: string) => {
    if (nameMap) return nameMap;
    const fresh = emotes ? answerCache.get(emotes)?.get(answerKey(channel, text)) : undefined;
    return fresh ?? (held?.channel === channel ? held.answers.get(text) : undefined) ?? NO_WORDS;
  };
}

interface EmoteTextProps {
  text: string;
  /** This text's emote words, from `useTextEmotes`. */
  emoteMap: EmoteWordMap;
  keyPrefix?: string;
  imgClassName?: string;
}

// Render plain text, swapping whitespace-delimited words that are emotes for
// inline emote images. For surfaces that arrive as plain text with no IRC
// emote ranges, such as prediction outcomes and pinned messages (the open pin
// and the collapsed pin bar in ChatWidget).
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

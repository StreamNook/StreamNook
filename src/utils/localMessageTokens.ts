// Tokenizer for the chat rows StreamNook builds itself (channel-points
// redemptions). Those never pass through the Rust IRC tokenizer, so nothing
// else turns their text into emotes. Pure on purpose: takes the emote set as an
// argument rather than reaching into the channel cache, so it is testable.
import { getEmoteLookup, type EmoteSet } from '../services/emoteService';
import { parseEmojisSync } from '../services/emojiService';
import type { MessageSegment } from '../services/twitchChat';

/**
 * Split plain text into emote/emoji/text segments by name lookup against a
 * channel's loaded emote sets.
 *
 * A locally-built row that pre-bakes its body as a single `text` segment wins
 * ChatMessage's segment fast-path, which passes segments straight through and
 * never runs name lookup, so every emote name in the body rendered as literal
 * text. Emits the Rust `MessageSegment` wire shape (snake_case) because that is
 * what every segment consumer reads: the chat row, the overlay, and the relay.
 */
export function tokenizeLocalBody(text: string, emotes: EmoteSet | null): MessageSegment[] {
  const lookup = emotes ? getEmoteLookup(emotes) : null;
  const out: MessageSegment[] = [];
  const push = (seg: MessageSegment) => {
    const last = out[out.length - 1];
    if (seg.type === 'text' && last?.type === 'text') last.content += seg.content;
    else out.push(seg);
  };

  text.split(' ').forEach((word, i) => {
    if (i > 0) push({ type: 'text', content: ' ' });

    // Exact-case, same as Twitch chat: a reward named "copium" is not the
    // COPIUM emote.
    const emote = lookup?.byName.get(word);
    if (emote) {
      push({
        type: 'emote',
        content: emote.name,
        emote_id: emote.id,
        emote_url: emote.url,
        is_zero_width: emote.isZeroWidth,
        modifier_flags: emote.modifierFlags,
      });
      return;
    }

    parseEmojisSync(word).forEach((seg) =>
      push(
        seg.type === 'emoji' && seg.emojiUrl
          ? { type: 'emoji', content: seg.content, emoji_url: seg.emojiUrl }
          : { type: 'text', content: seg.content },
      ),
    );
  });

  // The overlay reads a zero-length segment list as "no body", so never hand
  // back an empty array for a row that has text.
  return out.length > 0 ? out : [{ type: 'text', content: text }];
}

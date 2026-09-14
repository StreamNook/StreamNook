import { create } from 'zustand';
import type { Emote } from '../services/emoteService';
import type { MessageSegment } from '../services/twitchChat';

/**
 * Custom emotes learned from the messages of a provider channel.
 *
 * YouTube publishes no channel emote-set endpoint the way Twitch and Kick do —
 * `get_kick_channel_emotes` has no YouTube counterpart because there is nothing
 * to call. What it DOES do is ship each custom emoji inline with every message
 * that uses one (`youtube.rs` turns `shortcuts`/`emojiId` into an Emote segment
 * carrying the name and the image url). So the set is discoverable, just not
 * enumerable: we learn it by watching chat.
 *
 * The consequence, stated plainly because it is user-visible: the picker fills in
 * as a channel uses its emotes rather than being complete the moment you join. An
 * emote nobody has posted yet cannot be there. That is a real limitation of the
 * platform, not something to paper over — but a picker that grows is far better
 * than no picker, and in a busy channel it populates within seconds.
 */

/** Per channel, so a big channel can't push a small one out of the picker. */
const MAX_PER_CHANNEL = 400;

interface ProviderEmoteState {
  /** `provider:channel` -> emote name -> emote. A Map keeps insertion order and
   *  makes de-duplication by name free; the same emote arrives on every message
   *  that uses it. */
  byChannel: Record<string, Map<string, Emote>>;
  /** Learn any custom emotes carried by one message. */
  harvest: (channelKey: string, segments: MessageSegment[]) => void;
  /** Install the channel's real emoji set (from the live_chat page). Seeded
   *  entries win over harvested ones, since they carry the canonical shortcut. */
  seed: (channelKey: string, emotes: Emote[]) => void;
  /** Everything learned for a channel, in the order first seen. */
  emotesFor: (channelKey: string) => Emote[];
  /** Forget a channel — called when its chat is released. */
  clear: (channelKey: string) => void;
}

export const useProviderEmoteStore = create<ProviderEmoteState>((set, get) => ({
  byChannel: {},

  harvest: (channelKey, segments) => {
    // Cheap pre-check: the overwhelming majority of messages carry no emote at
    // all, and this runs on every single one.
    let fresh: Emote[] | null = null;
    const existing = get().byChannel[channelKey];
    for (const seg of segments) {
      if (seg.type !== 'emote' || !seg.emote_url) continue;
      const name = seg.content;
      if (!name || existing?.has(name)) continue;
      (fresh ??= []).push({
        id: seg.emote_id || name,
        name,
        url: seg.emote_url,
        // Typed as a Twitch-family provider upstream; YouTube emoji have no
        // third-party provider, and the picker only needs the url and name.
        provider: 'youtube' as unknown as Emote['provider'],
        isZeroWidth: seg.is_zero_width,
      });
    }
    if (!fresh) return;

    set((s) => {
      const next = new Map(s.byChannel[channelKey] ?? []);
      for (const e of fresh) {
        if (next.size >= MAX_PER_CHANNEL) break;
        next.set(e.name, e);
      }
      return { byChannel: { ...s.byChannel, [channelKey]: next } };
    });
  },

  seed: (channelKey, emotes) => {
    if (!emotes.length) return;
    set((s) => {
      // Seeded first so the authoritative entry wins, then anything harvested
      // that the set didn't include is appended rather than dropped.
      const next = new Map<string, Emote>();
      for (const e of emotes) next.set(e.name, e);
      for (const [name, e] of s.byChannel[channelKey] ?? []) {
        if (!next.has(name) && next.size < MAX_PER_CHANNEL) next.set(name, e);
      }
      return { byChannel: { ...s.byChannel, [channelKey]: next } };
    });
  },

  emotesFor: (channelKey) => {
    const map = get().byChannel[channelKey];
    return map ? Array.from(map.values()) : [];
  },

  clear: (channelKey) => {
    set((s) => {
      if (!s.byChannel[channelKey]) return s;
      const next = { ...s.byChannel };
      delete next[channelKey];
      return { byChannel: next };
    });
  },
}));

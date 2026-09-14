import { describe, it, expect } from 'vitest';
import { tokenizeLocalBody } from './localMessageTokens';
import type { EmoteSet } from '../services/emoteService';

const emote = (name: string, provider: 'twitch' | 'bttv' | '7tv' | 'ffz', id: string) => ({
  id,
  name,
  url: `https://cdn.example/${id}/2x.webp`,
  provider,
});

const makeSet = (over: Partial<EmoteSet> = {}): EmoteSet => ({
  twitch: [],
  bttv: [],
  '7tv': [],
  ffz: [],
  kick: [],
  youtube: [],
  ...over,
});

describe('tokenizeLocalBody', () => {
  it('turns a body that is exactly an emote name into an emote segment', () => {
    const set = makeSet({ bttv: [emote('COPIUM', 'bttv', 'bttv-1')] });

    expect(tokenizeLocalBody('COPIUM', set)).toEqual([
      {
        type: 'emote',
        content: 'COPIUM',
        emote_id: 'bttv-1',
        emote_url: 'https://cdn.example/bttv-1/2x.webp',
        is_zero_width: undefined,
        modifier_flags: undefined,
      },
    ]);
  });

  it('keeps the surrounding words and spacing around an emote', () => {
    const set = makeSet({ '7tv': [emote('Clap', '7tv', 'stv-1')] });

    expect(tokenizeLocalBody('big Clap now', set)).toEqual([
      { type: 'text', content: 'big ' },
      {
        type: 'emote',
        content: 'Clap',
        emote_id: 'stv-1',
        emote_url: 'https://cdn.example/stv-1/2x.webp',
        is_zero_width: undefined,
        modifier_flags: undefined,
      },
      { type: 'text', content: ' now' },
    ]);
  });

  it('matches emote names exactly, the way Twitch chat does', () => {
    const set = makeSet({ bttv: [emote('COPIUM', 'bttv', 'bttv-1')] });

    expect(tokenizeLocalBody('Copium', set)).toEqual([{ type: 'text', content: 'Copium' }]);
  });

  it('resolves a duplicated name in provider order, 7TV first', () => {
    const set = makeSet({
      '7tv': [emote('PogChamp', '7tv', 'stv-pog')],
      bttv: [emote('PogChamp', 'bttv', 'bttv-pog')],
      twitch: [emote('PogChamp', 'twitch', 'tw-pog')],
    });

    const [seg] = tokenizeLocalBody('PogChamp', set);
    expect(seg).toMatchObject({ type: 'emote', emote_id: 'stv-pog' });
  });

  it('falls back to plain text when the channel has no emotes loaded yet', () => {
    expect(tokenizeLocalBody('COPIUM', null)).toEqual([{ type: 'text', content: 'COPIUM' }]);
  });

  it('never returns an empty segment list, which the overlay reads as no body', () => {
    expect(tokenizeLocalBody('', makeSet())).toEqual([{ type: 'text', content: '' }]);
  });
});

// The emote fill menu is one question on every app: the desktop's colon list
// and both phones' suggestion strip ask Rust `match_emote_tokens` with profile
// "fill", so a word lists the same emotes in the same order everywhere. Rust's
// emote_match test proves a word and its colon form get one list over the same
// fixture words; this checks every app sends that question.
import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
  convertFileSrc: (p: string) => `asset://${p}`,
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }));

import { matchEmoteFill } from './emoteMatch';
import fixture from '../utils/emoteFill.fixture.json';

const root = resolve(__dirname, '..', '..');
const read = (p: string) => readFileSync(resolve(root, p), 'utf8');
const target = { provider: 'twitch' as const, channel: 'xqc', channelId: '71092938' };

describe('emote fill menu', () => {
  it('asks the fill question for every fixture word, as typed', async () => {
    expect(fixture.words.length).toBeGreaterThan(0);
    // One letter asks too: the full list starts at one character.
    expect(fixture.words).toContain(fixture.oneLetter);
    expect(fixture.oneLetter).toHaveLength(1);
    for (const word of fixture.words) {
      for (const typed of [word, `:${word}`]) {
        invoke.mockReset();
        invoke.mockResolvedValue({ rows: [], total: 0, ready: true });
        await matchEmoteFill(target, typed);
        expect(invoke).toHaveBeenCalledWith(
          'match_emote_tokens',
          expect.objectContaining({ profile: 'fill', query: typed, provider: 'twitch', channel: 'xqc' }),
        );
      }
    }
  });

  it('is what the desktop colon list and both phones ask', () => {
    // Desktop: the colon list is the fill menu.
    expect(read('src/components/ChatWidget.tsx')).toMatch(/source === 'colon'\s*\?\s*matchEmoteFill\(/);
    // Android: the suggestion strip.
    expect(read('src/mobile/chat/MobileChatInput.tsx')).toMatch(/matchEmoteFill\(\{ provider, channel, channelId \}, word\)/);
    // iPhone: the suggestion strip, through SNKit's one name for it.
    expect(read('ios/Sources/StreamNook/Chat/ChatComposerModel.swift')).toMatch(/"profile": EmoteFill\.profile/);
    expect(read('ios/Sources/SNKit/Composer.swift')).toMatch(/static let profile = "fill"/);
    // Rust resolves it in one place.
    expect(read('src-tauri/src/services/emote_match.rs')).toMatch(/profile == Some\("fill"\)/);
  });
});

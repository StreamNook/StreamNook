import { describe, expect, it, vi } from 'vitest';
import type { Emote } from './emoteService';

const saved = [
  // Starred before the picker knew which emotes animate.
  { id: 'a1', name: 'Still', url: 'https://old/a1', provider: '7tv', addedAt: 2 },
  { id: 'b2', name: 'Gone', url: 'https://old/b2', provider: '7tv', addedAt: 1 },
  { id: 'c3', name: 'Moving', url: 'https://old/c3', provider: 'bttv', addedAt: 3 },
];

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string) => (cmd === 'load_favorite_emotes_cache' ? JSON.stringify(saved) : null)),
}));

describe('getAvailableFavorites', () => {
  it("draws each favorite from the chat's own entry, in favorite order, keeping only the star date", async () => {
    const { loadFavoriteEmotes, getAvailableFavorites } = await import('./favoriteEmoteService');
    await loadFavoriteEmotes();
    const chat = [
      { id: 'c3', name: 'Moving', url: 'https://new/c3', provider: 'bttv', animated: true },
      { id: 'a1', name: 'Still', url: 'https://new/a1', provider: '7tv', animated: false, localUrl: 'asset://a1' },
    ] as Emote[];
    const shown = getAvailableFavorites(chat);
    expect(shown.map((e) => e.id)).toEqual(['a1', 'c3']);
    expect(shown[0]).toMatchObject({ animated: false, url: 'https://new/a1', localUrl: 'asset://a1', addedAt: 2 });
    expect(shown[1]).toMatchObject({ animated: true, addedAt: 3 });
  });
});

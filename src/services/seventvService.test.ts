import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(() => Promise.resolve()),
  convertFileSrc: (p: string) => p,
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

import { getBadgeImageUrl } from './seventvService';

describe('getBadgeImageUrl', () => {
  it('a still badge prefers _static', () => {
    const badge = {
      id: 'b',
      name: 'b',
      images: [
        { url: 'https://cdn.7tv.app/badge/b/4x.webp', scale: 4, mime: 'image/webp' },
        { url: 'https://cdn.7tv.app/badge/b/4x_static.webp', scale: 4, mime: 'image/webp' },
      ],
    };
    expect(getBadgeImageUrl(badge, true).endsWith('4x_static.webp')).toBe(true);
    expect(getBadgeImageUrl(badge).endsWith('4x.webp')).toBe(true);
    expect(getBadgeImageUrl(badge).endsWith('_static.webp')).toBe(false);
  });

  it('a badge that never moves keeps its cached file', () => {
    const badge = {
      id: 's',
      name: 's',
      localUrl: 'asset://badge-s',
      images: [{ url: 'https://cdn.7tv.app/badge/s/4x.png', scale: 4, mime: 'image/png', frameCount: 1 }],
    };
    expect(getBadgeImageUrl(badge, true)).toBe('asset://badge-s');
  });
});

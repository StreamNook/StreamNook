import { describe, expect, it } from 'vitest';
import { wornPreviewBadges } from './identity';
import type { ExternalPickerView, StreamNookPickerView } from '../../types/profileView';

const ext = {
  seventv_signed_in: false,
  paints: [],
  seventv_badges: [{ badge: { id: '7', url: 'https://cdn.7tv.app/badge/7/4x.webp', title: '' }, worn: true }],
  drops_signed_in: true,
  twitch_selectable: [{ set_id: 'premium', version: '1', image_url: 'https://twitch/premium', title: '', is_selected: true }],
  twitch_earned: [],
  chat_clients: [
    { id: 'ffz-1', url: 'https://ffz/1', title: '', provider: 'ffz' },
    { id: 'chatterino-1', url: 'https://chatterino/1', title: '', provider: 'chatterino' },
  ],
  named_colors: [],
} as unknown as ExternalPickerView;

const sn = {
  badges: [{ slug: 'streamnook-singularity', name: 'Singularity', image_url: 'https://cdn.streamnook.app/badges/s.webp', worn: true }],
} as unknown as StreamNookPickerView;

describe('wornPreviewBadges', () => {
  it('draws the chat apps after 7TV and the StreamNook badge last, as chat orders them', () => {
    const urls = wornPreviewBadges(sn, ext);
    expect(urls.slice(0, 4)).toEqual([
      'https://twitch/premium',
      'https://cdn.7tv.app/badge/7/4x.webp',
      'https://ffz/1',
      'https://chatterino/1',
    ]);
    expect(urls).toHaveLength(5);
    expect(urls[4]).toContain('s.webp');
  });

  it('keeps working against an older core that sends no chat apps', () => {
    const older = { ...ext, chat_clients: undefined };
    expect(wornPreviewBadges(null, older)).toEqual([
      'https://twitch/premium',
      'https://cdn.7tv.app/badge/7/4x.webp',
    ]);
  });
});

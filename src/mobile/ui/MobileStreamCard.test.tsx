// @vitest-environment node
// Row mode shows whose stream it is, as the card does (SN-30).
import { describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { createElement } from 'react';
import type { TwitchStream } from '../../types';

vi.mock('../../components/SevenTvAvatar', async () => {
  const { createElement: h } = await import('react');
  return { SevenTvAvatar: ({ src }: { src: string }) => h('img', { 'data-face': '', src }) };
});
vi.mock('../../stores/AppStore', () => ({
  useAppStore: (select: (s: unknown) => unknown) => select({ collaborations: {}, sharedChats: {} }),
}));
vi.mock('./ChannelHold', () => ({ useChannelHold: () => ({ handlers: {}, sheet: null }) }));
vi.mock('../../services/sevenTvPictures', () => ({ sevenTvIdForStream: () => null }));
vi.mock('../../utils/offscreenPause', () => ({ observeOffscreen: () => undefined }));
vi.mock('../followRefresh', () => ({ previewStamp: () => 1 }));
vi.mock('../../components/SharedViewers', () => ({ TogetherTag: () => null }));
vi.mock('../../components/StreamTitleWithEmojis', () => ({ default: ({ title }: { title: string }) => title }));
vi.mock('../../components/ProviderLogo', () => ({ ProviderMark: () => null }));

import { MobileStreamCard } from './MobileStreamCard';

const stream = {
  id: '1',
  user_id: '1',
  user_login: 'bas',
  user_name: 'Bas',
  title: 'Hello',
  game_name: 'Art',
  viewer_count: 12,
  thumbnail_url: 'https://thumb.test/{width}x{height}.jpg',
  profile_image_url: 'https://face.test/bas.png',
} as unknown as TwitchStream;

const row = (s: TwitchStream) => renderToStaticMarkup(createElement(MobileStreamCard, { stream: s, onPress: () => {}, variant: 'row' }));

describe('stream row', () => {
  it('draws the channel picture before the name', () => {
    const html = row(stream);
    expect(html).toContain('src="https://face.test/bas.png"');
    expect(html.indexOf('https://face.test/bas.png')).toBeLessThan(html.indexOf('>Bas<'));
  });

  it('draws no picture for a row without one', () => {
    expect(row({ ...stream, profile_image_url: '' } as TwitchStream)).not.toContain('data-face');
  });
});

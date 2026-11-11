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
vi.mock('./ChannelHold', () => ({ useChannelHold: () => ({ handlers: {}, sheet: null, consumeHeld: () => false }) }));
vi.mock('../../services/sevenTvPictures', () => ({ sevenTvIdForStream: () => null }));
vi.mock('../../utils/offscreenPause', () => ({ observeOffscreen: () => undefined }));
vi.mock('../followRefresh', () => ({ previewStamp: () => 1 }));
vi.mock('../../components/SharedViewers', () => ({ TogetherTag: () => null }));
vi.mock('../../components/StreamTitleWithEmojis', () => ({ default: ({ title }: { title: string }) => title }));
vi.mock('../../components/ProviderLogo', () => ({ ProviderMark: () => null }));

import { MobileStreamCard } from './MobileStreamCard';
import type { CardTagFilter } from './StreamCardTags';

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

  it('puts Drops on the preview, not in the text column', () => {
    const html = row({ ...stream, drops_earnable: true } as TwitchStream);
    const textColumn = html.indexOf('<h3');
    const chip = html.indexOf('aria-label="Drops enabled"');
    expect(chip).toBeGreaterThan(html.indexOf('sn-card-thumb'));
    expect(chip).toBeLessThan(textColumn);
    expect(html.slice(textColumn)).not.toContain('Drops');
    expect(html.slice(textColumn)).toContain('>Art<');
  });

  it('draws no Drops chip on a row without drops', () => {
    expect(row(stream)).not.toContain('Drops enabled');
  });
});

const card = (s: TwitchStream, tagFilter?: CardTagFilter) =>
  renderToStaticMarkup(createElement(MobileStreamCard, { stream: s, onPress: () => {}, tagFilter }));

describe('card tags (SN-78)', () => {
  const filter: CardTagFilter = { selected: ['English'], onToggle: () => {} };

  it('draws no tag line without a filter', () => {
    const html = card({ ...stream, tags: ['English'] } as TwitchStream);
    expect(html).not.toContain('sn-tag-line');
    expect(html).not.toContain('data-tag-chip');
  });

  it('draws the line after the card button, the picked tag pressed', () => {
    const html = card({ ...stream, tags: ['English', 'Chill'] } as TwitchStream, filter);
    const cardEnd = html.indexOf('</button>');
    expect(html.indexOf('data-tag-chip')).toBeGreaterThan(cardEnd);
    const chips = html.slice(cardEnd).split('data-tag-chip').slice(1);
    expect(chips).toHaveLength(2);
    expect(chips[0]).toContain('aria-pressed="true"');
    expect(chips[0]).toContain('English');
    expect(chips[1]).toContain('aria-pressed="false"');
    expect(chips[1]).toContain('Chill');
  });

  it('reserves the line on every card of the screen, tags or not', () => {
    for (const tags of [undefined, null, []]) {
      const html = card({ ...stream, tags } as unknown as TwitchStream, filter);
      expect(html).toContain('sn-tag-line');
      expect(html).not.toContain('data-tag-chip');
    }
  });
});

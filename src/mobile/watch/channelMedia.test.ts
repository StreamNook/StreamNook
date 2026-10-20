import { describe, expect, it } from 'vitest';
import {
  clock,
  countLine,
  emptyLine,
  mergePage,
  metaLine,
  rateLabel,
  recordedLine,
  seekAt,
  storyboardFrame,
  shelfArgs,
  skipTarget,
  type MediaRow,
} from './channelMedia';

const row = (over: Partial<MediaRow> = {}): MediaRow => ({
  kind: 'video',
  id: '1',
  title: 't',
  thumbnailUrl: '',
  url: 'https://www.twitch.tv/videos/1',
  lengthLabel: '3:12:04',
  ageLabel: '2 days ago',
  viewsLabel: '12K views',
  byline: 'Just Chatting',
  progress: null,
  watched: false,
  recording: false,
  vodId: null,
  vodOffset: null,
  ...over,
});

describe('the Channel sheet, as the iPhone lists it', () => {
  it('asks channel_media with the same arguments', () => {
    expect(shelfArgs({ tab: 'videos', kind: 'highlight', sort: 'popular' }, '42', null)).toEqual({
      channelId: '42',
      kind: 'videos',
      filter: 'highlight',
      sort: 'popular',
      cursor: null,
    });
    expect(shelfArgs({ tab: 'clips', period: '7d' }, '42', 'c1')).toEqual({
      channelId: '42',
      kind: 'clips',
      filter: '7d',
      cursor: 'c1',
    });
  });

  it('joins a row into the same lines', () => {
    expect(metaLine(row())).toBe('2 days ago · 3:12:04');
    expect(countLine(row())).toBe('12K views · Just Chatting');
    const clip = row({ kind: 'clip', byline: 'Clipped by ann', lengthLabel: '0:30' });
    expect(metaLine(clip)).toBe('Clipped by ann · 2 days ago');
    expect(countLine(clip)).toBe('0:30 · 12K views');
    expect(metaLine(row({ byline: null, lengthLabel: '' }))).toBe('2 days ago');
  });

  it('says what an empty shelf holds', () => {
    expect(emptyLine({ tab: 'videos', kind: 'archive', sort: 'recent' })).toBe('No past broadcasts yet');
    expect(emptyLine({ tab: 'clips', period: '30d' })).toBe('No clips in this period');
  });

  it('adds a page without repeating a row', () => {
    const merged = mergePage([row({ id: 'a' }), row({ id: 'b' })], [row({ id: 'b' }), row({ id: 'c' })]);
    expect(merged.map((r) => r.id)).toEqual(['a', 'b', 'c']);
  });
});

describe('the VOD controls', () => {
  it('shows the playhead like the iPhone', () => {
    expect(clock(0)).toBe('0:00');
    expect(clock(65.9)).toBe('1:05');
    expect(clock(3_725)).toBe('1:02:05');
    expect(clock(Number.NaN)).toBe('0:00');
    expect(clock(303, 7_200)).toBe('0:05:03');
  });

  it('keeps a skip inside the video, a second short of its end', () => {
    expect(skipTarget(5, -10, 100)).toBe(0);
    expect(skipTarget(95, 10, 100)).toBe(99);
    expect(skipTarget(50, 10, 0)).toBe(60);
  });

  it('cuts the scrub preview from the right sheet', () => {
    const sb = {
      base_url: 'https://x/',
      variants: [
        { quality: 'low', width: 80, height: 45, rows: 10, cols: 10, count: 150, interval_secs: 10, images: ['l0.jpg', 'l1.jpg'] },
        { quality: 'high', width: 220, height: 124, rows: 5, cols: 5, count: 150, interval_secs: 10, images: ['h0.jpg', 'h1.jpg'] },
      ],
    };
    const f = storyboardFrame(sb, 270);
    expect(f).toMatchObject({ url: 'https://x/h1.jpg', x: 440, y: 0, width: 220, sheetWidth: 1_100 });
    expect(storyboardFrame(sb, 270, 100)?.url).toBe('https://x/l0.jpg');
    expect(storyboardFrame(null, 10)).toBeNull();
  });

  it('says when the broadcast was recorded', () => {
    const now = new Date('2026-10-19T12:00:00Z');
    expect(recordedLine('2026-09-28T18:00:00Z', now)).toBe('Recorded Sep 28');
    expect(recordedLine('2025-09-28T18:00:00Z', now)).toBe('Recorded Sep 28, 2025');
    expect(recordedLine(undefined, now)).toBe('Recorded');
  });

  it('seeks to where the bar is pressed', () => {
    expect(seekAt(150, 100, 200, 3_600)).toBe(900);
    expect(seekAt(50, 100, 200, 3_600)).toBe(0);
    expect(seekAt(400, 100, 200, 3_600)).toBe(3_600);
    expect(seekAt(150, 100, 0, 3_600)).toBe(0);
  });

  it('names speeds as the iPhone does', () => {
    expect([1, 1.25, 1.5, 2].map(rateLabel)).toEqual(['1x', '1.25x', '1.5x', '2x']);
  });
});

import { describe, expect, it } from 'vitest';
import { isSoloAdapt, type PlaybackAdapt } from './linkAdapt';
import { qualityLabel } from './quality';

const event = (streamId?: string): PlaybackAdapt => ({
  streamId,
  quality: '720p60',
  auto: true,
  extraCushion: 0,
  url: null,
  reason: 'throughput below the bitrate',
});

describe('playback-adapt routing', () => {
  it('sends solo events (and older cores with no id) to the solo player', () => {
    expect(isSoloAdapt(event('solo'))).toBe(true);
    expect(isSoloAdapt(event(undefined))).toBe(true);
  });

  it('sends a tile id to that tile', () => {
    expect(isSoloAdapt(event('slot-3'))).toBe(false);
  });
});

describe('quality labels', () => {
  it('reads auto as Auto, naming the rendition it plays once Rust has said', () => {
    expect(qualityLabel('auto')).toBe('Auto');
    expect(qualityLabel('auto', null)).toBe('Auto');
    expect(qualityLabel('auto', '720p60')).toBe('Auto (720p60)');
  });

  it('reads best as the fixed source on the desktop and as Auto on the phones', () => {
    expect(qualityLabel('best', null, false)).toBe('Source');
    expect(qualityLabel('source', null, false)).toBe('Source');
    expect(qualityLabel('best', '720p60', true)).toBe('Auto (720p60)');
    expect(qualityLabel('best', null, true)).toBe('Auto');
  });

  it('leaves a named quality as it is, capitalized', () => {
    expect(qualityLabel('1080p60')).toBe('1080p60');
    expect(qualityLabel('worst')).toBe('Worst');
    expect(qualityLabel('1080p60', '720p60')).toBe('1080p60');
  });
});

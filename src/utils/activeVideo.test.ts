import { describe, expect, it } from 'vitest';
import { setActiveVideo, setVideoSource, sourceOfVideo, type PlayerSource } from './activeVideo';

const mainSource: PlayerSource = { provider: 'twitch', channel_login: 'xqc', channel_name: 'xQc', media: 'live' };
const tileSource: PlayerSource = { provider: 'kick', channel_login: 'adinross', channel_name: null, media: 'live' };

describe('where a captured song was playing', () => {
  it("takes a tile's own source", () => {
    const tile = {} as HTMLVideoElement;
    setVideoSource(tile, tileSource);
    expect(sourceOfVideo(tile, () => mainSource)).toEqual(tileSource);
  });

  it("reads the main player's stream only for the main player", () => {
    const main = {} as HTMLVideoElement;
    const stray = {} as HTMLVideoElement;
    setActiveVideo(main);
    expect(sourceOfVideo(main, () => mainSource)).toBe(mainSource);
    expect(sourceOfVideo(stray, () => mainSource)).toBeNull();
    setActiveVideo(null);
  });

  it('forgets a tile that left', () => {
    const tile = {} as HTMLVideoElement;
    setVideoSource(tile, tileSource);
    setVideoSource(tile, null);
    expect(sourceOfVideo(tile, () => mainSource)).toBeNull();
  });
});

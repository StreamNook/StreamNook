import { describe, expect, it } from 'vitest';
import { missingCodecsWords, missingTwitchCodecs } from './codecPreference';

describe('missingTwitchCodecs', () => {
  it('names nothing when the engine decodes H.264 and AAC', () => {
    expect(missingTwitchCodecs(() => true)).toEqual([]);
  });

  it('names both on an engine without proprietary codecs (prebuilt CEF)', () => {
    // The prebuilt CEF plays AV1 and Opus but no H.264 or AAC.
    const stock = (type: string) => type.includes('av01') || type.includes('opus');
    expect(missingTwitchCodecs(stock)).toEqual(['H.264', 'AAC']);
  });

  it('accepts either H.264 profile and survives a probe that throws', () => {
    expect(missingTwitchCodecs((t) => t.includes('avc1.42e01e') || t.includes('mp4a'))).toEqual([]);
    expect(
      missingTwitchCodecs(() => {
        throw new Error('no MediaSource');
      }),
    ).toEqual(['H.264', 'AAC']);
  });

  it('says what is missing in plain words', () => {
    expect(missingCodecsWords(['H.264', 'AAC'])).toContain("can't decode H.264 and AAC");
  });
});

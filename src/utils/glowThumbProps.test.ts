import { describe, expect, it } from 'vitest';

import { glowThumbProps } from './mediaGlow';

describe('glowThumbProps', () => {
  it('never puts crossOrigin on the image the user sees', () => {
    // The regression this exists for: `crossOrigin` was being spread onto the
    // displayed thumbnail so the glow could read its pixels. When the CORS
    // request failed, the image failed with it and the card fell back to its
    // placeholder — a decorative tint breaking the actual content. Sampling now
    // happens on a separate, undisplayed copy that is allowed to fail.
    for (const url of [
      'https://i.ytimg.com/vi/abc/hqdefault.jpg',
      'https://static-cdn.jtvnw.net/previews-ttv/live_user_x-640x360.jpg',
      'https://files.kick.com/thumb.jpg',
      undefined,
    ]) {
      expect(Object.keys(glowThumbProps(url))).not.toContain('crossOrigin');
    }
  });

  it('still hands back the hooks the cards need', () => {
    const p = glowThumbProps('https://i.ytimg.com/vi/abc/hqdefault.jpg');
    expect(typeof p.onLoad).toBe('function');
    expect(typeof p.ref).toBe('function');
  });
});

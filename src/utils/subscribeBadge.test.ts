import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { subscribeBadgePending } from './subscribeBadge';

const read = (rel: string) => readFileSync(resolve(__dirname, '..', rel), 'utf8');

describe('subscribeBadgePending', () => {
  it('waits for Twitch and Kick art, never for YouTube', () => {
    expect(subscribeBadgePending('twitch', null)).toBe(true);
    expect(subscribeBadgePending('kick', '')).toBe(true);
    expect(subscribeBadgePending('kick', 'data:image/png;base64,x')).toBe(false);
    expect(subscribeBadgePending('youtube', null)).toBe(false);
  });

  it('the hook asks Rust for the art and every desktop control keeps the slot', () => {
    expect(read('hooks/useChannelSocial.ts')).toContain("'channel_sub_badge'");
    // Asked only while no art is known, so a cold MultiNook makes no extra calls.
    expect(read('hooks/useChannelSocial.ts')).toContain('const artKey = !knownArt &&');
    for (const rel of ['components/VideoPlayer.tsx', 'components/multi-nook/MultiNookCell.tsx', 'components/ChannelAboutReveal.tsx']) {
      expect(read(rel), rel).toContain('subscriberBadgePending ?');
    }
  });
});

import { describe, expect, it } from 'vitest';
import { animateCard, animateRow, pick, plateVideo } from './discordPreviewLook';

const pair = { animated: 'https://cdn.discordapp.com/a.webp?animated=true', still: 'https://cdn.discordapp.com/a.webp' };

describe('discord preview look', () => {
  it('draws the moving picture only when it may move', () => {
    expect(pick(pair, true)).toBe(pair.animated);
    expect(pick(pair, false)).toBe(pair.still);
    expect(pick(null, true)).toBeNull();
  });

  it('moves the card only with Motion on Full, the default', () => {
    expect(animateCard(undefined)).toBe(true);
    expect(animateCard('full')).toBe(true);
    expect(animateCard('reduced')).toBe(false);
    expect(animateCard('off')).toBe(false);
  });

  it('moves the member-list row only while hovered', () => {
    expect(animateRow('full', false)).toBe(false);
    expect(animateRow('full', true)).toBe(true);
    expect(animateRow('reduced', true)).toBe(false);
  });

  it('plays the nameplate video only off macOS and while moving', () => {
    expect(plateVideo('v.webm', true, false)).toBe('v.webm');
    expect(plateVideo('v.webm', true, true)).toBeNull();
    expect(plateVideo('v.webm', false, false)).toBeNull();
    expect(plateVideo(null, true, false)).toBeNull();
  });
});

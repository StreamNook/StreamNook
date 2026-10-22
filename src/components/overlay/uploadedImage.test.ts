// Uploaded pictures draw on the overlay only from StreamNook's upload hosts,
// and under "Trusted chatters" only for the channel's roles and supporters.

import { describe, expect, it } from 'vitest';
import { isTrustedSender, isUploadedImage } from './uploadedImage';

describe('isUploadedImage', () => {
  it.each([
    'https://i.nuuls.com/a.png',
    'https://files.catbox.moe/x.GIF',
    'https://litter.catbox.moe/y.jpg',
    'https://d.uguu.se/z.webp',
    'https://kappa.lol/AbC.png',
    'https://imgdrip.com/i/x.webp',
  ])('draws %s', (url) => {
    expect(isUploadedImage(url)).toBe(true);
  });

  it.each([
    'http://i.nuuls.com/a.png',
    'https://evilnuuls.com/a.png',
    'https://i.nuuls.com/a.html',
    'https://imgur.com/a.png',
    'not a url',
  ])('leaves %s as a link', (url) => {
    expect(isUploadedImage(url)).toBe(false);
  });
});

describe('isTrustedSender', () => {
  it.each(['broadcaster', 'lead_moderator', 'moderator', 'vip', 'subscriber', 'founder', 'og', 'owner', 'member'])(
    'trusts a %s badge',
    (name) => {
      expect(isTrustedSender([{ name }])).toBe(true);
    },
  );

  it('ignores case and finds the role among other badges', () => {
    expect(isTrustedSender([{ name: 'bits' }, { name: 'Moderator' }])).toBe(true);
  });

  it('does not trust chatters without a role or support badge', () => {
    expect(isTrustedSender(undefined)).toBe(false);
    expect(isTrustedSender([])).toBe(false);
    expect(isTrustedSender([{ name: 'bits' }, { name: 'premium' }, { name: 'verified' }, { name: 'sub_gifter' }, { name: 'tiktok' }])).toBe(false);
  });
});

// @vitest-environment node
import { describe, expect, it } from 'vitest';
import { canEarnHere } from './channelEarnable';

// The Golden Frog Badge (2026-11-13): allow-listed to QTCinderella, filed under
// The Streamer Awards. She was live there, but Twitch's per-channel campaign
// list named a different campaign, so the picker showed nobody.
const QT = { userId: '247808909', currentGame: 'The Streamer Awards', currentGameId: '24537180' };
const FROG = {
  isAclBased: true,
  gameId: '24537180',
  gameName: 'The Streamer Awards',
};

describe('canEarnHere', () => {
  it('an allow-listed channel live in the campaign category earns, whatever the per-channel list says', () => {
    expect(canEarnHere(QT, { ...FROG, eligibleIds: new Set(['someone-else']) })).toBe(true);
    expect(canEarnHere(QT, { ...FROG, eligibleIds: new Set() })).toBe(true);
  });

  it('a creator campaign Twitch names on the channel earns in any category', () => {
    const elsewhere = { ...QT, currentGame: 'Just Chatting', currentGameId: '509658' };
    expect(canEarnHere(elsewhere, { ...FROG, eligibleIds: new Set([QT.userId]) })).toBe(true);
  });

  it('an allow-listed channel in another category that Twitch does not name does not earn', () => {
    const elsewhere = { ...QT, currentGame: 'Just Chatting', currentGameId: '509658' };
    expect(canEarnHere(elsewhere, { ...FROG, eligibleIds: new Set() })).toBe(false);
  });

  it('any of several counted categories is enough', () => {
    const other = { userId: '1', currentGame: 'Pokemon GO', currentGameId: '490655' };
    expect(canEarnHere(other, { ...FROG, eligibleIds: null, categoryIds: ['490655'] })).toBe(true);
  });

  it('falls back to the category name when the channel has no category id', () => {
    expect(canEarnHere({ userId: '1', currentGame: 'the streamer awards' }, { ...FROG, gameId: undefined, eligibleIds: null })).toBe(true);
  });

  it('an open campaign list is already category-correct', () => {
    expect(canEarnHere({ userId: '1' }, { isAclBased: false, eligibleIds: new Set() })).toBe(true);
  });
});

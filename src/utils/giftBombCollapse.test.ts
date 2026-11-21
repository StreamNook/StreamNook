import { describe, expect, it } from 'vitest';
import { isGiftBombChild, isGiftToViewer } from './giftBombCollapse';

describe('gift bomb collapse', () => {
  it('keeps the gift to the viewer out of the fold', () => {
    // Rust stamps the viewer's own gift a mention; everyone else's is not.
    const mine = { metadata: { msg_type: 'subgift', is_mentioned: true } };
    const theirs = { metadata: { msg_type: 'subgift', is_mentioned: false } };
    expect(isGiftBombChild(mine.metadata.msg_type) && isGiftToViewer(mine)).toBe(true);
    expect(isGiftToViewer(theirs)).toBe(false);
    expect(isGiftToViewer({ metadata: null })).toBe(false);
    expect(isGiftToViewer(undefined)).toBe(false);
  });
});

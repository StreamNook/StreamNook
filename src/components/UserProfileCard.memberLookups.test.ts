// A chatter's id is only a Twitch id on Twitch. A Kick or YouTube chatter's id is
// a number from the same range Twitch ids come from, so a lookup that takes it
// raw shows an unrelated Twitch member's BetterTTV Pro badge and curated badge
// list on a stranger's card (Kick user 676 wore Twitch user 676's). Both
// lookups go through `cardMemberId` (the Twitch id of the member who claimed
// this chat identity, null when nobody has). A node test cannot render the
// card, so this pins the calls in its source.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const source = readFileSync(new URL('./UserProfileCard.tsx', import.meta.url), 'utf8').replace(/\r\n/g, '\n');

describe('UserProfileCard member lookups', () => {
  it("asks BetterTTV Pro about the claimed member's Twitch id, and only when there is one", () => {
    expect(source).toContain("'get_bttv_pro_badge', { userId: cardMemberId }");
    expect(source).not.toMatch(/'get_bttv_pro_badge', \{ userId \}/);
    const effect = /setBttvProBadge\(null\);\n\s*(.*)\n/.exec(source);
    expect(effect?.[1]).toContain('if (!cardMemberId) return;');
  });

  it('reads the curated badge list of the claimed member, not of the chatter id', () => {
    expect(source).toContain('const identityLoadout = cardMemberId ? getIdentityFromMemoryCache(cardMemberId) : null;');
    expect(source).toContain('if (cardMemberId) void getIdentityWithCache(cardMemberId).catch(() => {});');
    expect(source).not.toContain('getIdentityFromMemoryCache(userId)');
    expect(source).not.toContain('getIdentityWithCache(userId)');
  });
});

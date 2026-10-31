// The suite has no DOM, so the wiring the Android favorites work depends on is
// guarded in the sources: one hold sheet for every card, Subscribe never
// falls back to a stand-in glyph, and every boot loads Rust's lists.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const read = (rel: string) => readFileSync(resolve(__dirname, rel), 'utf8').replace(/\r\n/g, '\n');

describe('android favorites wiring', () => {
  it('cards render no sheet; MobileApp mounts the one hold sheet once', () => {
    expect(read('ui/MobileStreamCard.tsx')).not.toContain('hold.sheet');
    const app = read('MobileApp.tsx');
    expect(app.split('<ChannelHoldSheet />').length - 1).toBe(1);
    const hold = read('ui/ChannelHold.tsx');
    const sheetAt = hold.indexOf('export const ChannelHoldSheet');
    expect(sheetAt).toBeGreaterThan(0);
    // The only MobileSheet lives in the hosted sheet, after the card hook.
    expect(hold.indexOf('<MobileSheet')).toBeGreaterThan(sheetAt);
  });

  it('the chat menu offers Favorite before Add a chat and draws Subscribe from badge art only', () => {
    const menu = read('chat/ComposerMenuSheet.tsx');
    const favoriteAt = menu.indexOf('{favoriteLabel(isFavorite)}');
    const addChatAt = menu.indexOf('Add a chat\n');
    expect(favoriteAt).toBeGreaterThan(0);
    expect(addChatAt).toBeGreaterThan(favoriteAt);
    expect(menu).toContain('<SubscribeBadge');
    expect(menu).not.toContain('PROVIDERS[provider].color');
  });

  it("the player's Subscribe pill keeps a slot instead of a star", () => {
    const player = read('player/MobilePlayer.tsx');
    expect(player).toContain('<SubscribeBadge');
    expect(player).not.toContain('<Star size={12} weight="fill" />');
  });

  it('boot loads the home snapshot whatever the start tab', () => {
    expect(read('boot/useMobileBoot.ts')).toContain('void ensureHomeSnapshotSync();');
  });

  it('Following reads its shelf and list from followingSections', () => {
    expect(read('screens/FollowingScreen.tsx')).toContain('followingSections(following, activeFilter)');
  });
});

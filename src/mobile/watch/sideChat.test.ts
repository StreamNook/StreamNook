// SN-268: the phone's side chat goes narrower, and a bar with no room folds.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  COMPOSER_FOLD_BELOW,
  PHONE_SIDE_CHAT_MIN,
  WRITING_WIDTH,
  composerFolds,
  sideChatMin,
  sideChatShownWidth,
} from './sideChat';

const read = (rel: string) => readFileSync(resolve(__dirname, rel), 'utf8');

describe('side chat width (SN-268)', () => {
  it('lets the phone go narrower than the desktop overlay', () => {
    expect(sideChatMin(true)).toBe(PHONE_SIDE_CHAT_MIN);
    expect(sideChatMin(true)).toBeLessThan(sideChatMin(false));
    expect(sideChatMin(false)).toBe(240);
  });

  it('folds the bar only below its room', () => {
    expect(composerFolds(PHONE_SIDE_CHAT_MIN)).toBe(true);
    expect(composerFolds(COMPOSER_FOLD_BELOW - 1)).toBe(true);
    expect(composerFolds(COMPOSER_FOLD_BELOW)).toBe(false);
    expect(composerFolds(340)).toBe(false);
  });

  it('opens a folded column to its writing width while writing, and only then', () => {
    expect(sideChatShownWidth(150, 600, false)).toBe(150);
    expect(sideChatShownWidth(150, 600, true)).toBe(WRITING_WIDTH);
    // Never past the room the picture leaves.
    expect(sideChatShownWidth(150, 280, true)).toBe(280);
    // Never narrower than the saved width.
    expect(sideChatShownWidth(150, 120, true)).toBe(150);
    // A column with room for the bar never changes.
    expect(sideChatShownWidth(300, 600, true)).toBe(300);
  });
});

describe('wiring (source pins)', () => {
  const watch = read('../screens/WatchScreen.tsx');
  const settings = read('../../components/settings/ChatSettings.tsx');

  it('clamps the watch column and its handles to the phone floor', () => {
    expect(watch).not.toMatch(/minWidth=\{240\}/);
    expect(watch).toContain('Math.max(PHONE_SIDE_CHAT_MIN, overlayWidthLive ?? fsChat?.width ?? 340)');
  });

  it('folds the bar beside the video and widens the column while writing', () => {
    expect(watch).toContain('composerFold={composerFold}');
    expect(watch).toContain('sideChatShownWidth(overlayWidth, overlayMaxWidth, writing)');
  });

  it("gives the phone's Width slider the phone floor", () => {
    expect(settings).toContain('sideChatMin(IS_MOBILE)');
  });
});

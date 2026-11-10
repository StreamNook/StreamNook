import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const read = (p: string) => readFileSync(resolve(__dirname, '..', p), 'latin1');

// Every menu that opens upward out of the desktop message box sits over the
// drops lip on the box's top edge (z-[55]), never under it.
const FLYOUTS = [
  'chat/EmotePickerPanel.tsx',
  'chat/ChannelWallet.tsx',
  'ChannelPointsMenu.tsx',
  'chat/managerKit.tsx',
  'SendAsPicker.tsx',
  'chat/CommandAutocomplete.tsx',
  'MentionAutocomplete.tsx',
  'chat/EmoteAutocomplete.tsx',
  'chat/EmoteSearchList.tsx',
  'chat/ChannelToolsMenu.tsx',
  'chat/CheerBar.tsx',
];

describe('menus over the message box', () => {
  it('the lip sits at z-[55]', () => {
    expect(read('chat/DropsChatBar.tsx')).toContain('`relative z-[55] -mb-px');
  });
  for (const file of FLYOUTS) {
    it(`${file} opens above the lip`, () => {
      const src = read(file);
      const flyout = src.split('\n').filter((l) => l.includes('glass-flyout') && /absolute/.test(l));
      expect(flyout.length, file).toBeGreaterThan(0);
      for (const line of flyout) expect(line, file).not.toMatch(/\bz-(10|20|30|40|50)\b/);
    });
  }
});

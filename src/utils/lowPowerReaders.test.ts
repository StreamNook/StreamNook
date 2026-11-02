// The still readers follow the still flag (Battery Saver, Freeze emotes), never
// quiet, so Performance mode without Freeze emotes keeps chat moving; Motion's
// step-down follows Battery Saver only, so Performance mode never swaps the
// Reduced rules on a focus change.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(__dirname, '..', '..');
const read = (p: string) => readFileSync(resolve(root, p), 'utf8');

const STILL_READERS = [
  'src/components/ChatMessage.tsx',
  'src/hooks/useChatterLook.ts',
  'src/components/chat/ChatBadgeIcons.tsx',
  'src/components/chat/EmotePickerPanel.tsx',
];

describe('cheap-draw readers', () => {
  it.each(STILL_READERS)('%s reads useStillChat, not useLowPower', (file) => {
    const src = read(file);
    expect(src).toMatch(/\buseStillChat\(\)/);
    expect(src).not.toMatch(/\buseLowPower\b/);
  });

  it('MotionScope steps Motion down from Battery Saver only', () => {
    const src = read('src/components/MotionScope.tsx');
    expect(src).toMatch(/\buseBatteryLowPower\(\)/);
    expect(src).not.toMatch(/\buseLowPower\b/);
  });

  it('ChatMessage passes still to both chat-row pictures', () => {
    const src = read('src/components/ChatMessage.tsx');
    expect(src.match(/animation=\{stillEmotes \? 'never' : undefined\}/g)).toHaveLength(2);
  });
});

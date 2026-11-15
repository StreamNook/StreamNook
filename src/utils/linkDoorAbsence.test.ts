import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';

const SRC = resolve(__dirname, '..');

function sourceFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...sourceFiles(full));
    else if (/\.(ts|tsx)$/.test(entry.name)) out.push(full);
  }
  return out;
}

const files = sourceFiles(SRC).map((full) => ({
  name: relative(SRC, full).replace(/\\/g, '/'),
  text: readFileSync(full, 'utf8'),
}));

// Every needle is built from pieces, so this file never holds a dead name
// itself and the scan over src/ cannot find it here. Some needles never
// existed at HEAD (the boot gate, the notice runner): absence pins of
// superseded names are guards and pass on HEAD by design.
// A needle matches as a whole identifier, so a longer private name that
// merely starts with it (a reader's answer type) is not a hit.
const NEEDLES: string[] = [
  'readBadge' + 'Link',
  'createBoot' + 'Gate',
  'runNotice' + 'Action',
  'badgesOverlayInitial' + 'TwitchBadge',
  'consumeInitial' + 'TwitchBadge',
  'pendingWatch' + 'ChannelRef',
  'pendingBadge' + 'LinkRef',
  'take_pending_' + 'watch_link',
  'take_pending_' + 'badge_link',
  'streamnook:' + 'watch',
  'streamnook:' + 'badge',
];

describe('the old link doors are gone from src', () => {
  // A wrong path or an empty walk would otherwise pass every needle.
  it('scans files, and its positive control finds the one drain in App.tsx', () => {
    expect(files.length).toBeGreaterThan(0);
    const app = files.find((file) => file.name === 'App.tsx');
    expect(app).toBeDefined();
    expect(app?.text).toContain('startLinkDrain');
  });

  it('finds a needle that is present (the matcher works)', () => {
    const needle = 'take_pending_' + 'link';
    expect(files.some((file) => file.text.includes(needle))).toBe(true);
  });

  it.each(NEEDLES.map((needle, index) => [index, needle] as const))('old name %i appears in no file', (_index, needle) => {
    const whole = new RegExp('\\b' + needle + '\\b');
    expect(files.filter((file) => whole.test(file.text)).map((file) => file.name)).toEqual([]);
  });
});

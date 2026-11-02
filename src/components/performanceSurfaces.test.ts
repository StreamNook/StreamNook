// Performance mode's flat look stops the player glow on both players without
// touching the setting, and the Immersive button only goes away in Always, so
// the title bar never shifts when the app comes back to the front.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(__dirname, '..', '..');
const read = (p: string) => readFileSync(resolve(root, p), 'utf8').replace(/\r\n/g, '\n');

describe('the glow under the flat look', () => {
  it('the main player stops sampling while flat', () => {
    const src = read('src/components/VideoPlayer.tsx');
    expect(src).toContain('const flat = useFlat();');
    expect(src).toContain('const glowOn = settings?.media_glow !== false && !flat;');
  });

  it('MultiNook tiles stop sampling while flat', () => {
    const src = read('src/components/multi-nook/MultiNookCell.tsx');
    expect(src).toContain('const flat = useFlat();');
    expect(src).toContain("useAppStore((s) => s.settings.media_glow !== false) && !flat;");
  });
});

describe('the Immersive button', () => {
  it('hides only while Performance mode is Always', () => {
    const src = read('src/components/TitleBar.tsx');
    expect(src).toContain(
      "{streamUrl && settings?.media_glow !== false && settings?.performance_mode !== 'always' && (",
    );
    expect(src).not.toMatch(/useFlat\(\)/);
  });
});

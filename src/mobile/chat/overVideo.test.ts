// Chat over the video (SN-40) takes dark ink in a light theme and looks exactly
// as today in a dark one. The variables come from Frosted Glass, cover every
// color variable the theme paints, follow Glassiness, and reach the lower
// third's list and the landscape overlay's column; names and the shared-chat
// mark over video are adjusted for a dark backing. Source pins read the files
// as text (CRLF-normalized), as the other wiring tests here do.
import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }));

import { overVideoStyle, overVideoVars } from './overVideo';
import { applyGlassStrength, getThemeById, glassSurfaces, phoneFloors } from '../../themes';
import { adjustNameColor, nameColorReadsLight } from '../../utils/nameColor';

const read = (rel: string) => readFileSync(resolve(__dirname, rel), 'utf8').replace(/\r\n/g, '\n');

describe('over-video ink', () => {
  it('covers every color variable the theme paints', () => {
    const themes = read('../../themes/index.ts');
    // Every color, and the palette's glass opacities; not the font, weight or glass strength.
    const painted = new Set(
      [...themes.matchAll(/setProperty\('(--(?:color-[a-z0-9-]+|glass-[a-z-]*opacity))'/g)].map((m) => m[1]),
    );
    expect(painted.size).toBeGreaterThan(20);
    const vars = overVideoVars();
    expect([...painted].filter((v) => !(v in vars))).toEqual([]);
  });

  it('is Frosted Glass, flattened for the phone like the theme itself', () => {
    const p = getThemeById('winters-glass')!.palette;
    const floors = phoneFloors(p.background, p.backgroundTertiary);
    const vars = overVideoVars();
    expect(vars['--color-background']).toBe(floors.background);
    expect(vars['--color-background-tertiary']).toBe(floors.tertiary);
    expect(vars['--color-text-primary']).toBe(p.textPrimary);
    expect(vars['--color-text-muted']).toBe(p.textMuted);
    expect(vars['--color-accent']).toBe(p.accent);
    expect(vars['--color-accent-rgb']).toBe('151, 177, 185');
    expect(nameColorReadsLight(vars['--color-background'])).toBe(false);
    expect(nameColorReadsLight(vars['--color-text-primary'])).toBe(true);
  });

  it('paints its surfaces at the glassiness on screen, solid at 0%', () => {
    const p = getThemeById('winters-glass')!.palette;
    const tertiary = phoneFloors(p.background, p.backgroundTertiary).tertiary;
    // The slider writes to <html>; this suite runs without a DOM.
    vi.stubGlobal('document', { documentElement: { style: { setProperty: () => {} }, setAttribute: () => {} } });
    applyGlassStrength(0);
    const flat = overVideoVars()['--color-surface'];
    expect(flat).toBe(glassSurfaces(p, tertiary).surface);
    expect(flat).toMatch(/, 1\.000\)$/);
    applyGlassStrength(100);
    expect(overVideoVars()['--color-surface']).not.toBe(flat);
    vi.unstubAllGlobals();
  });

  it('changes nothing for a dark theme', () => {
    expect(overVideoStyle(true)).toBeUndefined();
    expect(overVideoStyle(false)).toMatchObject({ '--color-background': overVideoVars()['--color-background'] });
  });

  it('reaches the lower third list and the landscape overlay column, not the composer', () => {
    const pane = read('./MobileChatPane.tsx');
    expect(pane).toContain('const lowerInk = useOverVideoStyle(folded);');
    expect(pane).toMatch(/ref=\{listRef\}[\s\S]{0,400}\.\.\.lowerInk/);
    expect(pane).toContain("${folded ? ` sn-chat-vertical ${OVER_VIDEO_CLASS}` : ''}");
    const watch = read('../screens/WatchScreen.tsx');
    expect(watch).toContain('const overlayInk = useOverVideoStyle(chatOverlay);');
    expect(watch).toMatch(/: chatOverlay\n\s*\? \(\{[\s\S]{0,200}\.\.\.overlayInk,/);
    expect(watch).toContain('sn-chat-overlay sn-chat-reveal ${OVER_VIDEO_CLASS}');
  });

  it('keeps the light-theme rules that reach into chat on their dark-theme look over video', () => {
    const css = read('../mobile.css');
    expect(css).toContain(":root[data-theme-tone='light'] .sn-mobile .sn-over-video .glaze-selected {\n  background-color: rgb(0 0 0 / 0.72);");
    expect(css).toContain(":root[data-theme-tone='light'] .sn-mobile .sn-over-video .sn-error-text {");
  });

  it('adjusts names and the shared-chat mark over video for the dark backing in a light theme', () => {
    const pane = read('./MobileChatPane.tsx');
    expect(pane).toContain('const nameTone = useDeferredValue(overVideo && !themeDark ? true : null);');
    expect(pane).toContain('<NameColorToneContext.Provider value={nameTone}>');
    const hook = read('../../hooks/useNameColor.ts');
    expect(hook).toContain('adjustNameColor(color, mode, tone ?? isThemeDark())');
    const row = read('../../components/ChatMessage.tsx');
    expect(row).toContain('const sourceDark = useContext(NameColorToneContext) ?? isThemeDark();');
    // A navy name is darkened for a light page and lifted over video.
    expect(nameColorReadsLight(adjustNameColor('#000080', 'hsl_loop', true))).toBe(true);
    expect(nameColorReadsLight(adjustNameColor('#000080', 'hsl_loop', false))).toBe(false);
  });
});

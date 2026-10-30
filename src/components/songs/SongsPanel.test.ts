// The song page and the list, read from source where node cannot draw them
// (no DOM here): the wash replaces the blurred cover glow, the edge fades
// carry the wash cut to their own box, the list is laid out in groups with
// exact heights, a row's cover tint shows only where there is a pointer, the
// heading is Inter with no mark tile, the scrollbars are the panel's own thin
// ones, and the moves are opacity and transform only.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const src = readFileSync(resolve(__dirname, 'SongsPanel.tsx'), 'utf8');
const host = readFileSync(resolve(__dirname, 'SongsPanelHost.tsx'), 'utf8');
const scroll = readFileSync(resolve(__dirname, 'songsScroll.css'), 'utf8');

describe("a song's page", () => {
  it('has no blurred copy of the cover: the wash took its place', () => {
    expect(src).not.toMatch(/blur-xl/);
    expect(src).toMatch(/useCoverFit\(song\?\.tint, kind, surface\.w, surface\.h\)/);
    expect(src).toMatch(/washBackground\(fit, surface, \{ x: 0, y: 0, w: surface\.w, h: surface\.h \}\)/);
  });

  it('its edge fades paint the wash cut to their own strip, fading in with it', () => {
    expect(src).toMatch(/washBackground\(fit, surface, \{ x: 0, y, w: surface\.w, h: FADE_PX \}\)/);
    expect(src).toMatch(/wash=\{washAt\(bodyTop\)\}\s*fadeIn=\{late\}/);
    expect(src).toMatch(/wash=\{washAt\(bodyBottom - FADE_PX\)\}\s*fadeIn=\{late\}/);
  });

  it('reads its song again when its cover gets its colors', () => {
    expect(src).toMatch(/const what = detailChange\(change, songKey\);/);
  });

  it('the artist and the section marks take the cover’s ink', () => {
    expect(src).toMatch(/style=\{ink \? \{ color: ink \} : undefined\}/);
    expect(src).toMatch(/style=\{mark \?\? MARK_STYLE\}/);
  });

  it('slides in over the list, which fades back, and plays it in reverse', () => {
    expect(src).toMatch(/initial=\{\{ opacity: 0, x: 24 \}\}/);
    expect(src).toMatch(/exit=\{\{ opacity: 0, x: 24,/);
    expect(src).toMatch(/\{ opacity: 0, x: -12, transitionEnd: \{ visibility: 'hidden' \} \}/);
  });

  it('a fault in the panel (or a QR dialog drawn from it) closes it and leaves the app running', () => {
    expect(host).toContain("import ErrorBoundary from '../ErrorBoundary';");
    expect(host).toMatch(/<ErrorBoundary[\s\S]*resetKeys=\{\[panel\]\}[\s\S]*fallbackRender=\{\(\) => <ClosedAfterFault \/>\}[\s\S]*<PanelHost \/>/);
    expect(host).toMatch(/function ClosedAfterFault\(\) \{[\s\S]*useEffect\(\(\) => closeSongs\(\), \[closeSongs\]\);[\s\S]*return null;/);
  });

  it('its cover is larger in the larger panel', () => {
    expect(src).toContain('const cover = screen ? 116 : 152;');
    expect(host).toContain('h-[720px]');
    expect(host).toContain('w-[600px]');
    expect(host).toContain('initial={{ scale: 0.97, opacity: 0 }}');
  });
});

describe('the list', () => {
  it('is laid out from Rust’s groups with every height known up front', () => {
    expect(src).toMatch(/VariableSizeList as List/);
    expect(src).toMatch(/makeLayout\(matched \?\? 0, groups, labels, !query, rowHeight\)/);
    expect(src).toMatch(/list\.resetAfterIndex\(0, true\)/);
  });

  it("a row's hover takes its cover's tint only where there is a pointer, and a press gives", () => {
    expect(src).toContain('[@media(hover:hover)]:hover:bg-[color:var(--row-tint)]');
    expect(src).not.toMatch(/hover:bg-glass-hover focus-visible:bg-glass-hover/);
    expect(src).toMatch(/const tint = data\.screen \? null : rowTint\(row\.tint, readSurface\(data\.kind\)\);/);
    expect(src).toContain('[html[data-motion=full]_&]:active:scale-[0.985]');
  });

  it('a song filed while scrolled waits behind the pill, which drops in', () => {
    expect(src).toMatch(/held === 1 \? 'New song'/);
    expect(src).toMatch(/initial=\{\{ opacity: 0, y: -10 \}\}/);
  });

  it('its heading is Inter, with no mark tile', () => {
    expect(src).toContain(`fontFamily: '"Inter", var(--app-font, sans-serif)'`);
    expect(src).toMatch(/style=\{HEADING\}/);
    expect(src).not.toContain('PanelMark');
  });
});

describe('the scrollbars', () => {
  it('are the panel’s own: thin, rounded, clear of the corners, the gutter kept', () => {
    expect(src).toContain("import './songsScroll.css';");
    expect(src).not.toContain('custom-scrollbar');
    expect(scroll).toMatch(/\.sn-songs-scroll::-webkit-scrollbar \{\s*width: 6px;/);
    expect(scroll).toMatch(/margin-top: 14px;\s*margin-bottom: 14px;/);
    expect(scroll).toMatch(/scrollbar-gutter: stable;/);
    // A standard scrollbar property would switch the webkit parts off in Chromium.
    expect(scroll.replace(/\/\*[\s\S]*?\*\//g, '')).not.toMatch(/scrollbar-(width|color)\s*:/);
  });
});

// The name swap that plays when you pick a paint (Profile > Customize) must keep
// the paint's drop-shadows. A CSS animation beats the inline style, so a swap
// that holds its end frame, or ends on `filter: none`, wipes the glow until the
// page is left. Read from the real stylesheet: jsdom runs no animations.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { computePaintStyleUncached, paintSwapStyle, type PaintV4 } from './paintStyle';

const css = readFileSync(resolve(__dirname, '../styles/globals.css'), 'utf8');
const rule = (selector: string) => {
  const at = css.indexOf(`${selector} {`);
  expect(at, `${selector} is in globals.css`).toBeGreaterThan(-1);
  return css.slice(at, css.indexOf('\n}', at));
};
const keyframes = (name: string) => {
  const at = css.indexOf(`@keyframes ${name} {`);
  expect(at, `@keyframes ${name} is in globals.css`).toBeGreaterThan(-1);
  return css.slice(at, css.indexOf('\n}', at));
};

const SHADOWED: PaintV4 = {
  id: 'p',
  name: 'Glow',
  data: {
    layers: [
      {
        id: 'l',
        opacity: 0.8,
        ty: {
          __typename: 'PaintLayerTypeLinearGradient',
          angle: 90,
          stops: [
            { at: 0, color: { hex: '#ff0000', r: 255, g: 0, b: 0, a: 255 } },
            { at: 1, color: { hex: '#0000ff', r: 0, g: 0, b: 255, a: 255 } },
          ],
        },
      },
    ],
    shadows: [{ offsetX: 0, offsetY: 0, blur: 4, color: { hex: '#ffcc00', r: 255, g: 204, b: 0, a: 255 } }],
  },
};

describe('the painted-name swap keeps the paint', () => {
  it('holds no end frame over the inline paint style', () => {
    expect(rule('.sn-cosmetic-swap--name')).toMatch(/animation-fill-mode:\s*backwards/);
  });

  it('every frame that sets a filter carries the paint filter, and none clears it', () => {
    const frames = keyframes('sn-cosmetic-swap-name');
    const filters = [...frames.matchAll(/filter:\s*([^;]+);/g)].map((m) => m[1]);
    expect(filters.length).toBeGreaterThan(0);
    for (const f of filters) expect(f).toContain('var(--sn-paint-filter,)');
    expect(frames).not.toMatch(/filter:\s*none/);
  });

  it('paintSwapStyle hands the paint filter and opacity to the keyframes', () => {
    const style = computePaintStyleUncached(SHADOWED, '#9146FF');
    const swap = paintSwapStyle(style) as Record<string, unknown>;
    expect(swap['--sn-paint-filter']).toBe(style.filter);
    expect(String(style.filter)).toContain('drop-shadow(#ffcc00');
    expect(swap['--sn-paint-opacity']).toBe(0.8);
    expect(swap.filter).toBe(style.filter);
  });

  it('a paint with no shadow sets no filter variable', () => {
    const flat = { ...SHADOWED, data: { ...SHADOWED.data, shadows: [] } };
    const swap = paintSwapStyle(computePaintStyleUncached(flat, '#9146FF')) as Record<string, unknown>;
    expect('--sn-paint-filter' in swap).toBe(false);
  });
});

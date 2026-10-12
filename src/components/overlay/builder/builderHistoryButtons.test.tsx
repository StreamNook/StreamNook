// @vitest-environment node
// Run with: npm test
//
// The builder draws Undo and Redo in its header, both disabled on a fresh
// overlay with nothing to undo. The history rules themselves are covered in
// editHistory.test.ts; this checks the builder wires the buttons in.

import { describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

// The dropdown portals its menu into document.body, which a server render has
// no use for.
vi.mock('../../ui/Dropdown', () => ({ Dropdown: () => null }));

import { OverlayBuilder } from './OverlayBuilder';
import { OverlayHostProvider, type OverlayHost } from './overlayHost';

const host: OverlayHost = {
  accountId: null,
  signIn: () => {},
  request: async () => ({ ok: false, status: 0, json: () => null }),
  resolveYouTubeIdentifier: async (id) => id,
  resolveKickSlug: async (slug) => slug,
  youTubeChannelTitle: async () => null,
  lookupError: () => '',
  LivePreview: () => null,
};

const html = renderToStaticMarkup(
  <OverlayHostProvider value={host}>
    <OverlayBuilder />
  </OverlayHostProvider>,
);

const button = (label: string): string | undefined =>
  html.match(new RegExp(`<button[^>]*aria-label="${label.replace(/[()+]/g, '\\$&')}"[^>]*>`))?.[0];

describe('overlay builder history buttons', () => {
  it('draws Undo and Redo with their shortcuts', () => {
    expect(button('Undo (Ctrl+Z)')).toBeDefined();
    expect(button('Redo (Ctrl+Shift+Z)')).toBeDefined();
  });

  it('both start disabled', () => {
    expect(button('Undo (Ctrl+Z)')).toMatch(/\sdisabled=""/);
    expect(button('Redo (Ctrl+Shift+Z)')).toMatch(/\sdisabled=""/);
  });

  it('sit before Reset in the header', () => {
    const undoAt = html.indexOf('aria-label="Undo (Ctrl+Z)"');
    const resetAt = html.indexOf('aria-label="Reset this overlay to defaults"');
    expect(undoAt).toBeGreaterThan(-1);
    expect(resetAt).toBeGreaterThan(undoAt);
  });
});

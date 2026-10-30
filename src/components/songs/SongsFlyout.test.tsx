// The Songs popover: the player keeps its overlay up while it is open (it is
// portaled, so the check reads the document), it opens fully on screen from
// the smallest player, its empty state, and Identify closed where the audio
// tap is (macOS) with the reason in its place.
import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { renderToStaticMarkup } from 'react-dom/server';

vi.mock('../../stores/AppStore', () => ({
  useAppStore: Object.assign(() => false, { getState: () => ({}), subscribe: () => () => {} }),
  mainChatRoom: () => null,
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(), emitTo: vi.fn() }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: () => ({ label: 'main', listen: vi.fn() }) }));

import { SongsFlyoutBody, type SongsFlyoutBodyProps } from './SongsFlyout';
import { FLYOUT_WIDTH, flyoutZIndex, placeFlyout, SONGS_FLYOUT_ATTR, songsFlyoutOpen } from './songsPopover';
import type { SongRow } from '../../utils/songHistory';

const root = resolve(__dirname, '..', '..', '..');
const read = (p: string) => readFileSync(resolve(root, p), 'utf8');

const row = (key: string, title: string): SongRow => ({
  key,
  title,
  artist: 'Sade',
  art: null,
  last_ms: Date.now() - 60_000,
  count: 1,
  heard: null,
});

const body = (over: Partial<SongsFlyoutBodyProps> = {}) =>
  renderToStaticMarkup(
    <SongsFlyoutBody
      recent={[row('a', 'Kiss of Life'), row('b', 'Smooth Operator')]}
      total={42}
      supported
      refusal="Identifying songs needs the audio tap."
      listening={false}
      tintFor={() => null}
      onIdentify={() => {}}
      onOpenSong={() => {}}
      onOpenAll={() => {}}
      {...over}
    />,
  );

describe('the player sees the popover as an open menu', () => {
  it('finds it on the document by its mark', () => {
    const asked: string[] = [];
    const doc = (open: boolean) => ({
      querySelector: (sel: string) => {
        asked.push(sel);
        return open && sel === '[data-songs-flyout]' ? ({} as Element) : null;
      },
    });
    expect(songsFlyoutOpen(doc(true))).toBe(true);
    expect(songsFlyoutOpen(doc(false))).toBe(false);
    expect(songsFlyoutOpen(null)).toBe(false);
    expect(asked[0]).toBe(`[${SONGS_FLYOUT_ATTR}]`);
  });

  it('the popover carries the mark, and the player checks for it on the document', () => {
    expect(read('src/components/songs/SongsFlyout.tsx')).toContain('[SONGS_FLYOUT_ATTR]');
    const player = read('src/components/VideoPlayer.tsx');
    const check = player.slice(player.indexOf('const isPlayerMenuOpen'), player.indexOf('const isSeekHeld'));
    expect(check).toContain('songsFlyoutOpen(document)');
  });
});

describe('where the popover opens', () => {
  // The smallest window, with the player's buttons at its top right.
  const smallest = { w: 800, h: 600 };
  const split = { left: 700, right: 752, top: 12, bottom: 44 };

  it('hangs right-aligned under the split button, inside the window', () => {
    const p = placeFlyout(split, smallest);
    expect(p.width).toBe(FLYOUT_WIDTH);
    expect(p.left + p.width).toBe(split.right);
    // Its heading, Identify, four recent rows and its foot fit the smallest window.
    expect(p.maxHeight).toBeGreaterThanOrEqual(520);
    expect(p.top).toBe(split.bottom + 6);
    expect(p.left).toBeGreaterThanOrEqual(8);
    expect(p.top + p.maxHeight).toBeLessThanOrEqual(smallest.h - 12);
  });

  it('stays inside the window when the button sits near either side', () => {
    const nearLeft = placeFlyout({ left: 20, right: 72, top: 12, bottom: 44 }, smallest);
    expect(nearLeft.left).toBe(8);
    const nearRight = placeFlyout({ left: 760, right: 812, top: 12, bottom: 44 }, smallest);
    expect(nearRight.left + nearRight.width).toBeLessThanOrEqual(smallest.w - 8);
  });

  it('narrows in a window narrower than itself, and its songs scroll in the room left', () => {
    const p = placeFlyout({ left: 250, right: 300, top: 400, bottom: 432 }, { w: 320, h: 480 });
    expect(p.width).toBe(304);
    expect(p.left).toBe(8);
    expect(p.maxHeight).toBe(480 - 438 - 12);
  });

  it('sits over the full-screen player on the title bar menu layer, else lifted over the player', () => {
    expect(flyoutZIndex(true)).toBe('var(--sn-z-titlebar-menu)');
    expect(flyoutZIndex(false)).toBe('calc(var(--sn-z-over-player-lift, 0) + 9999)');
  });
});

describe('what the popover holds', () => {
  it('its heading with the count, Identify, the recent songs, and the whole list', () => {
    const html = body();
    expect(html).toMatch(/<h2[^>]*font-family:&quot;Inter&quot;[^>]*>Your songs<\/h2>/);
    expect(html).toContain('42 songs');
    expect(html).toContain('Identify this song');
    expect(html).toContain('Recent');
    expect(html).toContain('Kiss of Life');
    expect(html).toContain('All your songs');
  });

  it('is roomy: 48 px Identify, 48 px covers, 14 and 13 px rows, a 52 px foot', () => {
    expect(FLYOUT_WIDTH).toBe(400);
    const html = body();
    expect(html).toMatch(/class="[^"]*h-12 w-full[^"]*"[^>]*>[\s\S]*?Identify this song/);
    expect(html).toMatch(/class="[^"]*h-12 w-12 shrink-0[^"]*rounded-lg/);
    expect(html).toContain('text-[14px] font-semibold');
    expect(html).toContain('text-[13px] leading-[18px] text-textSecondary');
    expect(html).toContain('h-[52px]');
  });

  it('no songs yet: the line under Identify, and no Recent block or count', () => {
    const html = body({ recent: [], total: 0 });
    expect(html).toContain('No songs yet');
    expect(html).not.toContain('Recent');
    expect(html).not.toContain('All your songs');
    expect(html).not.toContain('0 songs');
  });

  it('opens and closes by opacity and transform only, from its top right', () => {
    const src = read('src/components/songs/SongsFlyout.tsx');
    expect(src).toContain('initial={{ opacity: 0, scale: 0.96, y: -6 }}');
    expect(src).toContain("transformOrigin: 'top right'");
    expect(src).toMatch(/exit=\{\{ opacity: 0, scale: 0\.97, y: -4, transition: \{ duration: 0\.12/);
  });

  it('where identifying is closed (macOS), Identify is off and says why', () => {
    const html = body({ supported: false });
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>[\s\S]*?Identify this song/);
    expect(html).toContain('Identifying songs needs the audio tap.');
    expect(html).not.toContain('Listens to the stream');
    // The list is still one click away.
    expect(html).toContain('All your songs');
  });

  it('while a listening runs, Identify waits', () => {
    const html = body({ listening: true });
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>[\s\S]*?Listening in/);
  });

  it("a recent row's hover takes its cover's tint", () => {
    const html = body({ tintFor: (r) => (r.key === 'a' ? 'rgba(1, 2, 3, 0.2)' : null) });
    expect(html).toContain('--row-tint:rgba(1, 2, 3, 0.2)');
    expect(html).toContain('--row-tint:var(--color-surface-hover)');
  });
});

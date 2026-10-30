// The list's changes and bounds: a song filed while the list is scrolled waits
// behind the "New song" pill only when it really went to the top (a link
// refresh or an Undo applies at once, so the pill never shows for nothing); a
// brand-new song shifts the pages asked for meanwhile; the bounds keep their
// identity until the day moves on, and are sent with every page of the
// unsearched list.
import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

vi.mock('../../stores/AppStore', () => ({ useAppStore: { getState: () => ({}) } }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: () => ({ label: 'main', listen: vi.fn() }) }));

import { boundsMovedOn, stepForChange } from './useSongPages';
import { boundsFor, NOTHING_HELD } from './songGroups';

const filed = (key: string) => ({ kind: 'filed' as const, key });
const top = (key: string, last_ms: number, count = 1) => ({ key, last_ms, count });

describe('a change while the list is scrolled', () => {
  it('a new song at the top waits behind the pill and shifts the pages asked meanwhile', () => {
    const step = stepForChange(NOTHING_HELD, filed('new'), true, { before: top('old', 10, 3), after: top('new', 20) });
    expect(step).toEqual({ held: { filed: ['new'], more: false }, apply: false, shifts: true });
  });

  it('a song heard again waits too, but it was already in the list: no shift', () => {
    const step = stepForChange(NOTHING_HELD, filed('mid'), true, { before: top('old', 10), after: top('mid', 30, 4) });
    expect(step.apply).toBe(false);
    expect(step.held.filed).toEqual(['mid']);
    expect(step.shifts).toBe(false);
  });

  it('a song changed in place (its links looked up again) applies at once, with no pill', () => {
    const same = top('old', 10, 3);
    expect(stepForChange(NOTHING_HELD, filed('mid'), true, { before: same, after: same })).toEqual({
      held: NOTHING_HELD,
      apply: true,
      shifts: false,
    });
  });

  it('when the first row could not be read, it applies rather than show a pill for nothing', () => {
    expect(stepForChange(NOTHING_HELD, filed('new'), true, null).apply).toBe(true);
  });

  it('a song already waiting, filed again, is not counted twice', () => {
    const held = { filed: ['new'], more: false };
    const step = stepForChange(held, filed('new'), true, { before: top('new', 20), after: top('new', 25, 2) });
    expect(step.held.filed).toEqual(['new']);
    expect(step.shifts).toBe(false);
  });

  it('at the top of the list everything applies at once', () => {
    expect(stepForChange(NOTHING_HELD, filed('new'), false, null)).toEqual({ held: NOTHING_HELD, apply: true, shifts: false });
    expect(stepForChange(NOTHING_HELD, { kind: 'tinted', keys: ['a'] }, true, null).apply).toBe(true);
  });
});

describe('the bounds', () => {
  it('keep their identity while the day holds, and move on at midnight', () => {
    const evening = new Date(2026, 9, 29, 22, 0);
    const bounds = boundsFor(evening);
    expect(boundsMovedOn(bounds, new Date(2026, 9, 29, 23, 59))).toBeNull();
    const next = boundsMovedOn(bounds, new Date(2026, 9, 30, 0, 1));
    expect(next?.ms[0]).toBe(new Date(2026, 9, 30).getTime());
  });

  it('go with every page of the unsearched list, and are made again from the clock', () => {
    const src = readFileSync(resolve(__dirname, 'useSongPages.ts'), 'utf8');
    expect(src).toMatch(/const asked = query \? null : bounds\.current;/);
    expect(src).toMatch(/fetchSongPage\(query, page \* SONG_PAGE \+ shift\.current, SONG_PAGE, asked\?\.ms\)/);
    expect(src).toMatch(/boundsMovedOn\(bounds\.current, new Date\(\)\)/);
    // A page answer, local midnight, the window back in view, and focus.
    expect(src).toMatch(/useEffect\(\(\) => checkBounds\(\), \[counts\]\)/);
    expect(src).toContain('msToNextDay(new Date())');
    expect(src).toContain('onWindowVisibility(wake)');
    expect(src).toContain("window.addEventListener('focus', wake)");
  });
});

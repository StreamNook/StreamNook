import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { AccoladeWallView, PageLink } from '../../types/profileView';
import { AccoladeWall } from './AccoladeWall';
import { OwnAccoladeSection } from './OwnAccoladeSection';
import { WALL, collapsedWallHeight, wallHeight, wallRows } from './accoladeWallLayout';
import { ownWallSlot } from './ownWallSlot';
import { accoladeLabel, accoladePanel, plateSlot } from './profileSheetParts';

// Your own wall as Rust sends it: a locked secret is "???" with the lock icon and
// its cryptic hint; hints print your numbers.
const ownWall: AccoladeWallView = {
  earned: 2,
  total: 5,
  items: [
    { id: 'chatty', label: 'Chatty', icon: 'message-circle', colors: ['#60a5fa', '#2563eb'], earned: true, secret: false, hint: 'Messages sent: 8,200 / 1,000' },
    { id: 'insomniac', label: 'Restless', icon: 'moon', colors: ['#a78bfa', '#6d28d9'], earned: true, secret: true, hint: 'Click Test until it hurts' },
    { id: 'binge', label: 'Binge', icon: 'tv', colors: ['#34d399', '#047857'], earned: false, secret: false, hint: 'Streams watched: 321 / 500' },
    { id: 'ghost', label: '???', icon: 'lock', colors: ['#94a3b8', '#475569'], earned: false, secret: true, hint: 'Be a ghost in the machine' },
    { id: 'goblin', label: 'Drop Goblin', icon: 'package', colors: ['#fbbf24', '#b45309'], earned: false, secret: false, hint: 'Claim 25 drops' },
  ],
};

describe('a locked secret reads "???" everywhere it is drawn', () => {
  it('the label, even when the item carries a real name', () => {
    expect(accoladeLabel({ label: 'Ghost', secret: true, earned: false })).toBe('???');
    expect(accoladeLabel({ label: '???', secret: true, earned: false })).toBe('???');
  });

  it('an earned secret shows its name; a locked ordinary accolade shows its name', () => {
    expect(accoladeLabel({ label: 'Restless', secret: true, earned: true })).toBe('Restless');
    expect(accoladeLabel({ label: 'Binge', secret: false, earned: false })).toBe('Binge');
  });

  it('the tap panel: "???" over the cryptic hint, never the real name', () => {
    expect(accoladePanel({ label: 'Ghost', hint: 'Be a ghost in the machine', secret: true, earned: false })).toEqual({
      title: '???',
      detail: 'Be a ghost in the machine',
    });
    expect(accoladePanel({ label: 'Chatty', hint: 'Messages sent: 8,200 / 1,000', secret: false, earned: true })).toEqual({
      title: 'Chatty',
      detail: 'Messages sent: 8,200 / 1,000',
    });
  });

  it('the wall opens on the earned medallions with the locked count on the expander', () => {
    const out = renderToStaticMarkup(<AccoladeWall wall={ownWall} className="" />);
    expect(out).toContain('2 / 5 unlocked');
    expect(out).toContain('Chatty');
    expect(out).toContain('Restless');
    expect(out).not.toContain('Binge');
    expect(out).toContain('Show all 5 (3 locked)');
  });

  it('draws every cell at the layout height, and the panel with the layout padding', () => {
    const out = renderToStaticMarkup(<AccoladeWall wall={ownWall} className="" />);
    expect(out.match(new RegExp(`height:${WALL.cell}px`, 'g'))?.length).toBe(2);
    expect(out).toContain(`padding:${WALL.pad}px`);
    expect(out).toContain(`height:${WALL.header}px`);
    expect(out).toContain(`height:${WALL.toggle}px`);
    expect(out).toContain(`height:${WALL.progress}px`);
  });
});

describe('the You tab slot', () => {
  const dummyLink = {} as PageLink;

  it('follows the page plate rule for every input', () => {
    for (const registryLoaded of [true, false])
      for (const isMember of [true, false])
        for (const hasWall of [true, false])
          for (const answered of [true, false]) {
            expect(ownWallSlot(registryLoaded, isMember, hasWall ? ownWall : null, answered)).toBe(
              plateSlot(registryLoaded, isMember, hasWall ? dummyLink : null, answered),
            );
          }
  });

  it('shows a wall (a saved copy proves membership), hides after an answer of none', () => {
    expect(ownWallSlot(false, false, ownWall, false)).toBe('shown');
    expect(ownWallSlot(true, true, null, true)).toBe('hidden');
    expect(ownWallSlot(false, false, null, true)).toBe('hidden');
  });

  it('is held before the answer: for a member, or while the member list has not loaded', () => {
    expect(ownWallSlot(true, true, null, false)).toBe('reserved');
    expect(ownWallSlot(false, false, null, false)).toBe('reserved');
  });

  it('takes no room for a loaded list that says this is not a member', () => {
    expect(ownWallSlot(true, false, null, false)).toBe('hidden');
  });

  it('draws the held slot as invisible space the height of a collapsed wall', () => {
    // No member list in a node run, so the section is held.
    const out = renderToStaticMarkup(<OwnAccoladeSection userId="42" />);
    expect(out).toContain(`height:${collapsedWallHeight()}px`);
    expect(out).toContain('grid-template-rows:1fr');
    expect(out).toContain('aria-hidden="true"');
    expect(out).toContain('inert=""');
    expect(out).not.toContain('unlocked');
    // Invisible: no panel, no border, no fill.
    expect(out).not.toContain('glass-panel');
    expect(out).not.toContain('background');
  });
});

describe('the wall layout', () => {
  it('counts rows of five', () => {
    expect(wallRows(0)).toBe(0);
    expect(wallRows(1)).toBe(1);
    expect(wallRows(5)).toBe(1);
    expect(wallRows(6)).toBe(2);
    expect(wallRows(11)).toBe(3);
  });

  it('adds up: border, padding, heading, rows, the Show all row and the bar', () => {
    const rows = 2;
    const expected =
      2 * WALL.border + 2 * WALL.pad + WALL.header + (rows * WALL.cell + (rows - 1) * WALL.rowGap) + (WALL.sectionGap + WALL.toggle) + (WALL.sectionGap + WALL.progress);
    expect(wallHeight(rows, true)).toBe(expected);
    expect(wallHeight(rows, false)).toBe(expected - WALL.sectionGap - WALL.toggle);
  });

  it('holds the slot at a collapsed wall of two rows with Show all', () => {
    expect(collapsedWallHeight()).toBe(wallHeight(2, true));
    expect(collapsedWallHeight()).toBe(296);
    // Four across on a narrow phone holds the same ten medallions in three rows.
    expect(collapsedWallHeight(4)).toBe(wallHeight(3, true));
    expect(wallHeight(3, true)).toBeGreaterThan(collapsedWallHeight());
  });
});

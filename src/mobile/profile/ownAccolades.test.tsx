import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { AccoladeWallView } from '../../types/profileView';
import { AccoladeWall } from './AccoladeWall';
import { WALL } from './accoladeWallLayout';
import { accoladeLabel, accoladePanel } from './profileSheetParts';

// An own wall as Rust sends it: a locked secret is "???" with the lock icon and
// its cryptic hint; hints print the owner's numbers.
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
    const out = renderToStaticMarkup(<AccoladeWall wall={ownWall} />);
    expect(out).toContain('2 / 5 unlocked');
    expect(out).toContain('Chatty');
    expect(out).toContain('Restless');
    expect(out).not.toContain('Binge');
    expect(out).toContain('Show all 5 (3 locked)');
  });

  it('draws every cell at the layout height, and the panel with the layout padding', () => {
    const out = renderToStaticMarkup(<AccoladeWall wall={ownWall} />);
    expect(out.match(new RegExp(`height:${WALL.cell}px`, 'g'))?.length).toBe(2);
    expect(out).toContain(`padding:${WALL.pad}px`);
    expect(out).toContain(`height:${WALL.header}px`);
    expect(out).toContain(`height:${WALL.toggle}px`);
    expect(out).toContain(`height:${WALL.progress}px`);
  });
});

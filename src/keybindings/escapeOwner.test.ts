// @vitest-environment node
import { describe, expect, it } from 'vitest';
import { escapeHasOwner } from './escapeOwner';

// Esc leaves the player's full screen only when nothing on screen answers Esc
// itself. Checked over a small stand-in for the DOM: which elements match a
// selector, whether they are laid out, where they sit, and what is on top.

interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

class El {
  children: El[] = [];
  parent: El | null = null;
  constructor(
    public name: string,
    public selectors: string[] = [],
    public box: Box | null = { left: 0, top: 0, width: 100, height: 40 },
  ) {}
  add(child: El): El {
    child.parent = this;
    this.children.push(child);
    return child;
  }
  matches(list: string): boolean {
    return list.split(',').some((s) => this.selectors.includes(s.trim()));
  }
  closest(list: string): El | null {
    if (this.matches(list)) return this;
    return this.parent ? this.parent.closest(list) : null;
  }
  contains(other: unknown): boolean {
    if (other === this) return true;
    return this.children.some((c) => c.contains(other));
  }
  getClientRects() {
    return { length: this.box ? 1 : 0 };
  }
  getBoundingClientRect(): Box {
    return this.box ?? { left: 0, top: 0, width: 0, height: 0 };
  }
}

/** A document over `roots`; `topAt` says what the viewer sees at a point. */
const doc = (roots: El[], topAt: (x: number, y: number) => El | null = () => null) => {
  const all: El[] = [];
  const walk = (e: El) => {
    all.push(e);
    e.children.forEach(walk);
  };
  roots.forEach(walk);
  return {
    querySelectorAll: (list: string) => all.filter((e) => e.matches(list)),
    elementFromPoint: topAt,
  };
};

const OPEN = '[aria-expanded="true"]';
const MENU = '[role="menu"]';

describe('escapeHasOwner', () => {
  it('nothing open: Esc is free', () => {
    const player = new El('player', ['.plyr--fullscreen-fallback']);
    expect(escapeHasOwner(doc([player], () => player))).toBe(false);
  });

  it("Plyr's settings menu open in the full-screen player owns it", () => {
    const player = new El('player', ['.plyr--fullscreen-fallback']);
    player.add(new El('settings button', [OPEN]));
    expect(escapeHasOwner(doc([player], () => player))).toBe(true);
  });

  it("the Island's door open in the title bar owns it, even with the panel over the door", () => {
    const bar = new El('bar', ['[data-titlebar]']);
    bar.add(new El('door', [OPEN]));
    const panel = new El('panel');
    expect(escapeHasOwner(doc([bar, panel], () => panel))).toBe(true);
  });

  it('an open trigger laid out under the full-screen video does not', () => {
    const sidebar = new El('sidebar');
    sidebar.add(new El('expanded group', [OPEN]));
    const player = new El('player', ['.plyr--fullscreen-fallback']);
    expect(escapeHasOwner(doc([sidebar, player], () => player))).toBe(false);
  });

  it('a menu on top owns it, a menu under the video does not', () => {
    const menu = new El('menu', [MENU], { left: 10, top: 50, width: 200, height: 100 });
    const item = menu.add(new El('item', [], { left: 10, top: 90, width: 200, height: 20 }));
    expect(escapeHasOwner(doc([menu], () => item))).toBe(true);
    const player = new El('player', ['.plyr--fullscreen-fallback']);
    expect(escapeHasOwner(doc([menu, player], () => player))).toBe(false);
  });

  it("a menu kept hidden in the page (Plyr's) does not", () => {
    const hidden = new El('hidden menu', [MENU], null);
    expect(escapeHasOwner(doc([hidden], () => hidden))).toBe(false);
  });

  it('no document, no owner', () => {
    expect(escapeHasOwner(undefined)).toBe(false);
  });
});

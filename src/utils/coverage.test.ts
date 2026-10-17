// Run with: npm test
//
// The covering decision as a pure function over the stack at a point, topmost
// first: what `document.elementsFromPoint` hands it.
import { describe, expect, it } from 'vitest';
import { hiddenUnderLayer, type StackNode } from './coverage';

class Node implements StackNode {
  parent: Node | null;
  constructor(
    readonly name: string,
    parent: Node | null = null,
  ) {
    this.parent = parent;
  }
  contains(other: StackNode | null): boolean {
    for (let n = other as Node | null; n; n = n.parent) if (n === this) return true;
    return false;
  }
}

// html > body > app > sidebar > row > avatar, plus overlays and the player.
const body = new Node('body');
const app = new Node('app', body);
const sidebar = new Node('sidebar', app);
const row = new Node('row', sidebar);
const avatar = new Node('avatar', row);
const settings = new Node('settings overlay', app);
const settingsPanel = new Node('settings panel', settings);
const previewAvatar = new Node('preview avatar', settingsPanel);
const player = new Node('plyr full screen', app);
const video = new Node('video', player);
const fsChat = new Node('full-screen chat column', app);
const pinnedAvatar = new Node('pinned card avatar', fsChat);
const confirm = new Node('confirm modal', body);

describe('hiddenUnderLayer', () => {
  it('covers nothing when no layer is up', () => {
    expect(hiddenUnderLayer([avatar, row, sidebar, app, body], avatar, [])).toBe(false);
  });

  it('covers a sidebar picture under an open overlay', () => {
    expect(hiddenUnderLayer([settings, avatar, row, sidebar, app, body], avatar, [settings])).toBe(true);
  });

  it('leaves a picture inside the overlay moving', () => {
    expect(hiddenUnderLayer([previewAvatar, settingsPanel, settings, app, body], previewAvatar, [settings])).toBe(false);
  });

  it('covers a picture inside an overlay when another overlay opens over it', () => {
    expect(
      hiddenUnderLayer([confirm, previewAvatar, settingsPanel, settings, body], previewAvatar, [settings, confirm]),
    ).toBe(true);
  });

  it('covers the sidebar under the full-screen player', () => {
    expect(hiddenUnderLayer([video, player, avatar, row, sidebar, app, body], avatar, [player])).toBe(true);
  });

  it('leaves a picture in a layer drawn above the full-screen player moving', () => {
    expect(hiddenUnderLayer([pinnedAvatar, fsChat, video, player, app, body], pinnedAvatar, [player])).toBe(false);
  });

  it('covers nothing where no layer reaches that point', () => {
    expect(hiddenUnderLayer([avatar, row, sidebar, app, body], avatar, [settings])).toBe(false);
  });

  it('counts a layer hit directly, not only its children', () => {
    expect(hiddenUnderLayer([player, avatar], avatar, [player])).toBe(true);
  });
});

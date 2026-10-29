// The moderator's hover hold starts only from a counted move. Rows sliding
// under a resting pointer fire pointerover in Chromium (and no pointermove),
// and a hold started from that froze chat until the mouse moved. Mod buttons
// under a pointer that chat slid under must not take a press. A DOM test
// cannot run here (the suite has no DOM), so this guards the wiring in the
// sources; chat/hoverHold.test.ts holds the rules.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const read = (rel: string) => readFileSync(resolve(__dirname, '..', rel), 'utf8');

describe('hover hold wiring', () => {
  const list = read('components/ChatMessageList.tsx');
  // The list's source from one marker up to the next, both of which must be
  // there.
  const between = (start: string, end: string) => {
    const from = list.indexOf(start);
    const to = list.indexOf(end, from);
    expect(from).toBeGreaterThan(-1);
    expect(to).toBeGreaterThan(from);
    return list.slice(from, to);
  };

  it('nothing in the list listens to pointerover', () => {
    expect(list).not.toContain("addEventListener('pointerover'");
  });

  it('a move counts only past the tremor threshold, on screen, in the list and out of it', () => {
    expect(list.split('movedEnough(last, e.screenX, e.screenY)').length - 1).toBe(2);
  });

  it('rows moving under the pointer disarm the list after every commit, scroll and resize', () => {
    expect(list).toContain("scroller.addEventListener('scroll', onScroll");
    expect(list.split('checkMovedRef.current?.()').length - 1).toBe(2);
    expect(list).toContain('el.dataset.holdDisarmed');
    // The list's own box too, not only the rows inside it.
    expect(list).toContain('boxRo.observe(scroller)');
  });

  it('a resize looks after its re-pin, which moves every row', () => {
    expect(list).toMatch(/repin\(\);\s*checkMovedRef\.current\?\.\(\);/);
  });

  it('a move looks before it notes, and a press before it acts', () => {
    const move = between('const onMove = (e: PointerEvent) => {', 'const onOut = ');
    expect(move.indexOf('checkMoved();')).toBeGreaterThan(-1);
    expect(move.indexOf('checkMoved();')).toBeLessThan(move.indexOf('noteUnder(e.target)'));
    const press = between('const onPress = (e: PointerEvent) => {', 'const onRelease = ');
    expect(press.indexOf('checkMoved();')).toBeGreaterThan(-1);
    expect(press.indexOf('checkMoved();')).toBeLessThan(press.indexOf('if (disarmed'));
  });

  it('only a quick move re-arms it, heard from the whole window', () => {
    expect(list).toContain("window.addEventListener('pointermove', onAnyMove");
    expect(list).not.toContain("addEventListener('pointerleave'");
  });

  it('presses on mod buttons, or where they hide, meet the disarm before anything else hears them', () => {
    expect(list).toContain("el.addEventListener('pointerdown', onPress, true)");
    expect(list).toContain("el.addEventListener('click', onClick, true)");
    expect(list).toContain('onHiddenSpot(e)');
  });

  it('a press whose release never comes lets go when the window loses focus or hides', () => {
    expect(list).toContain("window.addEventListener('blur', onRelease)");
    expect(list).toContain("document.addEventListener('visibilitychange', onRelease)");
    expect(list).toContain("window.addEventListener('blur', release)");
    expect(list).toContain("document.addEventListener('visibilitychange', release)");
  });

  it('the list draws the pill for what the hold keeps back', () => {
    expect(list).toContain('<HoverHoldPill');
  });

  it('Chat Paused sits on the chat panel, where the pill sits', () => {
    const widget = read('components/ChatWidget.tsx');
    expect(widget).not.toContain('bottom-[60px]');
    expect(widget).toContain('pausedPill={');
  });

  it('the panel tells the list which feed it draws', () => {
    expect(read('components/ChatWidget.tsx')).toContain('feedKey={feedKey}');
  });

  it("a held message's Allow and Deny are mod buttons to the hold", () => {
    expect(read('components/chat/HeldMessageRow.tsx')).toContain('data-mod-act="true"');
  });

  it('an open reason picker holds its row', () => {
    expect(read('components/chat/HoverModButtons.tsx')).toContain('holdForPicker(row, true)');
  });
});

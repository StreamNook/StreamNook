// SN-284 S2: the floating stack never shrinks the list. The spacer is the
// stack's bottom minus the list's top, a browser without overflow anchoring
// gets the same change added to its scroll, and a jump centers a row below
// the stack in all three helpers.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { targetFor } from '../../utils/scrollChatToMessage';
import { chromeBottom, coveredTop, jumpTarget, spacerScrollDelta, stackBottomOf, stackSpacer } from './stackSpacer';

const read = (p: string) => readFileSync(resolve(__dirname, '..', '..', '..', p), 'utf8');

describe('the list spacer', () => {
  it('is the stack bottom when it reaches below the header', () => {
    expect(stackSpacer({ header: 40, bar: 0, stack: 180 })).toBe(180);
  });

  it('is the header alone when the stack draws nothing', () => {
    expect(stackSpacer({ header: 40, bar: 0, stack: 0 })).toBe(40);
  });

  it('is the stack bottom minus the combined-chat bar that already sits above the list', () => {
    expect(stackSpacer({ header: 40, bar: 100, stack: 180 })).toBe(80);
  });

  it('is zero where the combined-chat bar already cleared the stack', () => {
    expect(stackSpacer({ header: 40, bar: 100, stack: 90 })).toBe(0);
    expect(stackSpacer({ header: 40, bar: 100, stack: 0 })).toBe(0);
  });

  it('on a phone (no header) is the stack alone', () => {
    expect(stackSpacer({ header: 0, bar: 0, stack: 0 })).toBe(0);
    expect(stackSpacer({ header: 0, bar: 0, stack: 120 })).toBe(120);
  });

  it('chrome reaches as low as its lowest piece', () => {
    expect(chromeBottom({ header: 40, bar: 100, stack: 90 })).toBe(100);
    expect(chromeBottom({ header: 40, bar: 0, stack: 180 })).toBe(180);
  });

  it('the stack bottom is where it starts plus its height plus the gap, and 0 when empty', () => {
    expect(stackBottomOf(48, 100)).toBe(48 + 100 + 4);
    expect(stackBottomOf(48, 0)).toBe(0);
  });
});

describe('a browser without overflow anchoring', () => {
  it('adds the spacer change to the scroll in the same pass', () => {
    expect(spacerScrollDelta(40, 180, false)).toBe(140);
    expect(spacerScrollDelta(180, 40, false)).toBe(-140);
  });

  it('leaves it to the browser where anchoring works', () => {
    expect(spacerScrollDelta(40, 180, true)).toBe(0);
  });

  it('the list applies it in a layout effect on the spacer, and asks the browser which it is', () => {
    const list = read('src/components/ChatMessageList.tsx');
    expect(list).toContain('spacerScrollDelta(prev, topSpace, supportsOverflowAnchor())');
    expect(list).toMatch(/useLayoutEffect\(\(\) => \{\s*const prev = spacerRef\.current;/);
    expect(read('src/components/chat/stackSpacer.ts')).toContain("CSS.supports('overflow-anchor', 'auto')");
  });
});

describe('a jump lands below the stack', () => {
  // A 600 px list whose top 180 px the stack covers; the row sits at 1000.
  const base = { elementTop: 1000, elementHeight: 20, scrollTop: 0, containerHeight: 600, scrollHeight: 4000 };

  it('centers in what is left under the stack', () => {
    const open = jumpTarget({ ...base, align: 'center', inset: 0 });
    const covered = jumpTarget({ ...base, align: 'center', inset: 180 });
    // The row's middle is 90 px lower in the viewport than the plain center, so it sits mid-way below the stack.
    expect(open - covered).toBe(180 - 90);
    // Its middle lands at the center of the visible part: inset + (600 - inset) / 2 = 390.
    expect(base.elementTop + 10 - covered).toBe(390);
  });

  it('starts below the stack, not under it', () => {
    expect(jumpTarget({ ...base, align: 'start', inset: 180 })).toBe(820);
  });

  it('counts a row hidden under the stack as out of view', () => {
    const hidden = jumpTarget({ ...base, align: 'auto', elementTop: 1040, scrollTop: 1000, inset: 180 });
    expect(hidden).toBe(860);
    const shown = jumpTarget({ ...base, align: 'auto', elementTop: 1300, scrollTop: 1000, inset: 180 });
    expect(shown).toBe(1000);
  });

  it('is clamped to the list', () => {
    expect(jumpTarget({ ...base, align: 'center', elementTop: 10, inset: 180 })).toBe(0);
    expect(jumpTarget({ ...base, align: 'center', elementTop: 3990, inset: 180 })).toBe(3400);
  });

  it('agrees with the phone helper for the same list', () => {
    const container = { clientHeight: 600, scrollHeight: 4000, scrollTop: 0, getBoundingClientRect: () => ({ top: 0 }) } as unknown as HTMLElement;
    const el = { getBoundingClientRect: () => ({ top: 1000, height: 20 }) } as unknown as HTMLElement;
    expect(targetFor(container, el, 'center', 180)).toBe(jumpTarget({ ...base, align: 'center', inset: 180 }));
  });

  it('the part of a list the stack covers is its bottom minus the list top', () => {
    expect(coveredTop(180, 40, 0)).toBe(140);
    expect(coveredTop(30, 40, 0)).toBe(0);
  });

  it('all three helpers pass the stack in', () => {
    expect(read('src/components/ChatWidget.tsx')).toMatch(/jumpTarget\(\{[\s\S]*?inset: listSpacerRef\.current/);
    expect(read('src/components/multichat/BlendedChatPane.tsx')).toMatch(/jumpTarget\(\{[\s\S]*?inset: stackBottomRef\.current/);
    expect(read('src/utils/scrollChatToMessage.ts')).toContain('topInset');
  });

  it('a jump from search is made again as the stack returns when search closes', () => {
    const widget = read('src/components/ChatWidget.tsx');
    expect(widget).toContain('searchJumpRef.current = id');
    expect(widget).toMatch(/searchWasOpenRef[\s\S]*?mentionScrollRef\.current\(id, \{ highlight: false, align: 'center' \}\)/);
  });
});

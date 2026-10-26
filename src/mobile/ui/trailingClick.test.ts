import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { swallowTrailingClick, TRAILING_CLICK_MS } from './trailingClick';

function fire(doc: EventTarget, type: string): Event {
  const e = new Event(type, { cancelable: true, bubbles: true });
  doc.dispatchEvent(e);
  return e;
}

describe('swallowTrailingClick', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('eats the click that trails the tap', () => {
    const doc = new EventTarget();
    swallowTrailingClick(doc);
    expect(fire(doc, 'click').defaultPrevented).toBe(true);
  });

  it('eats one click only', () => {
    const doc = new EventTarget();
    swallowTrailingClick(doc);
    fire(doc, 'click');
    expect(fire(doc, 'click').defaultPrevented).toBe(false);
  });

  it('a new press first lets its own click through', () => {
    const doc = new EventTarget();
    swallowTrailingClick(doc);
    fire(doc, 'pointerdown');
    expect(fire(doc, 'click').defaultPrevented).toBe(false);
  });

  it('stops waiting once the window has passed', () => {
    const doc = new EventTarget();
    swallowTrailingClick(doc);
    vi.advanceTimersByTime(TRAILING_CLICK_MS);
    expect(fire(doc, 'click').defaultPrevented).toBe(false);
  });

  it('the returned disarm lets the next click through', () => {
    const doc = new EventTarget();
    const disarm = swallowTrailingClick(doc);
    disarm();
    expect(fire(doc, 'click').defaultPrevented).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });
});

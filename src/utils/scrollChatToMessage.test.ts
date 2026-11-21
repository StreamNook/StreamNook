// @vitest-environment happy-dom
// SN-284: a jump lands below the floating stack, and a pinned row (hosted by
// data-pin-id, outside the list) can never be the row a jump finds.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { scrollChatToMessage, targetFor } from './scrollChatToMessage';

function setup() {
  document.body.innerHTML = `
    <div id="pane">
      <div data-pin-id="m1" id="pin"></div>
      <div class="overflow-y-auto" id="list"><div data-message-id="m1" id="row"></div></div>
    </div>`;
  const list = document.getElementById('list') as HTMLElement;
  const row = document.getElementById('row') as HTMLElement;
  Object.defineProperty(list, 'clientHeight', { value: 400, configurable: true });
  Object.defineProperty(list, 'scrollHeight', { value: 2000, configurable: true });
  list.getBoundingClientRect = () => ({ top: 100, height: 400 }) as DOMRect;
  // The row is 1000 px down the list's content, 30 px tall.
  row.getBoundingClientRect = () => ({ top: 1100, height: 30 }) as DOMRect;
  return { list, row };
}

afterEach(() => {
  document.body.innerHTML = '';
  vi.useRealTimers();
});

describe('targetFor', () => {
  it('centres in the whole list with no inset, and in what the stack leaves with one', () => {
    const { list, row } = setup();
    expect(targetFor(list, row, 'center')).toBe(1000 - 200 + 15);
    // A 120 px stack: the middle of the remaining 280 px.
    expect(targetFor(list, row, 'center', 120)).toBe(1000 - 120 - 140 + 15);
  });
});

describe('scrollChatToMessage', () => {
  it('finds the list row, never the pin with the same id, and lands below the stack', () => {
    vi.useFakeTimers();
    const { list } = setup();
    const scrollTo = vi.fn();
    list.scrollTo = scrollTo as unknown as typeof list.scrollTo;
    expect(scrollChatToMessage('m1', { smooth: false, topInset: 120 })).toBe(true);
    expect(scrollTo).toHaveBeenCalledWith({ top: 1000 - 120 - 140 + 15, behavior: 'auto' });
  });

  it('returns false when only a pin carries the id', () => {
    document.body.innerHTML = '<div data-pin-id="m2"></div>';
    expect(scrollChatToMessage('m2')).toBe(false);
  });
});

// @vitest-environment happy-dom
// SN-284 openers: a pin from another chat opens the inbox at its entry. The
// list finds the row (loading older pages while it is missing, a few times),
// scrolls to it and highlights it, then lets the request go.
import { act, useRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const addToast = vi.hoisted(() => vi.fn());
vi.mock('../../stores/AppStore', () => ({ useAppStore: { getState: () => ({ addToast }) } }));

import { FOCUS_MISSING_WORDS, mentionRowIdsOf, rowHolding, useInboxFocus } from './useInboxFocus';
import {
  FOCUS_HIGHLIGHT_MS,
  FOCUS_MAX_PAGES,
  requestMentionFocus,
  resetMentionFocus,
  useMentionFocus,
} from '../../stores/mentionFocusStore';
import type { InboxItem, MentionEntry } from '../../types';

const entry = (id: string): MentionEntry =>
  ({
    id,
    platform: 'twitch',
    room: 'xqc',
    channel_login: 'xqc',
    sender_id: '1',
    sender_login: 'a',
    sender_name: 'A',
    text: '@you',
    kind: 'mention',
    at_ms: 0,
    read: false,
    removed: false,
  }) as MentionEntry;

const mentionRow = (id: string, folded: string[] = []): InboxItem =>
  ({ kind: 'mention', row_id: `row-${id}`, entry: entry(id), folded: folded.map(entry), unread: true, avatar: null }) as unknown as InboxItem;

describe('rowHolding', () => {
  it('finds a row by its newest entry or a folded copy', () => {
    const rows = [mentionRow('a'), mentionRow('b', ['c'])];
    expect(rowHolding(rows, 'a')).toBe('row-a');
    expect(rowHolding(rows, 'c')).toBe('row-b');
    expect(rowHolding(rows, 'zzz')).toBeNull();
    expect(mentionRowIdsOf(rows[1])).toEqual(['b', 'c']);
  });
});

let host: HTMLElement;
let root: Root;
const seen: { focused: string | null; asked: number; waiting: boolean } = { focused: null, asked: 0, waiting: false };

function Probe({ rows, hasMore, answer }: { rows: InboxItem[]; hasMore: boolean; answer?: () => Promise<boolean> }) {
  const ref = useRef<HTMLDivElement>(null);
  const focus = useMentionFocus();
  seen.waiting = focus !== null;
  seen.focused = useInboxFocus({
    rows,
    hasMore,
    loadOlder: () => {
      seen.asked += 1;
      return answer?.();
    },
    loaded: true,
    container: ref,
    active: true,
  });
  return (
    <div ref={ref}>
      {rows.map((r) => (
        <div key={r.row_id} data-mention-ids={mentionRowIdsOf(r).join(' ')} />
      ))}
    </div>
  );
}

beforeEach(() => {
  vi.useFakeTimers();
  resetMentionFocus();
  seen.focused = null;
  seen.asked = 0;
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.useRealTimers();
});

describe('useInboxFocus', () => {
  it('scrolls to the row and highlights it, then lets the request go', () => {
    act(() => root.render(<Probe rows={[mentionRow('a'), mentionRow('b')]} hasMore={false} />));
    act(() => requestMentionFocus({ id: 'b', room: 'xqc' }));
    expect(seen.focused).toBe('row-b');
    expect(Element.prototype.scrollIntoView).toHaveBeenCalled();
    act(() => {
      vi.advanceTimersByTime(FOCUS_HIGHLIGHT_MS + 10);
    });
    expect(seen.focused).toBeNull();
    expect(seen.waiting).toBe(false);
  });

  it('keeps paging while there are older pages, and finds an entry past the old five', () => {
    let rows = [mentionRow('a')];
    const render = (hasMore = true) => act(() => root.render(<Probe rows={rows} hasMore={hasMore} />));
    render();
    act(() => requestMentionFocus({ id: 'deep', room: 'xqc' }));
    expect(seen.asked).toBe(1);
    // each page that lands (a longer list) asks for one more
    for (let i = 0; i < 9; i++) {
      rows = [...rows, mentionRow(`p${i}`)];
      render();
    }
    expect(seen.asked).toBe(10);
    expect(seen.waiting).toBe(true);
    rows = [...rows, mentionRow('deep')];
    render();
    expect(seen.focused).toBe('row-deep');
    expect(addToast).not.toHaveBeenCalled();
  });

  it('says so at the end of the list instead of giving up silently', () => {
    addToast.mockClear();
    let rows = [mentionRow('a')];
    const render = (hasMore: boolean) => act(() => root.render(<Probe rows={rows} hasMore={hasMore} />));
    render(true);
    act(() => requestMentionFocus({ id: 'gone', room: 'xqc' }));
    rows = [...rows, mentionRow('b')];
    render(false);
    expect(addToast).toHaveBeenCalledWith(FOCUS_MISSING_WORDS, 'info');
    expect(seen.focused).toBeNull();
    expect(seen.waiting).toBe(false);
  });

  it.each([
    ['a page that failed', () => Promise.resolve(false)],
    ['a page that threw', () => Promise.reject(new Error('offline'))],
  ] as Array<[string, () => Promise<boolean>]>)('says so and lets the request go after %s', async (_name, answer) => {
    addToast.mockClear();
    const rows = [mentionRow('a')];
    act(() => root.render(<Probe rows={rows} hasMore answer={answer} />));
    act(() => requestMentionFocus({ id: 'deep', room: 'xqc' }));
    expect(seen.asked).toBe(1);
    expect(seen.waiting).toBe(true);
    // The list never changes again, so nothing would ask a second time.
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(addToast).toHaveBeenCalledTimes(1);
    expect(addToast).toHaveBeenCalledWith(FOCUS_MISSING_WORDS, 'info');
    expect(seen.waiting).toBe(false);
  });

  it('a page that landed and held the entry says nothing', async () => {
    addToast.mockClear();
    let rows = [mentionRow('a')];
    const view = () => <Probe rows={rows} hasMore answer={() => Promise.resolve(false)} />;
    act(() => root.render(view()));
    act(() => requestMentionFocus({ id: 'deep', room: 'xqc' }));
    // The entry arrives before the (late) answer does.
    rows = [...rows, mentionRow('deep')];
    act(() => root.render(view()));
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(seen.focused).toBe('row-deep');
    expect(addToast).not.toHaveBeenCalled();
  });

  it('a runaway list is stopped by the safety limit, with the same word', () => {
    addToast.mockClear();
    let rows = [mentionRow('a')];
    const render = () => act(() => root.render(<Probe rows={rows} hasMore />));
    render();
    act(() => requestMentionFocus({ id: 'never', room: 'xqc' }));
    for (let i = 0; i < FOCUS_MAX_PAGES + 2; i++) {
      rows = [...rows, mentionRow(`p${i}`)];
      render();
    }
    expect(seen.asked).toBe(FOCUS_MAX_PAGES);
    expect(addToast).toHaveBeenCalledTimes(1);
    expect(seen.waiting).toBe(false);
  });
});

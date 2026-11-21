// @vitest-environment happy-dom
// SN-284 openers: a pin from another chat opens the inbox at its entry. The
// list finds the row (loading older pages while it is missing, a few times),
// scrolls to it and highlights it, then lets the request go.
import { act, useRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

import { mentionRowIdsOf, rowHolding, useInboxFocus } from './useInboxFocus';
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

function Probe({ rows, hasMore }: { rows: InboxItem[]; hasMore: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const focus = useMentionFocus();
  seen.waiting = focus !== null;
  seen.focused = useInboxFocus({
    rows,
    hasMore,
    loadOlder: () => {
      seen.asked += 1;
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

  it('loads older pages while the entry is missing, then gives up quietly', () => {
    let rows = [mentionRow('a')];
    const render = () => act(() => root.render(<Probe rows={rows} hasMore />));
    render();
    act(() => requestMentionFocus({ id: 'old', room: 'xqc' }));
    expect(seen.asked).toBe(1);
    // each page that lands (a longer list) asks for one more, up to the limit
    for (let i = 0; i < FOCUS_MAX_PAGES + 2; i++) {
      rows = [...rows, mentionRow(`p${i}`)];
      render();
    }
    expect(seen.asked).toBe(FOCUS_MAX_PAGES);
    expect(seen.focused).toBeNull();
    expect(seen.waiting).toBe(false);
  });
});

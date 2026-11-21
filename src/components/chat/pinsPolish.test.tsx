// @vitest-environment happy-dom
// SN-284 polish pass: what the screenshots of the stack showed, pinned. A card Rust folded is
// not drawn (the poll and prediction ignored the fold and ran the stack past its cap); the
// channel pin card is as wide as the cards around it; the pins block reports the natural
// height of its rows (never its own clipped box) and fades the edge a row is cut at.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const state = vi.hoisted(() => ({ settings: {} as Record<string, unknown>, currentStream: null, addToast: () => {} }));
vi.mock('../../stores/AppStore', () => ({
  useAppStore: (sel?: (s: unknown) => unknown) => (sel ? sel(state) : state),
}));
vi.mock('../../utils/lowPower', () => ({ usePerformanceQuiet: () => false }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));

import PollOverlay from '../PollOverlay';
import type { StackCardControl } from './chatTopStack';
import type { PollView } from '../../types';

const src = (p: string) => readFileSync(resolve(__dirname, '..', '..', '..', p), 'utf8');

const poll: PollView = {
  channel_id: '1',
  poll_id: 'p1',
  title: 'What next?',
  phase: 'open',
  deadline_ms: null,
  badge: 'Open',
  choices: [{ id: 'a', title: 'A', percent: 100, votes_label: '1 vote', mine: false, winner: false }],
  can_vote: false,
  my_choice_id: null,
  footer: '',
  voters_line: '',
  result_line: null,
};

const control = (over: Partial<StackCardControl> = {}): StackCardControl => ({
  placed: true,
  open: false,
  form: 'strip',
  height: 36,
  moreCards: 0,
  order: 0,
  onToggle: () => {},
  onPresence: () => {},
  onWaiting: () => {},
  ...over,
});

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe('a poll that Rust folded into "N more cards"', () => {
  it('is drawn as a strip when Rust gave it one', () => {
    act(() => root.render(<PollOverlay previewView={poll} stack={control()} />));
    expect(host.textContent).toContain('What next?');
  });

  it('draws nothing when folded, so the stack never runs past its cap', () => {
    act(() => root.render(<PollOverlay previewView={poll} stack={control({ form: 'folded', height: 0 })} />));
    expect(host.textContent).not.toContain('What next?');
  });

  it('draws nothing before it has a slot, and keeps its presence', () => {
    const ids: Array<string | null> = [];
    act(() =>
      root.render(
        <PollOverlay previewView={poll} stack={control({ placed: false, form: null, height: 0, onPresence: (id) => ids.push(id) })} />,
      ),
    );
    expect(host.textContent).not.toContain('What next?');
    expect(ids).toContain('p1');
  });

  it('the prediction folds the same way, and a folded card has no exit to wait for', () => {
    expect(src('src/components/PredictionOverlay.tsx')).toContain("stack.form === 'folded'");
    expect(src('src/components/chat/chatTopStack.ts')).toContain("&& stack?.form !== 'folded'");
  });
});

describe('the channel pin card in the stack', () => {
  it('is as wide as the cards around it; only the standalone float keeps its 24 rem cap', () => {
    const bar = src('src/components/chat/PinnedBar.tsx');
    expect(bar).not.toMatch(/\$\{stack \? "" : "mt-2"\}[^`]*max-w-sm/);
    expect(bar.match(/"max-w-sm mt-2"/g)).toHaveLength(2);
    expect(bar).not.toMatch(/w-full max-w-sm/);
  });
});

describe('an open card held to Rust\'s height', () => {
  it('scrolls inside it: the card inside does not shrink (it would clip itself and never scroll)', () => {
    const stack = src('src/components/chat/ChatOverlayStack.tsx');
    expect(stack).toContain("'overflow-y-auto overscroll-contain [&>*]:shrink-0'");
  });

  it('carries no mask: it would be the backdrop root of the glass card inside and flatten its blur', () => {
    expect(src('src/components/chat/ChatOverlayStack.tsx')).not.toMatch(/mask-image|maskImage|edgeFade/);
  });
});

describe('the pins block', () => {
  const pins = src('src/components/mentions/MentionPins.tsx');

  it('reports the natural height of its rows and count line, never its own clipped box', () => {
    expect(pins).toContain('inner.scrollHeight + (el.clientHeight - inner.clientHeight)');
    expect(pins).toContain('Math.max(el.getBoundingClientRect().height, el.scrollHeight)');
    // A row growing inside a held-short scroller moves neither box: the rows are watched too.
    expect(pins).toContain("el.querySelectorAll('[data-pin-id]').forEach((row) => ro.observe(row))");
  });

  it('files a measure under the form that is drawn (read from the element), not the form of a stale callback', () => {
    expect(pins).toContain('(el.dataset.mentionPins as PinsForm | undefined) ?? form');
    expect(pins).toContain('onMeasure({ form: drawnForm, height: Math.round(h) })');
  });

  it('keeps the newest in view when a row grows, and fades an edge a row is cut at', () => {
    expect(pins).toContain('if (stuckToEnd.current && !frozenRef.current) inner.scrollTop = inner.scrollHeight');
    expect(pins).toContain('style={edgeFade(edges)}');
  });

  it('draws one divider per row inset from both edges, none above the first, and the row draws none', () => {
    expect(pins).toContain('before:inset-x-2');
    expect(pins).toContain('first:before:hidden');
    expect(src('src/components/ChatMessage.tsx')).toContain("const borderClass = !pinned && chatDesign?.show_dividers !== false");
  });

  it('pads each row for the clock, the run count and X, so no word runs under them', () => {
    // Desktop: dot 5 + gap 6 + X 24 + inset 6 = 41; phone: X is 32 (49). A run count adds about 24.
    expect(pins).toContain("'[&>*]:!pr-[46px]'");
    expect(pins).toContain("'[&>*]:!pr-[54px]'");
    expect(pins).toContain("'[&>*]:!pr-[70px]'");
    expect(pins).toContain("'[&>*]:!pr-[78px]'");
  });

  it('over the picture the phone card takes a film under the glaze, inline (the class sets the knob itself)', () => {
    expect(pins).toContain("'--glaze-frost': '55%'");
  });
});

describe('the stack reads as one layered group', () => {
  it('every card wears the one glaze and radius (hype, votes, channel pin, context card, pins, the more strip)', () => {
    for (const f of [
      'src/components/chat/OverlayBanner.tsx',
      'src/components/chat/PinnedBar.tsx',
      'src/mobile/watch/PinnedBanner.tsx',
      'src/mobile/mentions/MentionContextCard.tsx',
      'src/components/mentions/MentionPins.tsx',
      'src/components/chat/ChatOverlayStack.tsx',
    ]) {
      const text = src(f);
      expect(text, f).toContain('GLAZE_BANNER');
      expect(text, f).toContain('BANNER_STYLE');
      expect(text, f).not.toMatch(/glass-flyout|sn-popover|bg-background rounded-lg/);
    }
  });

  it('a shared glass backing sits behind the column as a sibling, so the gaps show glass, never cut rows', () => {
    const stack = src('src/components/chat/ChatOverlayStack.tsx');
    expect(stack).toContain('data-stack-backing=""');
    // It is not a stack: the drawn-height read skips it.
    expect(stack).toContain('!(c as HTMLElement).dataset.stackBacking');
  });

  it('the phones give the column its inset as left and right, so the backing meets the cards', () => {
    expect(src('src/mobile/screens/WatchScreen.tsx')).toContain('className="!left-2.5 !right-2.5"');
    expect(src('src/mobile/screens/ChatsScreen.tsx')).toContain('className="!left-2.5 !right-2.5"');
  });

  it('a held open card scrolls its body (masked), never its glass', () => {
    expect(src('src/components/chat/PinnedBar.tsx')).toContain('<FadeScroller>');
    expect(src('src/mobile/watch/PinnedBanner.tsx')).toContain('<FadeScroller');
    expect(src('src/components/chat/FadeScroller.tsx')).toContain('style={edgeFade(edges)}');
    expect(src('src/components/chat/ChatOverlayStack.tsx')).toContain("querySelectorAll<HTMLElement>('[data-fade-scroll]')");
  });

  it('the channel pin name keeps its shadow on dark themes only', () => {
    expect(src('src/components/chat/PinnedBar.tsx')).toContain('var(--pin-name-shadow, 0 1px 2px rgba(0,0,0,0.8))');
    expect(src('src/mobile/watch/PinnedBanner.tsx')).toContain('var(--pin-name-shadow, 0 1px 2px rgba(0,0,0,0.8))');
    expect(src('src/styles/globals.css')).toMatch(/html\[data-theme-tone='light'\] \{\s*--pin-name-shadow: none;/);
  });
});

// @vitest-environment happy-dom
// SN-284 S2: what the top-of-chat stack does on the desktop surfaces. The hype
// train is a card of the stack (never of the header), stays mounted when Rust
// folds it, on the other views of the column and through a form change, and the
// stack is the same for people with Pin mentions off.
import { act, useEffect } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const mounts = vi.hoisted(() => ({ n: 0, unmounts: 0 }));
vi.mock('../HypeTrainBanner', () => ({
  default: function Banner({ train }: { train: { id: string } }) {
    useEffect(() => {
      mounts.n += 1;
      return () => {
        mounts.unmounts += 1;
      };
    }, []);
    return <div data-banner={train.id} />;
  },
}));
vi.mock('../../stores/AppStore', () => ({ useAppStore: (sel: (s: unknown) => unknown) => sel({ settings: {} }) }));
vi.mock('../../utils/lowPower', () => ({ usePerformanceQuiet: () => false }));

import { HypeStackCard, hypeStackId } from './HypeStackCard';
import type { StackCardControl } from './chatTopStack';
import type { HypeTrainData } from '../../types';

const src = (p: string) => readFileSync(resolve(__dirname, '..', '..', '..', p), 'utf8');

const train = (id: string): HypeTrainData =>
  ({ id, broadcaster_user_id: id, broadcaster_user_login: id, broadcaster_user_name: id, level: 2, total: 1, progress: 1, goal: 5, top_contributions: [], started_at: '', expires_at: '' }) as HypeTrainData;

const control = (over: Partial<StackCardControl> = {}): StackCardControl => ({
  placed: true,
  open: false,
  form: 'strip',
  height: 40,
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
  mounts.n = 0;
  mounts.unmounts = 0;
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const card = () => host.querySelector<HTMLElement>('[data-stack-card="hype"]');

describe('the hype train card', () => {
  it('reports every running train as its identity, and nothing when none runs', () => {
    expect(hypeStackId([train('a'), train('b')])).toBe('a|b');
    expect(hypeStackId([])).toBeNull();
  });

  it('is drawn when Rust gave it a strip', () => {
    act(() => root.render(<HypeStackCard trains={[train('a')]} stack={control()} />));
    expect(card()?.className).not.toContain('hidden');
    expect(host.querySelector('[data-banner="a"]')).not.toBeNull();
  });

  it('stays mounted, not drawn, when folded into "N more cards"', () => {
    act(() => root.render(<HypeStackCard trains={[train('a')]} stack={control({ form: 'folded', height: 0 })} />));
    expect(card()?.className.split(' ')).toContain('hidden');
    expect(host.querySelector('[data-banner="a"]')).not.toBeNull();
    expect(mounts.n).toBe(1);
  });

  it('stays mounted with no slot at all (the other views of the column)', () => {
    act(() => root.render(<HypeStackCard trains={[train('a')]} stack={control({ placed: false, form: null, height: 0 })} />));
    expect(host.querySelector('[data-banner="a"]')).not.toBeNull();
  });

  it('never remounts the banner on a form change or an order change', () => {
    const forms: Partial<StackCardControl>[] = [
      { form: 'strip' },
      { form: 'folded', height: 0 },
      { form: 'strip', order: 3 },
      { placed: false, form: null },
      { form: 'strip' },
    ];
    for (const f of forms) act(() => root.render(<HypeStackCard trains={[train('a')]} stack={control(f)} />));
    expect(mounts.n).toBe(1);
    expect(mounts.unmounts).toBe(0);
  });

  it('draws one strip per train', () => {
    act(() => root.render(<HypeStackCard trains={[train('a'), train('b')]} stack={control()} />));
    expect(host.querySelectorAll('[data-banner]')).toHaveLength(2);
  });

  it('gives its slot back as the last train ends', () => {
    const ids: Array<string | null> = [];
    const stack = control({ onPresence: (id) => ids.push(id) });
    act(() => root.render(<HypeStackCard trains={[train('a')]} stack={stack} />));
    act(() => root.render(<HypeStackCard trains={[]} stack={stack} />));
    expect(ids).toContain('a');
    expect(ids.at(-1)).toBeNull();
    expect(card()).toBeNull();
  });
});

describe('the desktop chat widget', () => {
  const widget = src('src/components/ChatWidget.tsx');
  const between = (a: string, b: string) => {
    const i = widget.indexOf(a);
    const j = widget.indexOf(b, i);
    expect(i).toBeGreaterThan(-1);
    expect(j).toBeGreaterThan(i);
    return widget.slice(i, j);
  };

  it('the measured header (chromeHeight) no longer holds the hype train', () => {
    const header = between('ref={setChromeEl}', 'Staging area removed');
    expect(header).not.toContain('HypeTrainBanner');
    expect(header).not.toContain('HypeStackCard');
    expect(header).not.toContain('currentHypeTrain &&');
  });

  it('the hype train is a card of the stack, mounted on every view', () => {
    const stack = between('<ChatOverlayStack', '</ChatOverlayStack>');
    expect(stack).toContain('<HypeStackCard');
    expect(stack).toContain('topStack.control.hype');
    expect(stack).not.toMatch(/activeView === 'chat' &&\s*\n?\s*<HypeStackCard/);
    expect(stack).not.toMatch(/hidden=\{activeView/);
  });

  it('the other views of the column ask for the hype train alone', () => {
    expect(widget).toMatch(/only: activeView === 'chat' && !searchOpen \? undefined : HYPE_ONLY/);
    expect(widget).toContain("const HYPE_ONLY: readonly StackKind[] = ['hype']");
  });

  it('every user of the chrome height reads the stack bottom instead', () => {
    expect(widget).toContain('const paneTop = Math.max(chromeHeight, stackBottom)');
    expect(widget).toMatch(/animate-panel-slide-up" style=\{\{ paddingTop: paneTop \}\}/);
    expect(widget).toMatch(/key="modroom-pane"[\s\S]*?style=\{\{ paddingTop: paneTop \}\}/);
    expect(widget).toMatch(/<DropsChatBar[\s\S]*?headerInset=\{paneTop\}/);
    expect(widget).toMatch(/<ChatMessagesPanel[\s\S]*?headerInset=\{listSpacer\}/);
    // The header-only users keep the header: the dock list under it and the combined-chat bar.
    expect(widget).toContain('<ChatDockPanel top={chromeHeight} />');
  });

  it('the channel pin, the mention context card and the pins are stack cards, not floats', () => {
    const stack = between('<ChatOverlayStack', '</ChatOverlayStack>');
    for (const kind of ['channel_pin', 'mention_context', 'mention_pins']) expect(stack).toContain(`topStack.control.${kind}`);
    expect(widget).not.toContain('pinnedTop');
    expect(widget).not.toContain('overlayStackHeight');
  });

  it('every writer of the channel pin writes the one store the stack reads', () => {
    expect(widget).toContain('channelPinOpened: isPinnedExpanded');
    expect(widget).toContain('onChannelPinOpen: setIsPinnedExpanded');
    // The header's pin button and the bar's own taps write the same state.
    expect(widget).toContain('setIsPinnedExpanded(next)');
    expect(widget).toMatch(/onExpand=\{\(pinId\) => \{\s*setIsPinnedExpanded\(true\)/);
    expect(widget).toContain('onCollapse={() => setIsPinnedExpanded(false)}');
  });

  it('the stack is the same with Pin mentions off', () => {
    const stack = between('<ChatOverlayStack', '</ChatOverlayStack>');
    expect(stack).not.toContain('pin_mentions');
    expect(src('src/components/chat/chatTopStack.ts')).not.toContain('pin_mentions');
    expect(src('src/components/chat/ChatOverlayStack.tsx')).not.toContain('pin_mentions');
    expect(src('src/components/multichat/BlendedChatPane.tsx')).toMatch(/<HypeStackCard/);
  });

  it('the combined feed and the pane ids follow the design', () => {
    const blended = src('src/components/multichat/BlendedChatPane.tsx');
    expect(blended).toContain("useChatTopStack(feedEl, { surface:");
    expect(blended).not.toMatch(/<HypeTrainBanner/);
    const win = src('src/components/multichat/MultiChatWindow.tsx');
    expect(win).toContain('surfaceId={`pane:${l.id}`}');
    expect(win).toContain('surfaceId={`pane:${entryKey(entry)}`}');
  });
});

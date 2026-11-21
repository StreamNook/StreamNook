// @vitest-environment happy-dom
// SN-284 S2: the stack hook as a shell uses it. Every case of the shared case
// file is fed through the hook and each card must come out placed as Rust said;
// the other views of the chat column ask Rust for the hype train alone; and the
// channel pin's open state is one store that the strip and every other writer
// share.
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../../stores/AppStore', () => ({
  useAppStore: (sel: (s: unknown) => unknown) => sel({ settings: { chat_overlay_order: 'prediction-first', chat_design: {} } }),
}));
vi.mock('../../utils/logger', () => ({ Logger: { debug: vi.fn() } }));

import { invoke } from '@tauri-apps/api/core';
import {
  useChatTopStack,
  type ChatTopStackOptions,
  type StackKind,
  type StackLayout,
  type StackRequest,
} from './chatTopStack';

type Hook = ReturnType<typeof useChatTopStack>;

const root = resolve(__dirname, '..', '..', '..');
interface StackCase {
  name: string;
  request: StackRequest;
  layout: StackLayout;
}
const cases = JSON.parse(readFileSync(resolve(root, 'src-tauri/tests/fixtures/chat-top-stack-cases.json'), 'utf8')) as StackCase[];

class FakeResizeObserver {
  observe() {}
  disconnect() {}
  unobserve() {}
}

let host: HTMLElement;
let reactRoot: Root;
let latest: Hook;

function column(height: number): HTMLElement {
  const el = document.createElement('div');
  el.getBoundingClientRect = () => ({ height, width: 300, top: 0, left: 0, right: 300, bottom: height, x: 0, y: 0, toJSON() {} }) as DOMRect;
  return el;
}

function Harness({ cards, opts, col }: { cards: StackKind[]; opts?: ChatTopStackOptions; col: HTMLElement }) {
  const stack = useChatTopStack(col, opts);
  React.useLayoutEffect(() => {
    latest = stack;
  });
  // The cards report themselves, as a mounted card does.
  return (
    <>
      {cards.map((k) => (
        <Reporter key={k} kind={k} onPresence={stack.control[k].onPresence} />
      ))}
    </>
  );
}
function Reporter({ kind, onPresence }: { kind: StackKind; onPresence: (id: string | null) => void }) {
  React.useEffect(() => onPresence(kind), [kind, onPresence]);
  return null;
}
const flush = async () => {
  await act(async () => {
    await new Promise((r) => setTimeout(r, 5));
  });
};

function mount(cards: StackKind[], opts?: ChatTopStackOptions, h = 1000) {
  const col = column(h);
  act(() => reactRoot.render(<Harness cards={cards} opts={opts} col={col} />));
}

const requests = () => vi.mocked(invoke).mock.calls.filter(([c]) => c === 'chat_top_stack').map(([, a]) => (a as { request: StackRequest }).request);

beforeEach(() => {
  (globalThis as { ResizeObserver?: unknown }).ResizeObserver = FakeResizeObserver;
  vi.mocked(invoke).mockReset();
  host = document.createElement('div');
  document.body.appendChild(host);
  reactRoot = createRoot(host);
});
afterEach(() => {
  act(() => reactRoot.unmount());
  host.remove();
});

describe('the stack hook over the shared case file', () => {
  it('has cases', () => expect(cases.length).toBeGreaterThan(0));

  for (const c of cases) {
    it(`places every card as Rust said: ${c.name}`, async () => {
      vi.mocked(invoke).mockResolvedValue(c.layout);
      mount(c.request.cards as StackKind[]);
      await flush();
      for (const slot of c.layout.slots) {
        const control = latest.control[slot.kind as StackKind];
        expect(control.placed, slot.kind).toBe(true);
        expect(control.form).toBe(slot.form);
        expect(control.height).toBe(slot.height);
        expect(control.open).toBe(slot.open);
      }
      expect(latest.height).toBe(c.layout.height);
      expect(latest.more?.count ?? 0).toBe(c.layout.more?.count ?? 0);
      // A card the answer left out has no slot.
      const named = new Set(c.layout.slots.map((s) => s.kind));
      for (const k of c.request.cards as StackKind[]) {
        if (!named.has(k)) expect(latest.control[k].placed).toBe(false);
      }
    });
  }
});

describe('the other views of the chat column', () => {
  it('ask Rust about the hype train alone while every card stays mounted', async () => {
    vi.mocked(invoke).mockResolvedValue({
      slots: [{ kind: 'hype', open: false, form: 'strip', height: 40 }],
      max_height: 400,
      height: 40,
      room_key: '',
    } satisfies StackLayout);
    mount(['hype', 'poll', 'mention_pins'], { only: ['hype'] });
    await flush();
    const last = requests().at(-1)!;
    expect(last.cards).toEqual(['hype']);
    expect(latest.control.hype.placed).toBe(true);
    expect(latest.control.poll.placed).toBe(false);
    expect(latest.control.mention_pins.placed).toBe(false);
    // Chat again: the rest are asked for with no new report from the cards.
    act(() => reactRoot.render(<Harness cards={['hype', 'poll', 'mention_pins']} opts={{}} col={column(1000)} />));
    await flush();
    expect(requests().at(-1)!.cards).toEqual(['hype', 'poll', 'mention_pins']);
  });

  it('draw no height at all while no card is asked for', async () => {
    vi.mocked(invoke).mockResolvedValue({ slots: [], max_height: 400, height: 90, room_key: '' } satisfies StackLayout);
    mount([], { only: ['hype'] });
    await flush();
    expect(latest.height).toBe(0);
    expect(latest.maxHeight).toBe(0);
  });
});

describe('the channel pin has one open store', () => {
  const layout = (open: boolean): StackLayout => ({
    slots: [{ kind: 'channel_pin', open, form: open ? 'open' : 'strip', height: open ? 90 : 36 }],
    max_height: 400,
    height: open ? 90 : 36,
    room_key: 'xqc',
  });

  // Rust opens the pin when the viewer opened it or the store says so.
  const rustAnswers = () =>
    vi.mocked(invoke).mockImplementation(async (_cmd, args) => {
      const r = (args as { request: StackRequest }).request;
      const folded = r.folded.includes('channel_pin');
      return layout(r.opened === 'channel_pin' || (r.channel_pin_opened && !folded));
    });

  it('sends the store as the request, and a tap on the strip writes it back', async () => {
    rustAnswers();
    const writes: boolean[] = [];
    mount(['channel_pin'], { channelPinOpened: false, onChannelPinOpen: (o) => writes.push(o), provider: 'twitch', channel: 'xqc' });
    await flush();
    expect(requests().at(-1)!.channel_pin_opened).toBe(false);
    act(() => latest.control.channel_pin.onToggle());
    expect(writes).toEqual([true]);
    expect(latest.control.channel_pin.open).toBe(true);
    // Folding it again writes the other way.
    await flush();
    act(() => latest.control.channel_pin.onToggle());
    expect(writes).toEqual([true, false]);
  });

  it('follows the store when another writer changes it, as a tap on the strip', async () => {
    rustAnswers();
    const col = column(1000);
    const writes: boolean[] = [];
    const opts = (open: boolean): ChatTopStackOptions => ({ channelPinOpened: open, onChannelPinOpen: (o) => writes.push(o) });
    act(() => reactRoot.render(<Harness cards={['channel_pin']} opts={opts(false)} col={col} />));
    await flush();
    // The composer menu or the header's pin button opens it.
    act(() => reactRoot.render(<Harness cards={['channel_pin']} opts={opts(true)} col={col} />));
    await flush();
    const asked = requests().at(-1)!;
    expect(asked.channel_pin_opened).toBe(true);
    expect(asked.opened).toBe('channel_pin');
    // Following the store is not a write of its own.
    expect(writes).toEqual([]);
    // And the other way: closing it folds the pin.
    act(() => reactRoot.render(<Harness cards={['channel_pin']} opts={opts(false)} col={col} />));
    await flush();
    const closed = requests().at(-1)!;
    expect(closed.folded).toContain('channel_pin');
    expect(closed.opened).toBeNull();
  });

  it('opening another card closes the pin in the store', async () => {
    vi.mocked(invoke).mockResolvedValue({
      slots: [
        { kind: 'poll', open: false, form: 'strip', height: 36 },
        { kind: 'channel_pin', open: true, form: 'open', height: 90 },
      ],
      max_height: 400,
      height: 126,
      room_key: 'xqc',
    } satisfies StackLayout);
    const writes: boolean[] = [];
    mount(['poll', 'channel_pin'], { channelPinOpened: true, onChannelPinOpen: (o) => writes.push(o) });
    await flush();
    act(() => latest.control.poll.onToggle());
    expect(writes).toEqual([false]);
  });
});

describe('a card that toggles itself as it appears', () => {
  // The mention context card opens once as it is placed: its effect runs in the commit that
  // brought its slot, so the hook must already hold that layout, not the one before it.
  function OpensWhenPlaced({ kind, control }: { kind: StackKind; control: Hook['control'][StackKind] }) {
    const done = React.useRef(false);
    React.useEffect(() => {
      if (!control.placed || done.current) return;
      done.current = true;
      control.onToggle();
    }, [control]);
    return <Reporter kind={kind} onPresence={control.onPresence} />;
  }
  function Host({ col }: { col: HTMLElement }) {
    const stack = useChatTopStack(col);
    React.useLayoutEffect(() => {
      latest = stack;
    });
    return <OpensWhenPlaced kind="mention_context" control={stack.control.mention_context} />;
  }

  it('toggles against the layout that placed it', async () => {
    const first: StackLayout = {
      slots: [{ kind: 'mention_context', open: false, form: 'strip', height: 36 }],
      max_height: 400,
      height: 36,
      room_key: 'xqc',
    };
    // Rust's first answer places the card; a second one never comes in this test.
    vi.mocked(invoke).mockResolvedValueOnce(first).mockImplementation(() => new Promise(() => {}));
    act(() => reactRoot.render(<Host col={column(1000)} />));
    await flush();
    expect(latest.control.mention_context.placed).toBe(true);
    expect(latest.control.mention_context.open).toBe(true);
  });
});

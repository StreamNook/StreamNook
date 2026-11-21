// @vitest-environment happy-dom
// SN-284 S2, Android: the top-of-chat stack on the phone. Every surface asks
// Rust's one stack (the watch screen, the over-video column, the lower third
// and the Chats screen), floats it over the list, and the pinned mentions
// open by location: the chat's own row scrolls to it, another chat's opens the
// Mentions sheet at its entry.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { MotionGlobalConfig } from 'framer-motion';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

MotionGlobalConfig.skipAnimations = true;
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null), convertFileSrc: (p: string) => p, Channel: class {} }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn(async () => {}) }));
vi.mock('../../utils/platform', () => ({
  IS_MOBILE: true,
  IS_ANDROID: true,
  IS_MAC: false,
  IS_LINUX: false,
  IS_WEBKITGTK: false,
  IS_MAIN_WINDOW: true,
  isMainWindow: () => true,
}));

import { PhonePins } from './PhonePins';
import { MentionContextCard } from '../mentions/MentionContextCard';
import { requestMentionFocus } from '../../stores/mentionFocusStore';
import * as focusStore from '../../stores/mentionFocusStore';
import { useMentionsStore } from '../mentions/mentionInbox';
import { samplePin } from '../../components/settings/pinSamples';
import { foldedStack } from '../watch/verticalChatParts';
import type { StackCardControl, StackKind } from '../../components/chat/chatTopStack';
import type { MentionPin } from '../../types';

const src = (p: string) => readFileSync(resolve(__dirname, '..', '..', '..', p), 'utf8');

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  useMentionsStore.setState({ sheetIn: null });
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.restoreAllMocks();
});

const pin = (n: number, over: Partial<MentionPin> = {}): MentionPin => ({
  ...samplePin(n, 'mention', `user${n}`, '#ffffff', `message ${n}`, '9:00 PM'),
  ...over,
});

const control = (over: Partial<StackCardControl> = {}): StackCardControl => ({
  placed: true,
  open: false,
  form: 'floor',
  height: 64,
  moreCards: 0,
  order: 6,
  onToggle: () => {},
  onPresence: () => {},
  onWaiting: () => {},
  ...over,
});

const press = (el: Element | null) => {
  expect(el).not.toBeNull();
  act(() => {
    (el as HTMLElement).dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });
};

describe('the pinned mentions as the last card of the phone stack', () => {
  it("draw in Rust's form and report themselves while there are pins", () => {
    const ids: Array<string | null> = [];
    const stack = control({ onPresence: (id) => ids.push(id), form: 'count_line', height: 28, moreCards: 2 });
    act(() =>
      root.render(<PhonePins pins={[pin(1), pin(2)]} shown reportVisible={() => {}} inboxSurface="watch" stack={stack} />),
    );
    expect(host.querySelector('[data-stack-card="mention_pins"]')).not.toBeNull();
    expect(host.querySelector('[data-mention-pins="count_line"]')).not.toBeNull();
    expect(host.textContent).toContain('2 more cards');
    expect(ids).toContain('pins');
  });

  it('draw nothing without a slot or while folded, and give the slot back with no pins', () => {
    const ids: Array<string | null> = [];
    const stack = control({ onPresence: (id) => ids.push(id) });
    act(() =>
      root.render(
        <PhonePins pins={[pin(1)]} shown reportVisible={() => {}} inboxSurface="watch" stack={{ ...stack, placed: false, form: null }} />,
      ),
    );
    expect(host.querySelector('[data-mention-pins]')).toBeNull();
    act(() =>
      root.render(<PhonePins pins={[]} shown reportVisible={() => {}} inboxSurface="watch" stack={stack} />),
    );
    expect(ids.at(-1)).toBeNull();
  });

  it('wear the over-video ink in the lower third and the landscape column', () => {
    act(() =>
      root.render(<PhonePins pins={[pin(1)]} shown reportVisible={() => {}} inboxSurface="watch" overVideo stack={control()} />),
    );
    expect(host.querySelector('[data-mention-pins]')?.className).toContain('sn-over-video');
  });
});

describe('the opener route on the phone', () => {
  it("a pin from another chat opens the Mentions sheet of the surface at its entry", () => {
    const requested: Array<{ id: string; room: string }> = [];
    vi.spyOn(focusStore, 'requestMentionFocus').mockImplementation((e) => {
      requested.push(e);
    });
    const other = pin(3, { channel_mark: { platform: 'twitch', channel: 'other' } as MentionPin['channel_mark'] });
    act(() =>
      root.render(<PhonePins pins={[other]} shown reportVisible={() => {}} inboxSurface="chats" stack={control()} />),
    );
    press(host.querySelector('[data-pin-id] [role="button"]'));
    expect(useMentionsStore.getState().sheetIn).toBe('chats');
    expect(requested).toEqual([{ id: other.entry.id, room: other.entry.room }]);
    expect(typeof requestMentionFocus).toBe('function');
  });

  it("the lower third grows the chat before it opens a pin", () => {
    const grown = vi.fn();
    const other = pin(4, { channel_mark: { platform: 'twitch', channel: 'other' } as MentionPin['channel_mark'] });
    act(() =>
      root.render(
        <PhonePins pins={[other]} shown reportVisible={() => {}} inboxSurface="watch" onBeforeOpen={grown} stack={control()} />,
      ),
    );
    press(host.querySelector('[data-pin-id] [role="button"]'));
    expect(grown).toHaveBeenCalledTimes(1);
    expect(useMentionsStore.getState().sheetIn).toBe('watch');
  });
});

describe('the mention context card in the lower third', () => {
  const entry = pin(9).entry;
  const open = (autoOpen: boolean, onToggle: () => void) =>
    act(() => root.render(<MentionContextCard entry={entry} stack={control({ form: 'strip', onToggle })} autoOpen={autoOpen} />));

  it('opens once as it appears, except where opening would grow the chat by itself', () => {
    const onToggle = vi.fn();
    open(false, onToggle);
    expect(onToggle).not.toHaveBeenCalled();
    // Leaving the lower third (the chat grown by the viewer) lets it open, once.
    open(true, onToggle);
    expect(onToggle).toHaveBeenCalledTimes(1);
    open(true, onToggle);
    expect(onToggle).toHaveBeenCalledTimes(1);
  });
});

describe('the screens register through the shown rules', () => {
  it('the watch screen reports with the stream in the rule, so no stream is never in front', () => {
    const watch = src('src/mobile/screens/WatchScreen.tsx');
    expect(watch).toMatch(/const pinSurface = watchPinSurface\(\{\s*show,/);
    expect(watch).toContain('chatOutOfSight: pinSurface.pinsOutOfSight');
    expect(watch).toContain('const chatOutOfSight = pinSurface.chatOutOfSight');
    expect(src('src/mobile/screens/ChatsScreen.tsx')).toContain('chatOutOfSight: chatsChatOutOfSight({ shown, layerUp, idle })');
  });
});

describe('the cleared tall chat keeps its pins over the video', () => {
  it('a tap on a pin grows the chat first', () => {
    const grow = vi.fn();
    const other = pin(5, { channel_mark: { platform: 'twitch', channel: 'other' } as MentionPin['channel_mark'] });
    act(() =>
      root.render(
        <PhonePins pins={[other]} shown reportVisible={() => {}} inboxSurface="watch" overVideo onBeforeOpen={grow} stack={control()} />,
      ),
    );
    expect(host.querySelector('[data-mention-pins]')?.className).toContain('sn-over-video');
    press(host.querySelector('[data-pin-id] [role="button"]'));
    expect(grow).toHaveBeenCalledTimes(1);
  });

  it('the screen draws them outside the hidden column and asks the pins alone as strips', () => {
    const watch = src('src/mobile/screens/WatchScreen.tsx');
    expect(watch).toContain('only: chatOverlay || verticalCleared ? PINS_ONLY : undefined');
    expect(watch).toContain('{pinSurface.clearedOverVideo && mentionPins.pins.length > 0 && (');
    expect(watch).toContain('{!verticalCleared && (');
    expect(watch).toContain("onBeforeOpen={() => moveVerticalChat('expanded')}");
  });
});

describe('the lower third is strips only', () => {
  const kinds: StackKind[] = ['hype', 'prediction', 'poll', 'drop', 'channel_pin', 'mention_context', 'mention_pins'];
  const cards = Object.fromEntries(
    kinds.map((k) => [k, { ...control({ form: 'strip', open: true }), onToggle: vi.fn() }]),
  ) as unknown as Record<StackKind, StackCardControl>;

  it('shows every card closed and a tap on one grows the chat with it open', () => {
    const grow = vi.fn();
    const folded = foldedStack({ ...cards, channel_pin: { ...cards.channel_pin, open: false } }, true, grow);
    for (const k of kinds) expect(folded[k].open).toBe(false);
    folded.channel_pin.onToggle();
    expect(grow).toHaveBeenCalledTimes(1);
  });

  it('asks Rust for strips only, so nothing opens and the pins are their floor', () => {
    const cases = JSON.parse(src('src-tauri/tests/fixtures/chat-top-stack-cases.json')) as Array<{
      request: { strips_only: boolean };
      layout: { slots: Array<{ kind: string; open: boolean; form: string }> };
    }>;
    const lower = cases.filter((c) => c.request.strips_only);
    expect(lower.length).toBeGreaterThan(0);
    for (const c of lower) {
      expect(c.layout.slots.some((s) => s.open)).toBe(false);
      const pins = c.layout.slots.find((s) => s.kind === 'mention_pins');
      if (pins) expect(['floor', 'count_line']).toContain(pins.form);
    }
    expect(src('src/mobile/screens/WatchScreen.tsx')).toContain('stripsOnly: verticalLower');
  });
});

describe('the phone surfaces ask the one stack', () => {
  const watch = src('src/mobile/screens/WatchScreen.tsx');
  const chats = src('src/mobile/screens/ChatsScreen.tsx');
  const pane = src('src/mobile/chat/MobileChatPane.tsx');

  it('the watch screen lays every card out through Rust, the landscape column asks for the pins alone', () => {
    expect(watch).toContain("surface: 'phone-watch'");
    expect(watch).toMatch(/only: chatOverlay \|\| verticalCleared \? PINS_ONLY : undefined/);
    expect(watch).toContain("const PINS_ONLY: readonly StackKind[] = ['mention_pins']");
    for (const kind of ['hype', 'prediction', 'poll', 'channel_pin', 'mention_context', 'mention_pins']) {
      expect(watch).toContain(`stack={stackControl.${kind}}`);
    }
    // The old floating header and its measuring are gone.
    expect(watch).not.toContain('topStackBottom');
    expect(watch).not.toContain('<VerticalMentionStrip');
    expect(watch).not.toContain('<HypeTrainBanner');
  });

  it('the hype train is one compact strip in the same view, in the lower third too', () => {
    expect(watch).toMatch(/<HypeStackCard[\s\S]*?compact/);
  });

  it('the Chats screen no longer makes room for the pin in the flow', () => {
    expect(chats).toContain("surface: 'phone-chats'");
    expect(chats).not.toContain('pinsBox');
    expect(chats).not.toMatch(/style=\{\{ height: pinsBox \}\}/);
    expect(chats).toContain('topInset={stackBottom}');
  });

  it('the list keeps its whole height: the stack floats and the list gets a spacer', () => {
    expect(pane).toMatch(/absolute inset-x-0 top-0 z-20 h-0/);
    expect(pane).toContain('headerInset={Math.max(40, topInset)}');
    // Not in the flow between the tab strip and the list any more.
    expect(pane).not.toMatch(/\{strip && <ChatTabStrip \/>\}\s*\n\s*\{top\}/);
  });

  it('every writer of the pin writes the one per-room store the stack reads', () => {
    expect(watch).toContain('onChannelPinOpen: pins.setOpen');
    expect(chats).toContain('onChannelPinOpen: pins.setOpen');
    expect(src('src/mobile/chat/MobileChatInput.tsx')).toContain('pins.setOpen(true)');
    expect(src('src/mobile/watch/pinsOpen.ts')).toContain('usePinsOpenStore');
  });
});

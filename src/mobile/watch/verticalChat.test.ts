// The tall layout's chat on Android (SN-40): the lower third's rows are inert
// and live, its cards fold to strips that grow the chat, Back folds a grown
// chat first, and the watch layer wires every gesture and state through the
// shared rules. Source pins read the files as text (CRLF-normalized), as the
// other wiring tests here do.
import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn() }));
vi.mock('../../bootPreload', () => ({ takePreloadedSettings: () => null }));

import { foldedStack } from './verticalChatParts';
import type { StackCardControl, StackKind } from '../../components/chat/chatTopStack';
import { useMobileNavStore } from '../navStore';
import { useAppStore } from '../../stores/AppStore';

const read = (rel: string) => readFileSync(resolve(__dirname, rel), 'utf8').replace(/\r\n/g, '\n');

function control(open: boolean): StackCardControl & { toggled: number } {
  const c = {
    placed: true,
    open,
    order: 0,
    toggled: 0,
    onToggle: () => {
      c.toggled += 1;
    },
    onPresence: () => {},
    onWaiting: () => {},
  };
  return c;
}

describe('the lower third folds the poll and prediction cards', () => {
  it('draws every card as its strip, and a tap grows the chat with that card open', () => {
    const poll = control(true);
    const prediction = control(false);
    const drop = control(false);
    let grown = 0;
    const cards = { poll, prediction, drop } as Record<StackKind, StackCardControl>;
    const folded = foldedStack(cards, true, () => (grown += 1));
    expect(folded.poll.open).toBe(false);
    expect(folded.prediction.open).toBe(false);
    folded.prediction.onToggle();
    expect(grown).toBe(1);
    expect(prediction.toggled).toBe(1);
    // A card already open stays open as the chat grows.
    folded.poll.onToggle();
    expect(grown).toBe(2);
    expect(poll.toggled).toBe(0);
  });

  it('leaves the stack alone outside the lower third', () => {
    const cards = { poll: control(true), prediction: control(false), drop: control(false) } as Record<
      StackKind,
      StackCardControl
    >;
    expect(foldedStack(cards, false, () => {})).toBe(cards);
  });
});

describe('Back in the tall layout', () => {
  it('folds a grown chat to the lower third before the player shrinks', () => {
    useAppStore.setState({ streamUrl: 'http://localhost/relay.m3u8', isLoading: false });
    useMobileNavStore.setState({ playerMode: 'full', verticalTall: true, verticalChat: 'expanded', sheetStack: [] });
    expect(useMobileNavStore.getState().handleBack()).toBe(true);
    expect(useMobileNavStore.getState().verticalChat).toBe('lower');
    expect(useMobileNavStore.getState().playerMode).toBe('full');
    expect(useMobileNavStore.getState().handleBack()).toBe(true);
    expect(useMobileNavStore.getState().playerMode).toBe('mini');
  });
});

describe('the lower third gates (source pins)', () => {
  const pane = read('../chat/MobileChatPane.tsx');

  it('turns off row swipes, the reply swipe, the hold and the pause intent on its rows', () => {
    expect(pane).toContain("const folded = vertical === 'lower' || vertical === 'cleared';");
    expect(pane).toMatch(/const swipeOn = [^;]*&& !folded;/);
    expect(pane).toMatch(/const replyEnabled = [^;]*&& !folded;/);
    for (const handler of ['onPointerDown', 'onPointerMove', 'onPointerUp', 'onPointerCancel']) {
      expect(pane).toContain(`${handler}={folded ? undefined : press.${handler}}`);
    }
    expect(pane).toContain('onPauseIntent={folded ? undefined : () => pause(true)}');
    // The folded list carries sn-chat-vertical (the over-video ink beside it is
    // overVideo.test.ts's pin).
    expect(pane).toMatch(/\$\{folded \? [`'] sn-chat-vertical\b/);
    expect(pane).toContain('{folded && lowerTouch}');
  });

  it('is always live, and has no VOD switch', () => {
    expect(pane).toMatch(/if \(folded && isPaused\) resumeLive\(\);/);
    expect(pane).toContain('{vodHere && !readOnly && !folded && (');
  });

  it('keeps room for the chat button beside the message box', () => {
    expect(pane).toContain('trailingSlot={vertical ? CHAT_BUTTON_SLOT : 0}');
  });

  it('gives each row its own backing and leaves the rows inert', () => {
    const css = read('../mobile.css');
    expect(css).toContain('.sn-mobile .sn-chat-vertical .chat-message-row {\n  pointer-events: none;');
    expect(css).toMatch(/\.sn-chat-vertical \.chat-message-row > \* \{[^}]*var\(--fs-chat-opacity/);
  });
});

describe('the watch layer wires the tall layout (source pins)', () => {
  const watch = read('../screens/WatchScreen.tsx');

  it('decides the layout and the mini box by the shared rule', () => {
    expect(watch).toContain("const tall = tallLayout({ ...tallInputs, playerMode: mini ? 'mini' : 'full' });");
    expect(watch).toContain('const miniBox = useMemo(() => miniSize(tallBox), [tallBox]);');
    expect(watch).toContain('const miniScale = miniBox.width / viewport.w;');
  });

  it('latches a tall drag at the slop and decides it on release from the state at touch-down', () => {
    expect(watch).toContain('let drag = latch(g.tall.region, g.tall.from, dx, dy);');
    expect(watch).toContain("if (drag === 'swipe' && !g.swipeOk) drag = 'none';");
    expect(watch).toContain('release(t.drag, t.from, e.clientX - g.x, e.clientY - g.y');
    expect(watch).toContain('swipeOk: tallSwipeCanStart(e.clientX, window.innerWidth, ins.gestureLeft, ins.gestureRight),');
  });

  it('remembers shown and cleared only, by path', () => {
    expect(watch).toContain("void setSettingsFields({ 'fullscreen_chat.vertical_open': next !== 'cleared' })");
    expect(watch).toContain("entryState(useAppStore.getState().settings.fullscreen_chat?.vertical_open ?? true)");
  });

  it('pushes the PiP shape, and never takes a PiP window for a turn', () => {
    expect(watch).toContain('setPipTall(pipTall);');
    expect(watch).toContain('if (pip || isInPip() === true) {');
  });
});

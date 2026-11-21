// @vitest-environment happy-dom
// SN-284, Android: what each phone chat surface tells Rust about itself. A
// surface that draws nothing must say it is not shown (Rust ranks the watch
// screen before the Chats screen), and the cleared tall chat keeps its pins as
// strips over the video, as the iPhone does.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string) => (cmd === 'get_mention_pins' ? { pins: [], revision: 0, max: 0 } : null)),
  convertFileSrc: (p: string) => p,
  Channel: class {},
}));
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

import { usePhonePins, type PhonePinsInput } from './PhonePins';
import { chatsChatOutOfSight, watchChatOutOfSight, watchPinSurface, type WatchPinInputs } from './pinSurfaceRules';
import { pinSurfaces } from '../../utils/pinSurfaces';
import { useMentionsStore } from '../mentions/mentionInbox';

const base: WatchPinInputs = {
  show: true,
  mini: false,
  pip: false,
  chatClass: 'flex-1 min-h-0 relative flex flex-col',
  chatOverlay: false,
  verticalLower: false,
  verticalGrown: false,
  verticalCleared: false,
  sideBySide: false,
};

describe('the watch screen\'s rule', () => {
  it('is out of sight with no stream, in the mini box, in PiP and with the column hidden', () => {
    expect(watchChatOutOfSight({ show: true, mini: false, chatClass: 'flex' })).toBe(false);
    expect(watchChatOutOfSight({ show: false, mini: false, chatClass: 'flex' })).toBe(true);
    expect(watchChatOutOfSight({ show: true, mini: true, chatClass: 'flex' })).toBe(true);
    expect(watchChatOutOfSight({ show: true, mini: false, chatClass: 'hidden' })).toBe(true);
  });

  it('with no stream, neither the chat nor a cleared chat draws a pin', () => {
    expect(watchPinSurface({ ...base, show: false }).pinsOutOfSight).toBe(true);
    expect(watchPinSurface({ ...base, show: false, chatClass: 'hidden', verticalCleared: true }).pinsOutOfSight).toBe(true);
  });

  it('the mini box and PiP draw no pin, cleared or not', () => {
    for (const over of [{ mini: true }, { pip: true, chatClass: 'hidden' }] as Array<Partial<WatchPinInputs>>) {
      expect(watchPinSurface({ ...base, ...over }).pinsOutOfSight).toBe(true);
      expect(watchPinSurface({ ...base, ...over, verticalCleared: true, chatClass: 'hidden' }).pinsOutOfSight).toBe(true);
    }
  });

  it('a cleared tall chat keeps its pins in sight as over-video strips only', () => {
    const s = watchPinSurface({ ...base, chatClass: 'hidden', verticalCleared: true });
    expect(s.chatOutOfSight).toBe(true);
    expect(s.pinsOutOfSight).toBe(false);
    expect(s.clearedOverVideo).toBe(true);
    expect(s).toMatchObject({ mode: 'over-video', stripsOnly: true, overVideo: true });
  });

  it('the lower third is over-video strips, the grown chat is the vertical layout, a landscape overlay is over-video', () => {
    expect(watchPinSurface({ ...base, verticalLower: true })).toMatchObject({ mode: 'vertical', stripsOnly: true, overVideo: true, pinsOutOfSight: false });
    expect(watchPinSurface({ ...base, verticalGrown: true })).toMatchObject({ mode: 'vertical', stripsOnly: false, overVideo: false });
    expect(watchPinSurface({ ...base, chatOverlay: true, sideBySide: true })).toMatchObject({ mode: 'over-video', stripsOnly: false, overVideo: true });
    expect(watchPinSurface({ ...base, sideBySide: true }).mode).toBe('landscape');
    expect(watchPinSurface(base).mode).toBe('stacked');
  });

  it('the Chats screen is out of sight on another tab, under the player, or unseen', () => {
    expect(chatsChatOutOfSight({ shown: true, layerUp: false, idle: false })).toBe(false);
    expect(chatsChatOutOfSight({ shown: false, layerUp: false, idle: false })).toBe(true);
    expect(chatsChatOutOfSight({ shown: true, layerUp: true, idle: false })).toBe(true);
    expect(chatsChatOutOfSight({ shown: true, layerUp: false, idle: true })).toBe(true);
  });
});

let host: HTMLElement;
let root: Root;

function Probe({ input }: { input: PhonePinsInput }) {
  usePhonePins(input);
  return null;
}

const mount = (input: PhonePinsInput) => act(() => root.render(<Probe input={input} />));
const specOf = (id: string) => pinSurfaces.peek().find((s) => s.id === id);

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  pinSurfaces.setEnabled(true);
  useMentionsStore.setState({ sheetIn: null });
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  pinSurfaces.setEnabled(false);
});

describe('what the surfaces register with Rust', () => {
  const watchInput = (over: Partial<WatchPinInputs>): PhonePinsInput => {
    const s = watchPinSurface({ ...base, ...over });
    return {
      surfaceId: 'phone-watch',
      room: 'xqc',
      mode: s.mode,
      chatOutOfSight: s.pinsOutOfSight,
      stripsOnly: s.stripsOnly,
      overVideo: s.overVideo,
    };
  };

  it('with no stream the watch surface is not shown and the Chats surface is: the pin homes where it draws', () => {
    act(() =>
      root.render(
        <>
          <Probe input={watchInput({ show: false })} />
          <ChatsProbe />
        </>,
      ),
    );
    expect(specOf('phone-watch')).toMatchObject({ shown: false, active: false });
    const shown = pinSurfaces.peek().filter((s) => s.shown).map((s) => s.id);
    expect(shown).toEqual(['phone-chats']);
  });

  it.each([
    ['the mini box', { mini: true }],
    ['PiP', { pip: true, chatClass: 'hidden' }],
    ['a hidden column', { chatClass: 'hidden' }],
  ] as Array<[string, Partial<WatchPinInputs>]>)('%s registers the watch surface as not shown', (_name, over) => {
    mount(watchInput(over));
    expect(specOf('phone-watch')?.shown).toBe(false);
  });

  it('a cleared tall chat registers shown, over video, strips only', () => {
    mount(watchInput({ chatClass: 'hidden', verticalCleared: true }));
    expect(specOf('phone-watch')).toMatchObject({ shown: true, active: true, over_video: true, strips_only: true, mode: 'over-video' });
  });

  it('a full player layer over the Chats screen registers it as not shown', () => {
    mount({ surfaceId: 'phone-chats', room: 'xqc', mode: '', chatOutOfSight: chatsChatOutOfSight({ shown: true, layerUp: true, idle: false }) });
    expect(specOf('phone-chats')?.shown).toBe(false);
  });

  it('a sheet over the chat registers the surface as not shown', () => {
    useMentionsStore.setState({ sheetIn: 'watch' });
    mount(watchInput({}));
    expect(specOf('phone-watch')?.shown).toBe(false);
  });
});

function ChatsProbe() {
  usePhonePins({ surfaceId: 'phone-chats', room: 'xqc', mode: '', chatOutOfSight: chatsChatOutOfSight({ shown: true, layerUp: false, idle: false }) });
  return null;
}

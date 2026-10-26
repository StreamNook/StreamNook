import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('../nativeBridge', () => ({ setPipMuted: vi.fn() }));

import { setPipMuted } from '../nativeBridge';
import { liveVideoMuted, setLiveVideo, toggleLiveMute } from './liveVideo';

/** A stand-in <video>: an event target with a `muted` flag that announces
 *  changes the way the element does. */
function fakeVideo(startMuted = false) {
  const el = new EventTarget() as EventTarget & { muted: boolean };
  let value = startMuted;
  Object.defineProperty(el, 'muted', {
    get: () => value,
    set: (next: boolean) => {
      value = next;
      el.dispatchEvent(new Event('volumechange'));
    },
  });
  return el;
}

function attach(el: ReturnType<typeof fakeVideo>) {
  setLiveVideo(el as unknown as HTMLVideoElement);
}

describe('live video mute', () => {
  afterEach(() => {
    setLiveVideo(null);
    vi.mocked(setPipMuted).mockClear();
  });

  it('toggles the element and keeps the PiP action in step', () => {
    attach(fakeVideo());
    expect(toggleLiveMute()).toBe(true);
    expect(liveVideoMuted()).toBe(true);
    expect(setPipMuted).toHaveBeenLastCalledWith(true);
    expect(toggleLiveMute()).toBe(false);
    expect(liveVideoMuted()).toBe(false);
    expect(setPipMuted).toHaveBeenLastCalledWith(false);
  });

  it('follows a mute made by anything else (a clip ducking it)', () => {
    const el = fakeVideo();
    attach(el);
    el.muted = true;
    expect(liveVideoMuted()).toBe(true);
  });

  it('reads an element that arrives already muted', () => {
    attach(fakeVideo(true));
    expect(liveVideoMuted()).toBe(true);
  });

  it('does nothing without a live element', () => {
    expect(toggleLiveMute()).toBeNull();
    expect(setPipMuted).not.toHaveBeenCalled();
  });

  it('stops following an element once it is detached', () => {
    const el = fakeVideo();
    attach(el);
    setLiveVideo(null);
    el.muted = true;
    expect(liveVideoMuted()).toBe(false);
  });
});

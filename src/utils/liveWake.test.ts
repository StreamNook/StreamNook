import { describe, expect, it } from 'vitest';
import { liveWake } from './liveWake';

describe('liveWake', () => {
  it('resets the gate on a pause with a frame pending, and schedules nothing new', () => {
    expect(liveWake('pause', true)).toEqual({ resetGate: true, schedule: false });
  });

  it('resets the gate and schedules a frame on a pause when the loop is asleep', () => {
    expect(liveWake('pause', false)).toEqual({ resetGate: true, schedule: true });
  });

  it('treats play, seeking and seeked like a pause', () => {
    for (const ev of ['play', 'seeking', 'seeked']) {
      expect(liveWake(ev, true)).toEqual({ resetGate: true, schedule: false });
      expect(liveWake(ev, false)).toEqual({ resetGate: true, schedule: true });
    }
  });

  it('does nothing on timeupdate with a frame pending', () => {
    expect(liveWake('timeupdate', true)).toEqual({ resetGate: false, schedule: false });
  });

  it('wakes a sleeping loop on timeupdate', () => {
    expect(liveWake('timeupdate', false)).toEqual({ resetGate: true, schedule: true });
  });
});

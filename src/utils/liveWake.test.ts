import { describe, expect, it } from 'vitest';
import { liveTickWanted, liveWake } from './liveWake';

describe('liveWake', () => {
  it('recomputes at once on play, pause, seeking and seeked', () => {
    for (const ev of ['play', 'pause', 'seeking', 'seeked']) {
      expect(liveWake(ev)).toEqual({ recompute: true });
    }
  });

  it('recomputes nothing on timeupdate', () => {
    expect(liveWake('timeupdate')).toEqual({ recompute: false });
  });
});

describe('liveTickWanted', () => {
  const playing = { live: true, paused: false, seeking: false, hidden: false };

  it('runs while a live stream plays in a shown window', () => {
    expect(liveTickWanted(playing)).toBe(true);
  });

  it('runs while a paused stream seeks', () => {
    expect(liveTickWanted({ ...playing, paused: true, seeking: true })).toBe(true);
  });

  it('sleeps while paused, hidden, or not live', () => {
    expect(liveTickWanted({ ...playing, paused: true })).toBe(false);
    expect(liveTickWanted({ ...playing, hidden: true })).toBe(false);
    expect(liveTickWanted({ ...playing, live: false })).toBe(false);
  });
});

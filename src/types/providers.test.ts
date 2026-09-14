import { describe, it, expect } from 'vitest';
import {
  PROVIDER_IDS,
  PROVIDER_WATCH,
  WATCHABLE_PROVIDERS,
  GRID_BLOCKED,
  canGridProvider,
  gridRefusal,
} from './providers';

// The grid gate. YouTube is the case worth protecting: it plays back fine as
// the solo stream, so the obvious gate (PROVIDER_WATCH[p].playback) reads as
// declarative and admits exactly the one provider that must never be admitted.
describe('canGridProvider', () => {
  it('admits the platforms that can be a tile', () => {
    expect(canGridProvider('twitch')).toBe(true);
    expect(canGridProvider('kick')).toBe(true);
  });

  // YouTube was refused here until youtube_dash was keyed by stream id. The
  // assertion is flipped rather than deleted: it is the record of that gate
  // having been deliberately opened.
  it('admits YouTube now that the relay is per-stream', () => {
    expect(PROVIDER_WATCH.youtube.playback).toBe(true);
    expect(canGridProvider('youtube')).toBe(true);
  });

  it('refuses platforms with no playback at all', () => {
    for (const p of ['tiktok', 'rumble', 'x'] as const) {
      expect(canGridProvider(p)).toBe(false);
    }
  });
});

describe('gridRefusal', () => {
  it('returns null exactly when the provider is allowed', () => {
    for (const p of PROVIDER_IDS) {
      expect(gridRefusal(p) === null).toBe(canGridProvider(p));
    }
  });

  it('gives a non-empty reason for every refused provider', () => {
    for (const p of PROVIDER_IDS.filter((p) => !canGridProvider(p))) {
      expect(gridRefusal(p)).toBeTruthy();
    }
  });

  it('has no reason to give for a provider it allows', () => {
    expect(gridRefusal('youtube')).toBeNull();
    expect(GRID_BLOCKED.youtube).toBeUndefined();
  });
});

// A provider whose playback flips on later must not become grid-eligible by
// accident. This forces that to be a decision someone makes on purpose.
describe('the gate cannot widen silently', () => {
  it('every watchable provider is either grid-eligible or explicitly blocked', () => {
    for (const p of WATCHABLE_PROVIDERS) {
      expect(canGridProvider(p) || p in GRID_BLOCKED).toBe(true);
    }
  });
});

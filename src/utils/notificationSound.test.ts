import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (p: string) => p }));

// Counts tones started on the shared context: each play is one oscillator.
let tonesStarted = 0;

class FakeAudioContext {
  state = 'running';
  currentTime = 0;
  destination = {};
  resume() { return Promise.resolve(); }
  createOscillator() {
    const param = { setValueAtTime() {}, exponentialRampToValueAtTime() {}, linearRampToValueAtTime() {} };
    return { type: '', frequency: param, connect() {}, start() { tonesStarted += 1; }, stop() {} };
  }
  createGain() {
    const param = { setValueAtTime() {}, exponentialRampToValueAtTime() {}, linearRampToValueAtTime() {} };
    return { gain: param, connect() {} };
  }
}

describe('playNotificationSound', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000_000);
    tonesStarted = 0;
    vi.stubGlobal('window', { AudioContext: FakeAudioContext });
    vi.resetModules();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('plays one tone for a burst of go-live notifications', async () => {
    const { playNotificationSound } = await import('./notificationSound');
    for (let i = 0; i < 7; i++) playNotificationSound('boop');
    expect(tonesStarted).toBe(1);
  });

  it('plays again once the gap has passed', async () => {
    const { playNotificationSound } = await import('./notificationSound');
    playNotificationSound('boop');
    vi.advanceTimersByTime(1999);
    playNotificationSound('whisper');
    expect(tonesStarted).toBe(1);
    vi.advanceTimersByTime(1);
    playNotificationSound('whisper');
    expect(tonesStarted).toBe(2);
  });

  it('does not gate the settings preview', async () => {
    const { playNotificationSound, playSound } = await import('./notificationSound');
    playNotificationSound('boop');
    playSound('boop');
    playSound('tick');
    expect(tonesStarted).toBe(3);
  });

  it('plays a retro tone as its run of notes', async () => {
    const { playSound } = await import('./notificationSound');
    playSound('morse');
    // ... -- ... is eight marks.
    expect(tonesStarted).toBe(8);
  });
});

describe('built-in tones', () => {
  it('every built-in has a label, and Rust accepts exactly the same ids', async () => {
    const { BUILT_IN_SOUNDS, SOUND_LABELS } = await import('./notificationSound');
    expect(Object.keys(SOUND_LABELS).sort()).toEqual([...BUILT_IN_SOUNDS].sort());
    const rules = readFileSync(resolve(__dirname, '..', '..', 'src-tauri/src/services/chat_rules.rs'), 'utf8');
    const list = rules.match(/const VALID_SOUND_IDS: \[&str; \d+\] = \[([^\]]*)\]/);
    expect(list).not.toBeNull();
    const rustIds = [...list![1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
    expect(rustIds.sort()).toEqual([...BUILT_IN_SOUNDS].sort());
  });
});

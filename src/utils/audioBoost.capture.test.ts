// A song capture stops the moment it is canceled: an aborted signal returns
// at once, every await is followed by a check, the wait ends early, the tap
// is always disconnected, and the level goes out at most every 33 ms.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { abortableSleep, captureStreamSamples, LEVEL_MS } from './audioBoost';

class FakeNode {
  connected = 0;
  connect() {
    this.connected += 1;
  }
  disconnect() {
    this.connected = 0;
  }
}

class FakeWorkletNode extends FakeNode {
  static made: FakeWorkletNode[] = [];
  port: { onmessage: ((e: { data: Float32Array }) => void) | null } = { onmessage: null };
  constructor() {
    super();
    FakeWorkletNode.made.push(this);
  }
}

class FakeContext {
  static made: FakeContext[] = [];
  static resume: () => Promise<void> = async () => {};
  state = 'suspended';
  sampleRate = 16000;
  currentTime = 0;
  destination = new FakeNode();
  source = new FakeNode();
  audioWorklet = { addModule: vi.fn(async () => {}) };
  constructor() {
    FakeContext.made.push(this);
  }
  resume() {
    return FakeContext.resume();
  }
  createMediaElementSource() {
    return this.source;
  }
  createDynamicsCompressor() {
    return new FakeNode();
  }
  createGain() {
    return new FakeNode();
  }
  close() {
    return Promise.resolve();
  }
}

beforeEach(() => {
  FakeContext.made = [];
  FakeContext.resume = async () => {};
  FakeWorkletNode.made = [];
  vi.stubGlobal('window', { AudioContext: FakeContext });
  vi.stubGlobal('AudioWorkletNode', FakeWorkletNode);
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const element = () => ({}) as HTMLMediaElement;

describe('abortableSleep', () => {
  it('waits its time, ends early on an abort, and lets go of its listener', async () => {
    vi.useFakeTimers();
    const added: unknown[] = [];
    const removed: unknown[] = [];
    const signal = {
      aborted: false,
      addEventListener: (_t: string, f: unknown) => added.push(f),
      removeEventListener: (_t: string, f: unknown) => removed.push(f),
    } as unknown as AbortSignal;
    let done = false;
    void abortableSleep(1000, signal).then(() => (done = true));
    await vi.advanceTimersByTimeAsync(999);
    expect(done).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(done).toBe(true);
    expect(removed).toEqual(added);

    const ctl = new AbortController();
    let early = false;
    void abortableSleep(60_000, ctl.signal).then(() => (early = true));
    ctl.abort();
    await vi.advanceTimersByTimeAsync(0);
    expect(early).toBe(true);

    const gone = new AbortController();
    gone.abort();
    await expect(abortableSleep(60_000, gone.signal)).resolves.toBeUndefined();
  });
});

describe('captureStreamSamples', () => {
  it('returns at once for a signal already aborted, touching nothing', async () => {
    const ctl = new AbortController();
    ctl.abort();
    expect(await captureStreamSamples(element(), 5, { signal: ctl.signal })).toBeNull();
    expect(FakeContext.made).toHaveLength(0);
  });

  it('stops after the context resumes when canceled meanwhile, with no tap made', async () => {
    const ctl = new AbortController();
    let resumed!: () => void;
    FakeContext.resume = () => new Promise<void>((r) => (resumed = r));
    const run = captureStreamSamples(element(), 5, { signal: ctl.signal });
    await Promise.resolve();
    ctl.abort();
    resumed();
    expect(await run).toBeNull();
    expect(FakeWorkletNode.made).toHaveLength(0);
    expect(FakeContext.made[0].audioWorklet.addModule).not.toHaveBeenCalled();
  });

  it('ends the wait early on a cancel and always disconnects the tap', async () => {
    vi.useFakeTimers();
    const ctl = new AbortController();
    const onStart = vi.fn();
    const run = captureStreamSamples(element(), 5, { signal: ctl.signal, onStart });
    await vi.advanceTimersByTimeAsync(0);
    const node = FakeWorkletNode.made[0];
    expect(onStart).toHaveBeenCalledTimes(1);
    expect(node.connected).toBe(1);
    node.port.onmessage?.({ data: new Float32Array(16000).fill(0.5) });
    ctl.abort();
    await vi.advanceTimersByTimeAsync(0);
    expect(await run).toBeNull();
    expect(node.connected).toBe(0);
    expect(node.port.onmessage).toBeNull();
  });

  it('hands back what it heard, and the level at most every 33 ms', async () => {
    vi.useFakeTimers();
    const levels: number[] = [];
    const run = captureStreamSamples(element(), 1, { onLevel: (v) => levels.push(v) });
    await vi.advanceTimersByTimeAsync(0);
    const node = FakeWorkletNode.made[0];
    const chunk = (v: number) => node.port.onmessage?.({ data: new Float32Array(8000).fill(v) });
    chunk(0.5);
    await vi.advanceTimersByTimeAsync(10);
    chunk(0.1);
    await vi.advanceTimersByTimeAsync(LEVEL_MS);
    chunk(0.3);
    await vi.advanceTimersByTimeAsync(1000);
    const pcm = await run;
    expect(pcm).toHaveLength(24000);
    expect(levels).toHaveLength(2);
    expect(levels[0]).toBeCloseTo(0.5, 5);
    // The second covers both chunks since the first: RMS of 0.1 and 0.3.
    expect(levels[1]).toBeCloseTo(Math.sqrt((0.01 + 0.09) / 2), 5);
    expect(node.connected).toBe(0);
  });
});

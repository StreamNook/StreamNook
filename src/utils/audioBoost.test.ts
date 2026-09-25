// Run with: npm test
//
// Lifetime of the per-element audio graph. Once an element has played through
// its source node, only closing that node's AudioContext lets the element be
// collected, so each element owns a context and the player hands it back on
// unmount. These pin the ownership and the guard that keeps a still-mounted
// element (a StrictMode rehearsal unmount) from being silenced.

import { afterAll, beforeAll, test, vi } from 'vitest';
import assert from 'node:assert/strict';

class FakeNode {
  connected = 0;
  connect() {
    this.connected += 1;
  }
  disconnect() {
    this.connected = 0;
  }
}
class FakeParam {
  setValueAtTime() {}
}
class FakeContext {
  static created: FakeContext[] = [];
  state = 'running';
  currentTime = 0;
  destination = new FakeNode();
  closed = false;
  constructor() {
    FakeContext.created.push(this);
  }
  createMediaElementSource() {
    return new FakeNode();
  }
  createDynamicsCompressor() {
    return Object.assign(new FakeNode(), {
      threshold: new FakeParam(),
      knee: new FakeParam(),
      ratio: new FakeParam(),
      attack: new FakeParam(),
      release: new FakeParam(),
    });
  }
  createGain() {
    return Object.assign(new FakeNode(), { gain: new FakeParam() });
  }
  resume() {
    return Promise.resolve();
  }
  close() {
    this.closed = true;
    this.state = 'closed';
    return Promise.resolve();
  }
}

type AudioBoostModule = typeof import('./audioBoost');
let mod: AudioBoostModule;

beforeAll(async () => {
  vi.stubGlobal('window', { AudioContext: FakeContext });
  mod = await import('./audioBoost');
});
afterAll(() => {
  vi.unstubAllGlobals();
});

const element = (isConnected: boolean) => ({ isConnected }) as unknown as HTMLMediaElement;
const enabled = () => ({ ...mod.resolveAudioBoost(null), enabled: true });

test('each element gets its own context', () => {
  const before = FakeContext.created.length;
  mod.applyAudioBoost(element(true), enabled());
  mod.applyAudioBoost(element(true), enabled());
  assert.equal(FakeContext.created.length - before, 2);
});

test('re-applying to the same element reuses its context', () => {
  const el = element(true);
  const before = FakeContext.created.length;
  mod.applyAudioBoost(el, enabled());
  mod.applyAudioBoost(el, { ...enabled(), enabled: false });
  assert.equal(FakeContext.created.length - before, 1);
});

test('releasing an element closes its context, once', () => {
  const el = element(false);
  mod.applyAudioBoost(el, enabled());
  const ctx = FakeContext.created.at(-1)!;
  mod.releaseAudioGraph(el);
  assert.equal(ctx.closed, true);
  const before = FakeContext.created.length;
  mod.releaseAudioGraph(el);
  assert.equal(FakeContext.created.length, before);
});

test('an element that was never tapped is left alone', () => {
  const before = FakeContext.created.length;
  mod.applyAudioBoost(element(true), { ...enabled(), enabled: false });
  mod.releaseAudioGraph(element(false));
  assert.equal(FakeContext.created.length, before);
});

test('a still-mounted element is not released, a removed one is', async () => {
  const mounted = element(true);
  const removed = element(false);
  mod.applyAudioBoost(mounted, enabled());
  const mountedCtx = FakeContext.created.at(-1)!;
  mod.applyAudioBoost(removed, enabled());
  const removedCtx = FakeContext.created.at(-1)!;
  mod.releaseAudioGraphOnceGone(mounted);
  mod.releaseAudioGraphOnceGone(removed);
  await new Promise((r) => setTimeout(r, 5));
  assert.equal(mountedCtx.closed, false);
  assert.equal(removedCtx.closed, true);
});

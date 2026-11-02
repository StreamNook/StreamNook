// The live readout is repaired from a MutationObserver instead of a frame
// loop: a foreign write is undone, a rebuilt node is found again, an equal
// text is never rewritten, and the player no longer schedules frames for it.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { liveReadoutRepair, type ReadoutObserver } from './liveReadoutRepair';

interface FakeNode {
  isConnected: boolean;
  writes: number;
  textContent: string;
}

function fakeNode(text: string): FakeNode {
  let value = text;
  const node = { isConnected: true, writes: 0 } as FakeNode;
  Object.defineProperty(node, 'textContent', {
    get: () => value,
    set: (v: string) => {
      node.writes += 1;
      value = v;
    },
  });
  return node;
}

function harness(initial: FakeNode, text: () => string | null) {
  let fire: () => void = () => {};
  let disconnected = false;
  let current = initial;
  const stop = liveReadoutRepair({
    root: {} as Node,
    find: () => current as unknown as HTMLElement,
    text,
    observe: (onChange): ReadoutObserver => {
      fire = onChange;
      return { observe: () => {}, disconnect: () => (disconnected = true) };
    },
  });
  return {
    fire: () => fire(),
    stop,
    swap: (next: FakeNode) => (current = next),
    disconnected: () => disconnected,
  };
}

describe('liveReadoutRepair', () => {
  it('puts our text back after a foreign write', () => {
    const node = fakeNode('1:02:03');
    const h = harness(node, () => '1:02:03');
    node.textContent = '0:41';
    h.fire();
    expect(node.textContent).toBe('1:02:03');
  });

  it('never rewrites a text that is already right', () => {
    const node = fakeNode('1:02:03');
    const h = harness(node, () => '1:02:03');
    h.fire();
    h.fire();
    expect(node.writes).toBe(0);
  });

  it('finds the node again after the control bar is rebuilt', () => {
    const first = fakeNode('a');
    const h = harness(first, () => '5:00');
    h.fire();
    expect(first.textContent).toBe('5:00');
    first.isConnected = false;
    const second = fakeNode('0:00');
    h.swap(second);
    h.fire();
    expect(second.textContent).toBe('5:00');
  });

  it('leaves the node alone at the live edge', () => {
    const node = fakeNode('0:41');
    const h = harness(node, () => null);
    h.fire();
    expect(node.textContent).toBe('0:41');
    expect(node.writes).toBe(0);
  });

  it('disconnects on stop', () => {
    const h = harness(fakeNode('x'), () => 'x');
    h.stop();
    expect(h.disconnected()).toBe(true);
  });
});

describe('the player wires it, with no frame loop for the readout', () => {
  const root = resolve(__dirname, '..', '..');
  const src = readFileSync(resolve(root, 'src/components/VideoPlayer.tsx'), 'utf8').replace(/\r\n/g, '\n');

  it('never schedules a frame for the readout', () => {
    const start = src.indexOf('const updateLiveTimeDisplay = useCallback(');
    const end = src.indexOf('}, [stopLiveTick]);', start);
    expect(start).toBeGreaterThan(-1);
    expect(end).toBeGreaterThan(start);
    expect(src.slice(start, end)).not.toContain('requestAnimationFrame');
    expect(src).not.toMatch(/requestAnimationFrame\(\s*updateLiveTimeDisplay/);
  });

  it('repairs the readout from the observer and ticks on an interval', () => {
    expect(src).toContain('liveReadoutRepair({');
    expect(src).toContain('setInterval(updateLiveTimeDisplay, 250)');
    expect(src).toContain('stopLiveRepair?.();');
  });
});

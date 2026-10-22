import { describe, expect, it } from 'vitest';

import type { ModStep } from '../../services/moderation';
import { lineAction, lineButtons, lineHit, lineLayout, lineRequest, lineTargetAttrs, nextLength, scrolledTimeout, withLengths } from './modLine';

const STEPS: ModStep[] = [
  { action: 'delete', seconds: null, label: 'Delete' },
  { action: 'timeout', seconds: 600, label: '10m' },
  { action: 'ban', seconds: null, label: 'Ban' },
];

describe('line buttons', () => {
  it('keeps Rust order and labels, one key per action', () => {
    const buttons = lineButtons(STEPS);
    expect(buttons.map((b) => b.key)).toEqual(['d', 't600', 'b']);
    expect(buttons.map((b) => b.name)).toEqual(['Delete message', 'Time out 10m', 'Ban']);
    expect(buttons.every((b) => b.tint.length > 0)).toBe(true);
  });

  it('reads every key back to the action it stands for', () => {
    expect(lineAction('d', 'msg-1')).toEqual({ kind: 'delete', messageId: 'msg-1' });
    expect(lineAction('t600', 'msg-1')).toEqual({ kind: 'timeout', seconds: 600 });
    expect(lineAction('b', undefined)).toEqual({ kind: 'ban' });
  });

  it('sends nothing it cannot name exactly', () => {
    // Delete without a message, and anything malformed, is no action at all.
    expect(lineAction('d', undefined)).toBeNull();
    expect(lineAction('t', 'm')).toBeNull();
    expect(lineAction('t0', 'm')).toBeNull();
    expect(lineAction('t1.5', 'm')).toBeNull();
    expect(lineAction('x', 'm')).toBeNull();
  });

  it('lays the buttons out left to right with a gap, and hits only inside one', () => {
    // 14 px text: icon buttons 1.3em (18 px), the label at 30 px plus padding.
    const layout = lineLayout(lineButtons(STEPS), 14, () => 30);
    expect(layout.boxes).toEqual([
      { x: 0, w: 18 },
      { x: 20, w: 39 },
      { x: 61, w: 18 },
    ]);
    expect(layout.width).toBe(79);
    expect(layout.height).toBe(17);
    expect(lineHit(layout, 0)).toBe(0);
    expect(lineHit(layout, 17.9)).toBe(0);
    expect(lineHit(layout, 19)).toBe(-1);
    expect(lineHit(layout, 40)).toBe(1);
    expect(lineHit(layout, 78)).toBe(2);
    expect(lineHit(layout, 80)).toBe(-1);
  });

  it('turns a hit into the request for that chatter, from the strip', () => {
    const attrs = lineTargetAttrs(
      { provider: 'kick', broadcasterId: '42', channel: 'chan', targetUserId: '7', targetName: 'Spammer' },
      'msg-9',
    );
    expect(attrs).toMatchObject({ 'data-mod-line': '', 'data-p': 'kick', 'data-m': 'msg-9' });
    // The DOM's dataset drops the `data-` prefix.
    const d = { p: 'kick', b: '42', c: 'chan', u: '7', n: 'Spammer', m: 'msg-9' };
    const buttons = lineButtons(STEPS);
    expect(lineRequest(buttons, 1, d)).toEqual({
      request: {
        provider: 'kick',
        broadcasterId: '42',
        channel: 'chan',
        targetUserId: '7',
        targetName: 'Spammer',
        action: { kind: 'timeout', seconds: 600 },
        useSavedReason: true,
        onePress: true,
      },
      name: 'Time out 10m',
      label: '10m',
    });
    expect(lineRequest(buttons, 0, d)?.request.action).toEqual({ kind: 'delete', messageId: 'msg-9' });
    expect(lineRequest(buttons, 0, d)?.request.useSavedReason).toBe(false);
    expect(lineRequest(buttons, -1, d)).toBeNull();
  });
});

describe('scrolling a line timeout', () => {
  const LADDER: ModStep[] = [60, 300, 600, 1800, 3600, 86400, 604800].map((s) => ({
    action: 'timeout' as const,
    seconds: s,
    label: `${s}s`,
  }));
  const ladder = LADDER.map((s) => s.seconds as number);

  it('walks up and down the ladder and stops at either end', () => {
    expect(nextLength(600, ladder, 1)).toBe(1800);
    expect(nextLength(600, ladder, -1)).toBe(300);
    expect(nextLength(604800, ladder, 1)).toBeNull();
    expect(nextLength(60, ladder, -1)).toBeNull();
  });

  it('moves a length between two rungs to the nearer one that way', () => {
    expect(nextLength(900, ladder, 1)).toBe(1800);
    expect(nextLength(900, ladder, -1)).toBe(600);
  });

  it('scrolls the timeout under the pointer, else the first one', () => {
    expect(scrolledTimeout(STEPS, 1)).toBe(1);
    expect(scrolledTimeout(STEPS, 0)).toBe(1);
    expect(scrolledTimeout(STEPS, 2)).toBe(1);
    expect(scrolledTimeout([STEPS[0], STEPS[2]], 0)).toBe(-1);
  });

  it('swaps only overridden timeouts, with the ladder label', () => {
    const out = withLengths(STEPS, { 1: 3600 }, LADDER);
    expect(out.map((s) => s.seconds)).toEqual([null, 3600, null]);
    expect(out[1].label).toBe('3600s');
    expect(withLengths(STEPS, { 1: 42 }, LADDER)[1]).toBe(STEPS[1]);
    expect(withLengths(STEPS, { 0: 3600 }, LADDER)[0]).toBe(STEPS[0]);
  });

  it('keeps the strip one width whatever length it shows', () => {
    const measure = (label: string) => label.length * 7;
    const fit = ['10m', '30m', '1h', '1w'];
    const a = lineLayout(lineButtons(STEPS), 14, measure, fit);
    const b = lineLayout(lineButtons(withLengths(STEPS, { 1: 3600 }, [{ action: 'timeout', seconds: 3600, label: '1h' }])), 14, measure, fit);
    expect(a.width).toBe(b.width);
  });
});

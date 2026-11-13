// Run with: npm test
//
// The emote button's face follows Rust's emote of the moment for its own
// chat only, whatever order the ask's answer and the events arrive in.

import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

import { createMomentHolder, type MomentAnswer } from './useEmoteMoment';

const ans = (key: string, seq: number, name: string | null): MomentAnswer => ({
  key,
  seq,
  emote: name ? { name, url: `https://cdn/${name}.webp` } : null,
});

describe('createMomentHolder', () => {
  it('reads the shape Rust sends (the shared fixture)', () => {
    const fixture = JSON.parse(
      readFileSync(resolve(__dirname, '../../src-tauri/src/services/fixtures/emote_moment_answer.json'), 'utf8'),
    ) as MomentAnswer;
    const h = createMomentHolder();
    expect(h.onAnswer(fixture)).toBe(true);
    expect(h.current()).toEqual({
      name: 'catJAM',
      url: 'https://cdn.7tv.app/emote/60ae7316f7c927fad14e6ca2/2x.avif',
      id: '60ae7316f7c927fad14e6ca2',
    });
  });

  it('keeps an event heard before the answer when it is newer', () => {
    const h = createMomentHolder();
    expect(h.onEvent(ans('forsen', 5, 'LULW'))).toBe(false);
    expect(h.onAnswer(ans('forsen', 4, 'KEKW'))).toBe(true);
    expect(h.current()?.name).toBe('LULW');
  });

  it('drops an early event older than the answer', () => {
    const h = createMomentHolder();
    h.onEvent(ans('forsen', 3, 'LULW'));
    h.onAnswer(ans('forsen', 4, 'KEKW'));
    expect(h.current()?.name).toBe('KEKW');
  });

  it('ignores other chats and anything older than what it holds', () => {
    const h = createMomentHolder();
    h.onAnswer(ans('forsen', 10, 'Pog'));
    expect(h.onEvent(ans('kick:xqc', 11, 'catJAM'))).toBe(false);
    expect(h.onEvent(ans('forsen', 9, 'OLD'))).toBe(false);
    expect(h.current()?.name).toBe('Pog');
    expect(h.onEvent(ans('forsen', 12, null))).toBe(true);
    expect(h.current()).toBeNull();
  });

  it('says nothing changed when the same emote comes again', () => {
    const h = createMomentHolder();
    h.onAnswer(ans('forsen', 1, 'Pog'));
    expect(h.onEvent(ans('forsen', 2, 'Pog'))).toBe(false);
  });

  it('the hook listens before it asks, and drops answers after its chat changed', () => {
    const s = readFileSync(resolve(__dirname, 'useEmoteMoment.ts'), 'utf8');
    const body = s.slice(s.indexOf('export function useEmoteMoment('));
    expect(body.indexOf('listen<MomentAnswer>(')).toBeGreaterThan(0);
    expect(body.indexOf("invoke<MomentAnswer>('get_emote_moment'")).toBeGreaterThan(body.indexOf('listen<MomentAnswer>('));
    expect(body).toContain('if (!alive) {');
    expect(body).toContain('return shown.asked === asked ? shown.emote : null;');
  });
});

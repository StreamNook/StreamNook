// The shared case table (src/mobile/verticalViewCases.json) the iPhone's SNKit
// VerticalViewTests read too (SN-40): one row, one answer, on both phones.
import { describe, expect, it } from 'vitest';
import cases from '../verticalViewCases.json';
import {
  availableAfter,
  bandHeight,
  entryState,
  holdsLate,
  latch,
  lowerHeight,
  MissedCount,
  miniSize,
  missedSpoken,
  missedWords,
  nextChat,
  pill,
  queueSkips,
  release,
  restartOnPosture,
  tallLayout,
  tallSwipeCanStart,
  wantedView,
  type LayoutInputs,
  type Posture,
  type StreamView,
  type TallDrag,
  type TallRegion,
  type VerticalChatInput,
  type VerticalChatState,
} from './verticalView';

const INPUTS: VerticalChatInput[] = [
  'swipeRight',
  'swipeLeft',
  'dragUp',
  'dragDown',
  'tapChatArea',
  'tapChatButton',
  'tapPicture',
  'back',
  'rotate',
  'mini',
  'pip',
  'keyboardUp',
];

describe('vertical view, the shared case table', () => {
  it.each(cases.tallLayout)('tall layout: $note', (c) => {
    expect(tallLayout(c.in as LayoutInputs)).toBe(c.want);
  });

  it.each(cases.wantedView)('wanted view %#', (c) => {
    const got = wantedView({
      posture: c.in.posture as Posture,
      tv: c.in.tv,
      airplay: c.in.airplay,
      sessionPick: c.in.sessionPick as StreamView | null,
    });
    expect(got).toBe(c.want);
  });

  it.each(cases.restartOnPosture)('restart on a turn: $note', (c) => {
    const got = restartOnPosture({
      verticalAvailable: c.in.verticalAvailable,
      served: c.in.served as StreamView | null,
      wantedNow: c.in.wantedNow as StreamView | null,
    });
    expect(got).toBe(c.want);
  });

  it.each(cases.availableEvent)('a late answer: $note', (c) => {
    const event = { ticket: c.in.ticket, available: c.in.available };
    expect(availableAfter(event, c.in.current, c.in.currentTicket)).toBe(c.want);
    expect(holdsLate(event, c.in.currentTicket)).toBe(c.holds);
  });

  it.each(cases.verticalChat)('chat $state + $input -> $next', (c) => {
    expect(nextChat(c.state as VerticalChatState, c.input as VerticalChatInput)).toBe(c.next);
  });

  it('meets every state with every input', () => {
    for (const state of ['lower', 'cleared', 'expanded']) {
      for (const input of INPUTS) {
        expect(cases.verticalChat.some((c) => c.state === state && c.input === input), `${state} ${input}`).toBe(true);
      }
    }
  });

  it.each(cases.entryState)('entry with open $open is $want', (c) => {
    expect(entryState(c.open)).toBe(c.want);
  });

  it.each(cases.tallDrag)('tall drag: $note', (c) => {
    const drag = latch(c.region as TallRegion, c.state as VerticalChatState, c.dx, c.dy);
    expect(drag).toBe(c.latch as TallDrag);
    // The table's speeds are points per second (the iPhone's); Android measures
    // CSS px per ms.
    expect(release(drag, c.state as VerticalChatState, c.dx, c.dy, c.vx / 1000, c.vy / 1000)).toBe(c.next);
  });

  it.each(cases.missedWords)('missed $count', (c) => {
    expect(missedWords(c.count)).toBe(c.words);
    expect(missedSpoken(c.count)).toBe(c.spoken);
  });

  it('keeps one missed anchor per room', () => {
    const count = new MissedCount();
    cases.missedCount.forEach((c, i) => {
      if (c.op === 'clear') {
        count.clear(c.key, c.value);
        return;
      }
      count.see(c.key, c.value);
      expect(count.missed(c.key, c.value), `step ${i}`).toBe(c.missed);
    });
  });

  it.each(cases.pill)('pill while $served plays', (c) => {
    expect(pill(c.served as StreamView)).toEqual({ text: c.text, spoken: c.spoken });
  });

  it.each(cases.queueSkip)('queue skip: $note', (c) => {
    expect(
      queueSkips({
        ranSinceQueued: c.in.ranSinceQueued,
        served: c.in.served as StreamView | null,
        wanted: c.in.wanted as StreamView | null,
        healthy: c.in.healthy,
      }),
    ).toBe(c.want);
  });
});

describe('vertical view geometry', () => {
  it('fits the message box under the picture', () => {
    expect(bandHeight(393, 852, 59, 10)).toBeCloseTo((393 * 16) / 9, 6);
    expect(bandHeight(375, 667, 20, 0)).toBe(667 - 20 - 56);
    expect(lowerHeight(698.67)).toBe(252);
    expect(miniSize(true)).toEqual({ width: 112.5, height: 200 });
    expect(miniSize(false)).toEqual({ width: 200, height: 112.5 });
  });

  it('keeps a swipe off the Back strips at the side edges', () => {
    expect(tallSwipeCanStart(20, 400, 0, 0)).toBe(false);
    expect(tallSwipeCanStart(60, 400, 0, 0)).toBe(true);
    expect(tallSwipeCanStart(60, 400, 72, 0)).toBe(false);
    expect(tallSwipeCanStart(380, 400, 0, 0)).toBe(false);
    expect(tallSwipeCanStart(200, 400, 30, 30)).toBe(true);
  });
});

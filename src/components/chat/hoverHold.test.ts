// The hover hold's rules: what counts as a move and as a quick one, what
// holds chat and what lets it go, when its pill may show and where it steps
// aside, and the count it shows.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  AIM_MOVE_PX,
  AIM_QUICK_MS,
  AIM_QUICK_PX,
  createHoverHold,
  createMoveTrail,
  HOLD_GRACE_MS,
  HOLD_PILL_DELAY_MS,
  HOLD_PILL_MIN_MS,
  HOLD_PILL_RECHECK_MS,
  HOLD_REST_MS,
  isNear,
  movedEnough,
  rowsSinceHold,
  whenPillDue,
} from './hoverHold';

function setup() {
  const changes: boolean[] = [];
  const hold = createHoverHold((held) => changes.push(held));
  return { hold, changes };
}

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

describe('hover hold', () => {
  it('holds on a move and lets go once the pointer rests', () => {
    const { hold, changes } = setup();
    hold.move();
    expect(changes).toEqual([true]);
    vi.advanceTimersByTime(HOLD_REST_MS - 1);
    expect(changes).toEqual([true]);
    vi.advanceTimersByTime(1);
    expect(changes).toEqual([true, false]);
  });

  it('every move starts the rest over', () => {
    const { hold, changes } = setup();
    hold.move();
    vi.advanceTimersByTime(HOLD_REST_MS - 10);
    hold.move();
    vi.advanceTimersByTime(HOLD_REST_MS - 10);
    expect(changes).toEqual([true]);
    vi.advanceTimersByTime(10);
    expect(changes).toEqual([true, false]);
  });

  it('leaving the zones lets go after the grace, and a move back in keeps it', () => {
    const { hold, changes } = setup();
    hold.move();
    hold.leave();
    vi.advanceTimersByTime(HOLD_GRACE_MS - 1);
    hold.move();
    vi.advanceTimersByTime(HOLD_GRACE_MS * 4);
    expect(changes).toEqual([true]);
    hold.leave();
    vi.advanceTimersByTime(HOLD_GRACE_MS);
    expect(changes).toEqual([true, false]);
  });

  it('leaving while not held does nothing', () => {
    const { hold, changes } = setup();
    hold.leave();
    vi.advanceTimersByTime(HOLD_REST_MS * 2);
    expect(changes).toEqual([]);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('a wheel scroll lets go at once and the next move holds again', () => {
    const { hold, changes } = setup();
    hold.move();
    hold.drop();
    expect(changes).toEqual([true, false]);
    expect(vi.getTimerCount()).toBe(0);
    hold.move();
    expect(changes).toEqual([true, false, true]);
  });

  it('reports only changes', () => {
    const { hold, changes } = setup();
    hold.move();
    hold.move();
    hold.pressDown();
    hold.move();
    expect(changes).toEqual([true]);
  });

  it('tells how long it has left, and a leave in progress shortens that to the grace', () => {
    const { hold } = setup();
    expect(hold.restLeft()).toBe(0);
    hold.move();
    expect(hold.restLeft()).toBe(HOLD_REST_MS);
    vi.advanceTimersByTime(1000);
    expect(hold.restLeft()).toBe(HOLD_REST_MS - 1000);
    hold.leave();
    expect(hold.restLeft()).toBe(HOLD_GRACE_MS);
    vi.advanceTimersByTime(HOLD_GRACE_MS);
    expect(hold.restLeft()).toBe(0);
  });

  it('counts how long it has lasted from its start, not from the last move', () => {
    const { hold } = setup();
    expect(hold.heldFor()).toBe(0);
    hold.move();
    vi.advanceTimersByTime(1000);
    hold.move();
    expect(hold.heldFor()).toBe(1000);
    hold.drop();
    expect(hold.heldFor()).toBe(0);
  });

  it('a press holds until the button comes up, then a full rest', () => {
    const { hold, changes } = setup();
    hold.move();
    vi.advanceTimersByTime(HOLD_REST_MS - 50);
    hold.pressDown();
    vi.advanceTimersByTime(60_000);
    expect(changes).toEqual([true]);
    hold.pressUp();
    vi.advanceTimersByTime(HOLD_REST_MS - 1);
    expect(changes).toEqual([true]);
    vi.advanceTimersByTime(1);
    expect(changes).toEqual([true, false]);
  });

  it('a press that left the zones lets go a grace after the button comes up', () => {
    const { hold, changes } = setup();
    hold.pressDown();
    hold.leave();
    vi.advanceTimersByTime(HOLD_GRACE_MS * 4);
    expect(changes).toEqual([true]);
    hold.pressUp();
    vi.advanceTimersByTime(HOLD_GRACE_MS);
    expect(changes).toEqual([true, false]);
  });

  it('dispose lets go and leaves no timer behind', () => {
    const { hold, changes } = setup();
    hold.move();
    hold.leave();
    hold.dispose();
    expect(changes).toEqual([true, false]);
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe('counted moves', () => {
  it('the first move always counts', () => {
    expect(movedEnough(null, 10, 10)).toBe(true);
  });

  it('a pixel or two of tremor does not count; AIM_MOVE_PX does', () => {
    const last = { x: 100, y: 100 };
    expect(movedEnough(last, 100, 100)).toBe(false);
    expect(movedEnough(last, 101, 100)).toBe(false);
    expect(movedEnough(last, 102, 101)).toBe(false);
    expect(movedEnough(last, 100 + AIM_MOVE_PX, 100)).toBe(true);
  });

  it('measures x plus y, so a small diagonal adds up', () => {
    expect(movedEnough({ x: 0, y: 0 }, 2, 2)).toBe(true);
  });
});

describe('quick moves', () => {
  it('a reach of AIM_QUICK_PX within the window is quick', () => {
    const trail = createMoveTrail();
    trail.add(0, 0);
    vi.advanceTimersByTime(AIM_QUICK_MS - 50);
    trail.add(AIM_QUICK_PX, 0);
    expect(trail.quick()).toBe(true);
  });

  it('one jump of AIM_QUICK_PX from rest is quick', () => {
    const trail = createMoveTrail();
    trail.add(0, 0);
    vi.advanceTimersByTime(5000);
    trail.add(AIM_QUICK_PX, 0);
    expect(trail.quick()).toBe(true);
  });

  it('the same distance drifted a little at a time is not', () => {
    const trail = createMoveTrail();
    for (let x = 0; x <= AIM_QUICK_PX; x += 3) {
      trail.add(x, 0);
      expect(trail.quick()).toBe(false);
      vi.advanceTimersByTime(AIM_QUICK_MS + 50);
    }
  });

  it('tremor never is', () => {
    const trail = createMoveTrail();
    trail.add(100, 100);
    trail.add(101, 100);
    trail.add(102, 101);
    trail.add(101, 102);
    expect(trail.quick()).toBe(false);
  });

  it('clearing forgets the way there, but not where the pointer is', () => {
    const trail = createMoveTrail();
    trail.add(0, 0);
    trail.add(20, 0);
    trail.clear();
    expect(trail.quick()).toBe(false);
    trail.add(21, 0);
    expect(trail.quick()).toBe(false);
  });

  it('one reach from where it was cleared is quick, however long the pointer rested', () => {
    const trail = createMoveTrail();
    trail.add(0, 0);
    vi.advanceTimersByTime(5000);
    trail.clear();
    trail.add(AIM_QUICK_PX, 0);
    expect(trail.quick()).toBe(true);
  });
});

describe('when the pill is due', () => {
  it('shows once the hold has lasted the delay with time left, once', () => {
    const { hold } = setup();
    const show = vi.fn();
    hold.move();
    whenPillDue(hold.heldFor, hold.restLeft, show);
    vi.advanceTimersByTime(HOLD_PILL_DELAY_MS - 1);
    expect(show).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(show).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(HOLD_PILL_RECHECK_MS * 4);
    expect(show).toHaveBeenCalledTimes(1);
  });

  it('a hold that began before the pill mounted shows at once', () => {
    const { hold } = setup();
    const show = vi.fn();
    hold.move();
    vi.advanceTimersByTime(2000);
    whenPillDue(hold.heldFor, hold.restLeft, show);
    vi.advanceTimersByTime(0);
    expect(show).toHaveBeenCalledTimes(1);
  });

  it('waits while the hold is about to let go, and shows once a move gave it time', () => {
    const { hold } = setup();
    const show = vi.fn();
    hold.move();
    vi.advanceTimersByTime(HOLD_REST_MS - HOLD_PILL_MIN_MS + 100);
    whenPillDue(hold.heldFor, hold.restLeft, show);
    vi.advanceTimersByTime(0);
    expect(show).not.toHaveBeenCalled();
    hold.move();
    vi.advanceTimersByTime(HOLD_PILL_RECHECK_MS);
    expect(show).toHaveBeenCalledTimes(1);
  });

  it('a pointer on its way out never shows it', () => {
    const { hold } = setup();
    const show = vi.fn();
    hold.move();
    whenPillDue(hold.heldFor, hold.restLeft, show);
    vi.advanceTimersByTime(HOLD_PILL_DELAY_MS - 100);
    hold.leave();
    vi.advanceTimersByTime(HOLD_GRACE_MS + HOLD_PILL_RECHECK_MS * 4);
    expect(show).not.toHaveBeenCalled();
  });

  it('cancelling leaves no timer', () => {
    const show = vi.fn();
    const cancel = whenPillDue(() => 0, () => 0, show);
    vi.advanceTimersByTime(HOLD_PILL_DELAY_MS + HOLD_PILL_RECHECK_MS * 3);
    cancel();
    expect(vi.getTimerCount()).toBe(0);
    expect(show).not.toHaveBeenCalled();
  });
});

describe('near the pill', () => {
  const box = { left: 100, top: 400, right: 300, bottom: 436 };

  it('is near inside the box grown by the margin, not beyond it', () => {
    expect(isNear(box, { x: 200, y: 418 }, 32)).toBe(true);
    expect(isNear(box, { x: 68, y: 368 }, 32)).toBe(true);
    expect(isNear(box, { x: 67, y: 418 }, 32)).toBe(false);
    expect(isNear(box, { x: 200, y: 469 }, 32)).toBe(false);
  });

  it('no point is never near', () => {
    expect(isNear(box, null, 32)).toBe(false);
  });
});

describe('rows since the hold', () => {
  const idOf = (row: { id: string | null }) => row.id;
  const a = { id: 'a' };
  const b = { id: 'b' };
  const c = { id: 'c' };
  const d = { id: 'd' };

  it('counts the rows after the held tail', () => {
    expect(rowsSinceHold([a, b], [a, b, c, d], idOf)).toBe(2);
    expect(rowsSinceHold([a, b], [a, b], idOf)).toBe(0);
  });

  it('finds a restamped tail by its id', () => {
    expect(rowsSinceHold([a, b], [a, { id: 'b' }, c], idOf)).toBe(1);
  });

  it('finds a tail with no id by identity', () => {
    const tail = { id: null };
    expect(rowsSinceHold([a, tail], [a, { id: null }, tail, c], idOf)).toBe(1);
  });

  it('a tail trimmed out of the buffer reads as more than the buffer', () => {
    expect(rowsSinceHold([a], [c, d], idOf)).toBe(3);
  });

  it('an empty held view counts everything live', () => {
    expect(rowsSinceHold([], [a, b], idOf)).toBe(2);
  });
});

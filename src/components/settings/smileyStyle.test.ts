import { describe, expect, it, vi } from 'vitest';
import {
  INITIAL_RING,
  SMILEY_OPEN_FAILED,
  SMILEY_SWITCH_FAILED,
  handleTile,
  orderedSets,
  ringReducer,
  tileAction,
  tileLabel,
  type RingAction,
  type RingState,
  type SmileySetChoice,
  type SmileyStyle,
} from './smileyStyle';

const face = (id: string, text: string) => ({ id, text, url: `https://cdn/${id}.png` });

function choice(key: string, name: string, available: boolean): SmileySetChoice {
  return {
    key,
    name,
    available,
    faces: [face(`${key}-1`, ':)'), face(`${key}-2`, ';)'), face(`${key}-3`, '<3'), face(`${key}-4`, 'O_o')],
    preview: [face(`${key}-1`, ':)'), face(`${key}-2`, ';)'), face(`${key}-3`, '<3')],
  };
}

function view(over: Partial<SmileyStyle> = {}): SmileyStyle {
  return {
    can_switch: true,
    current: 'robots',
    upgrade_url: 'https://www.twitch.tv/settings/prime',
    sets: [choice('robots', 'Robots', true), choice('purple', 'Purple', true), choice('monkeys', 'Monkeys', true)],
    ...over,
  };
}

const noPrime = () =>
  view({
    can_switch: false,
    sets: [choice('robots', 'Robots', true), choice('purple', 'Purple', false), choice('monkeys', 'Monkeys', false)],
  });

function run(state: RingState, ...actions: RingAction[]): RingState {
  return actions.reduce(ringReducer, state);
}

describe('tileAction', () => {
  it('switches to another set the account can pick', () => {
    expect(tileAction(view(), 'monkeys', null)).toBe('switch');
  });

  it('does nothing on the current tile', () => {
    expect(tileAction(view(), 'robots', null)).toBe('none');
  });

  it('opens the Prime page from a set the account cannot pick', () => {
    expect(tileAction(noPrime(), 'purple', null)).toBe('open');
    expect(tileAction(noPrime(), 'monkeys', null)).toBe('open');
  });

  it('does nothing while a change is going out, even on a Prime tile', () => {
    expect(tileAction(view(), 'purple', 'monkeys')).toBe('none');
    expect(tileAction(noPrime(), 'purple', 'monkeys')).toBe('none');
  });

  it('does nothing for a set that is not in the view', () => {
    expect(tileAction(view(), 'set:99', null)).toBe('none');
  });

  it('does not switch when the account cannot switch', () => {
    expect(tileAction(view({ can_switch: false }), 'monkeys', null)).toBe('none');
  });
});

describe('tile order and names', () => {
  it('puts the sets that need Prime or Turbo last, keeping each group in order', () => {
    const v = view({
      sets: [choice('robots', 'Robots', true), choice('purple', 'Purple', false), choice('monkeys', 'Monkeys', true)],
    });
    expect(orderedSets(v).map((s) => s.key)).toEqual(['robots', 'monkeys', 'purple']);
  });

  it('names a tile, and says where a locked one leads', () => {
    expect(tileLabel(choice('robots', 'Robots', true))).toBe('Robots');
    expect(tileLabel(choice('purple', 'Purple', false))).toBe('Purple, opens Twitch Prime');
  });

  it('gives a set without a name a plain one', () => {
    expect(tileLabel(choice('set:77', '', true))).toBe('Smiley set');
    expect(tileLabel(choice('set:77', '', false))).toBe('Smiley set, opens Twitch Prime');
  });
});

describe('ringReducer', () => {
  it('draws the row only when the first answer has a view', () => {
    const drawn = run(INITIAL_RING, { type: 'load', view: view() });
    expect(drawn.view?.current).toBe('robots');

    const undrawn = run(INITIAL_RING, { type: 'load', view: null });
    expect(undrawn.view).toBeNull();
    expect(undrawn.loaded).toBe(true);
    // Nothing later draws a row that was not there at mount.
    expect(run(undrawn, { type: 'event', view: view() }).view).toBeNull();
    expect(run(undrawn, { type: 'load', view: view() }).view).toBeNull();
  });

  it('takes the first answer only', () => {
    const drawn = run(INITIAL_RING, { type: 'load', view: view() });
    expect(run(drawn, { type: 'load', view: view({ current: 'purple' }) })).toBe(drawn);
  });

  it('ignores an event before the first answer', () => {
    expect(run(INITIAL_RING, { type: 'event', view: view() }).view).toBeNull();
  });

  it('moves the ring on an event, and never removes a drawn row', () => {
    const drawn = run(INITIAL_RING, { type: 'load', view: view() });
    const moved = run(drawn, { type: 'event', view: view({ current: 'purple' }) });
    expect(moved.view?.current).toBe('purple');
    expect(run(moved, { type: 'event', view: null }).view?.current).toBe('purple');
  });

  it('moves the ring at once on a tap', () => {
    const tapped = run(INITIAL_RING, { type: 'load', view: view() }, { type: 'tap', key: 'monkeys' });
    expect(tapped.view?.current).toBe('monkeys');
    expect(tapped.pending).toBe('monkeys');
  });

  it('ignores a second tap while one is going out', () => {
    const tapped = run(INITIAL_RING, { type: 'load', view: view() }, { type: 'tap', key: 'monkeys' });
    expect(run(tapped, { type: 'tap', key: 'purple' })).toBe(tapped);
  });

  it('releases on the view the command returned', () => {
    const settled = run(
      INITIAL_RING,
      { type: 'load', view: view() },
      { type: 'tap', key: 'monkeys' },
      { type: 'settled', view: view({ current: 'monkeys' }) },
    );
    expect(settled.view?.current).toBe('monkeys');
    expect(settled.pending).toBeNull();
    expect(settled.before).toBeNull();
  });

  it('puts the ring back to the view from before the tap when the change fails', () => {
    const before = view();
    const failed = run(INITIAL_RING, { type: 'load', view: before }, { type: 'tap', key: 'monkeys' }, { type: 'failed' });
    expect(failed.view).toBe(before);
    expect(failed.pending).toBeNull();
  });

  it('keeps an event that arrives while a change is going out, and leaves the ring alone meanwhile', () => {
    const tapped = run(INITIAL_RING, { type: 'load', view: view() }, { type: 'tap', key: 'monkeys' });
    const during = run(tapped, { type: 'event', view: view({ current: 'purple' }) });
    expect(during.view?.current).toBe('monkeys');
    expect(during.queued?.current).toBe('purple');
  });

  it('takes a queued event that agrees with the committed set when the change settles', () => {
    const agreeing = view({ current: 'monkeys', can_switch: false });
    const settled = run(
      INITIAL_RING,
      { type: 'load', view: view() },
      { type: 'tap', key: 'monkeys' },
      { type: 'event', view: agreeing },
      { type: 'settled', view: view({ current: 'monkeys' }) },
    );
    expect(settled.view).toBe(agreeing);
    expect(settled.queued).toBeNull();
  });

  it('does not let a queued event that names another set undo a committed change', () => {
    const returned = view({ current: 'monkeys' });
    const settled = run(
      INITIAL_RING,
      { type: 'load', view: view() },
      { type: 'tap', key: 'monkeys' },
      { type: 'event', view: view({ current: 'robots' }) },
      { type: 'settled', view: returned },
    );
    expect(settled.view).toBe(returned);
  });

  it('applies a queued event as is when the change fails', () => {
    const queued = view({ current: 'purple' });
    const failed = run(
      INITIAL_RING,
      { type: 'load', view: view() },
      { type: 'tap', key: 'monkeys' },
      { type: 'event', view: queued },
      { type: 'failed' },
    );
    expect(failed.view).toBe(queued);
  });

  it('ignores a settle or a failure when nothing is going out', () => {
    const drawn = run(INITIAL_RING, { type: 'load', view: view() });
    expect(run(drawn, { type: 'settled', view: view({ current: 'purple' }) })).toBe(drawn);
    expect(run(drawn, { type: 'failed' })).toBe(drawn);
  });
});

describe('handleTile', () => {
  function deps(over: Partial<Omit<Parameters<typeof handleTile>[3], 'dispatch'>> = {}) {
    return {
      dispatch: vi.fn<(action: RingAction) => void>(),
      setStyle: vi.fn(async (key: string) => view({ current: key })),
      open: vi.fn(async () => true),
      toast: vi.fn(),
      ...over,
    };
  }

  it('switches: the ring moves, then the committed view releases it', async () => {
    const d = deps();
    await expect(handleTile(view(), 'monkeys', null, d)).resolves.toBe('switch');
    expect(d.setStyle).toHaveBeenCalledWith('monkeys');
    expect(d.dispatch.mock.calls.map((c) => c[0].type)).toEqual(['tap', 'settled']);
    const settled = d.dispatch.mock.calls[1][0];
    expect(settled.type === 'settled' && settled.view.current).toBe('monkeys');
    expect(d.toast).not.toHaveBeenCalled();
  });

  it('puts the ring back and shows the error text when the change is refused', async () => {
    const d = deps({
      setStyle: vi.fn(async () => {
        throw 'Twitch did not change your smiley style. Try again in a moment.';
      }),
    });
    await handleTile(view(), 'monkeys', null, d);
    expect(d.dispatch.mock.calls.map((c) => c[0].type)).toEqual(['tap', 'failed']);
    expect(d.toast).toHaveBeenCalledTimes(1);
    expect(d.toast).toHaveBeenCalledWith('Twitch did not change your smiley style. Try again in a moment.');
  });

  it('shows the standard text when the failure is not a string', async () => {
    const d = deps({
      setStyle: vi.fn(async () => {
        throw new Error('boom');
      }),
    });
    await handleTile(view(), 'monkeys', null, d);
    expect(d.toast).toHaveBeenCalledWith(SMILEY_SWITCH_FAILED);
  });

  it('opens the Prime page from a locked tile and changes nothing', async () => {
    const d = deps();
    await expect(handleTile(noPrime(), 'purple', null, d)).resolves.toBe('open');
    expect(d.open).toHaveBeenCalledWith('https://www.twitch.tv/settings/prime');
    expect(d.dispatch).not.toHaveBeenCalled();
    expect(d.setStyle).not.toHaveBeenCalled();
    expect(d.toast).not.toHaveBeenCalled();
  });

  it('says so when the browser did not open', async () => {
    const d = deps({ open: vi.fn(async () => false) });
    await handleTile(noPrime(), 'monkeys', null, d);
    expect(d.toast).toHaveBeenCalledWith(SMILEY_OPEN_FAILED);
    expect(SMILEY_OPEN_FAILED).toBe('Could not open your browser.');
  });

  it('does nothing on the current tile or while a change is going out', async () => {
    const d = deps();
    await handleTile(view(), 'robots', null, d);
    await handleTile(view(), 'monkeys', 'purple', d);
    expect(d.dispatch).not.toHaveBeenCalled();
    expect(d.setStyle).not.toHaveBeenCalled();
    expect(d.open).not.toHaveBeenCalled();
  });
});

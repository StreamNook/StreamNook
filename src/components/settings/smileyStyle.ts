// The Smiley style row's pure parts: the shapes Rust sends, what a tap on a tile
// does, and how the ring moves while a change is going out. The hook and the
// picker only wire these to React.

export interface SmileyFace {
  id: string;
  /** What you type, e.g. `:)`. */
  text: string;
  url: string;
}

export interface SmileySetChoice {
  /** `robots`, `purple`, `monkeys`, or `set:<id>` for a set this app does not name. */
  key: string;
  /** Empty for a set this app does not name. */
  name: string;
  faces: SmileyFace[];
  /** The three faces a tile shows: `:)` `;)` `<3`. */
  preview: SmileyFace[];
  /** False when the account cannot pick this set (it needs Prime or Turbo). */
  available: boolean;
}

export interface SmileyStyle {
  can_switch: boolean;
  /** The key of the set the account uses. */
  current: string;
  sets: SmileySetChoice[];
  upgrade_url: string;
}

/** A face row as Rust sends it; only the id is read here. */
export type SmileyRow = { id: string } & Record<string, unknown>;

export interface SmileySwap {
  remove_ids: string[];
  rows: SmileyRow[];
}

/** What `twitch://smilies-changed` and `get_own_smilies` carry. */
export interface OwnSmilies {
  rev: number;
  swap: SmileySwap | null;
  view: SmileyStyle | null;
  shown: boolean;
}

/** What `get_smiley_style` answers. */
export interface SmileyStyleRead {
  view: SmileyStyle | null;
  rev: number;
}

export const SMILEY_OPEN_FAILED = 'Could not open your browser.';
export const SMILEY_SWITCH_FAILED = 'Twitch did not change your smiley style. Try again in a moment.';

/** Available sets first, the ones that need Prime or Turbo after, each group in Rust's order. */
export function orderedSets(view: SmileyStyle): SmileySetChoice[] {
  return [...view.sets.filter((s) => s.available), ...view.sets.filter((s) => !s.available)];
}

/** The set's name, or a plain stand-in for one this app does not name. */
export function setTitle(set: SmileySetChoice): string {
  return set.name || 'Smiley set';
}

/** A tile's accessible name (and the desktop tooltip). */
export function tileLabel(set: SmileySetChoice): string {
  return set.available ? setTitle(set) : `${setTitle(set)}, opens Twitch Prime`;
}

export type TileAction = 'open' | 'switch' | 'none';

/** What a tap on the tile for `key` does. Nothing while a change is going out. */
export function tileAction(view: SmileyStyle, key: string, pending: string | null): TileAction {
  if (pending !== null) return 'none';
  const set = view.sets.find((s) => s.key === key);
  if (!set) return 'none';
  if (!set.available) return 'open';
  if (set.key === view.current) return 'none';
  return view.can_switch ? 'switch' : 'none';
}

export interface RingState {
  /** True once the first answer has been taken, drawn or not. */
  loaded: boolean;
  /** The view the row draws; null means the row is not drawn. */
  view: SmileyStyle | null;
  /** The key being switched to, while the change is going out. */
  pending: string | null;
  /** The view from before the tap, for putting the ring back. */
  before: SmileyStyle | null;
  /** The latest event that arrived while the change was going out. */
  queued: SmileyStyle | null;
}

export const INITIAL_RING: RingState = { loaded: false, view: null, pending: null, before: null, queued: null };

export type RingAction =
  /** The first answer from `get_smiley_style`. */
  | { type: 'load'; view: SmileyStyle | null }
  /** A later view from Rust. */
  | { type: 'event'; view: SmileyStyle | null }
  | { type: 'tap'; key: string }
  /** The command answered with the committed view. */
  | { type: 'settled'; view: SmileyStyle }
  /** The command failed. */
  | { type: 'failed' };

/**
 * The row draws only if the first answer had a view, so it never pops in, and a
 * drawn row never goes away. Later views move the ring only. While a change is
 * going out the ring stays where the tap put it: a view that arrives meanwhile
 * is kept and applied when the change settles (a success keeps the committed
 * ring and takes only a view that agrees with it; a failure puts the ring back
 * and takes it as is).
 */
export function ringReducer(state: RingState, action: RingAction): RingState {
  switch (action.type) {
    case 'load':
      if (state.loaded) return state;
      return { ...state, loaded: true, view: action.view };
    case 'event':
      if (!state.view || !action.view) return state;
      if (state.pending !== null) return { ...state, queued: action.view };
      return { ...state, view: action.view };
    case 'tap':
      if (!state.view || state.pending !== null) return state;
      return {
        ...state,
        view: { ...state.view, current: action.key },
        pending: action.key,
        before: state.view,
        queued: null,
      };
    case 'settled': {
      if (state.pending === null) return state;
      const agreeing = state.queued && state.queued.current === action.view.current ? state.queued : null;
      return { ...state, view: agreeing ?? action.view, pending: null, before: null, queued: null };
    }
    case 'failed':
      if (state.pending === null) return state;
      return { ...state, view: state.queued ?? state.before, pending: null, before: null, queued: null };
  }
}

export interface TileDeps {
  dispatch: (action: RingAction) => void;
  /** `invoke('set_smiley_style', { key })`. */
  setStyle: (key: string) => Promise<SmileyStyle>;
  /** `openExternal`; false when the browser did not open. */
  open: (url: string) => Promise<boolean>;
  /** An error toast. */
  toast: (message: string) => void;
}

/** Runs the tap on a tile: opens the Prime page, switches the set, or does nothing. */
export async function handleTile(
  view: SmileyStyle,
  key: string,
  pending: string | null,
  deps: TileDeps,
): Promise<TileAction> {
  const action = tileAction(view, key, pending);
  if (action === 'open') {
    if (!(await deps.open(view.upgrade_url))) deps.toast(SMILEY_OPEN_FAILED);
  } else if (action === 'switch') {
    deps.dispatch({ type: 'tap', key });
    try {
      deps.dispatch({ type: 'settled', view: await deps.setStyle(key) });
    } catch (err) {
      deps.dispatch({ type: 'failed' });
      deps.toast(typeof err === 'string' && err ? err : SMILEY_SWITCH_FAILED);
    }
  }
  return action;
}

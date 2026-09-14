// How the watch surface arranges player and chat on a screen big enough for
// both arrangements: a Fold's inner display, or a tablet.
//
// `auto` picks whichever gives the video more height while leaving chat usable,
// which on an unfolded Fold means stacked and on a landscape tablet means
// columns. But it stays a DEFAULT, not a verdict. On a near-square screen both
// arrangements are legitimate: columns give you video and chat at full height
// at the cost of a small picture, stacked gives you a big picture with
// pillarbox bars either side. Which of those is the better trade is the
// viewer's call, not ours.
//
// Persisted the same way the stream card/list preference is, so a choice made
// on the sofa is still there tomorrow.
export type WatchLayout = 'auto' | 'stacked' | 'columns';

const KEY = 'sn-watch-layout';

export function readWatchLayout(): WatchLayout {
  const v = localStorage.getItem(KEY);
  return v === 'stacked' || v === 'columns' ? v : 'auto';
}

export function writeWatchLayout(mode: WatchLayout): void {
  localStorage.setItem(KEY, mode);
}

// How much of the long axis chat gets, as a FRACTION rather than pixels.
//
// The desktop stores this as a pixel width because its window is the only shape
// it ever sees. A phone is not: the same preference has to survive a fold, a
// rotation and a move between a 840dp inner display and a 1280dp tablet. A
// fraction carries across all of them; 400px does not.
//
// Null means "follow the arrangement's natural split", which is a 16:9 player
// with chat taking the remainder. Once dragged, the choice sticks.
const SPLIT_KEY = 'sn-watch-split';

export function readWatchSplit(): number | null {
  const v = Number(localStorage.getItem(SPLIT_KEY));
  return Number.isFinite(v) && v > 0 && v < 1 ? v : null;
}

export function writeWatchSplit(frac: number | null): void {
  if (frac === null) localStorage.removeItem(SPLIT_KEY);
  else localStorage.setItem(SPLIT_KEY, String(frac));
}

// The tall layout's chat (SN-40): the pieces of VerticalChat.tsx that are not
// components, kept apart so the component file holds components only.
import { useState } from 'react';
import { useChannelCountableCount } from '../../stores/chatConnectionStore';
import type { StackCardControl, StackKind } from '../../components/chat/chatTopStack';

/** What the tall layout's chrome carries, so a tap on it is never a tap
 *  outside the grown chat. */
export const VERTICAL_CHROME = '[data-vertical-chrome]';

/**
 * Rows that arrived in the active room since chat was cleared. Each room keeps
 * its own anchor on the slice's countable counter (one that only grows): taken
 * on entering cleared, a room first seen anchors where it stands, and a counter
 * that fell (a new room under the same key) anchors again (MissedCount's rule,
 * kept in state so a render never mutates anything). Reads nothing while chat
 * shows, so the button never re-renders on a chat flush then.
 */
export function useMissedCount(room: string | null, counting: boolean): number {
  const key = (room ?? '').toLowerCase();
  const value = useChannelCountableCount(key || null, counting);
  const [anchors, setAnchors] = useState<Readonly<Record<string, number>>>({});
  const [wasCounting, setWasCounting] = useState(false);
  let next = anchors;
  if (counting && value != null && key) {
    if (!wasCounting) next = { ...next, [key]: value };
    else if (next[key] === undefined || value < next[key]) next = { ...next, [key]: value };
  }
  if (counting !== wasCounting) setWasCounting(counting);
  if (next !== anchors) setAnchors(next);
  const at = next[key];
  return counting && value != null && at !== undefined && value >= at ? value - at : 0;
}

/**
 * The poll and prediction cards in the lower third: every card a one-line
 * strip, and a tap on one grows the chat with that card open, so nothing full
 * sits over the video. Outside the lower third the stack's own layout stands.
 */
export function foldedStack(
  control: Record<StackKind, StackCardControl>,
  folded: boolean,
  onOpen: () => void,
): Record<StackKind, StackCardControl> {
  if (!folded) return control;
  const out = {} as Record<StackKind, StackCardControl>;
  for (const kind of Object.keys(control) as StackKind[]) {
    const c = control[kind];
    out[kind] = {
      ...c,
      open: false,
      onToggle: () => {
        onOpen();
        if (!c.open) c.onToggle();
      },
    };
  }
  return out;
}

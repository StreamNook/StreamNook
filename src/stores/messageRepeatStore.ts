import { create } from 'zustand';

/**
 * Repeat runs, keyed by the id of the FIRST message in the run.
 *
 * When repeat collapse is on, chatConnectionStore keeps only that first message
 * in chat and routes every later copy here. The surviving row subscribes via
 * `useMessageRepeat` and renders the count, so a copypasta wave shows as one row
 * with "x30" instead of 30 identical rows.
 *
 * This mirrors giftBombStore deliberately. Keeping the count in a side store and
 * subscribing from inside the row is what lets an already-mounted ChatMessage
 * update without new props: React.memo only gates re-renders driven by the
 * parent, not by a component's own hook subscriptions. Threading a count prop
 * through ChatMessageList would re-render the whole list instead.
 */
export interface RepeatParticipant {
  userId: string;
  displayName: string;
}

interface RepeatEntry {
  /** Total messages in the run, including the visible first one. */
  count: number;
  /** Who else said it, oldest first, for the counter's tooltip. */
  participants: RepeatParticipant[];
}

interface MessageRepeatState {
  byAnchor: Map<string, RepeatEntry>;
  /** Record the run size to display on one row. The ingest path owns the
   *  counting, since it already owns the run state; this is a display cache. */
  noteRun: (rowId: string, count: number, participants: RepeatParticipant[]) => void;
  /** Drop every run for a channel switch / disconnect. */
  clear: () => void;
}

// Cap on tracked runs so a long session can't grow this unbounded. The bound
// must comfortably exceed the largest visible buffer (message_buffer_cap maxes
// at 1000, plus the paused cushion, times MultiChat panes): evicting an anchor
// whose row is still on screen silently strips its xN badge, and every message
// folded into it vanishes with no trace. At 100 this fired constantly in fast
// chats; entries are tiny (a count plus up to 20 names), so a generous cap
// costs almost nothing.
const MAX_ANCHORS = 4096;

// How many names the tooltip is worth holding. A 200-person wave doesn't need
// 200 names retained, and this bounds a single entry's memory.
const MAX_PARTICIPANTS = 20;

const EMPTY_ENTRY: RepeatEntry = { count: 1, participants: [] };

// Replace byAnchor immutably and evict past the cap. A fresh Map keeps
// `get(anchorId)` reference-stable for untouched anchors, so rows for other
// runs don't re-render when one run grows.
function withEntry(
  prev: Map<string, RepeatEntry>,
  anchorId: string,
  entry: RepeatEntry,
): Map<string, RepeatEntry> {
  const next = new Map(prev);
  next.delete(anchorId); // re-insert so this anchor becomes the most-recent key
  next.set(anchorId, entry);
  while (next.size > MAX_ANCHORS) {
    const oldest = next.keys().next().value as string | undefined;
    if (oldest === undefined) break;
    next.delete(oldest);
  }
  return next;
}

export const useMessageRepeatStore = create<MessageRepeatState>((set, get) => ({
  byAnchor: new Map(),

  noteRun: (rowId, count, participants) => {
    const existing = get().byAnchor.get(rowId);
    // In label mode each row is written once and never grows, so skip the Map
    // rebuild when nothing actually changed.
    if (existing && existing.count === count && existing.participants === participants) return;
    set({
      byAnchor: withEntry(get().byAnchor, rowId, {
        count,
        participants: participants.slice(0, MAX_PARTICIPANTS),
      }),
    });
  },

  clear: () => set({ byAnchor: new Map() }),
}));

/**
 * Subscribe to one message's repeat run. Safe to call unconditionally for every
 * row — messages that never repeated return a stable empty entry, so the caller
 * satisfies the rules of hooks without branching and never re-renders for it.
 */
export function useMessageRepeat(messageId?: string): RepeatEntry {
  return useMessageRepeatStore((s) =>
    messageId ? s.byAnchor.get(messageId) ?? EMPTY_ENTRY : EMPTY_ENTRY,
  );
}

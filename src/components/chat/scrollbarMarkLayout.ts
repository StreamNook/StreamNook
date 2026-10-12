// Scrollbar marks: the ticks on a chat's scrollbar where marked rows sit.
//
// What counts as a marked row, and its colour, is Rust's: the rule engine
// stamps `metadata.scrollbar_mark` on every row it evaluates. Why the rest is
// here and not in Rust: a tick's position is the row's place in the browser's
// own layout (rows virtualize with `content-visibility: auto`, so only the page
// knows their heights), and the pinned kind joins the row list with the
// channel's pinned ids, which the page holds per chat. The functions below are
// a set lookup and a pixel layout; nothing here parses or classifies.

import type { BackendChatMessage, ScrollbarMarkKind } from '../../services/twitchChat';

/** One marked row: its id, the kind it is drawn as, and its stamped colour. */
export interface MarkedRow {
  kind: ScrollbarMarkKind;
  color?: string;
}

/** A marked row placed in the scroll content, in CSS px. */
export interface PlacedMark extends MarkedRow {
  top: number;
  height: number;
}

/** One drawn tick on the track, in track px. */
export interface Tick extends MarkedRow {
  y: number;
  h: number;
}

/** Strongest first: a pinned row with a weaker stamp is drawn as pinned, and
 *  where ticks of two kinds overlap the stronger one is drawn on top. */
export const MARK_PRIORITY: readonly ScrollbarMarkKind[] = [
  'mention',
  'reply',
  'pinned',
  'highlight',
  'announcement',
  'sub',
  'redeem',
  'raid',
  'first_time',
];

const RANK: Record<string, number> = Object.fromEntries(MARK_PRIORITY.map((k, i) => [k, i]));
const rank = (kind: string) => RANK[kind] ?? MARK_PRIORITY.length;

/** The kinds a viewer can switch, in settings order, with their labels.
 *  `mention` covers replies to you as well. */
export const MARK_KIND_OPTIONS: ReadonlyArray<{ kind: ScrollbarMarkKind; label: string }> = [
  { kind: 'mention', label: 'Mentions' },
  { kind: 'highlight', label: 'Highlights' },
  { kind: 'pinned', label: 'Pinned' },
  { kind: 'announcement', label: 'Announcements' },
  { kind: 'sub', label: 'Subs' },
  { kind: 'redeem', label: 'Redeems' },
  { kind: 'raid', label: 'Raids' },
  { kind: 'first_time', label: 'First messages' },
];

/** The kinds drawn when the settings store has not loaded yet. The saved
 *  default is Rust's (`default_scrollbar_mark_kinds`); this mirrors it. */
export const FALLBACK_MARK_KINDS: readonly string[] = [
  'mention',
  'highlight',
  'pinned',
  'announcement',
  'sub',
  'redeem',
  'raid',
];

/** Whether a stamped kind is drawn under the viewer's choice of kinds. */
export const kindShown = (kind: ScrollbarMarkKind, shown: ReadonlySet<string>) =>
  shown.has(kind === 'reply' ? 'mention' : kind);

/**
 * The marked rows of a message list, by row id. Rust's stamp decides the kind;
 * a row in `pinnedIds` is drawn as pinned unless its stamp is stronger.
 */
export function collectMarks(
  messages: readonly (string | BackendChatMessage)[],
  getMessageId: (message: string | BackendChatMessage) => string | null,
  shown: ReadonlySet<string>,
  pinnedIds?: ReadonlySet<string>,
): Map<string, MarkedRow> {
  const out = new Map<string, MarkedRow>();
  if (shown.size === 0) return out;
  const pinOn = !!pinnedIds && pinnedIds.size > 0 && shown.has('pinned');
  for (const message of messages) {
    const stamp = typeof message === 'string' ? undefined : message.metadata?.scrollbar_mark;
    const visible = stamp && kindShown(stamp.kind, shown) ? stamp : undefined;
    if (!visible && !pinOn) continue;
    const id = getMessageId(message);
    if (!id) continue;
    if (pinOn && pinnedIds!.has(id) && (!visible || rank('pinned') < rank(visible.kind))) {
      out.set(id, { kind: 'pinned' });
    } else if (visible) {
      out.set(id, visible.color ? { kind: visible.kind, color: visible.color } : { kind: visible.kind });
    }
  }
  return out;
}

export interface TickLayout {
  /** Shortest tick, px. */
  minHeight: number;
  /** Tallest tick for a single row, px; merged runs may be taller. */
  maxHeight: number;
  /** Ticks closer than this (px between them) merge into one. */
  gap: number;
}

export const DEFAULT_TICK_LAYOUT: TickLayout = { minHeight: 3, maxHeight: 5, gap: 1 };

/**
 * Places marked rows on a track of `trackHeight` px standing for
 * `contentHeight` px of scroll content, the same ratio the scrollbar thumb
 * uses. Each tick is centred on its row and sized by the row's share of the
 * content, clamped to the layout's bounds.
 *
 * Dense stretches: ticks of one kind and one colour that would touch merge
 * into one run spanning them (a gift bomb is one purple bar, not sixty), which
 * bounds the count by the track height per kind and colour. Ticks of different
 * kinds or colours never merge, so a single highlight beside a long sub run
 * cannot recolour the run, and two highlight rules keep their own colours;
 * they are returned weakest kind first, so the stronger one paints on top
 * where two overlap.
 */
export function layoutTicks(
  rows: readonly PlacedMark[],
  contentHeight: number,
  trackHeight: number,
  layout: TickLayout = DEFAULT_TICK_LAYOUT,
): Tick[] {
  if (rows.length === 0 || contentHeight <= 0 || trackHeight <= 0) return [];
  const scale = trackHeight / contentHeight;
  // Runs merge only within one kind AND one colour: two highlight rules with
  // their own colours stay two ticks.
  const groups = new Map<string, Tick[]>();
  for (const r of rows) {
    const h = Math.max(layout.minHeight, Math.min(layout.maxHeight, r.height * scale));
    const mid = (r.top + r.height / 2) * scale;
    const y = Math.max(0, Math.min(trackHeight - h, mid - h / 2));
    const t: Tick = r.color ? { kind: r.kind, color: r.color, y, h } : { kind: r.kind, y, h };
    const key = `${r.kind}|${r.color ?? ''}`;
    const list = groups.get(key);
    if (list) list.push(t);
    else groups.set(key, [t]);
  }

  const out: Tick[] = [];
  const ordered = [...groups.values()].sort((a, b) => rank(b[0].kind) - rank(a[0].kind));
  for (const group of ordered) {
    const placed = group.sort((a, b) => a.y - b.y);
    let last: Tick | undefined;
    for (const t of placed) {
      if (last && t.y <= last.y + last.h + layout.gap) {
        last.h = Math.max(last.y + last.h, t.y + t.h) - last.y;
        continue;
      }
      last = { ...t };
      out.push(last);
    }
  }
  // Whole pixels, so a tick never lands on a half pixel and blurs.
  for (const t of out) {
    const top = Math.round(t.y);
    t.h = Math.max(layout.minHeight, Math.round(t.y + t.h) - top);
    t.y = top;
  }
  return out;
}

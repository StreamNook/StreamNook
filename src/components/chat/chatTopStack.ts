// The cards at the top of a chat column (poll, prediction, the watched stream's
// drop) as one stack that Rust lays out (`chat_top_stack`,
// services/chat_top_stack.rs): their order, the one card that is open while the
// rest stay one-line strips, and the most height the stack may take. Opened
// side by side on a short column they left no room to read or type, and one
// card could paint over another's fold control.
//
// This hook only gathers what Rust needs (which cards have something to show,
// what the viewer opened and folded, the column's height) and hands each card
// its slot. The cards stay self-fetching: they report themselves present.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';

export type StackKind = 'prediction' | 'poll' | 'drop';

export interface StackSlot {
  kind: StackKind;
  open: boolean;
}

export interface StackLayout {
  /** Top to bottom. */
  slots: StackSlot[];
  /** The most height the whole stack may take, 0 while the column is unmeasured. */
  max_height: number;
}

/** `StackRequest` in services/chat_top_stack.rs. */
export interface StackRequest {
  cards: StackKind[];
  order: 'prediction-first' | 'poll-first';
  polls_start_collapsed: boolean;
  opened: StackKind | null;
  folded: StackKind[];
  chat_height: number;
}

/** What the host hands one card. */
export interface StackCardControl {
  /** Rust placed the card; until it has, the card draws nothing. */
  placed: boolean;
  /** Open with its body, or its one-line strip. */
  open: boolean;
  /** Its place top to bottom (a flex `order`). */
  order: number;
  /** The strip's or head's tap. */
  onToggle: () => void;
  /** The card's identity while it has something to show (the poll id, the
   *  drop's campaign), null while it has nothing. Stable for the card's life. */
  onPresence: (id: string | null) => void;
}

const KINDS: StackKind[] = ['prediction', 'poll', 'drop'];

// A drag of the split handle or a fold resizes the column every frame; Rust is
// asked once the size settles.
const RESIZE_SETTLE_MS = 150;

/**
 * What a tap on `kind`'s head does to what the viewer has told Rust, and the
 * slots to draw until Rust answers. Folding an open card marks it folded.
 * Opening one makes it the opened card, and the card it closes counts as
 * folded by the viewer, so it does not spring back open when this one folds.
 */
export function toggleCard(
  slots: StackSlot[],
  opened: StackKind | null,
  folded: StackKind[],
  kind: StackKind,
): { opened: StackKind | null; folded: StackKind[]; slots: StackSlot[] } {
  const isOpen = slots.some((s) => s.kind === kind && s.open);
  if (isOpen) {
    return {
      opened: opened === kind ? null : opened,
      folded: folded.includes(kind) ? folded : [...folded, kind],
      slots: slots.map((s) => ({ ...s, open: false })),
    };
  }
  const closing = slots.filter((s) => s.open && s.kind !== kind).map((s) => s.kind);
  return {
    opened: kind,
    folded: [...folded.filter((k) => k !== kind && !closing.includes(k)), ...closing],
    slots: slots.map((s) => ({ ...s, open: s.kind === kind })),
  };
}

/** Lays out the stack in `column`, the chat column whose height caps it. */
export function useChatTopStack(column: HTMLElement | null): {
  control: Record<StackKind, StackCardControl>;
  maxHeight: number;
} {
  const order = useAppStore((s) => s.settings.chat_overlay_order) ?? 'prediction-first';
  const pollsStartCollapsed = useAppStore((s) => s.settings.chat_design?.polls_start_collapsed ?? false);

  const [ids, setIds] = useState<Partial<Record<StackKind, string>>>({});
  const idsRef = useRef(ids);
  const [opened, setOpened] = useState<StackKind | null>(null);
  const [folded, setFolded] = useState<StackKind[]>([]);
  const [height, setHeight] = useState(0);
  const [layout, setLayout] = useState<StackLayout | null>(null);
  // Read by the tap handler, which stays one function for the hook's life.
  const layoutRef = useRef(layout);
  const openedRef = useRef(opened);
  const foldedRef = useRef(folded);
  useEffect(() => {
    layoutRef.current = layout;
    openedRef.current = opened;
    foldedRef.current = folded;
  }, [layout, opened, folded]);

  useEffect(() => {
    if (!column) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const read = () => setHeight(Math.round(column.getBoundingClientRect().height));
    read();
    const ro = new ResizeObserver(() => {
      clearTimeout(timer);
      timer = setTimeout(read, RESIZE_SETTLE_MS);
    });
    ro.observe(column);
    return () => {
      ro.disconnect();
      clearTimeout(timer);
    };
  }, [column]);

  const cards = useMemo(() => KINDS.filter((k) => ids[k] != null), [ids]);

  useEffect(() => {
    if (cards.length === 0) return;
    let live = true;
    const request: StackRequest = {
      cards,
      order,
      polls_start_collapsed: pollsStartCollapsed,
      opened,
      folded,
      chat_height: height,
    };
    invoke<StackLayout>('chat_top_stack', { request })
      .then((next) => {
        if (live) setLayout(next);
      })
      .catch((err) => {
        // Without an answer every card still shows and the viewer's own pick
        // still opens, so nothing is lost but the automatic open.
        Logger.debug('[ChatTopStack] layout failed:', err);
        if (live) setLayout({ slots: cards.map((kind) => ({ kind, open: kind === opened })), max_height: 0 });
      });
    return () => {
      live = false;
    };
  }, [cards, order, pollsStartCollapsed, opened, folded, height]);

  const report = useCallback((kind: StackKind, id: string | null) => {
    const prev = idsRef.current[kind] ?? null;
    if (prev === id) return;
    const next = { ...idsRef.current };
    if (id === null) delete next[kind];
    else next[kind] = id;
    idsRef.current = next;
    setIds(next);
    // A card that left, or a new one in its place, has no slot until Rust's
    // next answer: the old card's open or folded state is not the new one's.
    setLayout((l) => l && { ...l, slots: l.slots.filter((s) => s.kind !== kind) });
    if (id !== null) {
      // A new poll, prediction or campaign is a new card: the viewer's fold or
      // open of the last one does not carry over to it.
      setFolded((f) => (f.includes(kind) ? f.filter((k) => k !== kind) : f));
      setOpened((o) => (o === kind ? null : o));
    }
  }, []);

  const toggle = useCallback((kind: StackKind) => {
    const next = toggleCard(layoutRef.current?.slots ?? [], openedRef.current, foldedRef.current, kind);
    openedRef.current = next.opened;
    foldedRef.current = next.folded;
    setOpened(next.opened);
    setFolded(next.folded);
    // Drawn at once rather than a round trip later; Rust's answer confirms it.
    setLayout((l) => l && { ...l, slots: next.slots });
  }, []);

  // One callback pair per kind for the hook's life, so a card's presence
  // effect never re-runs (and re-reports) because the layout moved.
  const handlers = useMemo(
    () =>
      Object.fromEntries(
        KINDS.map((k) => [k, { onToggle: () => toggle(k), onPresence: (id: string | null) => report(k, id) }]),
      ) as Record<StackKind, Pick<StackCardControl, 'onToggle' | 'onPresence'>>,
    [toggle, report],
  );

  const control = useMemo(() => {
    const slots = layout?.slots ?? [];
    return Object.fromEntries(
      KINDS.map((k) => {
        const at = slots.findIndex((s) => s.kind === k);
        return [k, { placed: at >= 0, open: at >= 0 && slots[at].open, order: at >= 0 ? at : KINDS.length, ...handlers[k] }];
      }),
    ) as Record<StackKind, StackCardControl>;
  }, [layout, handlers]);

  return { control, maxHeight: layout?.max_height ?? 0 };
}

/** The space between two cards of the stack (ChatOverlayStack's gap), which a
 *  leaving card folds away along with its own height. */
export const STACK_GAP_PX = 4;

// A leaving card gives its slot back when its exit finishes; this bounds the
// wait if the exit never reports (a stalled frame), so the stack can never keep
// a slot for a card that is gone.
const LEAVE_FALLBACK_MS = 1000;

/**
 * Reports a card's presence to its stack for as long as it is mounted. A card
 * that appears is reported at once, so its slot opens as it fades in. A card
 * that was on screen and goes away keeps its slot until its exit has played:
 * the returned function, for AnimatePresence's onExitComplete, gives it back.
 * Released at once instead, the cards around it would move (and Rust could
 * open another) while it is still fading.
 */
export function useStackPresence(stack: StackCardControl | undefined, id: string | null): () => void {
  const onPresence = stack?.onPresence;
  const drawn = id !== null && (stack?.placed ?? false);
  const idRef = useRef(id);
  // Whether the last render drew the card, i.e. whether there is an exit to wait for.
  const drawnRef = useRef(false);

  useEffect(() => {
    idRef.current = id;
    if (id !== null || !drawnRef.current) {
      onPresence?.(id);
      return;
    }
    const timer = setTimeout(() => {
      if (idRef.current === null) onPresence?.(null);
    }, LEAVE_FALLBACK_MS);
    return () => clearTimeout(timer);
  }, [onPresence, id]);
  // Declared after the effect above, which reads the previous render's value.
  useEffect(() => {
    drawnRef.current = drawn;
  }, [drawn]);
  useEffect(() => () => onPresence?.(null), [onPresence]);

  return useCallback(() => {
    // A card that came back while the old one was leaving keeps its slot.
    if (idRef.current === null) onPresence?.(null);
  }, [onPresence]);
}

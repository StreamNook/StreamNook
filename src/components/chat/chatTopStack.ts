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
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';

/** The seven cards of the stack (SN-284), in Rust's order. The poll, prediction
 *  and drop cards report themselves; the others are reported by their host
 *  through `control[kind].onPresence`. */
export type StackKind =
  | 'hype'
  | 'prediction'
  | 'poll'
  | 'drop'
  | 'channel_pin'
  | 'mention_context'
  | 'mention_pins';

/** How Rust says a card is drawn (`Form` in services/chat_top_stack.rs). */
export type StackForm = 'strip' | 'open' | 'folded' | 'floor' | 'full' | 'count_line';

export interface StackSlot {
  kind: StackKind;
  open: boolean;
  form: StackForm;
  /** The height this card takes, whole pixels; 0 when folded. */
  height: number;
  /** `count_line` only: how many cards folded into it. */
  more_cards?: number;
}

/** The "N more cards" strip: the cards folded into it, lowest rank last. */
export interface StackMore {
  count: number;
  kinds: StackKind[];
  height: number;
}

export interface StackLayout {
  /** Top to bottom, every card present (folded ones included, by their form). */
  slots: StackSlot[];
  /** The most height the whole stack may take, 0 while the column is unmeasured. */
  max_height: number;
  /** The "N more cards" strip, drawn after the last slot, when cards folded. */
  more?: StackMore;
  /** What the whole stack takes, `more` included. */
  height: number;
  /** The key of the chat this surface shows (Rust's room key): the open and
   *  folded choices are kept by it, so no shell builds a key of its own. */
  room_key: string;
}

/** A card's last measured heights per form (`CardHeights` in Rust). */
export interface CardHeights {
  strip?: number;
  open?: number;
  line?: number;
}

/** `StackRequest` in services/chat_top_stack.rs. */
export interface StackRequest {
  cards: StackKind[];
  order: 'prediction-first' | 'poll-first';
  polls_start_collapsed: boolean;
  predictions_start_collapsed: boolean;
  prediction_waiting: boolean;
  opened: StackKind | null;
  folded: StackKind[];
  chat_height: number;
  /** The channel's pinned message is open (the per-room open store). */
  channel_pin_opened: boolean;
  /** Each card's last measured heights, per form. */
  heights: Partial<Record<StackKind, CardHeights>>;
  /** The lower third: strips only, nothing opens, the pins show as their floor. */
  strips_only: boolean;
  /** The chat this surface shows, so the answer names its room key. */
  provider: string | null;
  channel: string | null;
}

/** What the host hands one card. */
export interface StackCardControl {
  /** Rust placed the card; until it has, the card draws nothing. */
  placed: boolean;
  /** Open with its body, or its one-line strip. */
  open: boolean;
  /** How Rust draws the card (strip, open, folded, floor, full, count line). */
  form: StackForm | null;
  /** The height Rust gave it, 0 until placed. */
  height: number;
  /** Cards folded into the pins' count line. */
  moreCards: number;
  /** Its place top to bottom (a flex `order`). */
  order: number;
  /** The strip's or head's tap. */
  onToggle: () => void;
  /** The card's identity while it has something to show (the poll id, the
   *  drop's campaign), null while it has nothing. Stable for the card's life. */
  onPresence: (id: string | null) => void;
  /** The prediction says whether it is closed and waiting for its result,
   *  which keeps it from opening by itself. The other cards never call it. */
  onWaiting: (waiting: boolean) => void;
}

const KINDS: StackKind[] = ['hype', 'prediction', 'poll', 'drop', 'channel_pin', 'mention_context', 'mention_pins'];

// A drag of the split handle or a fold resizes the column every frame; Rust is
// asked once the size settles.
const RESIZE_SETTLE_MS = 150;

/**
 * What a tap on `kind`'s head does to what the viewer has told Rust, and the
 * slots to draw until Rust answers. Folding an open card marks it folded.
 * Opening one makes it the opened card, and the card it closes counts as
 * folded by the viewer, so it does not spring back open when this one folds.
 */
export function toggleCard<S extends { kind: StackKind; open: boolean }>(
  slots: S[],
  opened: StackKind | null,
  folded: StackKind[],
  kind: StackKind,
): { opened: StackKind | null; folded: StackKind[]; slots: S[] } {
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

/** The viewer's open and folded cards, kept per chat and per surface for the
 *  session. The chat is named by the room key Rust answers with (never one a
 *  shell builds); the phones key by room only, since the watch screen, the
 *  Chats tab and the composer's menu share one choice by design. */
const keptChoices = new Map<string, { opened: StackKind | null; folded: StackKind[] }>();
export const choiceKey = (surface: string, roomKey: string, phone: boolean): string =>
  phone ? roomKey : `${surface}|${roomKey}`;

/** What the host tells the stack about the chat it sits in. */
export interface ChatTopStackOptions {
  /** The pin surface id: part of the open/folded key on the desktop. */
  surface?: string;
  /** The phone's one store is keyed by room only. */
  phone?: boolean;
  /** The chat this surface shows, so Rust answers with its room key. */
  provider?: string | null;
  channel?: string | null;
  /** The lower third: strips only. */
  stripsOnly?: boolean;
  /** The channel's pinned message is open (every writer feeds this). */
  channelPinOpened?: boolean;
  /** The stack opened or folded the channel pin: written back to the one
   *  per-room store, so the banner, the composer menu and the header's pin
   *  button follow what a tap on the strip did. */
  onChannelPinOpen?: (open: boolean) => void;
  /** Ask Rust about these cards only (the other views of the chat column show
   *  the hype train and nothing else); the rest stay mounted without a slot. */
  only?: readonly StackKind[];
}

/** What a toggle did to the channel pin, for the store that owns its open
 *  state: the new state when it changed, else null. */
export function channelPinChange(
  before: readonly { kind: StackKind; open: boolean }[],
  after: readonly { kind: StackKind; open: boolean }[],
): boolean | null {
  const was = before.some((s) => s.kind === 'channel_pin' && s.open);
  const now = after.some((s) => s.kind === 'channel_pin' && s.open);
  return was === now ? null : now;
}

/** The viewer's choices after the channel pin's store changed from outside the
 *  stack (the composer menu, the header's pin button): the same as a tap on
 *  its strip. */
export function applyChannelPin(
  opened: StackKind | null,
  folded: StackKind[],
  open: boolean,
): { opened: StackKind | null; folded: StackKind[] } {
  if (open) return { opened: 'channel_pin', folded: folded.filter((k) => k !== 'channel_pin') };
  return {
    opened: opened === 'channel_pin' ? null : opened,
    folded: folded.includes('channel_pin') ? folded : [...folded, 'channel_pin'],
  };
}

const HEIGHT_SETTLE_MS = 150;

/** What Rust's slot for `kind` says about the card: placed or not, open, its
 *  form and height, and its place top to bottom. A card Rust has no slot for
 *  (not reported, or folded away with the others) is not placed. */
export function slotControl(
  slots: readonly StackSlot[],
  kind: StackKind,
): Pick<StackCardControl, 'placed' | 'open' | 'form' | 'height' | 'moreCards' | 'order'> {
  const at = slots.findIndex((s) => s.kind === kind);
  return {
    placed: at >= 0,
    open: at >= 0 && slots[at].open,
    form: at >= 0 ? slots[at].form : null,
    height: at >= 0 ? slots[at].height : 0,
    moreCards: at >= 0 ? (slots[at].more_cards ?? 0) : 0,
    order: at >= 0 ? at : KINDS.length,
  };
}

/** Lays out the stack in `column`, the chat column whose height caps it. */
export function useChatTopStack(
  column: HTMLElement | null,
  options: ChatTopStackOptions = {},
): {
  control: Record<StackKind, StackCardControl>;
  maxHeight: number;
  /** What the whole stack takes, `more` included (0 until Rust answers). */
  height: number;
  /** The "N more cards" strip, when cards folded into it. */
  more: StackMore | null;
  /** A card reports the height it drew in a form; Rust is sent the last of each. */
  measure: (kind: StackKind, form: 'strip' | 'open' | 'line', height: number) => void;
} {
  const {
    surface = '',
    phone = false,
    provider = null,
    channel = null,
    stripsOnly = false,
    channelPinOpened = false,
    onChannelPinOpen,
    only,
  } = options;
  const onChannelPinOpenRef = useRef(onChannelPinOpen);
  useEffect(() => {
    onChannelPinOpenRef.current = onChannelPinOpen;
  }, [onChannelPinOpen]);
  const onlyKey = only ? only.join('|') || '-' : '';
  const order = useAppStore((s) => s.settings.chat_overlay_order) ?? 'prediction-first';
  const pollsStartCollapsed = useAppStore((s) => s.settings.chat_design?.polls_start_collapsed ?? false);
  const predictionsStartCollapsed = useAppStore(
    (s) => s.settings.chat_design?.predictions_start_collapsed ?? false,
  );
  const [predictionWaiting, setPredictionWaiting] = useState(false);

  const [ids, setIds] = useState<Partial<Record<StackKind, string>>>({});
  const idsRef = useRef(ids);
  const [opened, setOpened] = useState<StackKind | null>(null);
  const [folded, setFolded] = useState<StackKind[]>([]);
  const [height, setHeight] = useState(0);
  const [layout, setLayout] = useState<StackLayout | null>(null);
  // Each card's last measured height per form, sent with every request.
  const [measured, setMeasured] = useState<Partial<Record<StackKind, CardHeights>>>({});
  const measuredRef = useRef(measured);
  const measureTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(measureTimer.current), []);
  // Read by the tap handler, which stays one function for the hook's life.
  const layoutRef = useRef(layout);
  const surfaceRef = useRef(surface);
  const phoneRef = useRef(phone);
  useEffect(() => {
    surfaceRef.current = surface;
    phoneRef.current = phone;
  }, [surface, phone]);
  const openedRef = useRef(opened);
  const foldedRef = useRef(folded);
  // Before any child's effect of the same commit: a card that toggles itself as it is
  // placed (the mention context card) reads the layout that placed it, not the one before.
  useLayoutEffect(() => {
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

  const cards = useMemo(
    () => KINDS.filter((k) => ids[k] != null && (onlyKey === '' || onlyKey.split('|').includes(k))),
    [ids, onlyKey],
  );

  // The room Rust named: the open and folded choices of that chat come back
  // when the surface moves to it, and are kept as the viewer makes them.
  const roomKey = layout?.room_key ?? '';
  const roomKeyRef = useRef('');
  useEffect(() => {
    if (!roomKey || roomKeyRef.current === roomKey) return;
    roomKeyRef.current = roomKey;
    const kept = keptChoices.get(choiceKey(surface, roomKey, phone));
    setOpened(kept?.opened ?? null);
    setFolded(kept?.folded ?? []);
  }, [roomKey, surface, phone]);

  // The channel pin's store changed from outside the stack (the composer menu,
  // the header's pin button): that is a tap on its strip. Not on the first
  // render, where the request's `channel_pin_opened` already says so.
  const seenPinOpen = useRef(channelPinOpened);
  useEffect(() => {
    if (seenPinOpen.current === channelPinOpened) return;
    seenPinOpen.current = channelPinOpened;
    const inLayout = layoutRef.current?.slots.some((s) => s.kind === 'channel_pin' && s.open) ?? false;
    if (inLayout === channelPinOpened) return;
    const next = applyChannelPin(openedRef.current, foldedRef.current, channelPinOpened);
    openedRef.current = next.opened;
    foldedRef.current = next.folded;
    setOpened(next.opened);
    setFolded(next.folded);
    const key = roomKeyRef.current;
    if (key) keptChoices.set(choiceKey(surfaceRef.current, key, phoneRef.current), next);
  }, [channelPinOpened]);

  useEffect(() => {
    if (cards.length === 0) return;
    let live = true;
    const request: StackRequest = {
      cards,
      order,
      polls_start_collapsed: pollsStartCollapsed,
      predictions_start_collapsed: predictionsStartCollapsed,
      prediction_waiting: predictionWaiting,
      opened,
      folded,
      chat_height: height,
      channel_pin_opened: channelPinOpened,
      heights: measured,
      strips_only: stripsOnly,
      provider,
      channel,
    };
    invoke<StackLayout>('chat_top_stack', { request })
      .then((next) => {
        if (live) setLayout(next);
      })
      .catch((err) => {
        // Without an answer every card still shows and the viewer's own pick
        // still opens, so nothing is lost but the automatic open.
        Logger.debug('[ChatTopStack] layout failed:', err);
        if (live) {
          setLayout({
            slots: cards.map((kind) => ({ kind, open: kind === opened, form: kind === opened ? 'open' : 'strip', height: 0 })),
            max_height: 0,
            height: 0,
            room_key: '',
          });
        }
      });
    return () => {
      live = false;
    };
  }, [
    cards,
    order,
    pollsStartCollapsed,
    predictionsStartCollapsed,
    predictionWaiting,
    opened,
    folded,
    height,
    channelPinOpened,
    measured,
    stripsOnly,
    provider,
    channel,
  ]);

  const measure = useCallback((kind: StackKind, form: 'strip' | 'open' | 'line', h: number) => {
    const prev = measuredRef.current[kind]?.[form] ?? 0;
    if (h <= 0 || Math.abs(prev - h) < 2) return;
    measuredRef.current = { ...measuredRef.current, [kind]: { ...measuredRef.current[kind], [form]: h } };
    clearTimeout(measureTimer.current);
    measureTimer.current = setTimeout(() => setMeasured(measuredRef.current), HEIGHT_SETTLE_MS);
  }, []);

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
    const before = layoutRef.current?.slots ?? [];
    const next = toggleCard(before, openedRef.current, foldedRef.current, kind);
    const pin = channelPinChange(before, next.slots);
    if (pin !== null) {
      // The strip's tap is the store's write too, so every other writer reads it.
      seenPinOpen.current = pin;
      onChannelPinOpenRef.current?.(pin);
    }
    openedRef.current = next.opened;
    foldedRef.current = next.folded;
    setOpened(next.opened);
    setFolded(next.folded);
    const key = roomKeyRef.current;
    if (key) keptChoices.set(choiceKey(surfaceRef.current, key, phoneRef.current), { opened: next.opened, folded: next.folded });
    // Drawn at once rather than a round trip later; Rust's answer confirms it.
    setLayout((l) => l && { ...l, slots: next.slots });
  }, []);

  // One callback pair per kind for the hook's life, so a card's presence
  // effect never re-runs (and re-reports) because the layout moved.
  const handlers = useMemo(
    () =>
      Object.fromEntries(
        KINDS.map((k) => [
          k,
          {
            onToggle: () => toggle(k),
            onPresence: (id: string | null) => report(k, id),
            onWaiting: (waiting: boolean) => {
              if (k === 'prediction') setPredictionWaiting(waiting);
            },
          },
        ]),
      ) as Record<StackKind, Pick<StackCardControl, 'onToggle' | 'onPresence' | 'onWaiting'>>,
    [toggle, report],
  );

  // No card asked for: nothing is laid out, whatever the last answer said.
  const asked = cards.length > 0;
  const control = useMemo(() => {
    const slots = asked ? (layout?.slots ?? []) : [];
    return Object.fromEntries(
      KINDS.map((k) => {
        return [k, { ...slotControl(slots, k), ...handlers[k] }];
      }),
    ) as Record<StackKind, StackCardControl>;
  }, [layout, handlers, asked]);

  return {
    control,
    maxHeight: asked ? (layout?.max_height ?? 0) : 0,
    height: asked ? (layout?.height ?? 0) : 0,
    more: asked ? (layout?.more ?? null) : null,
    measure,
  };
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
  // A folded card is not drawn, so it has no exit to wait for.
  const drawn = id !== null && (stack?.placed ?? false) && stack?.form !== 'folded';
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

/**
 * Reports a card's identity to its stack for as long as it is mounted, with no
 * exit to wait for (the hype train, the pinned message, the mention context
 * card and the pinned mentions draw no leave animation of their own). The id is
 * null while the card has nothing to show, which gives its slot back at once.
 */
export function useCardPresence(stack: StackCardControl | undefined, id: string | null): void {
  const onPresence = stack?.onPresence;
  useEffect(() => {
    onPresence?.(id);
  }, [onPresence, id]);
  useEffect(() => () => onPresence?.(null), [onPresence]);
}

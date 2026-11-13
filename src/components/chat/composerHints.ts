// The emote button's deck and the teaching placeholder (Brandon's "Deck plus
// placeholder", 2026-11-13): which face the button wears, which cards fan out
// from behind it, when the empty message box may teach, and which hint it
// shows this moment.
//
// Why TypeScript: this is presentation timing and a pick between a few looks
// from what the box shows right now. The facts it reads come from Rust (the
// chat's emote of the moment, `services/emote_moment.rs`; whether the box
// holds a GIF link, `link_detect::find_gif_link`; whether the channel allows
// GIFs, `get_gif_picker_status`); nothing here fetches, parses or ranks.
import { useEffect, useState, useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getGifPickerStatus } from '../../services/gifService';
import { isWindowHidden, onWindowVisibility } from '../../utils/windowVisibility';

/** What the empty box is teaching: the plain prompt, commands, or GIFs. */
export type HintKind = 'message' | 'slash' | 'gif';
/** The deck's members: the chat's emote of the moment (or the smiley), the
 *  slash card and the GIF chip. The one in front is the button's face. */
export type DeckCard = 'moment' | 'slash' | 'gif';

/** How long each hint stays. */
export const HINT_MS = 2200;

/** Each hint's words, full and for a narrow composer row. */
export const HINT_WORDS: Record<HintKind, { wide: string; narrow: string }> = {
  message: { wide: 'Send a message', narrow: 'Message' },
  slash: { wide: 'Type / for commands', narrow: 'Type /' },
  gif: { wide: 'Search a GIF', narrow: 'Find a GIF' },
};

/** The hints a box cycles through: the prompt, then what its menu holds. */
export function hintCycle(o: { slash: boolean; gifs: boolean }): HintKind[] {
  const out: HintKind[] = ['message'];
  if (o.slash) out.push('slash');
  if (o.gifs) out.push('gif');
  return out;
}

/** Whether motion lets the hints move: Motion at Full, and neither Battery
 *  Saver nor Performance mode quieting the window (`useLowPower`). */
export function hintsMayMove(motionMode: string | undefined, lowPower: boolean): boolean {
  return (motionMode ?? 'full') === 'full' && !lowPower;
}

/** `layer`: the hint layer stands over the empty box (the box may teach, so
 *  the field's own placeholder is hidden under it). `active`: the hints move;
 *  a layer that is not active rests on the plain prompt. */
export interface HintGate {
  layer: boolean;
  active: boolean;
}

/**
 * Chat's composer. The box teaches only while it shows the idle prompt
 * (`composerLook().teaches`: nothing you can act on is being said), is empty
 * and motion allows it; the hints move only while you are not writing (no
 * keystroke since the field was last left), no slow-mode wait runs, no reply
 * is being started, the menu is closed and no /remind is being typed.
 */
export function desktopHints(s: {
  teaches: boolean;
  empty: boolean;
  engaged: boolean;
  slowWait: number;
  replying: boolean;
  menuOpen: boolean;
  remindOverlay: boolean;
  motionMode: string | undefined;
  lowPower: boolean;
}): HintGate {
  const layer = s.teaches && s.empty && !s.remindOverlay && hintsMayMove(s.motionMode, s.lowPower);
  return { layer, active: layer && !s.engaged && s.slowWait <= 0 && !s.replying && !s.menuOpen };
}

/**
 * The phone's composer. It teaches GIFs only (its commands sit behind their
 * own button, and typing / brings no command list), so only where GIFs are
 * offered; only while the plain prompt shows and the box is empty; the hints
 * move while the keyboard is down and no panel is open.
 */
export function phoneHints(s: {
  gifs: boolean;
  plainPrompt: boolean;
  empty: boolean;
  typing: boolean;
  panelOpen: boolean;
  motionMode: string | undefined;
  lowPower: boolean;
}): HintGate {
  const layer = s.gifs && s.plainPrompt && s.empty && hintsMayMove(s.motionMode, s.lowPower);
  return { layer, active: layer && !s.typing && !s.panelOpen };
}

/** The hint shown `step` hint-lengths after a run started. */
export function hintAt(cycle: readonly HintKind[], step: number): HintKind {
  if (cycle.length === 0 || step <= 0) return 'message';
  return cycle[step % cycle.length];
}

// ── the window's hint clock ──────────────────────────────────────────────────
// One clock per window, so every composer in it (MultiChat panes, popouts'
// own windows aside) moves in step. It ticks only while some box is teaching
// and the window is on screen; only the leaves that draw a hint read it.

let clockStep = 0;
let clockId: number | undefined;
let offVisibility: (() => void) | undefined;
const clockListeners = new Set<() => void>();

function syncClock(): void {
  const want = clockListeners.size > 0 && !isWindowHidden();
  if (want && clockId === undefined) {
    clockId = window.setInterval(() => {
      clockStep += 1;
      clockListeners.forEach((l) => l());
    }, HINT_MS);
  } else if (!want && clockId !== undefined) {
    window.clearInterval(clockId);
    clockId = undefined;
  }
}

export function subscribeHintClock(listener: () => void): () => void {
  clockListeners.add(listener);
  if (clockListeners.size === 1) offVisibility = onWindowVisibility(syncClock);
  syncClock();
  return () => {
    clockListeners.delete(listener);
    if (clockListeners.size === 0) {
      offVisibility?.();
      offVisibility = undefined;
    }
    syncClock();
  };
}

export const hintClockStep = (): number => clockStep;
const subscribeNothing = (): (() => void) => () => {};

/** One teaching run: which box and cycle it belongs to, and the clock step it
 *  began at. */
export interface HintRun {
  key: string | null;
  start: number;
}

/** The run after this render: a new key (started, a new chat, a new cycle)
 *  starts again from the prompt at the current step. */
export function nextHintRun(run: HintRun, key: string | null, step: number): HintRun {
  return run.key === key ? run : { key, start: step };
}

/** The hint a run shows at `step`; the prompt when no run is going. The
 *  window's clock may tick a moment after a run starts (another composer
 *  keeps it going), so the run's first tick is skipped: the prompt stays at
 *  least one full hint-length before the first hint, never a flash. */
export function hintOfRun(run: HintRun, cycle: readonly HintKind[], step: number): HintKind {
  return run.key === null ? 'message' : hintAt(cycle, step - run.start - 1);
}

/** Whether the box has been written in since the composer was last left:
 *  anything in it (typed, picked, Tab, history) sets it; only leaving the
 *  composer row clears it (`left`). */
export function nextHintEngaged(prev: boolean, text: string, left = false): boolean {
  if (text !== '') return true;
  return left ? false : prev;
}

/**
 * The hint the empty box shows now: the prompt, then the cycle's next hint
 * every {@link HINT_MS} while `active`. `runKey` names the box's chat: a new
 * chat, a new cycle or a new activation starts again from the prompt.
 */
export function useComposerHint(active: boolean, cycle: readonly HintKind[], runKey: string): HintKind {
  const moving = active && cycle.length > 1;
  const step = useSyncExternalStore(moving ? subscribeHintClock : subscribeNothing, hintClockStep, hintClockStep);
  const key = moving ? `${runKey}|${cycle.join(',')}` : null;
  const [run, setRun] = useState<HintRun>({ key: null, start: 0 });
  const next = nextHintRun(run, key, step);
  // Derived from this render's inputs (React's "adjust state while
  // rendering"), so a new run never shows the old run's hint for a frame.
  if (next !== run) setRun(next);
  return hintOfRun(next, cycle, step);
}

// ── the face ─────────────────────────────────────────────────────────────────

/**
 * The button's face. What the box holds wins (a command being typed shows the
 * slash card, a GIF link the GIF chip), then the hint the empty box is
 * showing, else the chat's emote of the moment.
 */
export function deckFace(o: { text: string; gifLink: boolean; hint: HintKind; slash: boolean; gifs: boolean }): DeckCard {
  if (o.slash && o.text.startsWith('/')) return 'slash';
  if (o.gifs && o.gifLink) return 'gif';
  if (o.text === '') {
    if (o.hint === 'slash' && o.slash) return 'slash';
    if (o.hint === 'gif' && o.gifs) return 'gif';
  }
  return 'moment';
}

/** The cards that fan from behind the face on hover: the deck's other
 *  members, in a fixed order so a card keeps its side. */
export function fanCards(face: DeckCard, o: { slash: boolean; gifs: boolean }): DeckCard[] {
  const deck: DeckCard[] = ['moment'];
  if (o.gifs) deck.push('gif');
  if (o.slash) deck.push('slash');
  return deck.filter((c) => c !== face);
}

// ── facts from Rust ──────────────────────────────────────────────────────────

/** A box that could hold a link. Only such text is sent to Rust to classify,
 *  so plain typing costs no IPC per keystroke. It must let through every
 *  host in Rust's `GIF_HOSTS` (a test reads that list). */
export const LINKISH = /https?:\/\/|www\.|gph\.is|giphy\.|tenor\.|\.gif\b/i;
export function mayHoldGifLink(text: string): boolean {
  return LINKISH.test(text);
}

/** The GIF face stays while the box still holds the link Rust found in it. */
export function gifLinkInBox(found: string | null, text: string): boolean {
  return !!found && text.includes(found);
}

/** Whether the box holds a GIF link (GIPHY, Tenor, a `.gif` file), however it
 *  got there: pasted, dropped, typed or put in by the keyboard's clipboard. */
export function useGifLinkInBox(text: string, enabled: boolean): boolean {
  const [found, setFound] = useState<string | null>(null);
  const probe = enabled && mayHoldGifLink(text) ? text : null;
  useEffect(() => {
    if (probe === null) return;
    let alive = true;
    const id = window.setTimeout(() => {
      void invoke<string | null>('find_gif_link', { text: probe })
        .then((link) => {
          // A text that no longer holds one clears it (`cat.gif` typed on
          // into `cat.gifv` still contains the old link).
          if (alive) setFound(link ?? null);
        })
        .catch(() => {});
    }, 120);
    return () => {
      alive = false;
      window.clearTimeout(id);
    };
  }, [probe]);
  return enabled && gifLinkInBox(found, text);
}

/** Whether this Twitch channel offers GIFs (the menu's GIFs section shows on
 *  the same answer, which Rust caches per channel). False while unknown,
 *  signed out, or off Twitch. */
export function useGifsOffered(isTwitch: boolean, channelId: string | null | undefined): boolean {
  const [answer, setAnswer] = useState<{ id: string; on: boolean } | null>(null);
  useEffect(() => {
    if (!isTwitch || !channelId) return;
    let alive = true;
    getGifPickerStatus(channelId)
      .then((s) => {
        if (alive) setAnswer({ id: channelId, on: s.is_enabled });
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [isTwitch, channelId]);
  return isTwitch && !!channelId && answer?.id === channelId && answer.on;
}

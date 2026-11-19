// The emote button's deck and the teaching placeholder (Brandon's "Deck plus
// placeholder", 2026-11-13): which face the button wears, which cards fan out
// from behind it, when the empty message box may teach, and which hint it
// shows.
//
// Calm by design (Brandon 2026-11-19, "it flips way too much and is just too
// in your face"): nothing here runs on a timer. The hint changes only when
// you open a chat or finish a message, and the face only for what the box
// holds or a new emote of the moment (which Rust holds still for a while).
//
// Why TypeScript: this is presentation, a pick between a few looks from what
// the box shows right now. The facts it reads come from Rust (the chat's
// emote of the moment, `services/emote_moment.rs`; whether the box holds a
// GIF link, `link_detect::find_gif_link`; whether the channel allows GIFs,
// `get_gif_picker_status`); nothing here fetches, parses or ranks.
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getGifPickerStatus } from '../../services/gifService';

/** What the empty box is teaching: the plain prompt, commands, or GIFs. */
export type HintKind = 'message' | 'slash' | 'gif';
/** The deck's members: the chat's emote of the moment (or the smiley), the
 *  slash card and the GIF chip. The one in front is the button's face. */
export type DeckCard = 'moment' | 'slash' | 'gif';

/** Each hint's words, full and for a narrow composer row. */
export const HINT_WORDS: Record<HintKind, { wide: string; narrow: string }> = {
  message: { wide: 'Send a message', narrow: 'Message' },
  slash: { wide: 'Type / for commands', narrow: 'Type /' },
  gif: { wide: 'Search a GIF', narrow: 'Find a GIF' },
};

/** The hints a box may show: the prompt, then what its menu holds. */
export function hintCycle(o: { slash: boolean; gifs: boolean }): HintKind[] {
  const out: HintKind[] = ['message'];
  if (o.slash) out.push('slash');
  if (o.gifs) out.push('gif');
  return out;
}

/** Whether motion lets the hints change: Motion at Full, and neither Battery
 *  Saver nor Performance mode quieting the window (`useLowPower`). */
export function hintsMayMove(motionMode: string | undefined, lowPower: boolean): boolean {
  return (motionMode ?? 'full') === 'full' && !lowPower;
}

/** `layer`: the hint layer stands over the empty box (the box may teach, so
 *  the field's own placeholder is hidden under it). `active`: the box may
 *  show a tip; a layer that is not active rests on the plain prompt. */
export interface HintGate {
  layer: boolean;
  active: boolean;
}

/**
 * Chat's composer. The box teaches only while it shows the idle prompt
 * (`composerLook().teaches`: nothing you can act on is being said), is empty
 * and motion allows it; a tip shows only while you are not writing (no
 * keystroke since the field was last left), no slow-mode wait runs, no reply
 * is being started and no /remind is being typed. Opening the menu keeps the
 * tip: it is still words, and turning it away and back would be two changes.
 */
export function desktopHints(s: {
  teaches: boolean;
  empty: boolean;
  engaged: boolean;
  slowWait: number;
  replying: boolean;
  remindOverlay: boolean;
  motionMode: string | undefined;
  lowPower: boolean;
}): HintGate {
  const layer = s.teaches && s.empty && !s.remindOverlay && hintsMayMove(s.motionMode, s.lowPower);
  return { layer, active: layer && !s.engaged && s.slowWait <= 0 && !s.replying };
}

/**
 * The phone's composer. It teaches GIFs only (its commands sit behind their
 * own button, and typing / brings no command list), so only where GIFs are
 * offered; only while the plain prompt shows and the box is empty; a tip
 * shows while the keyboard is down (a panel opening keeps it, as on desktop).
 */
export function phoneHints(s: {
  gifs: boolean;
  plainPrompt: boolean;
  empty: boolean;
  typing: boolean;
  motionMode: string | undefined;
  lowPower: boolean;
}): HintGate {
  const layer = s.gifs && s.plainPrompt && s.empty && hintsMayMove(s.motionMode, s.lowPower);
  return { layer, active: layer && !s.typing };
}

// ── which hint, and when it changes ──────────────────────────────────────────
// No clock. A chat opens on its own hint and keeps it; finishing a message
// (sent or cleared) moves that chat to the next one. While you write, the box
// rests on the plain prompt (the gate), so the next hint arrives only after
// you have left the composer, never while you look at it.

/** Every hint in one fixed order, so a chat's pick does not move when its
 *  menu learns a moment later that GIFs are offered here. */
const ALL_HINTS: readonly HintKind[] = ['message', 'slash', 'gif'];

/** A steady number for a chat, so each chat opens on its own hint and opens
 *  on the same one every visit. */
export function chatSeed(runKey: string): number {
  let h = 0;
  for (let i = 0; i < runKey.length; i++) h = (h * 31 + runKey.charCodeAt(i)) >>> 0;
  return h;
}

/** The hint a chat's box shows after `finished` messages there: the chat's
 *  own pick, then the next one per message. A pick the box's menu does not
 *  hold is the plain prompt. */
export function hintOfVisit(cycle: readonly HintKind[], runKey: string, finished: number): HintKind {
  if (cycle.length <= 1) return 'message';
  const kind = ALL_HINTS[(chatSeed(runKey) + Math.max(0, finished)) % ALL_HINTS.length];
  return cycle.includes(kind) ? kind : 'message';
}

/** One visit to a chat: whether the box held text at the last look, and how
 *  many messages were finished (the box went from text to empty). */
export interface HintVisit {
  key: string;
  filled: boolean;
  finished: number;
}

/** The visit after this render: a new chat starts a new visit; the box going
 *  from text to empty finishes a message. Typing alone changes nothing. */
export function nextHintVisit(visit: HintVisit, key: string, text: string): HintVisit {
  const filled = text !== '';
  if (visit.key !== key) return { key, filled, finished: 0 };
  if (visit.filled === filled) return visit;
  return { key, filled, finished: filled ? visit.finished : visit.finished + 1 };
}

/** Whether the box has been written in since the composer was last left:
 *  anything in it (typed, picked, Tab, history) sets it; only leaving the
 *  composer row clears it (`left`). */
export function nextHintEngaged(prev: boolean, text: string, left = false): boolean {
  if (text !== '') return true;
  return left ? false : prev;
}

/**
 * The hint the empty box shows: the chat's hint while `active`, else the
 * plain prompt. `runKey` names the box's chat; `text` is the box, read only
 * to notice a message being finished.
 */
export function useComposerHint(active: boolean, cycle: readonly HintKind[], runKey: string, text: string): HintKind {
  const [visit, setVisit] = useState<HintVisit>(() => ({ key: runKey, filled: text !== '', finished: 0 }));
  const next = nextHintVisit(visit, runKey, text);
  // Derived from this render's inputs (React's "adjust state while
  // rendering"); it updates only when the box fills or empties, not per key.
  if (next !== visit) setVisit(next);
  return active ? hintOfVisit(cycle, runKey, next.finished) : 'message';
}

// ── the face ─────────────────────────────────────────────────────────────────

/**
 * The button's face: what the box holds (a command being typed shows the
 * slash card, a GIF link the GIF chip), else the chat's emote of the moment.
 * The hint never turns the face: the cards show on hover instead, so the
 * face changes only for something you did or a new moment in the chat.
 */
export function deckFace(o: { text: string; gifLink: boolean; slash: boolean; gifs: boolean }): DeckCard {
  if (o.slash && o.text.startsWith('/')) return 'slash';
  if (o.gifs && o.gifLink) return 'gif';
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

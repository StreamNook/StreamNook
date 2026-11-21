// The pinned mentions one chat surface draws (SN-284). Rust decides which
// mentions are pinned, where each one lives, when its clock runs and what its
// row looks like (services/mention_pins.rs); this registers the surface (one
// debounced `set_pin_surfaces` per window, utils/pinSurfaces.ts) and asks for
// the answer on `mention-pins-changed`. It never keeps a timer: Rust owns time.
//
// Only the answer to the latest request is drawn. With Pin mentions off it
// listens to nothing and asks nothing.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../stores/AppStore';
import { MENTION_PINS_CHANGED_EVENT } from '../utils/pinPresence';
import { pinSurfaces, type PinRoomRef, type PinSurfaceSpec } from '../utils/pinSurfaces';
import { Logger } from '../utils/logger';
import type { MentionPin, MentionPins } from '../types';

export { MENTION_PINS_CHANGED_EVENT };

const NO_PINS: MentionPin[] = [];

/** Whether two answers draw the same: same pins in order, each at the same row
 *  revision, clock state and run. Keeps the old array so nothing re-renders. */
export function samePins(a: readonly MentionPin[], b: readonly MentionPin[]): boolean {
  return (
    a.length === b.length &&
    a.every(
      (p, i) =>
        p.entry.id === b[i].entry.id &&
        p.rev === b[i].rev &&
        p.run === b[i].run &&
        p.day === b[i].day &&
        p.clock === b[i].clock &&
        p.left_ms === b[i].left_ms &&
        p.entry.removed === b[i].entry.removed &&
        p.channel_mark?.room === b[i].channel_mark?.room,
    )
  );
}

/** The random token one mount registers under. */
function newToken(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

export type PinSurfaceInput = Omit<PinSurfaceSpec, 'visible'>;

/**
 * Registers this mounted stack as a pin surface for as long as it is mounted,
 * and keeps its fields current. Returns `reportVisible`, which the stack calls
 * with the ids of the pins in view (empty whenever it is not shown).
 */
export function usePinSurface(spec: PinSurfaceInput | null): { reportVisible: (ids: string[]) => void } {
  const tokenRef = useRef<string | null>(null);
  if (tokenRef.current === null) tokenRef.current = newToken();
  const token = tokenRef.current;
  const visibleRef = useRef<string[]>([]);
  const present = spec !== null;
  const key = spec ? JSON.stringify(spec) : '';
  const specRef = useRef(spec);
  specRef.current = spec;

  useEffect(() => {
    const s = specRef.current;
    if (!s) return;
    pinSurfaces.register(token, { ...s, visible: visibleRef.current });
    return () => pinSurfaces.unregister(token);
  }, [token, present]);

  useEffect(() => {
    const s = specRef.current;
    if (s) pinSurfaces.update(token, s);
  }, [token, key]);

  const reportVisible = useCallback(
    (ids: string[]) => {
      visibleRef.current = ids;
      pinSurfaces.update(token, { visible: ids });
    },
    [token],
  );
  return { reportVisible };
}

/** One chat room as the shell knows it. */
export function roomRef(provider: string | undefined | null, channel: string | undefined | null): PinRoomRef | null {
  if (!channel) return null;
  return { provider: provider || 'twitch', channel };
}

export interface MentionPinsResult {
  pins: MentionPin[];
  revision: number;
  max: number;
}

const EMPTY: MentionPinsResult = { pins: NO_PINS, revision: -1, max: 0 };

/**
 * The pins for `surfaceId` of this window. Empty while Pin mentions is off, the
 * surface id is null, or Rust has nothing for it. The answer of the latest
 * request wins.
 */
export function useMentionPins(surfaceId: string | null): MentionPinsResult {
  const enabled = useAppStore((s) => s.settings.chat_design?.pin_mentions === true);
  const userId = useAppStore((s) => s.currentUser?.user_id ?? null);
  const [answer, setAnswer] = useState<{ surface: string; user: string | null; res: MentionPinsResult }>({
    surface: '',
    user: null,
    res: EMPTY,
  });
  const seq = useRef(0);

  useEffect(() => {
    if (!enabled || !surfaceId) return;
    let disposed = false;
    const ask = async () => {
      const mine = ++seq.current;
      let res: MentionPins;
      try {
        res = await invoke<MentionPins>('get_mention_pins', { page: String(performance.timeOrigin), surface: surfaceId });
      } catch (err) {
        Logger.warn('[MentionPins] get_mention_pins failed:', err);
        return;
      }
      if (disposed || mine !== seq.current) return;
      setAnswer((prev) =>
        prev.surface === surfaceId && prev.user === userId && prev.res.revision === res.revision && samePins(prev.res.pins, res.pins)
          ? prev
          : {
              surface: surfaceId,
              user: userId,
              res: { pins: res.pins.length ? res.pins : NO_PINS, revision: res.revision, max: res.max },
            },
      );
    };
    void ask();
    let off: (() => void) | undefined;
    void listen(MENTION_PINS_CHANGED_EVENT, () => void ask()).then((fn) => {
      if (disposed) fn();
      else off = fn;
    });
    // The surface registers shortly after it mounts, so ask once more then.
    const first = setTimeout(() => void ask(), 250);
    return () => {
      disposed = true;
      clearTimeout(first);
      off?.();
      // Coming back (or turning pins back on) starts empty rather than flashing
      // what was pinned the last time.
      setAnswer({ surface: '', user: null, res: EMPTY });
    };
  }, [enabled, surfaceId, userId]);

  // An answer for another surface or account, or with the switch off, is never
  // drawn: derived, so a change clears at once.
  return useMemo(
    () => (enabled && surfaceId && answer.surface === surfaceId && answer.user === userId ? answer.res : EMPTY),
    [enabled, surfaceId, userId, answer],
  );
}

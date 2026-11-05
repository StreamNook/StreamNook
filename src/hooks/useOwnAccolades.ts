// Your own accolade wall as Rust builds it (`own_accolades`): the same answer on
// desktop Settings > Profile and the Android You tab. The wall Rust saved on
// disk last time paints first (`own_accolades_cached`, a local read), then the
// fresh one replaces it. Rust counts what every platform counts, saves what you
// earn, and says `own-accolades-changed` after a save or a finished points walk;
// this reads again then. `wall` is null while nothing has answered, and after an
// answer of "no wall" (signed out, or not a member); `answered` is true once the
// fresh answer is in, which tells the two apart.
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Logger } from '../utils/logger';
import type { AccoladeWallView } from '../types/profileView';

/** The device's offset from UTC in minutes, east positive (what Rust's `tz_offset_min` takes). */
export function ownTzOffsetMin(now: Pick<Date, 'getTimezoneOffset'> = new Date()): number {
  return 0 - now.getTimezoneOffset();
}

/** The event's payload: whose wall changed. */
export interface OwnAccoladesChanged {
  user_id?: string;
}

/** Whether a change event is about this account (a payload without an id counts for everyone). */
export function isOwnAccoladesChange(payload: OwnAccoladesChanged | null | undefined, userId: string): boolean {
  const id = payload?.user_id;
  return id === undefined || id === userId;
}

/** Whether the wall saved on disk may paint: only before the fresh answer is in
 *  (it never covers a fresher one), and only a wall (a saved "nothing" says nothing). */
export function mayPaintSaved(freshArrived: boolean, saved: AccoladeWallView | null | undefined): boolean {
  return !freshArrived && !!saved;
}

/** What a failed fresh read leaves: an answer (the slot stops waiting) that keeps
 *  the saved wall already drawn for this account, and nothing of another's. */
export function answerOnFailure(
  prev: { userId: string; wall: AccoladeWallView | null; fresh: boolean } | null,
  userId: string,
): { userId: string; wall: AccoladeWallView | null; fresh: boolean } {
  return { userId, wall: prev?.userId === userId ? prev.wall : null, fresh: true };
}

export function useOwnAccolades(userId: string | null | undefined): {
  wall: AccoladeWallView | null;
  answered: boolean;
} {
  const [state, setState] = useState<{ userId: string; wall: AccoladeWallView | null; fresh: boolean } | null>(null);

  useEffect(() => {
    if (!userId) return;
    let cancelled = false;
    // Only the newest read may land, so a slow older answer never covers a newer one.
    let latest = 0;
    let freshArrived = false;
    // The saved wall first: a local read, so the first frame has something to draw.
    invoke<AccoladeWallView | null>('own_accolades_cached')
      .then((saved) => {
        if (!cancelled && mayPaintSaved(freshArrived, saved)) setState({ userId, wall: saved ?? null, fresh: false });
      })
      .catch((err) => Logger.debug('[OwnAccolades] no saved wall:', err));
    const read = () => {
      const mine = ++latest;
      invoke<AccoladeWallView | null>('own_accolades', { tzOffsetMin: ownTzOffsetMin() })
        .then((wall) => {
          if (cancelled || mine !== latest) return;
          freshArrived = true;
          setState({ userId, wall: wall ?? null, fresh: true });
        })
        .catch((err) => {
          Logger.debug('[OwnAccolades] wall unavailable:', err);
          // A failed read is an answer too: the slot stops waiting and keeps
          // whatever saved wall it already drew.
          if (cancelled || mine !== latest) return;
          freshArrived = true;
          setState((prev) => answerOnFailure(prev, userId));
        });
    };
    read();
    const unlisten = listen<OwnAccoladesChanged>('own-accolades-changed', (event) => {
      if (isOwnAccoladesChange(event.payload, userId)) read();
    });
    return () => {
      cancelled = true;
      void unlisten.then((fn) => fn());
    };
  }, [userId]);

  // An answer is only ever used for the account it was read for.
  const mine = userId && state?.userId === userId ? state : null;
  return { wall: mine?.wall ?? null, answered: mine?.fresh === true };
}

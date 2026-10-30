// The Discord Status editor's draft and its saves.
//
// The draft is the saved style with the fields being edited on top. A field
// stays "pending" until the saved settings show its value, never merely until
// a save answered, so a save never flashes the old value back and a keystroke
// made between the save and the re-read is kept. Saves go through one queue
// that outlives the editor: one write at a time, later edits merged after it,
// a failed write retried, also after the editor closed. Every write is
// numbered, and Rust drops one numbered at or below a write it already
// applied, so a flush on unload may overtake the write in flight safely.

import type { DiscordPresenceSettings } from '../../../types';
import { fieldsIn, setSettingsFields, type WriteOrder } from '../../../utils/settingsFields';
import { SENDER_ID } from '../../../utils/settingsBroadcast';

/** The style fields the editor writes (never `extra`). */
export type PresenceField = keyof DiscordPresenceSettings;
export type Pending = Partial<DiscordPresenceSettings>;

export function draftOf(canonical: DiscordPresenceSettings, pending: Pending): DiscordPresenceSettings {
  return { ...canonical, ...pending };
}

/** The pending fields the saved settings do not show yet. */
export function reconcile(canonical: DiscordPresenceSettings, pending: Pending): Pending {
  const next: Pending = {};
  for (const key of Object.keys(pending) as PresenceField[]) {
    if (canonical[key] !== pending[key]) {
      (next as Record<string, unknown>)[key] = pending[key];
    }
  }
  return next;
}

export type CommitStatus = 'idle' | 'saving' | 'retrying';

export interface Committer {
  /** Add fields to the next write (newer values win). */
  queue(fields: Pending): void;
  /** Write now: skips a retry wait, and with a write in flight sends what it
   *  carries plus everything queued at once, numbered after it. */
  flush(): void;
  status(): CommitStatus;
  subscribe(listener: () => void): () => void;
}

export interface CommitterDeps {
  write: (changes: Record<string, unknown>, order: WriteOrder) => Promise<void>;
  later: (fn: () => void, ms: number) => () => void;
  /** Names this page's stream of writes. */
  stream: string;
  /** The number before the first write. The app's committer starts at the
   *  clock, so a reloaded module never counts below what Rust already took. */
  first?: number;
}

export function createCommitter(deps: CommitterDeps): Committer {
  let outgoing: Pending = {};
  let flying: Pending | null = null;
  // An urgent (flush) write not answered yet: every later write carries its
  // fields too, because Rust drops an older numbered write whole.
  let urgentFlying: Pending | null = null;
  let n = deps.first ?? 0;
  // The newest write that succeeded: a failed older write is not queued again
  // once a newer one (which carried its fields) has landed.
  let lastOk = n;
  let cancelRetry: (() => void) | null = null;
  let current: CommitStatus = 'idle';
  const listeners = new Set<() => void>();
  const setStatus = (s: CommitStatus) => {
    if (s === current) return;
    current = s;
    listeners.forEach((l) => l());
  };
  const order = (): WriteOrder => {
    n += 1;
    return { stream: deps.stream, n };
  };

  const scheduleRetry = () => {
    setStatus('retrying');
    cancelRetry = deps.later(() => {
      cancelRetry = null;
      void run();
    }, 2000);
  };

  const run = async (): Promise<void> => {
    if (flying || cancelRetry || Object.keys(outgoing).length === 0) return;
    const batch: Pending = { ...(urgentFlying ?? {}), ...outgoing };
    outgoing = {};
    flying = batch;
    setStatus('saving');
    let failed = false;
    const numbered = order();
    try {
      await deps.write(fieldsIn('discord_presence', batch as Record<string, unknown>), numbered);
      lastOk = Math.max(lastOk, numbered.n);
    } catch {
      // Newer edits queued meanwhile win over the failed batch; a newer write
      // that already landed carried it, so then it is not sent again.
      if (numbered.n > lastOk) {
        outgoing = { ...batch, ...outgoing };
        failed = true;
      }
    } finally {
      flying = null;
    }
    if (failed) {
      scheduleRetry();
      return;
    }
    if (Object.keys(outgoing).length > 0) {
      void run();
    } else {
      setStatus('idle');
    }
  };

  return {
    queue(fields) {
      if (Object.keys(fields).length === 0) return;
      outgoing = { ...outgoing, ...fields };
      void run();
    },
    flush() {
      if (cancelRetry) {
        cancelRetry();
        cancelRetry = null;
      }
      if (!flying) {
        void run();
        return;
      }
      // A page going away cannot wait for the write in flight: send its fields
      // and the queued ones now, numbered after it, so it cannot land last.
      if (Object.keys(outgoing).length === 0) return;
      const urgent: Pending = { ...(urgentFlying ?? {}), ...flying, ...outgoing };
      outgoing = {};
      urgentFlying = urgent;
      const numbered = order();
      deps
        .write(fieldsIn('discord_presence', urgent as Record<string, unknown>), numbered)
        .then(() => {
          lastOk = Math.max(lastOk, numbered.n);
          if (urgentFlying === urgent) urgentFlying = null;
        })
        .catch(() => {
          if (urgentFlying === urgent) urgentFlying = null;
          if (numbered.n <= lastOk) return;
          outgoing = { ...urgent, ...outgoing };
          // Nothing else may be running by now (the editor is closed): retry.
          if (!flying && !cancelRetry) scheduleRetry();
        });
    },
    status: () => current,
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

/** The app's one committer for the Discord style. */
export const presenceCommitter: Committer = createCommitter({
  write: (changes, order) => setSettingsFields(changes, order),
  later: (fn, ms) => {
    const id = setTimeout(fn, ms);
    return () => clearTimeout(id);
  },
  stream: `discord_presence:${SENDER_ID}`,
  first: Date.now(),
});

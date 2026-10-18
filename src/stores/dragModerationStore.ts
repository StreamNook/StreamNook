import { create } from 'zustand';

// One chatter "picked up" by the drag-to-moderate gesture. All mod actions key
// off userId (id-based), so login/displayName are only for display + whisper.
export interface DragModUser {
  userId: string;
  login: string;
  displayName: string;
  color?: string;
  /** The message the user was lifted from (enables the Delete bucket). */
  messageId?: string;
  broadcasterId: string;
  /** Source platform. Twitch (default) uses the Helix mod commands; 'kick' routes
   *  ban/timeout/unban to the Kick API (and delete/pin buckets don't apply);
   *  'youtube' routes to the YouTube webview-session mod commands. */
  provider?: string;
  /** The source identifier (the slug after the provider prefix), which the YouTube
   *  mod commands need to resolve the channel. Unused by Twitch/Kick. */
  channel?: string;
  /** Gates the destructive buckets (delete/timeout/ban) to moderators. */
  isModerator: boolean;
  /** False for a chatter a ban or timeout cannot land on (yourself, the
   *  channel owner, another mod unless it is your channel): only Delete and
   *  Pin are offered then. */
  canPunish?: boolean;
  /** Current moderation state of the user (from the lifted message): drives the
   *  inverse Unban / Untimeout action instead of Ban / Timeout. */
  moderationState?: 'timeout' | 'ban' | null;
}

interface DragModerationState {
  dragged: DragModUser | null;
  /** Pointer position where the lift started, so the chip spawns under the cursor. */
  origin: { x: number; y: number };
  startDrag: (user: DragModUser, x: number, y: number) => void;
  endDrag: () => void;
}

export const useDragModerationStore = create<DragModerationState>((set) => ({
  dragged: null,
  origin: { x: 0, y: 0 },
  startDrag: (user, x, y) => set({ dragged: user, origin: { x, y } }),
  endDrag: () => set({ dragged: null }),
}));

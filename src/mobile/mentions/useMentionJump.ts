// The last step of opening a mention: find its message in the chat that just
// opened, the same three steps on every app.
//
// 1. The message is in the loaded buffer: pause, glide to it, flash it.
// 2. It is not there once the room's join history has had time to land: one
//    history backfill, where the platform has one (Twitch), then look again.
// 3. Still not there: the entry is pinned over the chat as the "Older than
//    chat history" card (WatchScreen draws it), with Dismiss.
import { useEffect, useRef, useState } from 'react';
import { ensureChannelHistory } from '../../stores/chatConnectionStore';
import type { BackendChatMessage } from '../../services/twitchChat';
import { scrollChatToMessage } from '../../utils/scrollChatToMessage';
import type { ProviderId } from '../../types/providers';
import { useMentionsStore } from './mentionInbox';

/** The join history arrives on a ~1.5 s hold plus its fetch; give it this long
 *  after the room is on screen before deciding the message is not loaded. */
const SETTLE_MS = 3000;
/** After the backfill answers, rows still have to render. */
const AFTER_BACKFILL_MS = 1500;
/** A chat that never opened (a stream that failed to start) drops the jump. */
const GIVE_UP_MS = 25_000;

type Phase = {
  id: string;
  started: number;
  roomSeenAt: number | null;
  backfill: 'none' | 'running' | 'done';
  backfillDoneAt: number;
  landed: boolean;
};

export function useMentionJump({
  activeChannel,
  messages,
  getMessageId,
  pause,
  login,
  provider,
  channelId,
}: {
  activeChannel: string | null;
  messages: readonly (string | BackendChatMessage)[];
  getMessageId: (m: string | BackendChatMessage) => string | null;
  pause: (paused: boolean, opts?: { force?: boolean }) => void;
  login: string | null;
  provider: ProviderId;
  channelId: string | null;
}): void {
  const pending = useMentionsStore((s) => s.pendingJump);
  const phase = useRef<Phase | null>(null);
  // Re-runs the check on a timer while waiting; messages arriving re-run it too.
  const [tick, setTick] = useState(0);

  useEffect(() => {
    if (!pending) return;
    const now = Date.now();
    if (phase.current?.id !== pending.id) {
      phase.current = { id: pending.id, started: now, roomSeenAt: null, backfill: 'none', backfillDoneAt: 0, landed: false };
    }
    const p = phase.current;
    if (p.landed) return;
    const later = (ms: number) => {
      const t = window.setTimeout(() => setTick((n) => n + 1), ms);
      return () => window.clearTimeout(t);
    };
    if (now - p.started > GIVE_UP_MS) {
      useMentionsStore.setState({ pendingJump: null });
      return;
    }
    if (activeChannel !== pending.room) return later(500);
    p.roomSeenAt ??= now;

    if (messages.some((m) => getMessageId(m) === pending.id)) {
      p.landed = true;
      // A frame for the row to render, then pause first: an unpaused list
      // re-pins to the bottom on the next message and undoes the glide.
      let tries = 0;
      const land = () => {
        pause(true, { force: true });
        if (scrollChatToMessage(pending.id, { align: 'center', flash: true }) || ++tries >= 6) {
          useMentionsStore.setState({ pendingJump: null });
          return;
        }
        window.setTimeout(land, 150);
      };
      requestAnimationFrame(land);
      return;
    }

    if (p.backfill === 'none') {
      if (now - p.roomSeenAt < SETTLE_MS) return later(SETTLE_MS - (now - p.roomSeenAt));
      const id = channelId ?? pending.channel_id ?? null;
      if (provider === 'twitch' && login && id) {
        p.backfill = 'running';
        void ensureChannelHistory(login, id).finally(() => {
          p.backfill = 'done';
          p.backfillDoneAt = Date.now();
          setTick((n) => n + 1);
        });
        return;
      }
      p.backfill = 'done';
      p.backfillDoneAt = now - AFTER_BACKFILL_MS;
    }
    if (p.backfill === 'running') return;
    if (now - p.backfillDoneAt < AFTER_BACKFILL_MS) return later(AFTER_BACKFILL_MS - (now - p.backfillDoneAt));
    p.landed = true;
    useMentionsStore.setState({ pendingJump: null, contextCard: pending });
  }, [pending, activeChannel, messages, getMessageId, pause, login, provider, channelId, tick]);
}

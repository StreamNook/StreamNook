// Notices a chat that has gone silent the way a dead upstream goes silent, and
// rebuilds it.
//
// Resuming the app covers the common case, but a connection can also die while
// the phone is awake and in the foreground, typically on a Wi-Fi to mobile
// handoff. Nothing would ever recover that on its own.
//
// The signal already exists and costs nothing to read. Rust sends a bare
// heartbeat every thirty seconds and that stops the instant its IRC task dies,
// so the connection store marks a room stale once no frames at all have arrived
// for two minutes. That message is what this watches for.
//
// Two things about it are worth knowing. It flaps, because the store resets its
// own timer a minute after raising it, so the same dead connection raises and
// clears the message repeatedly; the arming flag below means one quiet spell
// produces one rebuild attempt. And it cannot fire early, because a channel
// with no chatters still receives heartbeats, so silence really does mean the
// connection rather than the room.
import { useEffect, useRef } from 'react';
import { useChannelChat } from '../../stores/chatConnectionStore';
import { hardCycleChat } from './chatRecovery';

// Matched as a prefix because the store appends a live seconds count. Stops
// short of the dash so the match does not depend on punctuation.
const STALE_PREFIX = 'Connection may be stale';

/** Watches one room, normally the active tab. All rooms share a connection, so
 *  watching more than one would only produce duplicate triggers. */
export function useChatWatchdog(channel: string | null | undefined): void {
  const { error } = useChannelChat(channel);
  // Which channel we have already responded to in the current quiet spell.
  const handledFor = useRef<string | null>(null);

  useEffect(() => {
    if (!channel) return;
    const stale = typeof error === 'string' && error.startsWith(STALE_PREFIX);

    if (!stale) {
      // Frames are flowing again, either because the rebuild worked or because
      // the store recovered on its own. Re-arm for the next spell.
      if (handledFor.current === channel) handledFor.current = null;
      return;
    }

    if (handledFor.current === channel) return;
    handledFor.current = channel;
    void hardCycleChat('no frames for two minutes');
  }, [channel, error]);
}

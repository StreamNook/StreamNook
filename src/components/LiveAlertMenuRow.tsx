// "Live alerts" in a channel's right-click menu (cards and MultiNook tiles).
//
// Near the screen's bottom both menus flip up and pin their BOTTOM edge, so a
// row that arrives or leaves late moves every row above it under the pointer.
// So whether the row is there is decided at once, from the channel alone (any
// channel with an identity: like the phones' row, a mute on a channel you do
// not follow is harmless and holds if you follow later), and only its value
// waits for Rust, in a slot that is already there (a spinner, disabled). The menu
// stays open on a press so the value flips in place, as the confirmation.

import { Bell, BellOff, Loader2 } from 'lucide-react';
import { favoriteIdOf } from '../utils/favorites';
import { streamProvider } from '../utils/streamProvider';
import { useLiveAlert } from '../hooks/useLiveAlert';
import type { TwitchStream } from '../types';

interface LiveAlertMenuRowProps {
  stream: TwitchStream;
  className: string;
}

/** Whether a channel can be muted at all, decided with no wait: a Twitch
 *  login, or the favourites identity (`favoriteIdOf`, the twin of Rust's
 *  `favorite_id`, which is what `live_mute::canonical` uses elsewhere). */
function hasAlertIdentity(stream: TwitchStream): boolean {
  return streamProvider(stream) === 'twitch' ? !!stream.user_login : !!favoriteIdOf(stream);
}

export function LiveAlertMenuRow({ stream, className }: LiveAlertMenuRowProps) {
  if (!hasAlertIdentity(stream)) return null;
  return <LiveAlertMenuRowButton stream={stream} className={className} />;
}

function LiveAlertMenuRowButton({ stream, className }: LiveAlertMenuRowProps) {
  const { on, busy, setOn } = useLiveAlert({
    provider: streamProvider(stream),
    userId: stream.user_id,
    userLogin: stream.user_login,
  });
  const known = on !== null;
  return (
    <button
      type="button"
      onClick={(e) => {
        e.stopPropagation();
        if (known) setOn(!on);
      }}
      disabled={busy || !known}
      aria-pressed={known ? on : undefined}
      className={className}
    >
      {on === false ? <BellOff size={16} /> : <Bell size={16} />}
      <span className="flex-1 text-left">Live alerts</span>
      {/* One fixed slot: the menu's own loading glyph until Rust answers, then
          the value, so the row never changes width. */}
      <span className="inline-flex w-6 justify-end text-xs text-textMuted">
        {known ? (on ? 'On' : 'Off') : <Loader2 size={12} className="animate-spin" aria-label="Loading" />}
      </span>
    </button>
  );
}

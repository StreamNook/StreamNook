// A go-live row's "Turn off alerts" in the notification centre.
//
// Mounted quiet: until the pointer (or keyboard focus) reaches it, it asks
// Rust nothing, so a long list of rows costs no calls. On reach it reads the
// channel's live state through `useLiveAlert` (one read per hover), so it is
// never stale after a change made elsewhere. It is one button the whole time,
// so waking never drops keyboard focus or the pointer's tooltip. A press sets
// every platform's channel the row stands for to the opposite of what it
// shows, through the one ordered door.

import { useState } from 'react';
import { Bell, BellOff } from 'lucide-react';
import { Tooltip } from './ui/Tooltip';
import { useLiveAlert } from '../hooks/useLiveAlert';
import { setLiveAlert } from '../utils/liveAlerts';
import { Logger } from '../utils/logger';
import type { LiveNotificationData } from '../types';

const BUTTON = 'text-white/30 hover:text-white transition-all p-0.5';

export function LiveAlertIslandAction({ data }: { data: LiveNotificationData }) {
  const [awake, setAwake] = useState(false);
  const channels = data.channels ?? [];
  const [first, ...rest] = channels;
  const { on, busy, setOn } = useLiveAlert(
    { provider: first?.provider ?? '', userId: first?.user_id ?? '', userLogin: first?.user_login ?? '' },
    awake,
  );
  if (data.is_test || !first) return null;
  const name = data.streamer_name;
  const known = awake && on !== null;
  const label = !known ? `Live alerts for ${name}` : on ? `Turn off alerts for ${name}` : `Turn on alerts for ${name}`;
  const wake = () => setAwake(true);
  // aria-disabled, never disabled: a disabled button would drop the focus
  // that just woke it while its state is still being read.
  const unavailable = awake && (busy || !known);
  return (
    <Tooltip content={label} side="top">
      <button
        type="button"
        aria-label={label}
        aria-pressed={known ? on : undefined}
        aria-disabled={unavailable || undefined}
        onMouseEnter={wake}
        onFocus={wake}
        onClick={(e) => {
          e.stopPropagation();
          if (!awake) {
            wake();
            return;
          }
          if (!known || busy) return;
          const next = !on;
          setOn(next);
          for (const c of rest) {
            setLiveAlert({ provider: c.provider, userId: c.user_id, userLogin: c.user_login }, next).catch((err) =>
              Logger.warn('[LiveAlertIslandAction] change failed:', err),
            );
          }
        }}
        className={BUTTON}
        data-keeps-player-fullscreen
      >
        {known && on === false ? <BellOff size={14} /> : <Bell size={14} />}
      </button>
    </Tooltip>
  );
}

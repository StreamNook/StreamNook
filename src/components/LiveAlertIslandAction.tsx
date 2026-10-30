// A go-live row's "Turn off alerts" in the notification centre.
//
// Mounted quiet: until the pointer (or keyboard focus) reaches it, it is a
// plain bell that asks Rust nothing, so a long list of rows costs no calls.
// On reach it reads the channel's live state through `useLiveAlert` (one read
// per hover), so it is never stale after a change made elsewhere. A press
// sets every platform's channel the row stands for to the opposite of what it
// shows, through the one ordered door.

import { useState } from 'react';
import { Bell, BellOff } from 'lucide-react';
import { Tooltip } from './ui/Tooltip';
import { useLiveAlert } from '../hooks/useLiveAlert';
import { setLiveAlert } from '../utils/liveAlerts';
import { Logger } from '../utils/logger';
import type { AlertChannel, LiveNotificationData } from '../types';

const BUTTON = 'text-white/30 hover:text-white transition-all p-0.5';

export function LiveAlertIslandAction({ data }: { data: LiveNotificationData }) {
  const [awake, setAwake] = useState(false);
  const channels = data.channels ?? [];
  if (data.is_test || channels.length === 0) return null;
  const name = data.streamer_name;
  if (!awake) {
    return (
      <button
        type="button"
        aria-label={`Live alerts for ${name}`}
        onMouseEnter={() => setAwake(true)}
        onFocus={() => setAwake(true)}
        onClick={(e) => {
          e.stopPropagation();
          setAwake(true);
        }}
        className={BUTTON}
        data-keeps-player-fullscreen
      >
        <Bell size={14} />
      </button>
    );
  }
  return <LiveAlertIslandActionLive name={name} channels={channels} />;
}

function LiveAlertIslandActionLive({ name, channels }: { name: string; channels: AlertChannel[] }) {
  const [first, ...rest] = channels;
  const { on, busy, setOn } = useLiveAlert({ provider: first.provider, userId: first.user_id, userLogin: first.user_login });
  const known = on !== null;
  const label = !known ? `Live alerts for ${name}` : on ? `Turn off alerts for ${name}` : `Turn on alerts for ${name}`;
  return (
    <Tooltip content={label} side="top">
      <button
        type="button"
        aria-label={label}
        aria-pressed={known ? on : undefined}
        disabled={busy || !known}
        onClick={(e) => {
          e.stopPropagation();
          if (!known) return;
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
        {on === false ? <BellOff size={14} /> : <Bell size={14} />}
      </button>
    </Tooltip>
  );
}

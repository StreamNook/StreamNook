import { useEffect, useState } from 'react';
import type { HypeTrainData } from '../../types';
import type { ProviderId } from '../../types/providers';
import { recordHypeTrainActivity, watchHypeTrains } from '../../services/hypeTrainWatch';

// Blended-mode hype trains: the equivalent of MultiChatPane's per-pane banner,
// for ALL the blended Twitch channels at once (blended mounts no per-channel
// panes). Rust polls each channel once for every surface showing it; this
// collects the active trains keyed by channel login for the banner and puts
// starts and level-ups into the activity feed.

interface HypeSource {
  channel: string;
  provider?: ProviderId;
  channelName: string;
}

export function useBlendedHypeTrains(channels: HypeSource[]): Map<string, HypeTrainData> {
  const [trains, setTrains] = useState<Map<string, HypeTrainData>>(new Map());
  // Re-run only when the Twitch channel SET changes, not on every render.
  const sig = channels
    .filter((c) => (c.provider ?? 'twitch') === 'twitch')
    .map((c) => `${c.channel.toLowerCase()}|${c.channelName}`)
    .join(',');

  useEffect(() => {
    const twitch = channels.filter((c) => (c.provider ?? 'twitch') === 'twitch');
    const names = new Map(twitch.map((c) => [c.channel.toLowerCase(), c.channelName || c.channel]));
    const stop = watchHypeTrains(
      twitch.map((c) => ({ login: c.channel.toLowerCase(), name: c.channelName })),
      (login, train, levelChanged) => {
        setTrains((prev) => {
          const next = new Map(prev);
          if (train) next.set(login, train);
          else next.delete(login);
          return next;
        });
        if (train && levelChanged) recordHypeTrainActivity(login, names.get(login) ?? login, train);
      },
    );
    return () => {
      stop();
      setTrains(new Map());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sig]);

  return trains;
}

// The pinned chat in the main window: the chat that stays put while the viewer
// moves between streams. It is an ordinary ChatWidget pointed at the pinned
// channel through the same `channelOverride` seam MultiChat panes and provider
// streams use, so sending, emotes, mod tools and the viewer's identity all
// work as they do anywhere else.
//
// Its header numbers (viewers, uptime, title, category) come from Rust's
// channel state, which ChatWidget already watches for a Twitch channel; this
// pane only reads them. The pinned channel is not the one being watched, so,
// as in a MultiChat pane, EventSub-driven extras stay with the watched stream;
// the hype train follows the pinned channel through the shared Rust watch.
import { useEffect, useMemo, useState } from 'react';
import ChatWidget, { type ChatWidgetChannelOverride } from '../ChatWidget';
import type { HypeTrainData } from '../../types';
import type { ChatPin } from '../../stores/chatPinStore';
import { useChannelState } from '../../stores/channelStateStore';
import { watchHypeTrains } from '../../services/hypeTrainWatch';

export default function ChatPinPane({ pin }: { pin: ChatPin }) {
  const isTwitch = pin.provider === 'twitch';
  const state = useChannelState(isTwitch ? pin.login : null);
  // Unknown until Rust's first answer; treated as live so the header does not
  // flash "offline" on a channel that is on air.
  const answered = state?.viewers_at != null;
  const live = answered ? !!state?.started_at : true;

  const [hypeTrain, setHypeTrain] = useState<HypeTrainData | null>(null);
  const trainsOn = isTwitch && !!pin.channel_id && answered && live;
  if (!trainsOn && hypeTrain !== null) setHypeTrain(null);
  useEffect(() => {
    if (!trainsOn) return;
    return watchHypeTrains(
      [{ login: pin.login, channelId: pin.channel_id, name: pin.display_name || pin.login }],
      (_login, train) => setHypeTrain(train),
    );
  }, [trainsOn, pin.login, pin.channel_id, pin.display_name]);

  const channelOverride = useMemo<ChatWidgetChannelOverride>(
    () => ({
      provider: pin.provider,
      context: 'main',
      user_login: pin.login,
      user_id: pin.channel_id,
      user_name: pin.display_name || pin.login,
      profile_image_url: pin.avatar_url ?? undefined,
      title: state?.title ?? undefined,
      game_name: state?.game_name ?? undefined,
      viewer_count: state?.viewer_count ?? undefined,
      started_at: state?.started_at ?? undefined,
      is_live: live,
      is_active: true,
    }),
    [pin, state?.title, state?.game_name, state?.viewer_count, state?.started_at, live],
  );

  return <ChatWidget channelOverride={channelOverride} hypeTrainOverride={isTwitch ? hypeTrain : undefined} />;
}

// A docked chat on screen in the main window: one of the chats kept open
// beside the stream being watched. It is an ordinary ChatWidget pointed at the
// docked channel through the same `channelOverride` seam MultiChat panes and
// provider streams use, so sending, emotes, mod tools and the viewer's
// identity all work as they do anywhere else.
//
// Its header numbers (viewers, uptime, title, category) come from Rust's
// channel state, which ChatWidget already watches for a Twitch channel; this
// pane only reads them. The docked channel is not the one being watched, so,
// as in a MultiChat pane, EventSub-driven extras stay with the watched stream;
// the hype train follows the docked channel through the shared Rust watch.
import { useEffect, useMemo, useState } from 'react';
import ChatWidget, { type ChatWidgetChannelOverride } from '../ChatWidget';
import type { HypeTrainData } from '../../types';
import type { DockedChat } from '../../stores/chatDockStore';
import { useChannelState } from '../../stores/channelStateStore';
import { watchHypeTrains } from '../../services/hypeTrainWatch';

export default function ChatDockPane({ chat }: { chat: DockedChat }) {
  const isTwitch = chat.provider === 'twitch';
  const state = useChannelState(isTwitch ? chat.login : null);
  // Unknown until Rust's first answer; treated as live so the header does not
  // flash "offline" on a channel that is on air.
  const answered = state?.viewers_at != null;
  const live = answered ? !!state?.started_at : true;

  const [hypeTrain, setHypeTrain] = useState<HypeTrainData | null>(null);
  const trainsOn = isTwitch && !!chat.channel_id && answered && live;
  if (!trainsOn && hypeTrain !== null) setHypeTrain(null);
  useEffect(() => {
    if (!trainsOn) return;
    return watchHypeTrains(
      [{ login: chat.login, channelId: chat.channel_id, name: chat.display_name || chat.login }],
      (_login, train) => setHypeTrain(train),
    );
  }, [trainsOn, chat.login, chat.channel_id, chat.display_name]);

  const channelOverride = useMemo<ChatWidgetChannelOverride>(
    () => ({
      provider: chat.provider,
      context: 'main',
      user_login: chat.login,
      user_id: chat.channel_id,
      user_name: chat.display_name || chat.login,
      profile_image_url: chat.avatar_url ?? undefined,
      title: state?.title ?? undefined,
      game_name: state?.game_name ?? undefined,
      viewer_count: state?.viewer_count ?? undefined,
      started_at: state?.started_at ?? undefined,
      is_live: live,
      is_active: true,
    }),
    [chat, state?.title, state?.game_name, state?.viewer_count, state?.started_at, live],
  );

  return <ChatWidget channelOverride={channelOverride} hypeTrainOverride={isTwitch ? hypeTrain : undefined} />;
}
